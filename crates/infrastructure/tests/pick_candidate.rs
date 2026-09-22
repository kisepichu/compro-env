//! Integration tests for `ce internal pick-candidate` (spec §15, plan 063).
//!
//! Each test mounts a temporary repo containing:
//!
//! * `config.toml` — a minimal `[library.languages.rust]` with the
//!   librarychecker mapping so `LibraryDiscovery::discover` resolves the OJ
//!   language.
//! * `libraries/rust/algebra/monoid.rs` — a public library the solution
//!   verifies.
//! * `solutions/librarychecker-aplusb/aplusb/main/ce.toml` — `publish = true`
//!   with a `[verify]` block referencing the library above.
//! * A parallel `state/` directory that stands in for the in-flight
//!   `automation/verify` overlay (`state/verification/results/**`).
//! * Optionally `verification/results/**` at the repo root, standing in for
//!   the records already merged into `main`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use domain::verification::{
    AttemptId, ErrorKind, FailureStage, InfrastructureFailure, VerificationRecord,
    VerificationState, VerifyFingerprint,
};

use infrastructure::verify_pick_candidate::pick_candidate_with_io;

const SOLUTION_ID: &str = "librarychecker-aplusb/aplusb/main";

fn now() -> DateTime<FixedOffset> {
    Utc.with_ymd_and_hms(2026, 8, 14, 12, 0, 0)
        .unwrap()
        .fixed_offset()
}

fn write_repo(root: &Path) {
    std::fs::create_dir_all(root.join("libraries/rust/algebra")).unwrap();
    std::fs::write(
        root.join("libraries/rust/algebra/monoid.rs"),
        b"pub trait Monoid {}\n",
    )
    .unwrap();

    std::fs::write(
        root.join("config.toml"),
        r#"
[library.site]
title = "compro-env"
description = "Competitive programming libraries and solutions"
language = "en"
repository_url = "https://github.com/owner/compro-env"

[library.languages.rust]
display_name = "Rust"
root = "libraries/rust"
include = ["**/*.rs"]
exclude = []
check_command = "cargo test"
check_timeout_seconds = 600
syntax_highlight = "rust"
expected_toolchains = [
  { name = "rustc", version = "1.92.0" },
]

[library.languages.rust.analyzer]
command = ["./target/library-analyzers/bin/rust-analyzer"]
timeout_seconds = 600

[library.languages.rust.online_judges.librarychecker]
language_id = "rust"
"#,
    )
    .unwrap();

    let sol_root = root.join(format!("solutions/{SOLUTION_ID}"));
    std::fs::create_dir_all(sol_root.join("src")).unwrap();
    std::fs::write(sol_root.join("src/main.rs"), b"fn main() {}\n").unwrap();
    std::fs::write(
        sol_root.join("ce.toml"),
        r#"
language = "rust"
test_command = "./test.sh"
publish = true
solved_at = "2026-08-02T14:30:00+09:00"
test_timeout_seconds = 600

[verify]
libraries = ["libraries/rust/algebra/monoid.rs"]
language_id = "rust"
"#,
    )
    .unwrap();
}

fn fingerprint() -> VerifyFingerprint {
    VerifyFingerprint::parse(
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap()
}

fn future_deadline() -> DateTime<FixedOffset> {
    Utc.with_ymd_and_hms(2026, 8, 14, 15, 0, 0)
        .unwrap()
        .fixed_offset()
}

fn elapsed_deadline() -> DateTime<FixedOffset> {
    Utc.with_ymd_and_hms(2026, 8, 14, 10, 0, 0)
        .unwrap()
        .fixed_offset()
}

/// Write a retryable `InfrastructureFailure` record under
/// `<base>/verification/results/`. `base` is the repo root for merged
/// records and the overlay checkout for in-flight ones.
fn write_retry_record(base: &Path, solution_id: &str, deadline: DateTime<FixedOffset>) {
    let record = VerificationRecord {
        schema_version: 1,
        solution_id: domain::library::SolutionId::parse(solution_id).unwrap(),
        attempt_id: AttemptId::parse("attempt-1").unwrap(),
        replaces_attempt_id: None,
        fingerprint: fingerprint(),
        state: VerificationState::InfrastructureFailure(InfrastructureFailure {
            stage: FailureStage::Poll,
            error_kind: ErrorKind::Network,
            retryable: true,
            retry_count: 1,
            next_retry_at: Some(deadline),
            updated_at: now(),
            summary: "transient network failure".into(),
            plan_hash: None,
            handle: None,
        }),
        plan_context: None,
    };
    let results_dir = base.join("verification/results");
    std::fs::create_dir_all(&results_dir).unwrap();
    let path = results_dir.join(format!(
        "{}.json",
        record.solution_id.as_str().replace('/', "__")
    ));
    let bytes = serde_json::to_vec_pretty(&record).unwrap();
    std::fs::write(&path, bytes).unwrap();
}

/// Write a terminal `Completed{Accepted}` record under
/// `<base>/verification/results/`, reusing the schema fixture so the test
/// does not hand-roll a `CompletedState`. The stored fingerprint is the
/// fixture's placeholder, i.e. guaranteed to differ from the recomputed one.
fn write_completed_record(base: &Path, solution_id: &str) {
    const ACCEPTED: &str = include_str!("fixtures/verification/accepted.json");
    let mut record: VerificationRecord = serde_json::from_str(ACCEPTED).unwrap();
    record.solution_id = domain::library::SolutionId::parse(solution_id).unwrap();
    let results_dir = base.join("verification/results");
    std::fs::create_dir_all(&results_dir).unwrap();
    let path = results_dir.join(format!("{}.json", solution_id.replace('/', "__")));
    std::fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
}

fn make_state_dir(root: &Path) -> PathBuf {
    let state = root.join("state");
    std::fs::create_dir_all(&state).unwrap();
    state
}

#[test]
fn empty_state_overlay_returns_the_only_published_solution() {
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let state = make_state_dir(tmp.path());

    let picked = pick_candidate_with_io(tmp.path(), &state, now()).unwrap();
    assert_eq!(picked.as_ref().map(|id| id.as_str()), Some(SOLUTION_ID));
}

#[test]
fn retry_deadline_in_the_future_yields_no_candidate() {
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let state = make_state_dir(tmp.path());
    write_retry_record(&state, SOLUTION_ID, future_deadline());

    let picked = pick_candidate_with_io(tmp.path(), &state, now()).unwrap();
    assert_eq!(picked, None);
}

#[test]
fn retry_deadline_in_the_past_picks_the_solution() {
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let state = make_state_dir(tmp.path());
    write_retry_record(&state, SOLUTION_ID, elapsed_deadline());

    let picked = pick_candidate_with_io(tmp.path(), &state, now()).unwrap();
    assert_eq!(picked.as_ref().map(|id| id.as_str()), Some(SOLUTION_ID));
}

#[test]
fn overlay_records_for_unknown_solution_are_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let state = make_state_dir(tmp.path());
    // Retry record points at a solution that is not in the current
    // publication set — picker must ignore it and still select the
    // configured solution instead.
    write_retry_record(
        &state,
        "librarychecker-aplusb/aplusb/removed",
        elapsed_deadline(),
    );

    let picked = pick_candidate_with_io(tmp.path(), &state, now()).unwrap();
    assert_eq!(picked.as_ref().map(|id| id.as_str()), Some(SOLUTION_ID));
}

#[test]
fn missing_config_toml_returns_a_config_error() {
    let tmp = tempfile::tempdir().unwrap();
    // No config.toml, no solutions.
    let state = make_state_dir(tmp.path());
    let err = pick_candidate_with_io(tmp.path(), &state, now()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("config.toml"), "unexpected error: {msg}");
}

#[test]
fn malformed_record_json_names_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let state = make_state_dir(tmp.path());
    let results_dir = state.join("verification/results");
    std::fs::create_dir_all(&results_dir).unwrap();
    let bad_path = results_dir.join("librarychecker-aplusb__aplusb__main.json");
    std::fs::write(&bad_path, b"{not valid json").unwrap();

    let err = pick_candidate_with_io(tmp.path(), &state, now()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("librarychecker-aplusb__aplusb__main.json"),
        "expected error to name the file, got: {msg}"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_state_directory_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let real_state = tmp.path().join("real-state");
    std::fs::create_dir_all(&real_state).unwrap();
    let link = tmp.path().join("state");
    std::os::unix::fs::symlink(&real_state, &link).unwrap();

    let err = pick_candidate_with_io(tmp.path(), &link, now()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("symlink"), "unexpected error: {msg}");
}

#[cfg(unix)]
#[test]
fn symlinked_results_directory_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let state = make_state_dir(tmp.path());
    // Stage a benign target dir; the picker must reject the symlink at the
    // `verification/results` layer even though `state/` itself is fine.
    let target = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::create_dir_all(state.join("verification")).unwrap();
    std::os::unix::fs::symlink(&target, state.join("verification/results")).unwrap();

    let err = pick_candidate_with_io(tmp.path(), &state, now()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("symlink"), "unexpected error: {msg}");
}

#[cfg(unix)]
#[test]
fn symlinked_merged_results_directory_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let state = make_state_dir(tmp.path());
    // Same guard as the overlay: the merged tree under `--root` is walked
    // too, so a symlinked `verification/results` there must not let the
    // walker escape the repository.
    let target = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::create_dir_all(tmp.path().join("verification")).unwrap();
    std::os::unix::fs::symlink(&target, tmp.path().join("verification/results")).unwrap();

    let err = pick_candidate_with_io(tmp.path(), &state, now()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("symlink"), "unexpected error: {msg}");
}

#[test]
fn merged_record_gates_eligibility_when_the_overlay_is_absent() {
    // Merging the automation PR deletes `automation/verify`, so the next
    // tick fetches nothing and the overlay is empty. The record that landed
    // on `main` must still gate eligibility; otherwise every verified
    // solution looks unverified and gets resubmitted.
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let state = make_state_dir(tmp.path());
    write_retry_record(tmp.path(), SOLUTION_ID, future_deadline());

    let picked = pick_candidate_with_io(tmp.path(), &state, now()).unwrap();
    assert_eq!(picked, None);
}

#[test]
fn merged_record_with_elapsed_retry_deadline_is_picked() {
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let state = make_state_dir(tmp.path());
    write_retry_record(tmp.path(), SOLUTION_ID, elapsed_deadline());

    let picked = pick_candidate_with_io(tmp.path(), &state, now()).unwrap();
    assert_eq!(picked.as_ref().map(|id| id.as_str()), Some(SOLUTION_ID));
}

#[test]
fn overlay_record_supersedes_a_ready_merged_record() {
    // The overlay observation is newer than anything on `main`, so an
    // in-flight retry window must win over an already-elapsed merged one.
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let state = make_state_dir(tmp.path());
    write_retry_record(tmp.path(), SOLUTION_ID, elapsed_deadline());
    write_retry_record(&state, SOLUTION_ID, future_deadline());

    let picked = pick_candidate_with_io(tmp.path(), &state, now()).unwrap();
    assert_eq!(picked, None);
}

#[test]
fn overlay_record_supersedes_a_blocked_merged_record() {
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let state = make_state_dir(tmp.path());
    write_retry_record(tmp.path(), SOLUTION_ID, future_deadline());
    write_retry_record(&state, SOLUTION_ID, elapsed_deadline());

    let picked = pick_candidate_with_io(tmp.path(), &state, now()).unwrap();
    assert_eq!(picked.as_ref().map(|id| id.as_str()), Some(SOLUTION_ID));
}

#[test]
fn merged_completed_record_triggers_the_fingerprint_recompute() {
    // A `Completed` record is only eligible on fingerprint drift, so the
    // picker has to recompute the current fingerprint — which fans out to
    // the language analyzer declared in `config.toml`. This repo ships no
    // analyzer executable, so the recompute fails loudly and names the
    // configured command. The failure is the proof: while merged records
    // were invisible, the picker took the "no completed records" fast path
    // and reported the solution as a fresh candidate.
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let state = make_state_dir(tmp.path());
    write_completed_record(tmp.path(), SOLUTION_ID);

    let err = pick_candidate_with_io(tmp.path(), &state, now()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("rust-analyzer"), "unexpected error: {msg}");
}
