//! Integration tests for [`GitHistoryImpl::head_snapshot`] — specifically the
//! working-tree cleanliness signal that gates `ce site-data generate --mode
//! production`.
//!
//! Each test drives a real `git` subprocess against a fresh ephemeral
//! repository, because the behaviour under test is entirely about how the
//! porcelain pathspec is spelled.

use std::fs;
use std::path::Path;
use std::process::Command;

use infrastructure::git_history::GitHistoryImpl;
use tempfile::TempDir;
use usecases::git_history::GitHistory;

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

fn write(root: &Path, rel: &str, body: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// Repository seeded with one source file and one committed verification
/// record, mirroring `main` after an automation PR has merged.
fn seeded_repo() -> TempDir {
    let td = tempfile::tempdir().expect("tempdir");
    let root = td.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["config", "user.email", "t@t"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    write(
        root,
        "libraries/rust/algebra/monoid.rs",
        b"pub trait M {}\n",
    );
    write(root, "verification/results/c/p/rust.json", b"{\"v\":1}\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "seed"]);
    td
}

fn snapshot(root: &Path) -> usecases::git_history::RepositorySnapshot {
    GitHistoryImpl::new(root.to_path_buf())
        .head_snapshot()
        .expect("head_snapshot")
}

#[test]
fn clean_tree_reports_no_uncommitted_changes() {
    let td = seeded_repo();
    assert!(!snapshot(td.path()).uncommitted_changes);
}

#[test]
fn overlaid_verification_records_do_not_count_as_uncommitted() {
    // What `pages.yml` does: overwrite the merged record with the newer copy
    // from `automation/verify` and drop in records for solutions whose
    // automation PR has not merged yet. Neither is a source change, so the
    // production build must not refuse to run.
    let td = seeded_repo();
    let root = td.path();
    write(root, "verification/results/c/p/rust.json", b"{\"v\":2}\n");
    write(root, "verification/results/c/q/rust.json", b"{\"v\":1}\n");

    assert!(!snapshot(root).uncommitted_changes);
}

#[test]
fn source_changes_still_count_as_uncommitted() {
    let td = seeded_repo();
    let root = td.path();
    write(
        root,
        "libraries/rust/algebra/monoid.rs",
        b"pub trait N {}\n",
    );

    assert!(snapshot(root).uncommitted_changes);
}

#[test]
fn untracked_source_files_still_count_as_uncommitted() {
    let td = seeded_repo();
    let root = td.path();
    write(root, "libraries/rust/algebra/group.rs", b"pub trait G {}\n");

    assert!(snapshot(root).uncommitted_changes);
}

#[test]
fn a_path_merely_prefixed_with_verification_is_not_excluded() {
    // The exclusion must be the `verification/results` directory, not any
    // path starting with that string.
    let td = seeded_repo();
    let root = td.path();
    write(root, "verification/results-notes.md", b"hi\n");

    assert!(snapshot(root).uncommitted_changes);
}
