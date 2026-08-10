//! Integration tests for solution discovery (spec §4.2, §5, §7.2).

use std::path::{Path, PathBuf};

use infrastructure::library_project::config::ProjectLibraryConfigLoader;
use infrastructure::library_project::discovery::LibraryDiscovery;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("library-project")
}

fn load() -> (PathBuf, domain::library::LibraryProjectConfig) {
    let root = fixture_root();
    let config = ProjectLibraryConfigLoader::load(&root).unwrap();
    (root, config)
}

#[test]
fn public_solution_appears_with_solved_at() {
    let (root, config) = load();
    let manifest = LibraryDiscovery::discover(&root, &config).unwrap();
    let solution = manifest
        .solutions
        .iter()
        .find(|s| s.id.as_str() == "librarychecker-aplusb/aplusb/main")
        .expect("public solution missing from manifest");
    assert_eq!(solution.language.as_str(), "rust");
    assert_eq!(solution.entry, "src/main.rs");
    assert_eq!(solution.test_command, "./test.sh");
    let verify = solution.verify.as_ref().expect("verify spec expected");
    assert_eq!(verify.libraries.len(), 1);
    assert_eq!(verify.oj_language_id, "rust");
    assert_eq!(solution.solved_at.to_rfc3339(), "2026-08-02T14:30:00+09:00");
}

#[test]
fn private_solution_is_omitted() {
    let (root, config) = load();
    let manifest = LibraryDiscovery::discover(&root, &config).unwrap();
    assert!(
        !manifest
            .solutions
            .iter()
            .any(|s| s.id.as_str().starts_with("abc999/")),
        "private solution should be omitted: {:?}",
        manifest.solutions
    );
}

#[test]
fn verify_on_private_solution_is_rejected() {
    let tmp = copy_fixture_tree();
    let bad = tmp.path().join("solutions/abc999/a/private/ce.toml");
    std::fs::write(
        &bad,
        r#"
language = "rust"
test_command = "./test.sh"
solved_at = "2026-08-02T14:30:00+09:00"

[verify]
libraries = ["libraries/rust/public.rs"]
language_id = "rust"
"#,
    )
    .unwrap();
    let config = ProjectLibraryConfigLoader::load(tmp.path()).unwrap();
    let err = LibraryDiscovery::discover(tmp.path(), &config).unwrap_err();
    assert!(format!("{err:#}").contains("publish = true"), "{err:#}");
}

#[test]
fn verify_pointing_to_private_library_is_rejected() {
    let tmp = copy_fixture_tree();
    // Replace the public solution's verify libraries with a private target.
    let target = tmp
        .path()
        .join("solutions/librarychecker-aplusb/aplusb/main/ce.toml");
    std::fs::write(
        &target,
        r#"
language = "rust"
test_command = "./test.sh"
publish = true
solved_at = "2026-08-02T14:30:00+09:00"

[verify]
libraries = ["libraries/rust/private.rs"]
language_id = "rust"
"#,
    )
    .unwrap();
    let config = ProjectLibraryConfigLoader::load(tmp.path()).unwrap();
    let err = LibraryDiscovery::discover(tmp.path(), &config).unwrap_err();
    assert!(
        format!("{err:#}").contains("not a public discovered library"),
        "{err:#}"
    );
}

#[test]
fn public_solution_missing_solved_at_is_rejected() {
    let tmp = copy_fixture_tree();
    let target = tmp
        .path()
        .join("solutions/librarychecker-aplusb/aplusb/main/ce.toml");
    std::fs::write(
        &target,
        r#"
language = "rust"
test_command = "./test.sh"
publish = true
"#,
    )
    .unwrap();
    let config = ProjectLibraryConfigLoader::load(tmp.path()).unwrap();
    let err = LibraryDiscovery::discover(tmp.path(), &config).unwrap_err();
    assert!(format!("{err:#}").contains("solved_at"), "{err:#}");
}

#[test]
fn unknown_solution_language_is_rejected() {
    let tmp = copy_fixture_tree();
    let target = tmp
        .path()
        .join("solutions/librarychecker-aplusb/aplusb/main/ce.toml");
    std::fs::write(
        &target,
        r#"
language = "kotlin"
test_command = "./test.sh"
publish = true
solved_at = "2026-08-02T14:30:00+09:00"

[verify]
libraries = ["libraries/rust/public.rs"]
language_id = "kotlin"
"#,
    )
    .unwrap();
    let config = ProjectLibraryConfigLoader::load(tmp.path()).unwrap();
    let err = LibraryDiscovery::discover(tmp.path(), &config).unwrap_err();
    assert!(format!("{err:#}").contains("not declared under"), "{err:#}");
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn copy_fixture_tree() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir_recursive(&fixture_root(), tmp.path()).unwrap();
    tmp
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
