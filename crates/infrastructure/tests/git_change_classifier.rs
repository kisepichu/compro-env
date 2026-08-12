//! Integration tests for [`classify_changes`] — the safe-automation gate
//! that decides whether a commit range is result-only (safe to write back
//! without human review) or contains source/config changes.
//!
//! Each test spins up a fresh ephemeral git repository via `tempfile` and
//! drives real `git` subprocesses. No `include_str!` fixtures, no ambient
//! repository — the test suite is fully hermetic.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

use infrastructure::git_change_classifier::{
    ChangeClass, ChangeClassificationError, classify_changes,
};
use tempfile::TempDir;

// ─── Helpers ────────────────────────────────────────────────────────────────

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .expect("failed to spawn git");
    assert!(
        status.success(),
        "git {args:?} failed at {}",
        root.display()
    );
}

fn git_output(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("failed to spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed at {}: {}",
        root.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn init_repo() -> TempDir {
    let td = tempfile::tempdir().expect("tempdir");
    git(td.path(), &["init", "-q", "-b", "main"]);
    git(td.path(), &["config", "user.name", "t"]);
    git(td.path(), &["config", "user.email", "t@t"]);
    git(td.path(), &["config", "commit.gpgsign", "false"]);
    td
}

fn write(root: &Path, rel: &str, body: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn commit(root: &Path, msg: &str) -> String {
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", msg]);
    git_output(root, &["rev-parse", "HEAD"])
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[test]
fn empty_diff_same_commit_returns_empty() {
    let td = init_repo();
    let root = td.path();
    write(root, "README.md", b"hi\n");
    let sha = commit(root, "init");

    let got = classify_changes(root, &sha, &sha).expect("classify");
    assert_eq!(got, ChangeClass::Empty);
}

#[test]
fn one_result_only_json_change_is_result_only() {
    let td = init_repo();
    let root = td.path();
    write(root, "README.md", b"hi\n");
    let before = commit(root, "init");

    write(root, "verification/results/x.json", b"{}\n");
    let after = commit(root, "add result");

    let got = classify_changes(root, &before, &after).expect("classify");
    assert_eq!(got, ChangeClass::ResultOnly);
}

#[test]
fn many_result_only_json_changes_are_result_only() {
    let td = init_repo();
    let root = td.path();
    write(root, "README.md", b"hi\n");
    let before = commit(root, "init");

    for i in 0..5 {
        write(
            root,
            &format!("verification/results/case_{i}.json"),
            b"{}\n",
        );
    }
    let after = commit(root, "batch results");

    let got = classify_changes(root, &before, &after).expect("classify");
    assert_eq!(got, ChangeClass::ResultOnly);
}

#[test]
fn mixed_result_and_source_is_source_or_config() {
    let td = init_repo();
    let root = td.path();
    write(root, "README.md", b"hi\n");
    let before = commit(root, "init");

    write(root, "verification/results/x.json", b"{}\n");
    write(root, "src/foo.rs", b"pub fn f() {}\n");
    let after = commit(root, "mixed");

    let got = classify_changes(root, &before, &after).expect("classify");
    assert_eq!(got, ChangeClass::SourceOrConfig);
}

#[test]
fn rename_of_source_is_source_or_config() {
    let td = init_repo();
    let root = td.path();
    write(root, "README.md", b"hi\n");
    let before = commit(root, "init");

    // Rename via git mv so both delete + add land in the diff.
    git(root, &["mv", "README.md", "README2.md"]);
    let after = commit(root, "rename");

    let got = classify_changes(root, &before, &after).expect("classify");
    assert_eq!(got, ChangeClass::SourceOrConfig);
}

#[test]
fn delete_of_result_json_is_result_only() {
    let td = init_repo();
    let root = td.path();
    write(root, "verification/results/x.json", b"{}\n");
    let before = commit(root, "seed result");

    fs::remove_file(root.join("verification/results/x.json")).unwrap();
    let after = commit(root, "drop result");

    let got = classify_changes(root, &before, &after).expect("classify");
    assert_eq!(got, ChangeClass::ResultOnly);
}

#[test]
fn delete_of_source_is_source_or_config() {
    let td = init_repo();
    let root = td.path();
    write(root, "src/foo.rs", b"pub fn f() {}\n");
    let before = commit(root, "seed src");

    fs::remove_file(root.join("src/foo.rs")).unwrap();
    let after = commit(root, "drop src");

    let got = classify_changes(root, &before, &after).expect("classify");
    assert_eq!(got, ChangeClass::SourceOrConfig);
}

#[test]
fn symlink_under_results_is_rejected() {
    let td = init_repo();
    let root = td.path();
    // Ensure git records the symlink as mode 120000 rather than a copy.
    git(root, &["config", "core.symlinks", "true"]);
    write(root, "README.md", b"hi\n");
    let before = commit(root, "init");

    // Create a real file the symlink can point at, then a symlink under the
    // result path. The symlink's own path ends in `.json` and lives under
    // `verification/results/`, so a purely path-based classifier would be
    // fooled — but the mode check kicks it into SourceOrConfig.
    write(root, "target.json", b"{}\n");
    fs::create_dir_all(root.join("verification/results")).unwrap();
    symlink(
        "../../target.json",
        root.join("verification/results/link.json"),
    )
    .unwrap();
    let after = commit(root, "add symlink");

    let got = classify_changes(root, &before, &after).expect("classify");
    assert_eq!(got, ChangeClass::SourceOrConfig);
}

#[test]
fn invalid_sha_returns_invalid_revision_error() {
    let td = init_repo();
    let root = td.path();
    write(root, "README.md", b"hi\n");
    let sha = commit(root, "init");

    let err = classify_changes(root, &sha, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef")
        .expect_err("expected invalid-sha error");
    assert!(
        matches!(err, ChangeClassificationError::InvalidRevision { .. }),
        "expected InvalidRevision, got {err:?}"
    );
}

#[test]
fn newline_in_result_path_is_still_result_only() {
    let td = init_repo();
    let root = td.path();
    write(root, "README.md", b"hi\n");
    let before = commit(root, "init");

    // Filename literally contains a newline. NUL-separated diff parsing must
    // handle this without splitting on the newline.
    let rel = "verification/results/we\nird.json";
    write(root, rel, b"{}\n");
    let after = commit(root, "gnarly path");

    let got = classify_changes(root, &before, &after).expect("classify");
    assert_eq!(got, ChangeClass::ResultOnly);
}

#[test]
fn more_than_three_hundred_result_files_are_still_result_only() {
    let td = init_repo();
    let root = td.path();
    write(root, "README.md", b"hi\n");
    let before = commit(root, "init");

    // Well above any plausible `paths` filter limit on GitHub Actions and
    // any command-line-length concern.
    for i in 0..350 {
        write(
            root,
            &format!("verification/results/case_{i:04}.json"),
            b"{}\n",
        );
    }
    let after = commit(root, "batch large");

    let got = classify_changes(root, &before, &after).expect("classify");
    assert_eq!(got, ChangeClass::ResultOnly);
}
