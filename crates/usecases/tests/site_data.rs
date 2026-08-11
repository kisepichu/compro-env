//! Integration tests for `project_site_data` (spec §12, §14).
//!
//! The fixture spans all three MVP languages so cross-language relations,
//! transitive closure walks, and language sort order are covered by every
//! test.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use domain::analysis::{
    AnalysisSnapshot, AnalysisState, DiagnosticSeverity, DiscoveredLanguage, DiscoveryManifest,
    LibraryFile, NormalizedDiagnostic, NormalizedLanguageAnalysis, NormalizedLibraryAnalysis,
    NormalizedSolutionAnalysis, NormalizedSymbol, SourceLocation, TargetAnalysisState,
};
use domain::library::{
    AnalyzerConfig, DEFAULT_TIMEOUT_SECONDS, ExpectedToolchain, LanguageConfig, LanguageId,
    LibraryId, LibraryProjectConfig, SiteConfig, SolutionId,
};
use domain::online_judge::{RecoveryMode, ResultDetail, SubmissionCapabilities, SubmissionMode};
use domain::solution::{PublishedSolution, VerifySpec};
use domain::verification::{
    AttemptId, CompletedState, ContentHash, LanguageBinding, SubmissionHandle, SubmissionSummary,
    UnavailableReason, UnavailableState, Verdict, VerdictKind, VerificationRecord,
    VerificationState, VerifyFingerprint,
};
use site_schema::{
    BuildMode, EvidenceStatus, LibraryVerificationStatus, SITE_SCHEMA_VERSION,
    SolutionVerificationStatus,
};
use usecases::site_data::{
    BuildContext, LibraryGitUpdate, ProjectedRelation, PublicProjectionInput, SiteDataError,
    project_site_data,
};

// ─── Fixture builders ───────────────────────────────────────────────────────

fn fp(byte: u8) -> VerifyFingerprint {
    let hex = std::iter::repeat_n(format!("{byte:02x}"), 32).collect::<String>();
    VerifyFingerprint::parse(&format!("sha256:{hex}")).unwrap()
}

fn content_hash(byte: u8) -> ContentHash {
    let hex = std::iter::repeat_n(format!("{byte:02x}"), 32).collect::<String>();
    ContentHash::parse(&format!("sha256:{hex}")).unwrap()
}

fn capabilities() -> SubmissionCapabilities {
    SubmissionCapabilities {
        submission_mode: SubmissionMode::UnattendedTrackable,
        result_detail: ResultDetail::TestcaseDetails,
        recovery_mode: RecoveryMode::BestEffort,
    }
}

fn handle(oj: &str, url: &str) -> SubmissionHandle {
    SubmissionHandle {
        oj: oj.into(),
        submission_id: "sub1".into(),
        submission_url: url.into(),
        locator: None,
        submitted_at: DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap(),
    }
}

fn language_config(id: &str, root: &str, syntax: &str) -> LanguageConfig {
    LanguageConfig {
        id: LanguageId::parse(id).unwrap(),
        display_name: None,
        root: root.into(),
        include: vec!["**/*".into()],
        exclude: vec![],
        check_command: None,
        check_timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        syntax_highlight: Some(syntax.into()),
        analyzer: AnalyzerConfig {
            command: vec![format!("./bin/{id}")],
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        },
        expected_toolchains: vec![],
        online_judges: BTreeMap::new(),
        entry_file: "src/main.rs".into(),
    }
}

fn library(id: &str, language: &str, published: bool, title: Option<&str>) -> LibraryFile {
    LibraryFile {
        id: LibraryId::parse(id).unwrap(),
        language: LanguageId::parse(language).unwrap(),
        source_path: id.to_string(),
        description_path: None,
        published,
        managed: true,
        title: title.map(str::to_string),
    }
}

fn library_analysis(
    id: &str,
    deps: &[&str],
    dep_state: AnalysisState,
    symbols: Vec<NormalizedSymbol>,
    diagnostics: Vec<NormalizedDiagnostic>,
) -> NormalizedLibraryAnalysis {
    NormalizedLibraryAnalysis {
        id: LibraryId::parse(id).unwrap(),
        state: TargetAnalysisState {
            dependency_state: dep_state,
            symbol_state: AnalysisState::Complete,
        },
        direct_dependencies: deps.iter().map(|d| LibraryId::parse(d).unwrap()).collect(),
        symbols,
        diagnostics,
    }
}

fn solution_analysis(
    id: &str,
    deps: &[&str],
    state: AnalysisState,
    diagnostics: Vec<NormalizedDiagnostic>,
) -> NormalizedSolutionAnalysis {
    NormalizedSolutionAnalysis {
        solution_id: SolutionId::parse(id).unwrap(),
        dependency_state: state,
        direct_dependencies: deps.iter().map(|d| LibraryId::parse(d).unwrap()).collect(),
        diagnostics,
    }
}

fn solution(id: &str, language: &str, verify: Option<VerifySpec>) -> PublishedSolution {
    let sid = SolutionId::parse(id).unwrap();
    let root = format!("solutions/{}", sid.as_str());
    PublishedSolution {
        id: sid,
        language: LanguageId::parse(language).unwrap(),
        root,
        entry: "src/main.rs".into(),
        solved_at: DateTime::parse_from_rfc3339("2026-08-01T14:30:00+09:00").unwrap(),
        test_command: "./test.sh".into(),
        test_timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        verify,
    }
}

fn git_update(byte: u8) -> LibraryGitUpdate {
    LibraryGitUpdate {
        updated_at: DateTime::parse_from_rfc3339("2026-08-05T09:00:00+00:00").unwrap(),
        source_commit_sha: std::iter::repeat_n(format!("{byte:02x}"), 20).collect::<String>(),
    }
}

fn build_context() -> BuildContext {
    BuildContext {
        mode: BuildMode::Production,
        generated_at: DateTime::parse_from_rfc3339("2026-08-11T12:00:00+00:00").unwrap(),
        source_commit_sha: "deadbeef00000000000000000000000000000000".into(),
        source_commit_short_sha: "deadbee".into(),
        source_committed_at: DateTime::parse_from_rfc3339("2026-08-11T11:59:00+00:00").unwrap(),
        uncommitted_changes: false,
    }
}

fn site_config() -> SiteConfig {
    SiteConfig {
        title: "compro-env".into(),
        description: "Competitive programming libraries and solutions".into(),
        language: "en".into(),
        repository_url: "https://github.com/example/compro-env".into(),
    }
}

/// Build a mixed 3-language fixture with private/public libraries, an
/// override-driven manual edge, a dependency cycle, verified/rejected/never/
/// stale/unavailable/not_configured solutions, and cross-language relations.
struct Fixture {
    config: LibraryProjectConfig,
    manifest: DiscoveryManifest,
    snapshot: AnalysisSnapshot,
    verifications: BTreeMap<SolutionId, VerificationRecord>,
    current_fingerprints:
        BTreeMap<SolutionId, Result<VerifyFingerprint, usecases::verification::FingerprintError>>,
    library_sources: BTreeMap<LibraryId, Vec<u8>>,
    library_descriptions: BTreeMap<LibraryId, String>,
    library_updates: BTreeMap<LibraryId, LibraryGitUpdate>,
    solution_sources: BTreeMap<SolutionId, Vec<u8>>,
    solution_has_preprocess: BTreeMap<SolutionId, bool>,
    oj_by_contest: BTreeMap<String, String>,
    relations: BTreeMap<LibraryId, Vec<ProjectedRelation>>,
    manual_dependency_edges: BTreeMap<LibraryId, BTreeSet<LibraryId>>,
    build: BuildContext,
}

fn build_fixture() -> Fixture {
    // Languages
    let mut languages = BTreeMap::new();
    languages.insert(
        LanguageId::parse("rust").unwrap(),
        language_config("rust", "libraries/rust", "rust"),
    );
    languages.insert(
        LanguageId::parse("cpp").unwrap(),
        language_config("cpp", "libraries/cpp", "cpp"),
    );
    languages.insert(
        LanguageId::parse("lean").unwrap(),
        language_config("lean", "libraries/lean", "lean"),
    );

    let config = LibraryProjectConfig {
        languages,
        site: Some(site_config()),
    };

    // Library files
    let monoid = library(
        "libraries/rust/algebra/monoid.rs",
        "rust",
        true,
        Some("Monoid"),
    );
    let magma = library(
        "libraries/rust/algebra/magma.rs",
        "rust",
        true,
        Some("Magma"),
    );
    let private_helper = library(
        "libraries/rust/algebra/helper.rs",
        "rust",
        false,
        Some("Helper"),
    );
    let bit_hpp = library(
        "libraries/cpp/data_structures/bit.hpp",
        "cpp",
        true,
        Some("BIT"),
    );
    let group_lean = library(
        "libraries/lean/Algebra/Group.lean",
        "lean",
        true,
        Some("Group"),
    );

    // Cycle: monoid ⇄ magma via manual override on magma → monoid.
    let mut manual_dependency_edges: BTreeMap<LibraryId, BTreeSet<LibraryId>> = BTreeMap::new();
    manual_dependency_edges.insert(
        LibraryId::parse("libraries/rust/algebra/magma.rs").unwrap(),
        BTreeSet::from([LibraryId::parse("libraries/rust/algebra/monoid.rs").unwrap()]),
    );

    // Analyses
    let symbol_monoid = NormalizedSymbol {
        name: "Monoid".into(),
        kind: "trait".into(),
        qualified_name: Some("algebra::Monoid".into()),
        search_names: vec!["Monoid".into()],
        signature: Some("pub trait Monoid".into()),
        location: Some(SourceLocation {
            path: "libraries/rust/algebra/monoid.rs".into(),
            start_line: 1,
            start_column: Some(1),
            end_line: None,
            end_column: None,
        }),
    };
    let monoid_analysis = library_analysis(
        "libraries/rust/algebra/monoid.rs",
        &[
            "libraries/rust/algebra/magma.rs",
            "libraries/rust/algebra/helper.rs",
        ],
        AnalysisState::Complete,
        vec![symbol_monoid],
        vec![],
    );
    let magma_analysis = library_analysis(
        "libraries/rust/algebra/magma.rs",
        // adapter reports no direct edge; manual override adds monoid.
        &[],
        AnalysisState::Complete,
        vec![],
        vec![],
    );
    let helper_analysis = library_analysis(
        "libraries/rust/algebra/helper.rs",
        &[],
        AnalysisState::Complete,
        vec![],
        vec![],
    );

    let bit_analysis = library_analysis(
        "libraries/cpp/data_structures/bit.hpp",
        &[],
        AnalysisState::Complete,
        vec![],
        vec![NormalizedDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: "cpp.symbol.partial".into(),
            message: "Some declarations were skipped.".into(),
            location: Some(SourceLocation {
                path: "libraries/cpp/data_structures/bit.hpp".into(),
                start_line: 5,
                start_column: Some(1),
                end_line: None,
                end_column: None,
            }),
        }],
    );

    let group_analysis = library_analysis(
        "libraries/lean/Algebra/Group.lean",
        &[],
        AnalysisState::Failed,
        vec![],
        vec![NormalizedDiagnostic {
            severity: DiagnosticSeverity::Error,
            code: "lean.header.failed".into(),
            message: "Failed to parse module header.".into(),
            location: None,
        }],
    );

    // Solutions
    let verified_solution = solution(
        "abc999/a/verified",
        "rust",
        Some(VerifySpec {
            libraries: vec![LibraryId::parse("libraries/rust/algebra/monoid.rs").unwrap()],
            oj_language_id: "rust".into(),
        }),
    );
    let rejected_solution = solution(
        "abc999/b/rejected",
        "rust",
        Some(VerifySpec {
            libraries: vec![LibraryId::parse("libraries/rust/algebra/magma.rs").unwrap()],
            oj_language_id: "rust".into(),
        }),
    );
    let unavail_solution = solution(
        "abc999/c/unavailable",
        "cpp",
        Some(VerifySpec {
            libraries: vec![LibraryId::parse("libraries/cpp/data_structures/bit.hpp").unwrap()],
            oj_language_id: "cpp".into(),
        }),
    );
    let stale_solution = solution(
        "abc999/d/stale",
        "cpp",
        Some(VerifySpec {
            libraries: vec![],
            oj_language_id: "cpp".into(),
        }),
    );
    let never_solution = solution(
        "abc999/e/never",
        "lean",
        Some(VerifySpec {
            libraries: vec![LibraryId::parse("libraries/lean/Algebra/Group.lean").unwrap()],
            oj_language_id: "lean".into(),
        }),
    );
    let not_configured_solution = solution("abc999/f/manual", "rust", None);

    let solutions = vec![
        verified_solution,
        rejected_solution,
        unavail_solution,
        stale_solution,
        never_solution,
        not_configured_solution,
    ];

    // Solution analyses
    let sol_analyses = [
        solution_analysis(
            "abc999/a/verified",
            &["libraries/rust/algebra/monoid.rs"],
            AnalysisState::Complete,
            vec![],
        ),
        solution_analysis(
            "abc999/b/rejected",
            &["libraries/rust/algebra/magma.rs"],
            AnalysisState::Complete,
            vec![],
        ),
        solution_analysis(
            "abc999/c/unavailable",
            &["libraries/cpp/data_structures/bit.hpp"],
            AnalysisState::Complete,
            vec![NormalizedDiagnostic {
                severity: DiagnosticSeverity::Info,
                code: "cpp.deps.info".into(),
                message: "Diagnostic in non-entry file.".into(),
                location: Some(SourceLocation {
                    path: "solutions/abc999/c/unavailable/other.cpp".into(),
                    start_line: 3,
                    start_column: None,
                    end_line: None,
                    end_column: None,
                }),
            }],
        ),
        solution_analysis("abc999/d/stale", &[], AnalysisState::Complete, vec![]),
        solution_analysis(
            "abc999/e/never",
            &["libraries/lean/Algebra/Group.lean"],
            AnalysisState::Complete,
            vec![],
        ),
        solution_analysis("abc999/f/manual", &[], AnalysisState::Complete, vec![]),
    ];

    // Build snapshot
    let mut libraries_by_lang: BTreeMap<
        LanguageId,
        BTreeMap<LibraryId, NormalizedLibraryAnalysis>,
    > = BTreeMap::new();
    libraries_by_lang
        .entry(LanguageId::parse("rust").unwrap())
        .or_default()
        .extend([
            (monoid_analysis.id.clone(), monoid_analysis.clone()),
            (magma_analysis.id.clone(), magma_analysis.clone()),
            (helper_analysis.id.clone(), helper_analysis.clone()),
        ]);
    libraries_by_lang
        .entry(LanguageId::parse("cpp").unwrap())
        .or_default()
        .insert(bit_analysis.id.clone(), bit_analysis.clone());
    libraries_by_lang
        .entry(LanguageId::parse("lean").unwrap())
        .or_default()
        .insert(group_analysis.id.clone(), group_analysis.clone());

    let mut solutions_by_lang: BTreeMap<
        LanguageId,
        BTreeMap<SolutionId, NormalizedSolutionAnalysis>,
    > = BTreeMap::new();
    let route = |sol_id: &str| -> LanguageId {
        match sol_id.contains("/e/") {
            true => LanguageId::parse("lean").unwrap(),
            false => match sol_id.contains("/c/") || sol_id.contains("/d/") {
                true => LanguageId::parse("cpp").unwrap(),
                false => LanguageId::parse("rust").unwrap(),
            },
        }
    };
    for sa in sol_analyses {
        let key = route(sa.solution_id.as_str());
        solutions_by_lang
            .entry(key)
            .or_default()
            .insert(sa.solution_id.clone(), sa);
    }

    let mut snapshot_languages: BTreeMap<LanguageId, NormalizedLanguageAnalysis> = BTreeMap::new();
    for lang_id in [
        LanguageId::parse("cpp").unwrap(),
        LanguageId::parse("lean").unwrap(),
        LanguageId::parse("rust").unwrap(),
    ] {
        snapshot_languages.insert(
            lang_id.clone(),
            NormalizedLanguageAnalysis {
                language: lang_id.clone(),
                adapter_name: format!("compro-env-{lang_id}-analyzer"),
                adapter_version: "1.0.0".into(),
                observed_toolchains: vec![ExpectedToolchain {
                    name: match lang_id.as_str() {
                        "rust" => "rustc".into(),
                        "cpp" => "clang".into(),
                        _ => "lean".into(),
                    },
                    version: "1.0.0".into(),
                }],
                analyzer_command: vec![format!("./bin/{lang_id}")],
                libraries: libraries_by_lang.remove(&lang_id).unwrap_or_default(),
                solutions: solutions_by_lang.remove(&lang_id).unwrap_or_default(),
            },
        );
    }

    let snapshot = AnalysisSnapshot {
        schema_version: 1,
        repository_revision: "deadbeef00000000000000000000000000000000".into(),
        created_at: DateTime::parse_from_rfc3339("2026-08-11T11:00:00+00:00").unwrap(),
        discovery_hash: "d".into(),
        source_hashes: BTreeMap::new(),
        languages: snapshot_languages,
        snapshot_hash: "h".into(),
    };

    // Manifest
    let mut manifest_languages = BTreeMap::new();
    for lang in ["cpp", "lean", "rust"] {
        let id = LanguageId::parse(lang).unwrap();
        manifest_languages.insert(
            id.clone(),
            DiscoveredLanguage {
                id: id.clone(),
                root: format!("libraries/{lang}"),
                display_name: match lang {
                    "cpp" => "C++".into(),
                    "lean" => "Lean".into(),
                    _ => "Rust".into(),
                },
                description_path: None,
                analyzer_command: vec![format!("./bin/{lang}")],
            },
        );
    }
    let manifest = DiscoveryManifest {
        languages: manifest_languages,
        libraries: vec![monoid, magma, private_helper, bit_hpp, group_lean],
        solutions,
        diagnostics: vec![],
    };

    // Sources
    let sources = |id: &str, body: &str| (LibraryId::parse(id).unwrap(), body.as_bytes().to_vec());
    let library_sources: BTreeMap<LibraryId, Vec<u8>> = [
        sources("libraries/rust/algebra/monoid.rs", "pub trait Monoid {}\n"),
        sources("libraries/rust/algebra/magma.rs", "pub trait Magma {}\n"),
        sources("libraries/rust/algebra/helper.rs", "fn helper() {}\n"),
        sources("libraries/cpp/data_structures/bit.hpp", "#pragma once\n"),
        sources("libraries/lean/Algebra/Group.lean", "import Init\n"),
    ]
    .into_iter()
    .collect();

    let mut library_descriptions = BTreeMap::new();
    library_descriptions.insert(
        LibraryId::parse("libraries/rust/algebra/monoid.rs").unwrap(),
        "Monoid trait.".to_string(),
    );

    let mut library_updates = BTreeMap::new();
    for (i, id) in [
        "libraries/rust/algebra/monoid.rs",
        "libraries/rust/algebra/magma.rs",
        "libraries/rust/algebra/helper.rs",
        "libraries/cpp/data_structures/bit.hpp",
        "libraries/lean/Algebra/Group.lean",
    ]
    .iter()
    .enumerate()
    {
        library_updates.insert(LibraryId::parse(id).unwrap(), git_update((i + 1) as u8));
    }

    let mut solution_sources = BTreeMap::new();
    for sid in [
        "abc999/a/verified",
        "abc999/b/rejected",
        "abc999/c/unavailable",
        "abc999/d/stale",
        "abc999/e/never",
        "abc999/f/manual",
    ] {
        solution_sources.insert(SolutionId::parse(sid).unwrap(), b"fn main(){}\n".to_vec());
    }

    let mut solution_has_preprocess = BTreeMap::new();
    solution_has_preprocess.insert(SolutionId::parse("abc999/f/manual").unwrap(), true);

    let mut oj_by_contest = BTreeMap::new();
    oj_by_contest.insert("abc999".into(), "librarychecker".into());

    // Relations: monoid --impl--> magma (published target only).
    let mut relations: BTreeMap<LibraryId, Vec<ProjectedRelation>> = BTreeMap::new();
    relations.insert(
        LibraryId::parse("libraries/rust/algebra/monoid.rs").unwrap(),
        vec![ProjectedRelation {
            kind: "impl".into(),
            target: LibraryId::parse("libraries/rust/algebra/magma.rs").unwrap(),
            manual: false,
        }],
    );

    // Verifications
    let mut verifications: BTreeMap<SolutionId, VerificationRecord> = BTreeMap::new();
    let mut current_fingerprints: BTreeMap<
        SolutionId,
        Result<VerifyFingerprint, usecases::verification::FingerprintError>,
    > = BTreeMap::new();

    // Verified — fingerprint matches, Accepted.
    let verified_id = SolutionId::parse("abc999/a/verified").unwrap();
    let verified_fp = fp(1);
    verifications.insert(
        verified_id.clone(),
        make_completed_record(&verified_id, verified_fp.clone(), VerdictKind::Accepted),
    );
    current_fingerprints.insert(verified_id.clone(), Ok(verified_fp));

    // Rejected — fingerprint matches, WrongAnswer.
    let rejected_id = SolutionId::parse("abc999/b/rejected").unwrap();
    let rejected_fp = fp(2);
    verifications.insert(
        rejected_id.clone(),
        make_completed_record(&rejected_id, rejected_fp.clone(), VerdictKind::WrongAnswer),
    );
    current_fingerprints.insert(rejected_id.clone(), Ok(rejected_fp));

    // Unavailable — fingerprint matches.
    let unavail_id = SolutionId::parse("abc999/c/unavailable").unwrap();
    let unavail_fp = fp(3);
    verifications.insert(
        unavail_id.clone(),
        VerificationRecord {
            schema_version: 1,
            solution_id: unavail_id.clone(),
            attempt_id: AttemptId::parse("att-unavail").unwrap(),
            replaces_attempt_id: None,
            fingerprint: unavail_fp.clone(),
            state: VerificationState::Unavailable(UnavailableState {
                reason: UnavailableReason::InteractiveUntrackable,
                capabilities: capabilities(),
                observed_at: DateTime::parse_from_rfc3339("2026-08-05T10:00:00+00:00").unwrap(),
                summary: "interactive".into(),
            }),
        },
    );
    current_fingerprints.insert(unavail_id.clone(), Ok(unavail_fp));

    // Stale — saved record with old fp, current fp differs.
    let stale_id = SolutionId::parse("abc999/d/stale").unwrap();
    verifications.insert(
        stale_id.clone(),
        make_completed_record(&stale_id, fp(4), VerdictKind::Accepted),
    );
    current_fingerprints.insert(stale_id.clone(), Ok(fp(44)));

    // Never — no record, spec has verify libraries but no submission.
    let never_id = SolutionId::parse("abc999/e/never").unwrap();
    current_fingerprints.insert(never_id.clone(), Ok(fp(5)));

    // Not configured — no verify spec.
    let manual_id = SolutionId::parse("abc999/f/manual").unwrap();
    current_fingerprints.insert(manual_id.clone(), Ok(fp(6)));

    Fixture {
        config,
        manifest,
        snapshot,
        verifications,
        current_fingerprints,
        library_sources,
        library_descriptions,
        library_updates,
        solution_sources,
        solution_has_preprocess,
        oj_by_contest,
        relations,
        manual_dependency_edges,
        build: build_context(),
    }
}

fn make_completed_record(
    solution: &SolutionId,
    fingerprint: VerifyFingerprint,
    kind: VerdictKind,
) -> VerificationRecord {
    VerificationRecord {
        schema_version: 1,
        solution_id: solution.clone(),
        attempt_id: AttemptId::parse(&format!("att-{}", solution.solution_name())).unwrap(),
        replaces_attempt_id: None,
        fingerprint,
        state: VerificationState::Completed(CompletedState {
            verdict: Verdict {
                kind,
                raw: format!("{kind:?}"),
            },
            verified_libraries: vec![],
            language: LanguageBinding {
                language_id: LanguageId::parse("rust").unwrap(),
                oj_language_id: "rust".into(),
            },
            verified_at: DateTime::parse_from_rfc3339("2026-08-06T09:00:00+00:00").unwrap(),
            capabilities: capabilities(),
            submitted_source_hash: content_hash(0x11),
            input_hashes: BTreeMap::new(),
            summary: SubmissionSummary {
                max_execution_time_ms: Some(120),
                max_memory_bytes: Some(2048 * 1024),
            },
            test_cases: None,
            handle: handle(
                "librarychecker",
                &format!(
                    "https://judge.yosupo.jp/submission/{}",
                    solution.solution_name()
                ),
            ),
            extra: BTreeMap::new(),
        }),
    }
}

fn as_input<'a>(fx: &'a Fixture) -> PublicProjectionInput<'a> {
    PublicProjectionInput {
        config: &fx.config,
        manifest: &fx.manifest,
        snapshot: &fx.snapshot,
        verifications: &fx.verifications,
        current_fingerprints: &fx.current_fingerprints,
        library_sources: &fx.library_sources,
        library_descriptions: &fx.library_descriptions,
        library_updates: &fx.library_updates,
        solution_sources: &fx.solution_sources,
        solution_has_preprocess: &fx.solution_has_preprocess,
        oj_by_contest: &fx.oj_by_contest,
        relations: &fx.relations,
        manual_dependency_edges: &fx.manual_dependency_edges,
        build: &fx.build,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[test]
fn projection_produces_expected_top_level_shape() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).expect("projection succeeds");

    assert_eq!(data.schema_version, SITE_SCHEMA_VERSION);
    assert_eq!(data.build.schema_version, SITE_SCHEMA_VERSION);
    assert_eq!(data.build.source_commit_sha, fx.build.source_commit_sha);
    assert_eq!(data.build.mode, BuildMode::Production);
    assert_eq!(data.site.title, "compro-env");

    let language_ids: Vec<&str> = data.languages.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(
        language_ids,
        vec!["cpp", "lean", "rust"],
        "sorted by UTF-8 bytes"
    );
    assert_eq!(data.libraries.len(), 4, "only 4 published libraries");
    assert_eq!(data.solutions.len(), 6);
}

#[test]
fn private_libraries_never_appear_in_public_projection() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    let ids: Vec<&str> = data
        .libraries
        .iter()
        .map(|l| l.library_id.as_str())
        .collect();
    assert!(!ids.contains(&"libraries/rust/algebra/helper.rs"));
    // No dependency edge, relation, reverse edge, or evidence link should
    // mention the private helper.
    let raw = serde_json::to_string(&data).unwrap();
    assert!(!raw.contains("helper.rs"), "private helper leaked: {raw}");
}

#[test]
fn monoid_has_private_dep_flag_and_public_direct_link() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    let monoid = data
        .libraries
        .iter()
        .find(|l| l.library_id == "libraries/rust/algebra/monoid.rs")
        .expect("monoid published");

    let direct_ids: Vec<&str> = monoid
        .dependency_analysis
        .direct
        .iter()
        .map(|l| l.library_id.as_str())
        .collect();
    assert_eq!(direct_ids, vec!["libraries/rust/algebra/magma.rs"]);
    assert!(monoid.dependency_analysis.has_private_dependencies);
}

#[test]
fn manual_override_edge_marks_link_as_manual() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    // Fixture: manual override adds magma → monoid.
    let magma = data
        .libraries
        .iter()
        .find(|l| l.library_id == "libraries/rust/algebra/magma.rs")
        .unwrap();
    let link = magma
        .dependency_analysis
        .direct
        .iter()
        .find(|l| l.library_id == "libraries/rust/algebra/monoid.rs")
        .expect("manual edge present");
    assert!(link.manual, "manual override should mark direct link");

    // Non-manual edges: monoid → magma is discovered by the analyzer, not
    // manual, so its link stays `manual = false`.
    let monoid = data
        .libraries
        .iter()
        .find(|l| l.library_id == "libraries/rust/algebra/monoid.rs")
        .unwrap();
    let non_manual = monoid
        .dependency_analysis
        .direct
        .iter()
        .find(|l| l.library_id == "libraries/rust/algebra/magma.rs")
        .unwrap();
    assert!(!non_manual.manual);
}

#[test]
fn cycle_between_magma_and_monoid_terminates() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    let magma = data
        .libraries
        .iter()
        .find(|l| l.library_id == "libraries/rust/algebra/magma.rs")
        .unwrap();
    // Manual edge injected magma → monoid; monoid → magma comes from adapter.
    let direct_ids: Vec<&str> = magma
        .dependency_analysis
        .direct
        .iter()
        .map(|l| l.library_id.as_str())
        .collect();
    assert_eq!(direct_ids, vec!["libraries/rust/algebra/monoid.rs"]);
}

#[test]
fn reverse_dependencies_only_include_public_sources() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    let magma = data
        .libraries
        .iter()
        .find(|l| l.library_id == "libraries/rust/algebra/magma.rs")
        .unwrap();
    let rev_ids: Vec<&str> = magma
        .reverse_dependencies
        .iter()
        .map(|l| l.library_id.as_str())
        .collect();
    assert_eq!(rev_ids, vec!["libraries/rust/algebra/monoid.rs"]);
}

#[test]
fn verification_statuses_reflect_all_variants() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    let by_id: BTreeMap<&str, &site_schema::SolutionPageData> = data
        .solutions
        .iter()
        .map(|s| (s.solution_id.as_str(), s))
        .collect();
    assert_eq!(
        by_id["abc999/a/verified"].verification.status,
        SolutionVerificationStatus::Verified
    );
    assert_eq!(
        by_id["abc999/b/rejected"].verification.status,
        SolutionVerificationStatus::Rejected
    );
    assert_eq!(
        by_id["abc999/c/unavailable"].verification.status,
        SolutionVerificationStatus::Unavailable
    );
    assert_eq!(
        by_id["abc999/d/stale"].verification.status,
        SolutionVerificationStatus::Stale
    );
    assert_eq!(
        by_id["abc999/e/never"].verification.status,
        SolutionVerificationStatus::Never
    );
    assert_eq!(
        by_id["abc999/f/manual"].verification.status,
        SolutionVerificationStatus::NotConfigured
    );
}

#[test]
fn library_with_no_direct_verifiers_reports_never() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    let bit = data
        .libraries
        .iter()
        .find(|l| l.library_id == "libraries/cpp/data_structures/bit.hpp")
        .unwrap();
    // c/unavailable *does* verify bit.hpp, so status should be Unavailable, not Never.
    assert_eq!(
        bit.verification.aggregate_status,
        LibraryVerificationStatus::Unavailable
    );
    let never_lib = fx
        .manifest
        .libraries
        .iter()
        .find(|l| l.published && l.id.as_str() == "libraries/lean/Algebra/Group.lean")
        .unwrap();
    let _ = never_lib;
    // Group.lean is verified by never solution → aggregate should be `Never`.
    let group = data
        .libraries
        .iter()
        .find(|l| l.library_id == "libraries/lean/Algebra/Group.lean")
        .unwrap();
    assert_eq!(
        group.verification.aggregate_status,
        LibraryVerificationStatus::Never
    );
}

#[test]
fn stale_solution_carries_public_stale_reason_without_paths() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    let stale = data
        .solutions
        .iter()
        .find(|s| s.solution_id == "abc999/d/stale")
        .unwrap();
    let result = stale
        .verification
        .result
        .as_ref()
        .expect("stale keeps result");
    let reason = result.stale_reason.as_deref().expect("stale reason set");
    assert!(!reason.contains('/'), "stale reason must not name paths");
    assert!(!reason.contains("libraries"));
}

#[test]
fn diagnostics_in_non_entry_solution_files_lose_location_but_keep_notice() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    let sol = data
        .solutions
        .iter()
        .find(|s| s.solution_id == "abc999/c/unavailable")
        .unwrap();
    let diag = &sol.diagnostics[0];
    assert!(diag.location.is_none(), "location stripped");
    let notice = diag.location_notice.as_deref().expect("notice populated");
    assert!(notice.contains("non-displayed"));
}

#[test]
fn symbols_expose_location_only_on_own_file() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    let monoid = data
        .libraries
        .iter()
        .find(|l| l.library_id == "libraries/rust/algebra/monoid.rs")
        .unwrap();
    assert_eq!(monoid.symbol_analysis.symbols.len(), 1);
    let sym = &monoid.symbol_analysis.symbols[0];
    assert!(sym.location.is_some());
    assert_eq!(sym.name, "Monoid");
}

#[test]
fn projection_is_byte_stable_across_runs() {
    let fx = build_fixture();
    let a = serde_json::to_vec(&project_site_data(as_input(&fx)).unwrap()).unwrap();
    let b = serde_json::to_vec(&project_site_data(as_input(&fx)).unwrap()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn projection_matches_checked_in_schema() {
    // The schema drift test lives in site-schema; here we only assert that
    // the projected output serializes without deny-unknown-fields errors and
    // survives a round trip.
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    let json = serde_json::to_string(&data).unwrap();
    let _: site_schema::SiteData = serde_json::from_str(&json).unwrap();
}

#[test]
fn dep_state_failed_projects_to_failed_analysis_state() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    let group = data
        .libraries
        .iter()
        .find(|l| l.library_id == "libraries/lean/Algebra/Group.lean")
        .unwrap();
    assert_eq!(
        group.dependency_analysis.state,
        site_schema::AnalysisState::Failed
    );
}

#[test]
fn manual_edge_target_that_does_not_exist_is_rejected() {
    let mut fx = build_fixture();
    fx.manual_dependency_edges.insert(
        LibraryId::parse("libraries/rust/algebra/monoid.rs").unwrap(),
        BTreeSet::from([LibraryId::parse("libraries/rust/algebra/missing.rs").unwrap()]),
    );
    let err = project_site_data(as_input(&fx)).unwrap_err();
    assert!(
        matches!(err, SiteDataError::UnknownDependencyTarget { .. }),
        "got: {err:?}"
    );
}

#[test]
fn missing_site_config_fails_production() {
    let mut fx = build_fixture();
    fx.config.site = None;
    let err = project_site_data(as_input(&fx)).unwrap_err();
    assert_eq!(err, SiteDataError::MissingProductionSiteConfig);
}

#[test]
fn preview_mode_allows_missing_site_config() {
    let mut fx = build_fixture();
    fx.config.site = None;
    fx.build.mode = BuildMode::Preview;
    let data = project_site_data(as_input(&fx)).unwrap();
    assert_eq!(data.site.title, "");
    assert!(data.site.repository_url.is_none());
}

#[test]
fn languages_summarize_verification_state_counts_publicly() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    let rust = data.languages.iter().find(|l| l.id == "rust").unwrap();
    assert_eq!(rust.library_count, 2);
    let cpp = data.languages.iter().find(|l| l.id == "cpp").unwrap();
    assert_eq!(cpp.library_count, 1);
    let lean = data.languages.iter().find(|l| l.id == "lean").unwrap();
    assert_eq!(lean.library_count, 1);
}

#[test]
fn evidence_uses_public_status_variants_only() {
    let fx = build_fixture();
    let data = project_site_data(as_input(&fx)).unwrap();
    let magma = data
        .libraries
        .iter()
        .find(|l| l.library_id == "libraries/rust/algebra/magma.rs")
        .unwrap();
    let evidence_statuses: Vec<EvidenceStatus> = magma
        .verification
        .evidence
        .iter()
        .map(|e| e.status)
        .collect();
    assert!(evidence_statuses.contains(&EvidenceStatus::Rejected));
}

// Convince the compiler `Utc` import is used.
fn _touch_utc(_t: &chrono::DateTime<Utc>) {}
