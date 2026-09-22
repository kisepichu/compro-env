//! Acceptance fixture for plan 061 (library platform activation).
//!
//! Ensures the on-disk production config, the three-language library sources,
//! the librarychecker-aplusb Rust solution, and the representative accepted
//! verification record are all wired together and cross-referenced correctly.

use std::path::{Path, PathBuf};

use domain::verification::{VerdictKind, VerificationRecord, VerificationState};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repository root")
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("library-platform")
}

fn read_fixture(name: &str) -> VerificationRecord {
    let path = fixture_dir().join("verification").join(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
}

fn assert_repo_file(relative: &str) {
    let path = repository_root().join(relative);
    assert!(
        path.is_file(),
        "expected production repo file at {}",
        path.display()
    );
}

#[test]
fn production_config_and_libraries_are_present() {
    assert_repo_file("config.toml");
    assert_repo_file("libraries/rust/algebra/monoid.rs");
    assert_repo_file("libraries/rust/algebra/monoid.rs.md");
    assert_repo_file("libraries/cpp/algebra/monoid.hpp");
    assert_repo_file("libraries/cpp/algebra/monoid.hpp.md");
    assert_repo_file("libraries/lean/Algebra/Monoid.lean");
    assert_repo_file("libraries/lean/Algebra/Monoid.lean.md");
    assert_repo_file("solutions/librarychecker-aplusb/aplusb/rust/ce.toml");
    assert_repo_file("solutions/librarychecker-aplusb/aplusb/rust/src/main.rs");
}

#[test]
fn config_toml_configures_all_languages_and_librarychecker_mapping() {
    // Issue #106 re-enabled cpp / lean once the adapter env plumbing landed.
    // All three languages must appear with their `librarychecker`
    // language-id bindings so `ce site-data generate` and library-analyzer
    // fan-out cover the full site.
    let root = repository_root();
    let config = infrastructure::library_project::config::ProjectLibraryConfigLoader::load(&root)
        .expect("root config.toml loads");
    let langs: Vec<&str> = config.languages.keys().map(|k| k.as_str()).collect();
    assert_eq!(
        langs.len(),
        3,
        "expected rust/cpp/lean in config.toml, got {langs:?}"
    );
    for expected in ["rust", "cpp", "lean"] {
        assert!(
            langs.contains(&expected),
            "missing `{expected}` language: {langs:?}"
        );
    }

    for (lang_id, expected_root) in [
        ("rust", "libraries/rust"),
        ("cpp", "libraries/cpp"),
        ("lean", "libraries/lean"),
    ] {
        let id = domain::library::LanguageId::parse(lang_id).unwrap();
        let cfg = config
            .languages
            .get(&id)
            .unwrap_or_else(|| panic!("missing config for {lang_id}"));
        assert_eq!(cfg.root, expected_root);
        assert!(
            !cfg.expected_toolchains.is_empty(),
            "{lang_id} must declare pinned toolchains"
        );
        assert!(
            cfg.online_judges.contains_key("librarychecker"),
            "{lang_id} must map to a librarychecker language id"
        );
    }

    let site = config.site.expect("site config present");
    assert_eq!(site.language, "en");
    assert!(site.repository_url.starts_with("https://"));
}

#[test]
fn accepted_fixture_matches_platform_solution() {
    let record = read_fixture("accepted.json");
    assert_eq!(
        record.solution_id.as_str(),
        "librarychecker-aplusb/aplusb/rust"
    );
    let VerificationState::Completed(state) = &record.state else {
        panic!("expected completed state");
    };
    assert_eq!(state.verdict.kind, VerdictKind::Accepted);
    let verified: Vec<&str> = state
        .verified_libraries
        .iter()
        .map(|l| l.as_str())
        .collect();
    assert!(
        verified.contains(&"libraries/rust/algebra/monoid.rs"),
        "verified_libraries should include the platform monoid: {verified:?}"
    );
    assert_eq!(state.handle.oj, "librarychecker");
    assert_eq!(state.language.language_id.as_str(), "rust");
    assert_eq!(state.language.oj_language_id, "rust");

    // Referenced repo paths exist on disk so drifting the fixture requires
    // touching the platform files, not just the JSON.
    for lib in &state.verified_libraries {
        let p: &Path = Path::new(lib.as_str());
        assert!(
            repository_root().join(p).is_file(),
            "verified library referenced by fixture is missing from disk: {}",
            lib.as_str()
        );
    }
}
