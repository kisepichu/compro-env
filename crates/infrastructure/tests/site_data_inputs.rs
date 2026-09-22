//! Integration tests for the caller-supplied `ce site-data generate` inputs
//! (spec §5.1, §6.5, §12): sidecar bodies, declared relations, manual
//! dependency edges, the per-solution preprocess flag, and contest → OJ names.

use std::path::{Path, PathBuf};

use domain::analysis::DiscoveryManifest;
use domain::library::{LibraryId, LibraryProjectConfig, SolutionId};
use infrastructure::library_project::config::ProjectLibraryConfigLoader;
use infrastructure::library_project::discovery::LibraryDiscovery;
use infrastructure::library_project::site_inputs::SiteDataInputs;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("library-project")
}

fn discover(root: &Path) -> (LibraryProjectConfig, DiscoveryManifest) {
    let config = ProjectLibraryConfigLoader::load(root).unwrap();
    let manifest = LibraryDiscovery::discover(root, &config).unwrap();
    (config, manifest)
}

fn collect(root: &Path, submit_preprocess: Option<&str>) -> anyhow::Result<SiteDataInputs> {
    let (_config, manifest) = discover(root);
    SiteDataInputs::collect(root, &manifest, submit_preprocess)
}

fn lib(id: &str) -> LibraryId {
    LibraryId::parse(id).unwrap()
}

/// Minimal single-language project so error cases can be written inline.
/// `libraries/rust` holds `a.rs` and `b.rs`; `libraries/cpp` holds `c.hpp`.
fn temp_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "config.toml",
        r#"
[library.site]
title = "test"
description = "test"
language = "en"
repository_url = "https://example.test/repo"

[library.languages.rust]
root = "libraries/rust"
include = ["**/*.rs"]

[library.languages.rust.analyzer]
command = ["./bin/rust-analyzer"]

[library.languages.cpp]
root = "libraries/cpp"
include = ["**/*.hpp"]

[library.languages.cpp.analyzer]
command = ["./bin/cpp-analyzer"]
"#,
    );
    write(root, "libraries/rust/a.rs", "pub struct A;\n");
    write(root, "libraries/rust/b.rs", "pub struct B;\n");
    write(root, "libraries/cpp/c.hpp", "#pragma once\n");
    dir
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// Writes a publishable solution under `solutions/<contest>/<problem>/<name>`.
fn write_solution(root: &Path, contest: &str, problem: &str, name: &str) {
    write(
        root,
        &format!("solutions/{contest}/{problem}/{name}/ce.toml"),
        r#"
language = "rust"
test_command = "./test.sh"
publish = true
solved_at = "2026-08-02T14:30:00+09:00"
"#,
    );
}

// ─── library_descriptions ───────────────────────────────────────────────────

#[test]
fn sidecar_body_lands_in_library_descriptions_without_frontmatter() {
    let inputs = collect(&fixture_root(), None).unwrap();
    assert_eq!(
        inputs
            .library_descriptions
            .get(&lib("libraries/rust/public.rs"))
            .map(String::as_str),
        Some("Public documentation for the marker."),
    );
}

#[test]
fn library_without_sidecar_has_no_description() {
    let inputs = collect(&fixture_root(), None).unwrap();
    assert!(
        !inputs
            .library_descriptions
            .contains_key(&lib("libraries/cpp/monoid.hpp")),
        "unexpected description: {:?}",
        inputs.library_descriptions,
    );
}

#[test]
fn sidecar_with_frontmatter_only_has_no_description() {
    let dir = temp_project();
    write(
        dir.path(),
        "libraries/rust/a.rs.md",
        "+++\ntitle = \"A\"\n+++\n",
    );
    let inputs = collect(dir.path(), None).unwrap();
    assert!(
        !inputs
            .library_descriptions
            .contains_key(&lib("libraries/rust/a.rs")),
        "unexpected description: {:?}",
        inputs.library_descriptions,
    );
}

// ─── relations ──────────────────────────────────────────────────────────────

#[test]
fn declared_relations_are_collected_per_library() {
    let inputs = collect(&fixture_root(), None).unwrap();
    let relations = inputs
        .relations
        .get(&lib("libraries/rust/public.rs"))
        .unwrap();
    assert_eq!(relations.len(), 1);
    assert_eq!(relations[0].kind, "companion");
    assert_eq!(relations[0].target, lib("libraries/rust/private.rs"));
    // `manual` marks hand-added dependency edges (spec §6.5), not declarations.
    assert!(!relations[0].manual);
}

#[test]
fn relation_to_unknown_library_is_rejected() {
    let dir = temp_project();
    write(
        dir.path(),
        "libraries/rust/a.rs.md",
        "+++\n[[relations]]\nkind = \"impl\"\nto = \"libraries/rust/ghost.rs\"\n+++\n",
    );
    let err = collect(dir.path(), None).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("libraries/rust/ghost.rs"), "{msg}");
}

#[test]
fn self_relation_is_rejected() {
    let dir = temp_project();
    write(
        dir.path(),
        "libraries/rust/a.rs.md",
        "+++\n[[relations]]\nkind = \"impl\"\nto = \"libraries/rust/a.rs\"\n+++\n",
    );
    let err = collect(dir.path(), None).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("itself"), "{msg}");
}

#[test]
fn duplicate_relation_is_rejected() {
    let dir = temp_project();
    write(
        dir.path(),
        "libraries/rust/a.rs.md",
        "+++\n\
         [[relations]]\nkind = \"impl\"\nto = \"libraries/rust/b.rs\"\n\
         [[relations]]\nkind = \"impl\"\nto = \"libraries/rust/b.rs\"\n+++\n",
    );
    let err = collect(dir.path(), None).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("duplicate relation"), "{msg}");
}

// ─── dependency overrides ───────────────────────────────────────────────────

#[test]
fn add_override_becomes_a_manual_dependency_edge() {
    let dir = temp_project();
    write(
        dir.path(),
        "libraries/rust/a.rs.md",
        "+++\n[[dependency_overrides]]\naction = \"add\"\n\
         to = \"libraries/rust/b.rs\"\nreason = \"macro-generated\"\n+++\n",
    );
    let inputs = collect(dir.path(), None).unwrap();
    let edges = inputs
        .manual_dependency_edges
        .get(&lib("libraries/rust/a.rs"))
        .unwrap();
    assert!(edges.contains(&lib("libraries/rust/b.rs")), "{edges:?}");
}

#[test]
fn cross_language_add_override_is_rejected() {
    let dir = temp_project();
    write(
        dir.path(),
        "libraries/rust/a.rs.md",
        "+++\n[[dependency_overrides]]\naction = \"add\"\n\
         to = \"libraries/cpp/c.hpp\"\nreason = \"macro-generated\"\n+++\n",
    );
    let err = collect(dir.path(), None).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("same language"), "{msg}");
}

#[test]
fn remove_override_is_rejected_as_unsupported() {
    let dir = temp_project();
    write(
        dir.path(),
        "libraries/rust/a.rs.md",
        "+++\n[[dependency_overrides]]\naction = \"remove\"\n\
         to = \"libraries/rust/b.rs\"\nreason = \"false positive\"\n+++\n",
    );
    let err = collect(dir.path(), None).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("remove"), "{msg}");
    assert!(msg.contains("not supported"), "{msg}");
}

// ─── solution_has_preprocess ────────────────────────────────────────────────

#[test]
fn preprocess_flag_follows_the_configured_hook() {
    let root = fixture_root();
    let id = SolutionId::parse("librarychecker-aplusb/aplusb/main").unwrap();

    let without = collect(&root, None).unwrap();
    assert_eq!(without.solution_has_preprocess.get(&id), Some(&false));

    let with = collect(&root, Some("hooks/expand-libraries.sh")).unwrap();
    assert_eq!(with.solution_has_preprocess.get(&id), Some(&true));
}

// ─── oj_by_contest ──────────────────────────────────────────────────────────

#[test]
fn contest_ce_toml_wins_over_the_contest_id_convention() {
    let dir = temp_project();
    write_solution(dir.path(), "abc999", "a", "main");
    write(
        dir.path(),
        "solutions/abc999/.ce.toml",
        "online_judge = \"librarychecker\"\ncontest_id = \"abc999\"\n",
    );
    let inputs = collect(dir.path(), None).unwrap();
    assert_eq!(
        inputs.oj_by_contest.get("abc999"),
        Some(&"librarychecker".to_string()),
    );
}

#[test]
fn oj_by_contest_falls_back_to_contest_id_detection() {
    let dir = temp_project();
    write_solution(dir.path(), "abc999", "a", "main");
    write_solution(dir.path(), "handmade", "a", "main");
    let inputs = collect(dir.path(), None).unwrap();

    // No `.ce.toml` anywhere: `abc999` is recognisable, `handmade` is not and
    // stays absent so the projection can degrade to "unknown".
    assert_eq!(
        inputs.oj_by_contest.get("abc999"),
        Some(&"atcoder".to_string()),
    );
    assert_eq!(inputs.oj_by_contest.get("handmade"), None);
}
