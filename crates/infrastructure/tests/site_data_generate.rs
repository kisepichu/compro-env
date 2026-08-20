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
    use usecases::submission::StarterRegistry;

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
        starters: &StarterRegistry::new(),
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
    use usecases::submission::StarterRegistry;

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
        starters: &StarterRegistry::new(),
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

// ── Task 2 tests: current-fingerprint recomputation ────────────────────────

#[cfg(test)]
mod fingerprint_recomputation {
    use super::*;

    use parking_lot::Mutex;

    use anyhow::Result as AnyResult;
    use domain::analysis::{DiscoveredLanguage, LibraryFile};
    use domain::entity::{OJKind, Session};
    use domain::library::{
        AnalyzerConfig, DEFAULT_TIMEOUT_SECONDS, LanguageConfig, LanguageId, LibraryId,
        LibraryProjectConfig, SiteConfig, SolutionId,
    };
    use domain::solution::{PublishedSolution, VerifySpec};
    use domain::verification::{
        AttemptId, CompletedState, ContentHash, LanguageBinding, PlanContext, SubmissionHandle,
        SubmissionSummary, Verdict, VerdictKind, VerificationRecord, VerificationState,
        VerifyFingerprint,
    };
    use site_schema::SolutionVerificationStatus;
    use usecases::site_data_generator::{GenerateSiteData, generate_site_data};
    use usecases::submission::{
        RecoveryMode, ResultDetailLevel, StartSubmissionError, StarterRegistry,
        SubmissionAdapterDescriptor, SubmissionMode, SubmissionRequest, SubmissionStart,
        SubmissionStarter,
    };
    use usecases::verification::fingerprint::{
        AdapterIdentity, FingerprintMaterial, FingerprintSource, OjBinding, calculate_fingerprint,
        capabilities_from_descriptor, hash_verify_config,
    };

    /// Minimal starter that reports the descriptor `librarychecker` / `1.0.0`
    /// with the same capability tuple the real LC adapter declares. The
    /// starter never actually submits — site-data recomputation only calls
    /// `descriptor()`.
    struct StubStarter;

    impl SubmissionStarter for StubStarter {
        fn descriptor(&self) -> SubmissionAdapterDescriptor {
            SubmissionAdapterDescriptor {
                name: "librarychecker".into(),
                version: "1.0.0".into(),
                submission_mode: SubmissionMode::UnattendedTrackable,
                result_detail: ResultDetailLevel::TestcaseDetails,
                recovery_mode: RecoveryMode::BestEffort,
            }
        }
        fn start_submission(
            &self,
            _request: &SubmissionRequest,
            _session: Option<&Session>,
        ) -> Result<SubmissionStart, StartSubmissionError> {
            unreachable!("site-data must not call start_submission")
        }
    }

    struct SingleRecordVerifications {
        inner: Mutex<BTreeMap<SolutionId, VerificationRecord>>,
    }

    impl VerificationRepository for SingleRecordVerifications {
        fn load(&self, id: &SolutionId) -> AnyResult<Option<VerificationRecord>> {
            Ok(self.inner.lock().get(id).cloned())
        }
        fn load_all(
            &self,
            _discovered: &BTreeSet<SolutionId>,
        ) -> AnyResult<BTreeMap<SolutionId, VerificationRecord>> {
            Ok(self.inner.lock().clone())
        }
        fn compare_and_swap(
            &self,
            _id: &SolutionId,
            _expected: Option<&AttemptId>,
            _next: &VerificationRecord,
        ) -> AnyResult<()> {
            anyhow::bail!("not used")
        }
        fn remove_if_attempt(&self, _id: &SolutionId, _expected: &AttemptId) -> AnyResult<()> {
            anyhow::bail!("not used")
        }
    }

    fn fixture(
        entry_bytes: &[u8],
        library_bytes: &[u8],
    ) -> (
        tempfile::TempDir,
        PathBuf,
        LibraryProjectConfig,
        DiscoveryManifest,
    ) {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("libraries/rust/algebra")).unwrap();
        fs::write(root.join("libraries/rust/algebra/monoid.rs"), library_bytes).unwrap();
        fs::create_dir_all(root.join("solutions/librarychecker-aplusb/aplusb/rust/src")).unwrap();
        fs::write(
            root.join("solutions/librarychecker-aplusb/aplusb/rust/src/main.rs"),
            entry_bytes,
        )
        .unwrap();
        fs::write(root.join(".ce-project"), "").unwrap();

        let rust = LanguageId::parse("rust").unwrap();
        let mut lc_mapping = BTreeMap::new();
        lc_mapping.insert(
            "librarychecker".to_string(),
            domain::library::OnlineJudgeLanguageMapping {
                language_id: "rust".into(),
            },
        );
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
                online_judges: lc_mapping,
                entry_file: "src/main.rs".into(),
            },
        );
        let config = LibraryProjectConfig {
            languages,
            site: Some(SiteConfig {
                title: "compro-env".into(),
                description: "test".into(),
                language: "en".into(),
                repository_url: "https://example.test/repo".into(),
            }),
        };

        let mut manifest_langs = BTreeMap::new();
        manifest_langs.insert(
            rust.clone(),
            DiscoveredLanguage {
                id: rust.clone(),
                root: "libraries/rust".into(),
                display_name: "Rust".into(),
                description_path: None,
                analyzer_command: vec!["./bin/rust".into()],
            },
        );
        let lib_id = LibraryId::parse("libraries/rust/algebra/monoid.rs").unwrap();
        let sol_id = SolutionId::parse("librarychecker-aplusb/aplusb/rust").unwrap();
        let manifest = DiscoveryManifest {
            languages: manifest_langs,
            libraries: vec![LibraryFile {
                id: lib_id.clone(),
                language: rust.clone(),
                source_path: "libraries/rust/algebra/monoid.rs".into(),
                description_path: None,
                published: true,
                managed: true,
                title: Some("Monoid".into()),
            }],
            solutions: vec![PublishedSolution {
                id: sol_id,
                language: rust,
                root: "solutions/librarychecker-aplusb/aplusb/rust".into(),
                entry: "src/main.rs".into(),
                solved_at: DateTime::parse_from_rfc3339("2026-08-12T00:00:00+00:00").unwrap(),
                test_command: "true".into(),
                test_timeout_seconds: 600,
                verify: Some(VerifySpec {
                    libraries: vec![lib_id],
                    oj_language_id: "rust".into(),
                }),
            }],
            diagnostics: vec![],
        };

        (dir, root, config, manifest)
    }

    fn compute_expected_fingerprint(
        sol: &PublishedSolution,
        verify: &VerifySpec,
        entry_bytes: &[u8],
        library_bytes: &[u8],
    ) -> VerifyFingerprint {
        let starter = StubStarter;
        let descriptor = starter.descriptor();
        let adapter = AdapterIdentity {
            name: descriptor.name.clone(),
            version: descriptor.version.clone(),
            capabilities: capabilities_from_descriptor(&descriptor),
        };
        let mut dep_sources = BTreeMap::new();
        dep_sources.insert(
            verify.libraries[0].clone(),
            FingerprintSource {
                path: "libraries/rust/algebra/monoid.rs".into(),
                bytes: library_bytes.to_vec(),
            },
        );
        let material = FingerprintMaterial {
            solution_id: sol.id.clone(),
            raw_source: FingerprintSource {
                path: format!("{}/{}", sol.root, sol.entry),
                bytes: entry_bytes.to_vec(),
            },
            verified_libraries: verify.libraries.iter().cloned().collect(),
            dependency_library_sources: dep_sources,
            binding: OjBinding {
                oj: OJKind::LibraryChecker.as_str().to_string(),
                problem_id: sol.id.problem_code().to_string(),
                language_id: sol.language.clone(),
                oj_language_id: verify.oj_language_id.clone(),
            },
            adapter,
            verify_config_hash: hash_verify_config(verify),
        };
        calculate_fingerprint(&material).unwrap()
    }

    fn completed_record(
        sol_id: &SolutionId,
        fingerprint: VerifyFingerprint,
        submitted_source_hash: ContentHash,
    ) -> VerificationRecord {
        let binding = LanguageBinding {
            language_id: LanguageId::parse("rust").unwrap(),
            oj_language_id: "rust".into(),
        };
        let handle = SubmissionHandle {
            oj: "librarychecker".into(),
            submission_id: "1".into(),
            submission_url: "https://example.test/1".into(),
            locator: None,
            submitted_at: DateTime::parse_from_rfc3339("2026-08-16T10:59:58+00:00").unwrap(),
        };
        VerificationRecord {
            schema_version: 1,
            solution_id: sol_id.clone(),
            attempt_id: AttemptId::parse("attempt-104-1").unwrap(),
            replaces_attempt_id: None,
            fingerprint,
            state: VerificationState::Completed(CompletedState {
                verdict: Verdict {
                    kind: VerdictKind::Accepted,
                    raw: "AC".into(),
                },
                verified_libraries: vec![],
                language: binding.clone(),
                verified_at: DateTime::parse_from_rfc3339("2026-08-16T10:59:58+00:00").unwrap(),
                capabilities: capabilities_from_descriptor(&StubStarter.descriptor()),
                submitted_source_hash: submitted_source_hash.clone(),
                input_hashes: BTreeMap::new(),
                summary: SubmissionSummary {
                    max_execution_time_ms: Some(1),
                    max_memory_bytes: Some(1024),
                },
                test_cases: None,
                handle,
                extra: BTreeMap::new(),
            }),
            plan_context: Some(PlanContext {
                language: binding,
                submitted_source_hash,
            }),
        }
    }

    fn source_hash(bytes: &[u8]) -> ContentHash {
        FingerprintSource {
            path: "unused".into(),
            bytes: bytes.to_vec(),
        }
        .hash()
    }

    fn run_generate(
        root: &Path,
        config: &LibraryProjectConfig,
        manifest: &DiscoveryManifest,
        verifications: &dyn VerificationRepository,
    ) -> site_schema::SiteData {
        let git = FakeGitHistory {
            head: RepositorySnapshot {
                commit_sha: "cccccccccccccccccccccccccccccccccccccccc".into(),
                short_sha: "ccccccc".into(),
                committed_at: DateTime::parse_from_rfc3339("2026-08-16T10:00:00+00:00").unwrap(),
                uncommitted_changes: true,
            },
            per_path_sha: "cccccccccccccccccccccccccccccccccccccccc".into(),
        };
        let analyzer = FakeAnalyzer;
        let mut starters = StarterRegistry::new();
        starters.register(OJKind::LibraryChecker, Box::new(StubStarter));

        let empty_oj: BTreeMap<String, String> = BTreeMap::new();
        let empty_rel: BTreeMap<LibraryId, Vec<usecases::site_data::ProjectedRelation>> =
            BTreeMap::new();
        let empty_manual: BTreeMap<LibraryId, BTreeSet<LibraryId>> = BTreeMap::new();
        let empty_preprocess: BTreeMap<SolutionId, bool> = BTreeMap::new();
        let empty_desc: BTreeMap<LibraryId, String> = BTreeMap::new();

        let spec = GenerateSiteData {
            repository_root: root,
            config,
            manifest,
            analyzer: &analyzer,
            verifications,
            git_history: &git,
            oj_by_contest: &empty_oj,
            relations: &empty_rel,
            manual_dependency_edges: &empty_manual,
            solution_has_preprocess: &empty_preprocess,
            library_descriptions: &empty_desc,
            starters: &starters,
            mode: BuildMode::Preview,
        };
        generate_site_data(&spec).unwrap()
    }

    #[test]
    fn saved_verified_record_stays_verified_when_source_matches() {
        let entry_bytes = b"fn main() {}\n";
        let library_bytes = b"pub trait Monoid {}\n";
        let (_dir, root, config, manifest) = fixture(entry_bytes, library_bytes);

        let sol = manifest.solutions[0].clone();
        let verify = sol.verify.clone().unwrap();
        let expected_fp = compute_expected_fingerprint(&sol, &verify, entry_bytes, library_bytes);
        let record = completed_record(&sol.id, expected_fp.clone(), source_hash(entry_bytes));
        let mut store = BTreeMap::new();
        store.insert(sol.id.clone(), record);
        let verifications = SingleRecordVerifications {
            inner: Mutex::new(store),
        };

        let data = run_generate(&root, &config, &manifest, &verifications);
        assert_eq!(data.solutions.len(), 1);
        let published = &data.solutions[0];
        assert_eq!(
            published.verification.status,
            SolutionVerificationStatus::Verified,
            "expected Verified, got {:?}",
            published.verification.status
        );
    }

    #[test]
    fn saved_record_becomes_stale_when_source_bytes_change() {
        let entry_bytes = b"fn main() {}\n";
        let library_bytes = b"pub trait Monoid {}\n";
        let (dir, root, config, manifest) = fixture(entry_bytes, library_bytes);

        let sol = manifest.solutions[0].clone();
        let verify = sol.verify.clone().unwrap();
        let old_fp = compute_expected_fingerprint(&sol, &verify, entry_bytes, library_bytes);
        let record = completed_record(&sol.id, old_fp, source_hash(entry_bytes));
        let mut store = BTreeMap::new();
        store.insert(sol.id.clone(), record);
        let verifications = SingleRecordVerifications {
            inner: Mutex::new(store),
        };

        // Mutate the entry source; keep the record with the previous fp.
        fs::write(
            root.join("solutions/librarychecker-aplusb/aplusb/rust/src/main.rs"),
            b"fn main() { let _ = 1; }\n",
        )
        .unwrap();

        let data = run_generate(&root, &config, &manifest, &verifications);
        assert_eq!(data.solutions.len(), 1);
        let published = &data.solutions[0];
        assert_eq!(
            published.verification.status,
            SolutionVerificationStatus::Stale,
            "expected Stale after source drift, got {:?}",
            published.verification.status
        );

        drop(dir);
    }
}
