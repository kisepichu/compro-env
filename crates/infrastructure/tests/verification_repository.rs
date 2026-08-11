//! Integration tests for [`VerificationRepositoryImpl`] (spec §11).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use domain::library::SolutionId;
use domain::verification::{AttemptId, VerificationRecord};
use infrastructure::repository_impl::verification_repository_impl::{
    VerificationRepositoryError, VerificationRepositoryImpl,
};
use tempfile::TempDir;
use usecases::repository::verification_repository::VerificationRepository;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("verification")
}

fn load_fixture(name: &str) -> VerificationRecord {
    let path = fixture_dir().join(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("failed to parse fixture {}: {e}", path.display()))
}

fn load_fixture_raw(name: &str) -> String {
    let path = fixture_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()))
}

fn make_repo(dir: &TempDir) -> VerificationRepositoryImpl {
    VerificationRepositoryImpl::new(dir.path().to_path_buf())
}

fn sid(raw: &str) -> SolutionId {
    SolutionId::parse(raw).expect("valid solution id")
}

fn attempt(raw: &str) -> AttemptId {
    AttemptId::parse(raw).expect("valid attempt id")
}

fn expect_error<T>(result: anyhow::Result<T>) -> anyhow::Error {
    result.err().expect("expected error, got Ok(_)")
}

fn expected_json_path(root: &Path, id: &SolutionId) -> PathBuf {
    root.join("verification")
        .join("results")
        .join(id.contest_id())
        .join(id.problem_code())
        .join(format!("{}.json", id.solution_name()))
}

// ── 1. Canonical path ────────────────────────────────────────────────────────

#[test]
fn compare_and_swap_writes_to_canonical_path() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let record = load_fixture("accepted.json");
    let id = record.solution_id.clone();

    repo.compare_and_swap(&id, None, &record).unwrap();

    let expected = expected_json_path(dir.path(), &id);
    assert!(
        expected.exists(),
        "expected canonical record at {}",
        expected.display()
    );
}

// ── 2. Fresh CAS round-trips ────────────────────────────────────────────────

#[test]
fn fresh_compare_and_swap_round_trips() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let record = load_fixture("accepted.json");
    let id = record.solution_id.clone();

    repo.compare_and_swap(&id, None, &record).unwrap();

    let loaded = repo.load(&id).unwrap().expect("record must load");
    assert_eq!(loaded, record);
}

// ── 3. Existing-record CAS success (replaces prior attempt) ─────────────────

#[test]
fn existing_record_cas_succeeds_when_expected_matches() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let first = load_fixture("accepted.json");
    let id = first.solution_id.clone();
    let first_attempt = first.attempt_id.clone();
    repo.compare_and_swap(&id, None, &first).unwrap();

    let second = load_fixture("stale-attempt.json");
    assert_eq!(second.replaces_attempt_id.as_ref(), Some(&first_attempt));

    repo.compare_and_swap(&id, Some(&first_attempt), &second)
        .unwrap();

    let loaded = repo.load(&id).unwrap().expect("record must load");
    assert_eq!(loaded, second);
}

// ── 4. CAS refuses to clobber ───────────────────────────────────────────────

#[test]
fn cas_none_fails_when_record_already_exists() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let record = load_fixture("accepted.json");
    let id = record.solution_id.clone();
    repo.compare_and_swap(&id, None, &record).unwrap();

    let err = expect_error(repo.compare_and_swap(&id, None, &record));
    let downcast = err
        .downcast_ref::<VerificationRepositoryError>()
        .expect("must downcast to VerificationRepositoryError");
    assert!(
        matches!(downcast, VerificationRepositoryError::AlreadyExists { id: reported } if reported == &id),
        "expected AlreadyExists, got {downcast:?}",
    );
}

// ── 5. Wrong-expected CAS conflict ──────────────────────────────────────────

#[test]
fn cas_wrong_expected_reports_current_attempt() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let first = load_fixture("accepted.json");
    let id = first.solution_id.clone();
    let first_attempt = first.attempt_id.clone();
    repo.compare_and_swap(&id, None, &first).unwrap();

    let other = attempt("attempt-999");
    // Reuse `first` payload as next; its replaces_attempt_id would need to match
    // for a real CAS, but the conflict must be reported first.
    let mut next = load_fixture("stale-attempt.json");
    // Align replaces_attempt_id with the (wrong) expected so we only ever hit
    // the conflict check.
    next.replaces_attempt_id = Some(other.clone());
    let err = expect_error(repo.compare_and_swap(&id, Some(&other), &next));
    let downcast = err
        .downcast_ref::<VerificationRepositoryError>()
        .expect("must downcast");
    match downcast {
        VerificationRepositoryError::ConflictingAttempt {
            id: reported_id,
            current,
        } => {
            assert_eq!(reported_id, &id);
            assert_eq!(current, &first_attempt);
        }
        other => panic!("expected ConflictingAttempt, got {other:?}"),
    }
}

// ── 6. Inconsistent replacement ────────────────────────────────────────────

#[test]
fn cas_rejects_replaces_attempt_id_disagreement() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let first = load_fixture("accepted.json");
    let id = first.solution_id.clone();
    let first_attempt = first.attempt_id.clone();
    repo.compare_and_swap(&id, None, &first).unwrap();

    // Same body as `stale-attempt.json` but with replaces_attempt_id cleared:
    // that violates the "expected == next.replaces_attempt_id" invariant.
    let mut next = load_fixture("stale-attempt.json");
    next.replaces_attempt_id = None;

    let err = expect_error(repo.compare_and_swap(&id, Some(&first_attempt), &next));
    let downcast = err
        .downcast_ref::<VerificationRepositoryError>()
        .expect("must downcast");
    assert!(
        matches!(
            downcast,
            VerificationRepositoryError::InconsistentReplacement { .. }
        ),
        "expected InconsistentReplacement, got {downcast:?}",
    );
}

// ── 7. replaces_attempt_id must match expected exactly ─────────────────────

#[test]
fn cas_rejects_when_replaces_targets_wrong_attempt() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let first = load_fixture("accepted.json");
    let id = first.solution_id.clone();
    let first_attempt = first.attempt_id.clone();
    repo.compare_and_swap(&id, None, &first).unwrap();

    // Second write pointing at an attempt that isn't the current one.
    let mut next = load_fixture("stale-attempt.json");
    next.replaces_attempt_id = Some(attempt("attempt-does-not-exist"));

    let err = expect_error(repo.compare_and_swap(&id, Some(&first_attempt), &next));
    let downcast = err
        .downcast_ref::<VerificationRepositoryError>()
        .expect("must downcast");
    assert!(
        matches!(
            downcast,
            VerificationRepositoryError::InconsistentReplacement { .. }
        ),
        "expected InconsistentReplacement, got {downcast:?}",
    );
}

// ── 8. remove_if_attempt success ───────────────────────────────────────────

#[test]
fn remove_if_attempt_removes_record_when_attempt_matches() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let first = load_fixture("accepted.json");
    let id = first.solution_id.clone();
    let att = first.attempt_id.clone();
    repo.compare_and_swap(&id, None, &first).unwrap();

    repo.remove_if_attempt(&id, &att).unwrap();

    assert!(repo.load(&id).unwrap().is_none());
    assert!(!expected_json_path(dir.path(), &id).exists());
}

// ── 9. remove_if_attempt conflict ──────────────────────────────────────────

#[test]
fn remove_if_attempt_rejects_when_attempt_mismatches() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let first = load_fixture("accepted.json");
    let id = first.solution_id.clone();
    let att = first.attempt_id.clone();
    repo.compare_and_swap(&id, None, &first).unwrap();

    let other = attempt("attempt-999");
    let err = expect_error(repo.remove_if_attempt(&id, &other));
    let downcast = err
        .downcast_ref::<VerificationRepositoryError>()
        .expect("must downcast");
    match downcast {
        VerificationRepositoryError::ConflictingAttempt { current, .. } => {
            assert_eq!(current, &att);
        }
        other => panic!("expected ConflictingAttempt, got {other:?}"),
    }
    // File is still there.
    assert!(expected_json_path(dir.path(), &id).exists());
}

// ── 10. remove_if_attempt on missing record ────────────────────────────────

#[test]
fn remove_if_attempt_reports_not_found_when_record_missing() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let id = sid("abc999/a/main");
    let err = expect_error(repo.remove_if_attempt(&id, &attempt("attempt-001")));
    let downcast = err
        .downcast_ref::<VerificationRepositoryError>()
        .expect("must downcast");
    assert!(
        matches!(downcast, VerificationRepositoryError::NotFound { .. }),
        "expected NotFound, got {downcast:?}",
    );
}

// ── 11. load_all returns declared records ──────────────────────────────────

#[test]
fn load_all_returns_declared_records() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let record_a = load_fixture("accepted.json");
    let id_a = record_a.solution_id.clone();

    let mut record_b = load_fixture("accepted.json");
    record_b.solution_id = sid("abc999/a/other");
    repo.compare_and_swap(&id_a, None, &record_a).unwrap();
    repo.compare_and_swap(&record_b.solution_id.clone(), None, &record_b)
        .unwrap();

    let mut discovered = BTreeSet::new();
    discovered.insert(id_a.clone());
    discovered.insert(record_b.solution_id.clone());

    let all = repo.load_all(&discovered).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all.get(&id_a).unwrap(), &record_a);
    assert_eq!(all.get(&record_b.solution_id).unwrap(), &record_b);
}

#[test]
fn load_all_returns_empty_when_tree_absent() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let all = repo.load_all(&BTreeSet::new()).unwrap();
    assert!(all.is_empty());
}

// ── 12. load_all rejects orphan ────────────────────────────────────────────

#[test]
fn load_all_rejects_orphan_records() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let record = load_fixture("accepted.json");
    let id = record.solution_id.clone();
    repo.compare_and_swap(&id, None, &record).unwrap();

    let mut discovered = BTreeSet::new();
    discovered.insert(sid("abc999/a/other"));

    let err = expect_error(repo.load_all(&discovered));
    let downcast = err
        .downcast_ref::<VerificationRepositoryError>()
        .expect("must downcast");
    match downcast {
        VerificationRepositoryError::OrphanRecord { id: reported } => {
            assert_eq!(reported, &id);
        }
        other => panic!("expected OrphanRecord, got {other:?}"),
    }
}

// ── 13. load_all rejects non-JSON entries ─────────────────────────────────

#[test]
fn load_all_rejects_non_json_entries() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let record = load_fixture("accepted.json");
    let id = record.solution_id.clone();
    repo.compare_and_swap(&id, None, &record).unwrap();

    let stray = dir
        .path()
        .join("verification")
        .join("results")
        .join(id.contest_id())
        .join(id.problem_code())
        .join("main.txt");
    std::fs::write(&stray, "not a record").unwrap();

    let mut discovered = BTreeSet::new();
    discovered.insert(id);

    let err = expect_error(repo.load_all(&discovered));
    let downcast = err
        .downcast_ref::<VerificationRepositoryError>()
        .expect("must downcast");
    assert!(
        matches!(downcast, VerificationRepositoryError::NonJsonEntry { .. }),
        "expected NonJsonEntry, got {downcast:?}",
    );
}

// ── 14. load_all rejects symlink records ──────────────────────────────────

#[cfg(unix)]
#[test]
fn load_all_rejects_symlinked_records() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    // Materialize a real record for the sibling target.
    let record = load_fixture("accepted.json");
    let mut sibling = record.clone();
    sibling.solution_id = sid("abc999/a/sibling");
    repo.compare_and_swap(&sibling.solution_id.clone(), None, &sibling)
        .unwrap();

    // Now replace the primary record's location with a symlink to the sibling.
    let primary_dir = dir
        .path()
        .join("verification")
        .join("results")
        .join(record.solution_id.contest_id())
        .join(record.solution_id.problem_code());
    std::fs::create_dir_all(&primary_dir).unwrap();
    let primary_path = primary_dir.join(format!("{}.json", record.solution_id.solution_name()));
    let sibling_path = primary_dir.join(format!("{}.json", sibling.solution_id.solution_name()));
    std::os::unix::fs::symlink(&sibling_path, &primary_path).unwrap();

    let mut discovered = BTreeSet::new();
    discovered.insert(record.solution_id.clone());
    discovered.insert(sibling.solution_id.clone());

    let err = expect_error(repo.load_all(&discovered));
    let downcast = err
        .downcast_ref::<VerificationRepositoryError>()
        .expect("must downcast");
    assert!(
        matches!(
            downcast,
            VerificationRepositoryError::SymlinkNotAllowed { .. }
        ),
        "expected SymlinkNotAllowed, got {downcast:?}",
    );
}

// ── 15. load_all rejects solution_id / path mismatch ──────────────────────

#[test]
fn load_all_rejects_path_solution_id_mismatch() {
    let dir = TempDir::new().unwrap();

    // Manually drop a record whose solution_id disagrees with its path.
    let raw = load_fixture_raw("accepted.json");
    let liar_path = dir
        .path()
        .join("verification")
        .join("results")
        .join("abc999")
        .join("b")
        .join("main.json");
    std::fs::create_dir_all(liar_path.parent().unwrap()).unwrap();
    std::fs::write(&liar_path, raw).unwrap();

    let repo = make_repo(&dir);
    let mut discovered = BTreeSet::new();
    discovered.insert(sid("abc999/b/main"));

    let err = expect_error(repo.load_all(&discovered));
    let downcast = err
        .downcast_ref::<VerificationRepositoryError>()
        .expect("must downcast");
    assert!(
        matches!(downcast, VerificationRepositoryError::PathMismatch { .. }),
        "expected PathMismatch, got {downcast:?}",
    );
}

// ── 16. load returns Err on corrupt JSON ──────────────────────────────────

#[test]
fn load_returns_error_on_corrupt_json() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let id = sid("abc999/a/main");
    let path = expected_json_path(dir.path(), &id);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{ not valid json ").unwrap();

    let err = repo.load(&id).err().expect("must be an error");
    // The parse failure surfaces the anyhow context.
    assert!(
        format!("{err:?}").contains("failed to parse record"),
        "expected parse-record context, got {err:?}",
    );
}

// ── 17. failed CAS does not clobber the stored record ────────────────────

#[test]
fn failed_cas_leaves_previous_record_intact() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let first = load_fixture("accepted.json");
    let id = first.solution_id.clone();
    repo.compare_and_swap(&id, None, &first).unwrap();

    let mut next = load_fixture("stale-attempt.json");
    // Point at a wrong "expected" attempt so the CAS fails before writing.
    next.replaces_attempt_id = Some(attempt("attempt-does-not-exist"));
    let _ = repo.compare_and_swap(&id, Some(&attempt("attempt-does-not-exist")), &next);

    let loaded = repo.load(&id).unwrap().expect("record still present");
    assert_eq!(loaded, first);
}

// ── 19. verification/ symlink pointing outside the tree is rejected ──────

#[cfg(unix)]
#[test]
fn compare_and_swap_rejects_symlinked_verification_dir() {
    // Regression: the earlier walk stopped at `results_root`, so a symlink at
    // `root/verification` was invisible. Confirm every op now rejects it.
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("verification")).unwrap();

    let repo = make_repo(&dir);
    let record = load_fixture("accepted.json");
    let id = record.solution_id.clone();

    let err = expect_error(repo.compare_and_swap(&id, None, &record));
    let downcast = err
        .downcast_ref::<VerificationRepositoryError>()
        .expect("must downcast");
    assert!(
        matches!(
            downcast,
            VerificationRepositoryError::SymlinkNotAllowed { .. }
        ),
        "expected SymlinkNotAllowed, got {downcast:?}",
    );

    // load and load_all should also refuse to descend through the symlink.
    let err = expect_error(repo.load(&id));
    assert!(
        err.downcast_ref::<VerificationRepositoryError>()
            .is_some_and(|e| matches!(e, VerificationRepositoryError::SymlinkNotAllowed { .. })),
        "load: expected SymlinkNotAllowed, got {err:?}",
    );

    let mut discovered = BTreeSet::new();
    discovered.insert(id.clone());
    let err = expect_error(repo.load_all(&discovered));
    assert!(
        err.downcast_ref::<VerificationRepositoryError>()
            .is_some_and(|e| matches!(e, VerificationRepositoryError::SymlinkNotAllowed { .. })),
        "load_all: expected SymlinkNotAllowed, got {err:?}",
    );
}

// ── 20. load / CAS / remove reject records whose stored id disagrees ─────

#[test]
fn single_id_ops_reject_path_id_mismatch() {
    // Regression: `load`, `compare_and_swap`, and `remove_if_attempt` used to
    // trust the on-disk `solution_id` field. Confirm each surfaces
    // `PathMismatch` when the file's contents don't match the caller-supplied
    // id.
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    // Place accepted.json (solution_id = abc999/a/main) under
    // abc999/a/other.json — the on-disk path derives id abc999/a/other but the
    // JSON says abc999/a/main.
    let victim_id = sid("abc999/a/other");
    let raw = load_fixture_raw("accepted.json");
    let victim_path = expected_json_path(dir.path(), &victim_id);
    std::fs::create_dir_all(victim_path.parent().unwrap()).unwrap();
    std::fs::write(&victim_path, raw).unwrap();

    // load
    let err = expect_error(repo.load(&victim_id));
    assert!(
        err.downcast_ref::<VerificationRepositoryError>()
            .is_some_and(|e| matches!(e, VerificationRepositoryError::PathMismatch { .. })),
        "load: expected PathMismatch, got {err:?}",
    );

    // compare_and_swap
    let next = load_fixture("accepted.json");
    let err = expect_error(repo.compare_and_swap(&victim_id, None, &next));
    assert!(
        err.downcast_ref::<VerificationRepositoryError>()
            .is_some_and(|e| matches!(e, VerificationRepositoryError::PathMismatch { .. })),
        "compare_and_swap: expected PathMismatch, got {err:?}",
    );

    // remove_if_attempt
    let stored_attempt = load_fixture("accepted.json").attempt_id;
    let err = expect_error(repo.remove_if_attempt(&victim_id, &stored_attempt));
    assert!(
        err.downcast_ref::<VerificationRepositoryError>()
            .is_some_and(|e| matches!(e, VerificationRepositoryError::PathMismatch { .. })),
        "remove_if_attempt: expected PathMismatch, got {err:?}",
    );
}

// ── 18. successful CAS leaves no stray temp files ────────────────────────

#[test]
fn successful_cas_leaves_no_temp_files() {
    let dir = TempDir::new().unwrap();
    let repo = make_repo(&dir);

    let record = load_fixture("accepted.json");
    let id = record.solution_id.clone();
    repo.compare_and_swap(&id, None, &record).unwrap();

    let record_dir = expected_json_path(dir.path(), &id)
        .parent()
        .unwrap()
        .to_path_buf();
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&record_dir).unwrap() {
        let entry = entry.unwrap();
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    assert_eq!(names, vec!["main.json".to_string()], "found {names:?}",);
}
