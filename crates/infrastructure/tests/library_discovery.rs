//! Integration tests for `LibraryDiscovery` (spec §6.1).

use std::path::{Path, PathBuf};

use domain::analysis::DiscoverySeverity;
use domain::library::LanguageId;
use infrastructure::library_project::config::ProjectLibraryConfigLoader;
use infrastructure::library_project::discovery::LibraryDiscovery;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("library-project")
}

fn load_valid_config() -> domain::library::LibraryProjectConfig {
    ProjectLibraryConfigLoader::load(&fixture_root()).unwrap()
}

#[test]
fn discovers_all_managed_libraries_in_utf8_order() {
    let config = load_valid_config();
    let manifest = LibraryDiscovery::discover(&fixture_root(), &config).unwrap();

    let ids: Vec<&str> = manifest.libraries.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "libraries/cpp/monoid.hpp",
            "libraries/lean/Monoid.lean",
            "libraries/rust/private.rs",
            "libraries/rust/public.rs",
        ]
    );
}

#[test]
fn private_file_stays_managed_but_unpublished() {
    let config = load_valid_config();
    let manifest = LibraryDiscovery::discover(&fixture_root(), &config).unwrap();
    let private = manifest
        .libraries
        .iter()
        .find(|l| l.id.as_str() == "libraries/rust/private.rs")
        .unwrap();
    assert!(private.managed);
    assert!(!private.published);

    let public = manifest
        .libraries
        .iter()
        .find(|l| l.id.as_str() == "libraries/rust/public.rs")
        .unwrap();
    assert!(public.published);
    assert_eq!(
        public.description_path.as_deref(),
        Some("libraries/rust/public.rs.md")
    );
    assert!(public.title.as_deref() == Some("Public marker"));
}

#[test]
fn sidecar_markdown_is_not_treated_as_source() {
    let config = load_valid_config();
    let manifest = LibraryDiscovery::discover(&fixture_root(), &config).unwrap();
    for library in &manifest.libraries {
        assert!(
            !library.id.as_str().ends_with(".md"),
            "unexpected sidecar treated as source: {}",
            library.id
        );
    }
}

#[test]
fn missing_root_returns_error() {
    let mut config = load_valid_config();
    // Rewrite one language's root to a nonexistent directory.
    let rust = LanguageId::parse("rust").unwrap();
    config.languages.get_mut(&rust).unwrap().root = "does-not-exist/rust".into();
    let err = LibraryDiscovery::discover(&fixture_root(), &config).unwrap_err();
    assert!(format!("{err:#}").contains("root not found"), "{err:#}");
}

#[test]
fn empty_language_produces_warning_diagnostic() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("libraries/rust")).unwrap();
    std::fs::create_dir_all(tmp.path().join("libraries/cpp")).unwrap();
    std::fs::create_dir_all(tmp.path().join("libraries/lean")).unwrap();
    let src = fixture_root().join("config.toml");
    std::fs::copy(&src, tmp.path().join("config.toml")).unwrap();
    let config = ProjectLibraryConfigLoader::load(tmp.path()).unwrap();

    let manifest = LibraryDiscovery::discover(tmp.path(), &config).unwrap();
    assert!(manifest.libraries.is_empty());
    assert!(manifest
        .diagnostics
        .iter()
        .any(|d| d.code == "empty_language" && matches!(d.severity, DiscoverySeverity::Warning)));
}

#[test]
fn orphan_sidecar_is_reported_as_error_diagnostic() {
    let tmp = copy_fixture_tree();
    // Add a sidecar without a sibling source.
    let orphan = tmp.path().join("libraries/rust/ghost.rs.md");
    std::fs::write(&orphan, "+++\ntitle = \"Ghost\"\n+++\n").unwrap();

    let config = ProjectLibraryConfigLoader::load(tmp.path()).unwrap();
    let manifest = LibraryDiscovery::discover(tmp.path(), &config).unwrap();

    assert!(
        manifest
            .diagnostics
            .iter()
            .any(|d| d.code == "orphan_sidecar" && matches!(d.severity, DiscoverySeverity::Error)),
        "diagnostics: {:?}",
        manifest.diagnostics
    );
}

#[test]
fn results_are_stable_when_walking_order_differs() {
    // Discovery must sort by UTF-8 bytes regardless of underlying filesystem
    // ordering. Adding files in a different order should not change the
    // manifest.
    let a = discover_ids_from(&fixture_root());

    let tmp = copy_fixture_tree();
    // Create additional files in reverse order and delete them again to
    // ensure filesystem inode ordering differs from the fixture directory.
    let extra_dir = tmp.path().join("libraries/rust/tmp-inode-shift");
    std::fs::create_dir_all(&extra_dir).unwrap();
    for i in (0..5).rev() {
        let name = format!("z{i}.rs");
        std::fs::write(extra_dir.join(&name), "").unwrap();
    }
    for i in 0..5 {
        let name = format!("z{i}.rs");
        std::fs::remove_file(extra_dir.join(&name)).unwrap();
    }
    std::fs::remove_dir(&extra_dir).unwrap();

    let b = discover_ids_from(tmp.path());
    assert_eq!(a, b);
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

fn discover_ids_from(root: &Path) -> Vec<String> {
    let config = ProjectLibraryConfigLoader::load(root).unwrap();
    let manifest = LibraryDiscovery::discover(root, &config).unwrap();
    manifest
        .libraries
        .into_iter()
        .map(|l| l.id.as_str().to_string())
        .collect()
}
