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
//! * A parallel `state/` directory that stands in for the
//!   `automation/verify` overlay (`state/verification/results/**`).

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

fn write_retry_record(state_dir: &Path, solution_id: &str, deadline: DateTime<FixedOffset>) {
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
    let results_dir = state_dir.join("verification/results");
    std::fs::create_dir_all(&results_dir).unwrap();
    let path = results_dir.join(format!(
        "{}.json",
        record.solution_id.as_str().replace('/', "__")
    ));
    let bytes = serde_json::to_vec_pretty(&record).unwrap();
    std::fs::write(&path, bytes).unwrap();
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
    let deadline = Utc
        .with_ymd_and_hms(2026, 8, 14, 15, 0, 0)
        .unwrap()
        .fixed_offset();
    write_retry_record(&state, SOLUTION_ID, deadline);

    let picked = pick_candidate_with_io(tmp.path(), &state, now()).unwrap();
    assert_eq!(picked, None);
}

#[test]
fn retry_deadline_in_the_past_picks_the_solution() {
    let tmp = tempfile::tempdir().unwrap();
    write_repo(tmp.path());
    let state = make_state_dir(tmp.path());
    let deadline = Utc
        .with_ymd_and_hms(2026, 8, 14, 10, 0, 0)
        .unwrap()
        .fixed_offset();
    write_retry_record(&state, SOLUTION_ID, deadline);

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
    let deadline = Utc
        .with_ymd_and_hms(2026, 8, 14, 10, 0, 0)
        .unwrap()
        .fixed_offset();
    write_retry_record(&state, "librarychecker-aplusb/aplusb/removed", deadline);

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
