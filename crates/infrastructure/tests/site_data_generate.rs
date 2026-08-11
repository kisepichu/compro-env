//! Integration tests for the atomic site-data write repository and the
//! end-to-end `ce site-data generate` pipeline (spec §12, §14).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::DateTime;
use domain::analysis::DiscoveryManifest;
use domain::library::LanguageId;
use infrastructure::repository_impl::site_data_repository_impl::SiteDataRepositoryImpl;
use library_adapter_protocol::{
    AdapterIdentity as ProtoAdapterIdentity, AnalysisResponse, AnalysisState as ProtoAnalysisState,
    DependencyAnalysis as ProtoDependencyAnalysis, LibraryAnalysis as ProtoLibraryAnalysis,
    SCHEMA_VERSION as PROTOCOL_SCHEMA_VERSION, SolutionAnalysis as ProtoSolutionAnalysis,
    SymbolAnalysis as ProtoSymbolAnalysis, ToolchainIdentity as ProtoToolchainIdentity,
};
use site_schema::{BuildMetadata, BuildMode, SITE_SCHEMA_VERSION, SiteData, SiteMetadata};
use tempfile::tempdir;
use usecases::git_history::{GitHistory, PathUpdate, RepositorySnapshot};
use usecases::library_analyzer::LibraryAnalyzer;
use usecases::repository::site_data_repository::SiteDataRepository;
use usecases::repository::verification_repository::VerificationRepository;

fn sample_site_data(revision: &str) -> SiteData {
    SiteData {
        schema_version: SITE_SCHEMA_VERSION,
        build: BuildMetadata {
            schema_version: SITE_SCHEMA_VERSION,
            generated_at: "2026-08-11T12:00:00+00:00".into(),
            mode: BuildMode::Production,
            source_commit_sha: revision.into(),
            source_commit_short_sha: revision[..7.min(revision.len())].into(),
            source_committed_at: "2026-08-11T11:59:00+00:00".into(),
            uncommitted_changes: false,
            observed_toolchains: vec![],
            adapters: vec![],
        },
        site: SiteMetadata {
            title: "compro-env".into(),
            description: "d".into(),
            language: "en".into(),
            repository_url: None,
        },
        languages: vec![],
        libraries: vec![],
        solutions: vec![],
    }
}

fn read_site_data(dir: &Path) -> SiteData {
    let json = fs::read(dir.join("site-data.json")).expect("site-data.json missing");
    serde_json::from_slice(&json).expect("valid JSON")
}

#[test]
fn writes_site_data_json_atomically() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("ce-site-data");

    let repo = SiteDataRepositoryImpl::new();
    let data = sample_site_data("deadbeef00000000000000000000000000000000");
    repo.write_atomically(&output, &data).unwrap();

    assert!(output.is_dir(), "output directory created");
    assert!(output.join("site-data.json").is_file());
    let round_trip = read_site_data(&output);
    assert_eq!(
        round_trip.build.source_commit_sha,
        data.build.source_commit_sha
    );
}

#[test]
fn replaces_existing_output_directory() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("ce-site-data");

    let repo = SiteDataRepositoryImpl::new();
    let old = sample_site_data("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    repo.write_atomically(&output, &old).unwrap();

    // Add a stray file to prove the whole directory is replaced.
    fs::write(output.join("stray.txt"), b"gone").unwrap();

    let new = sample_site_data("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    repo.write_atomically(&output, &new).unwrap();

    let round_trip = read_site_data(&output);
    assert_eq!(
        round_trip.build.source_commit_sha,
        new.build.source_commit_sha
    );
    assert!(
        !output.join("stray.txt").exists(),
        "old stray file must not survive replacement"
    );
}

#[test]
fn creates_missing_parent_directories() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("nested").join("deeper").join("out");

    let repo = SiteDataRepositoryImpl::new();
    let data = sample_site_data("cccccccccccccccccccccccccccccccccccccccc");
    repo.write_atomically(&output, &data).unwrap();

    assert!(output.join("site-data.json").is_file());
}

// ─── End-to-end pipeline test using fake ports ──────────────────────────────

struct FakeAnalyzer;

impl LibraryAnalyzer for FakeAnalyzer {
    fn analyze_all(
        &self,
        _repository_root: &Path,
        manifest: &DiscoveryManifest,
    ) -> Result<BTreeMap<LanguageId, AnalysisResponse>> {
        let mut responses = BTreeMap::new();
        for lang_id in manifest.languages.keys() {
            let libraries: Vec<ProtoLibraryAnalysis> = manifest
                .libraries
                .iter()
                .filter(|lib| &lib.language == lang_id)
                .map(|lib| ProtoLibraryAnalysis {
                    path: lib.source_path.clone(),
                    dependency_analysis: ProtoDependencyAnalysis {
                        state: ProtoAnalysisState::Complete,
                        dependencies: vec![],
                    },
                    symbol_analysis: ProtoSymbolAnalysis {
                        state: ProtoAnalysisState::Complete,
                        symbols: vec![],
                    },
                    diagnostics: vec![],
                })
                .collect();
            let solutions: Vec<ProtoSolutionAnalysis> = manifest
                .solutions
                .iter()
                .filter(|sol| &sol.language == lang_id)
                .map(|sol| ProtoSolutionAnalysis {
                    id: sol.id.as_str().to_string(),
                    dependency_analysis: ProtoDependencyAnalysis {
                        state: ProtoAnalysisState::Complete,
                        dependencies: vec![],
                    },
                    diagnostics: vec![],
                })
                .collect();
            responses.insert(
                lang_id.clone(),
                AnalysisResponse {
                    schema_version: PROTOCOL_SCHEMA_VERSION,
                    adapter: ProtoAdapterIdentity {
                        name: format!("fake-{lang_id}"),
                        version: "0.0.1".into(),
                        toolchains: vec![ProtoToolchainIdentity {
                            name: lang_id.as_str().to_string(),
                            version: "1.0.0".into(),
                            target: None,
                        }],
                    },
                    libraries,
                    solutions,
                },
            );
        }
        Ok(responses)
    }
}

struct FakeGitHistory {
    head: RepositorySnapshot,
    per_path_sha: String,
}

impl GitHistory for FakeGitHistory {
    fn head_snapshot(&self) -> Result<RepositorySnapshot> {
        Ok(self.head.clone())
    }
    fn last_touched(&self, paths: &[&str]) -> Result<BTreeMap<String, PathUpdate>> {
        let mut out = BTreeMap::new();
        for p in paths {
            out.insert(
                p.to_string(),
                PathUpdate {
                    committer_time: self.head.committed_at,
                    commit_sha: self.per_path_sha.clone(),
                },
            );
        }
        Ok(out)
    }
}

struct EmptyVerifications;

impl VerificationRepository for EmptyVerifications {
    fn load(
        &self,
        _id: &domain::library::SolutionId,
    ) -> Result<Option<domain::verification::VerificationRecord>> {
        Ok(None)
    }
    fn load_all(
        &self,
        _discovered: &BTreeSet<domain::library::SolutionId>,
    ) -> Result<BTreeMap<domain::library::SolutionId, domain::verification::VerificationRecord>>
    {
        Ok(BTreeMap::new())
    }
    fn compare_and_swap(
        &self,
        _id: &domain::library::SolutionId,
        _expected: Option<&domain::verification::AttemptId>,
        _next: &domain::verification::VerificationRecord,
    ) -> Result<()> {
        anyhow::bail!("not implemented in test fake")
    }
    fn remove_if_attempt(
        &self,
        _id: &domain::library::SolutionId,
        _expected: &domain::verification::AttemptId,
    ) -> Result<()> {
        anyhow::bail!("not implemented in test fake")
    }
}

fn init_fixture_repo() -> (tempfile::TempDir, PathBuf) {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    // Language root with one library file.
    fs::create_dir_all(root.join("libraries/rust")).unwrap();
    fs::write(
        root.join("libraries/rust/monoid.rs"),
        "pub trait Monoid {}\n",
    )
    .unwrap();
    // Config
    let config = r#"
[library]

[library.site]
title = "compro-env"
description = "Test build"
language = "en"
repository_url = "https://example.test/repo"

[library.languages.rust]
root = "libraries/rust"
include = ["**/*.rs"]
exclude = []

[library.languages.rust.analyzer]
command = ["./bin/rust-analyzer"]
"#;
    fs::write(root.join("ce.toml"), config).unwrap();
    fs::write(root.join(".ce-project"), "").unwrap();
    (dir, root)
}

#[test]
fn end_to_end_pipeline_writes_projected_site_data() {
    use domain::analysis::{DiscoveredLanguage, LibraryFile};
    use domain::library::{
        AnalyzerConfig, DEFAULT_TIMEOUT_SECONDS, LanguageConfig, LanguageId, LibraryId,
        LibraryProjectConfig, SiteConfig,
    };
    use interfaces::controller::input::SiteDataBuildMode;
    use interfaces::controller::input::SiteDataGenerateInput;
    use usecases::site_data::ProjectedRelation;
    use usecases::site_data_generator::{GenerateSiteData, generate_site_data, write_site_data};

    let (_dir, root) = init_fixture_repo();
    // Build config + manifest by hand — avoids depending on the TOML loader
    // (which we exercise elsewhere).
    let rust = LanguageId::parse("rust").unwrap();
    let mut languages = BTreeMap::new();
    languages.insert(
        rust.clone(),
        LanguageConfig {
            id: rust.clone(),
            display_name: None,
            root: "libraries/rust".into(),
            include: vec!["**/*.rs".into()],
            exclude: vec![],
            check_command: None,
            check_timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            syntax_highlight: None,
            analyzer: AnalyzerConfig {
                command: vec!["./bin/rust".into()],
                timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            },
            expected_toolchains: vec![],
            online_judges: BTreeMap::new(),
            entry_file: "src/main.rs".into(),
        },
    );
    let config = LibraryProjectConfig {
        languages,
        site: Some(SiteConfig {
            title: "compro-env".into(),
            description: "Test build".into(),
            language: "en".into(),
            repository_url: "https://example.test/repo".into(),
        }),
    };

    let mut manifest_languages = BTreeMap::new();
    manifest_languages.insert(
        rust.clone(),
        DiscoveredLanguage {
            id: rust.clone(),
            root: "libraries/rust".into(),
            display_name: "Rust".into(),
            description_path: None,
            analyzer_command: vec!["./bin/rust".into()],
        },
    );
    let manifest = DiscoveryManifest {
        languages: manifest_languages,
        libraries: vec![LibraryFile {
            id: LibraryId::parse("libraries/rust/monoid.rs").unwrap(),
            language: rust.clone(),
            source_path: "libraries/rust/monoid.rs".into(),
            description_path: None,
            published: true,
            managed: true,
            title: Some("Monoid".into()),
        }],
        solutions: vec![],
        diagnostics: vec![],
    };

    let git = FakeGitHistory {
        head: RepositorySnapshot {
            commit_sha: "deadbeef00000000000000000000000000000000".into(),
            short_sha: "deadbee".into(),
            committed_at: DateTime::parse_from_rfc3339("2026-08-11T11:59:00+00:00").unwrap(),
            uncommitted_changes: false,
        },
        per_path_sha: "cafebabecafebabecafebabecafebabecafebabe".into(),
    };
    let analyzer = FakeAnalyzer;
    let verifications = EmptyVerifications;

    let empty_oj: BTreeMap<String, String> = BTreeMap::new();
    let empty_rel: BTreeMap<LibraryId, Vec<ProjectedRelation>> = BTreeMap::new();
    let empty_manual: BTreeMap<LibraryId, BTreeSet<LibraryId>> = BTreeMap::new();
    let empty_preprocess: BTreeMap<domain::library::SolutionId, bool> = BTreeMap::new();
    let empty_desc: BTreeMap<LibraryId, String> = BTreeMap::new();

    let spec = GenerateSiteData {
        repository_root: &root,
        config: &config,
        manifest: &manifest,
        analyzer: &analyzer,
        verifications: &verifications,
        git_history: &git,
        oj_by_contest: &empty_oj,
        relations: &empty_rel,
        manual_dependency_edges: &empty_manual,
        solution_has_preprocess: &empty_preprocess,
        library_descriptions: &empty_desc,
        mode: BuildMode::Production,
    };
    let data = generate_site_data(&spec).unwrap();
    assert_eq!(data.libraries.len(), 1);
    assert_eq!(data.libraries[0].library_id, "libraries/rust/monoid.rs");
    assert_eq!(
        data.build.source_commit_sha,
        "deadbeef00000000000000000000000000000000"
    );
    assert_eq!(data.build.mode, BuildMode::Production);

    let output = root.join("target").join("ce-site-data");
    let repo = SiteDataRepositoryImpl::new();
    write_site_data(&repo, &output, &data).unwrap();
    let raw = fs::read(output.join("site-data.json")).unwrap();
    let round_trip: SiteData = serde_json::from_slice(&raw).unwrap();
    assert_eq!(round_trip, data);

    // Type-check that the CLI input trait is wired.
    fn _assert_input(_i: &dyn SiteDataGenerateInput) {}
    struct D;
    impl SiteDataGenerateInput for D {
        fn output(&self) -> Option<String> {
            None
        }
        fn mode(&self) -> SiteDataBuildMode {
            SiteDataBuildMode::Production
        }
    }
    _assert_input(&D);
}

#[test]
fn production_mode_rejects_uncommitted_tree() {
    use domain::analysis::DiscoveredLanguage;
    use domain::library::{
        AnalyzerConfig, DEFAULT_TIMEOUT_SECONDS, LanguageConfig, LanguageId, LibraryProjectConfig,
        SiteConfig,
    };
    use usecases::site_data_generator::{GenerateSiteData, generate_site_data};

    let (_dir, root) = init_fixture_repo();
    let rust = LanguageId::parse("rust").unwrap();
    let mut languages = BTreeMap::new();
    languages.insert(
        rust.clone(),
        LanguageConfig {
            id: rust.clone(),
            display_name: None,
            root: "libraries/rust".into(),
            include: vec!["**/*.rs".into()],
            exclude: vec![],
            check_command: None,
            check_timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            syntax_highlight: None,
            analyzer: AnalyzerConfig {
                command: vec!["./bin/rust".into()],
                timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            },
            expected_toolchains: vec![],
            online_judges: BTreeMap::new(),
            entry_file: "src/main.rs".into(),
        },
    );
    let config = LibraryProjectConfig {
        languages,
        site: Some(SiteConfig {
            title: "t".into(),
            description: "d".into(),
            language: "en".into(),
            repository_url: "https://example.test/repo".into(),
        }),
    };
    let mut mlangs = BTreeMap::new();
    mlangs.insert(
        rust.clone(),
        DiscoveredLanguage {
            id: rust.clone(),
            root: "libraries/rust".into(),
            display_name: "Rust".into(),
            description_path: None,
            analyzer_command: vec!["./bin/rust".into()],
        },
    );
    let manifest = DiscoveryManifest {
        languages: mlangs,
        libraries: vec![],
        solutions: vec![],
        diagnostics: vec![],
    };
    let git = FakeGitHistory {
        head: RepositorySnapshot {
            commit_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            short_sha: "aaaaaaa".into(),
            committed_at: DateTime::parse_from_rfc3339("2026-08-11T11:59:00+00:00").unwrap(),
            uncommitted_changes: true,
        },
        per_path_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
    };
    let analyzer = FakeAnalyzer;
    let verifications = EmptyVerifications;
    let empty_oj: BTreeMap<String, String> = BTreeMap::new();
    let empty_rel = BTreeMap::new();
    let empty_manual = BTreeMap::new();
    let empty_preprocess = BTreeMap::new();
    let empty_desc = BTreeMap::new();
    let spec = GenerateSiteData {
        repository_root: &root,
        config: &config,
        manifest: &manifest,
        analyzer: &analyzer,
        verifications: &verifications,
        git_history: &git,
        oj_by_contest: &empty_oj,
        relations: &empty_rel,
        manual_dependency_edges: &empty_manual,
        solution_has_preprocess: &empty_preprocess,
        library_descriptions: &empty_desc,
        mode: BuildMode::Production,
    };
    let err = generate_site_data(&spec).unwrap_err();
    assert!(
        err.to_string().contains("clean working tree"),
        "unexpected error: {err}"
    );
}

#[test]
fn output_json_ends_with_newline_for_readability() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("out");
    let repo = SiteDataRepositoryImpl::new();
    let data = sample_site_data("dddddddddddddddddddddddddddddddddddddddd");
    repo.write_atomically(&output, &data).unwrap();

    let raw = fs::read(output.join("site-data.json")).unwrap();
    assert_eq!(*raw.last().unwrap(), b'\n');
}
