//! Tests for the deterministic build-input digest (plan 040 Task 1).

use std::fs;
use std::path::{Path, PathBuf};

use domain::adapter_build::{BuildInputKind, TargetPlatform};
use infrastructure::library_adapter::inputs::{
    BUILD_INPUTS_CONFIG_PATH, BuildInputError, calculate_input_digest, load_build_inputs,
};
use tempfile::TempDir;

fn write_file(root: &Path, rel: &str, contents: &str) -> PathBuf {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, contents).unwrap();
    path
}

fn write_config(root: &Path, contents: &str) {
    write_file(root, BUILD_INPUTS_CONFIG_PATH, contents);
}

fn linux() -> TargetPlatform {
    TargetPlatform {
        os: "linux".into(),
        arch: "x86_64".into(),
    }
}

// ─── load_build_inputs ───────────────────────────────────────────────────────

#[test]
fn load_build_inputs_parses_directories_and_files() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"
directories = ["tools/library-analyzers/rust"]
files = ["Cargo.toml", "rust-toolchain.toml"]
"#,
    );
    let inputs = load_build_inputs(dir.path()).unwrap();
    assert_eq!(inputs.entries.len(), 3);
    assert_eq!(inputs.entries[0].kind, BuildInputKind::Directory);
    assert_eq!(inputs.entries[0].path, "tools/library-analyzers/rust");
    assert_eq!(inputs.entries[1].kind, BuildInputKind::File);
    assert_eq!(inputs.entries[1].path, "Cargo.toml");
}

#[test]
fn load_build_inputs_errors_when_config_missing() {
    let dir = TempDir::new().unwrap();
    let err = load_build_inputs(dir.path()).unwrap_err();
    assert!(matches!(err, BuildInputError::Io { .. }));
}

#[test]
fn load_build_inputs_rejects_absolute_path() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"files = ["/etc/passwd"]
"#,
    );
    let err = load_build_inputs(dir.path()).unwrap_err();
    assert!(matches!(err, BuildInputError::NotRelative { .. }));
}

#[test]
fn load_build_inputs_rejects_parent_reference() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"files = ["../secret"]
"#,
    );
    let err = load_build_inputs(dir.path()).unwrap_err();
    assert!(matches!(err, BuildInputError::InvalidPath { .. }));
}

#[test]
fn load_build_inputs_rejects_duplicate_paths() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"
directories = ["tools/a"]
files = ["tools/a"]
"#,
    );
    let err = load_build_inputs(dir.path()).unwrap_err();
    assert!(matches!(err, BuildInputError::Duplicate { .. }));
}

#[test]
fn load_build_inputs_rejects_overlapping_directories() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"
directories = ["tools", "tools/library-analyzers"]
"#,
    );
    let err = load_build_inputs(dir.path()).unwrap_err();
    assert!(matches!(err, BuildInputError::Overlap { .. }));
}

#[test]
fn load_build_inputs_rejects_file_inside_declared_directory() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"
directories = ["tools"]
files = ["tools/x.txt"]
"#,
    );
    let err = load_build_inputs(dir.path()).unwrap_err();
    assert!(matches!(err, BuildInputError::Overlap { .. }));
}

#[test]
fn load_build_inputs_rejects_unknown_toml_key() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"
directories = []
files = []
unknown = 1
"#,
    );
    let err = load_build_inputs(dir.path()).unwrap_err();
    assert!(matches!(err, BuildInputError::Toml { .. }));
}

// ─── calculate_input_digest ──────────────────────────────────────────────────

#[test]
fn calculate_input_digest_is_stable_across_creation_order() {
    let a = TempDir::new().unwrap();
    write_config(
        a.path(),
        r#"directories = ["src"]
files = []
"#,
    );
    write_file(a.path(), "src/b.rs", "hello b\n");
    write_file(a.path(), "src/a.rs", "hello a\n");
    let inputs_a = load_build_inputs(a.path()).unwrap();
    let digest_a = calculate_input_digest(a.path(), &inputs_a, &linux()).unwrap();

    let b = TempDir::new().unwrap();
    write_config(
        b.path(),
        r#"directories = ["src"]
files = []
"#,
    );
    // Create the same content in the opposite order.
    write_file(b.path(), "src/a.rs", "hello a\n");
    write_file(b.path(), "src/b.rs", "hello b\n");
    let inputs_b = load_build_inputs(b.path()).unwrap();
    let digest_b = calculate_input_digest(b.path(), &inputs_b, &linux()).unwrap();

    assert_eq!(digest_a, digest_b);
}

#[test]
fn calculate_input_digest_changes_when_file_content_changes() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"directories = ["src"]
files = []
"#,
    );
    write_file(dir.path(), "src/a.rs", "hello a\n");
    let inputs = load_build_inputs(dir.path()).unwrap();
    let digest_before = calculate_input_digest(dir.path(), &inputs, &linux()).unwrap();

    // Simulate an uncommitted content change.
    write_file(dir.path(), "src/a.rs", "hello a modified\n");
    let digest_after = calculate_input_digest(dir.path(), &inputs, &linux()).unwrap();
    assert_ne!(digest_before, digest_after);
}

#[test]
fn calculate_input_digest_changes_when_platform_changes() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"directories = []
files = ["Cargo.toml"]
"#,
    );
    write_file(dir.path(), "Cargo.toml", "[package]\nname = \"x\"\n");
    let inputs = load_build_inputs(dir.path()).unwrap();
    let linux_digest = calculate_input_digest(dir.path(), &inputs, &linux()).unwrap();
    let macos_digest = calculate_input_digest(
        dir.path(),
        &inputs,
        &TargetPlatform {
            os: "darwin".into(),
            arch: "aarch64".into(),
        },
    )
    .unwrap();
    assert_ne!(linux_digest, macos_digest);
}

#[test]
fn calculate_input_digest_treats_new_files_as_additions() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"directories = ["src"]
files = []
"#,
    );
    write_file(dir.path(), "src/a.rs", "hello\n");
    let inputs = load_build_inputs(dir.path()).unwrap();
    let before = calculate_input_digest(dir.path(), &inputs, &linux()).unwrap();

    // Add a previously untracked file — must be picked up.
    write_file(dir.path(), "src/nested/new.rs", "new\n");
    let after = calculate_input_digest(dir.path(), &inputs, &linux()).unwrap();
    assert_ne!(before, after);
}

#[test]
fn calculate_input_digest_rejects_missing_directory() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"directories = ["src"]
files = []
"#,
    );
    let inputs = load_build_inputs(dir.path()).unwrap();
    let err = calculate_input_digest(dir.path(), &inputs, &linux()).unwrap_err();
    assert!(matches!(err, BuildInputError::Missing { .. }));
}

#[test]
fn calculate_input_digest_rejects_missing_file() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"directories = []
files = ["Cargo.toml"]
"#,
    );
    let inputs = load_build_inputs(dir.path()).unwrap();
    let err = calculate_input_digest(dir.path(), &inputs, &linux()).unwrap_err();
    assert!(matches!(err, BuildInputError::Missing { .. }));
}

#[test]
fn calculate_input_digest_rejects_wrong_kind_when_file_is_directory() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"directories = []
files = ["src"]
"#,
    );
    fs::create_dir(dir.path().join("src")).unwrap();
    let inputs = load_build_inputs(dir.path()).unwrap();
    let err = calculate_input_digest(dir.path(), &inputs, &linux()).unwrap_err();
    assert!(matches!(err, BuildInputError::WrongKind { .. }));
}

// ─── symlink handling (Unix only) ────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn calculate_input_digest_rejects_file_symlink_input() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"directories = []
files = ["link"]
"#,
    );
    write_file(dir.path(), "real.txt", "content\n");
    symlink(dir.path().join("real.txt"), dir.path().join("link")).unwrap();
    let inputs = load_build_inputs(dir.path()).unwrap();
    let err = calculate_input_digest(dir.path(), &inputs, &linux()).unwrap_err();
    assert!(matches!(err, BuildInputError::Symlink { .. }));
}

#[cfg(unix)]
#[test]
fn calculate_input_digest_rejects_directory_symlink_input() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"directories = ["linked"]
files = []
"#,
    );
    write_file(dir.path(), "real/a.rs", "a\n");
    symlink(dir.path().join("real"), dir.path().join("linked")).unwrap();
    let inputs = load_build_inputs(dir.path()).unwrap();
    let err = calculate_input_digest(dir.path(), &inputs, &linux()).unwrap_err();
    assert!(matches!(err, BuildInputError::Symlink { .. }));
}

#[cfg(unix)]
#[test]
fn calculate_input_digest_rejects_symlink_within_input_directory() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"directories = ["src"]
files = []
"#,
    );
    write_file(dir.path(), "src/a.rs", "a\n");
    write_file(dir.path(), "external.rs", "external\n");
    symlink(
        dir.path().join("external.rs"),
        dir.path().join("src/linked.rs"),
    )
    .unwrap();
    let inputs = load_build_inputs(dir.path()).unwrap();
    let err = calculate_input_digest(dir.path(), &inputs, &linux()).unwrap_err();
    assert!(matches!(err, BuildInputError::SymlinkInside { .. }));
}

#[cfg(unix)]
#[test]
fn calculate_input_digest_rejects_repository_escape_via_parent_symlink() {
    use std::os::unix::fs::symlink;
    // Real root outside the "repository" holds a file we should never hash.
    let real_root = TempDir::new().unwrap();
    write_file(real_root.path(), "secret.txt", "secret\n");

    let repo = TempDir::new().unwrap();
    // Symlink a subdirectory to point at the outer directory.
    symlink(real_root.path(), repo.path().join("hidden")).unwrap();
    write_config(
        repo.path(),
        r#"directories = []
files = ["hidden/secret.txt"]
"#,
    );

    let inputs = load_build_inputs(repo.path()).unwrap();
    let err = calculate_input_digest(repo.path(), &inputs, &linux()).unwrap_err();
    assert!(matches!(
        err,
        BuildInputError::Symlink { .. } | BuildInputError::RepositoryEscape { .. }
    ));
}
