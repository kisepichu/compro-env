//! Schema drift and privacy tests for `site-schema`.

use site_schema::schema::{find_forbidden_key, forbidden_key, serialize_schema, site_data_schema};
use site_schema::{
    AdapterIdentity, AnalysisState, BuildMetadata, BuildMode, DependencyAnalysisPublic,
    DiagnosticPublic, DiagnosticSeverity, EvidenceStatus, LanguageSummary, LibraryLink,
    LibraryPageData, LibraryVerificationStatus, LibraryVerificationView, LinePosition,
    PublicLocation, RelationPublic, SITE_SCHEMA_VERSION, SiteData, SiteMetadata,
    SolutionDiagnosticPublic, SolutionPageData, SolutionVerificationStatus,
    SolutionVerificationView, SymbolAnalysisPublic, SymbolPublic, TestcaseVerdictPublic,
    ToolchainIdentity, VerificationCounts, VerificationEvidence, VerificationResultPublic,
};

const CHECKED_IN_SCHEMA: &str = include_str!("../../../web/schema/site-data-v1.schema.json");

#[test]
fn checked_in_schema_matches_generated_schema() {
    let generated = String::from_utf8(serialize_schema(&site_data_schema()))
        .expect("schema serializes to valid UTF-8");
    if generated != CHECKED_IN_SCHEMA {
        panic!(
            "web/schema/site-data-v1.schema.json is stale. Regenerate via \
             `cargo run -p ce --bin site-data-schema` or update the schema binary. \
             First diff around:\n---generated---\n{}\n---checked-in---\n{}\n",
            &generated[..generated.len().min(600)],
            &CHECKED_IN_SCHEMA[..CHECKED_IN_SCHEMA.len().min(600)],
        );
    }
}

#[test]
fn schema_advertises_current_version_and_denies_unknown_fields() {
    let schema = serde_json::to_value(site_data_schema()).unwrap();
    assert_eq!(
        schema["title"].as_str().unwrap(),
        format!("compro-env site-data v{SITE_SCHEMA_VERSION}"),
    );
    let defs = schema["$defs"].as_object().expect("$defs exists");
    let site_data = defs.get("SiteData").expect("SiteData in $defs");
    assert_eq!(site_data["additionalProperties"], serde_json::json!(false));
}

#[test]
fn empty_site_data_round_trips() {
    let data = SiteData {
        schema_version: SITE_SCHEMA_VERSION,
        build: BuildMetadata {
            schema_version: SITE_SCHEMA_VERSION,
            generated_at: "2026-08-11T12:00:00+00:00".into(),
            mode: BuildMode::Production,
            source_commit_sha: "0000000000000000000000000000000000000000".into(),
            source_commit_short_sha: "0000000".into(),
            source_committed_at: "2026-08-11T11:59:00+00:00".into(),
            uncommitted_changes: false,
            observed_toolchains: vec![],
            adapters: vec![],
        },
        site: SiteMetadata {
            title: "compro-env".into(),
            description: "Competitive programming libraries and solutions".into(),
            language: "en".into(),
            repository_url: Some("https://github.com/example/compro-env".into()),
        },
        languages: vec![],
        libraries: vec![],
        solutions: vec![],
    };

    let encoded = serde_json::to_value(&data).unwrap();
    let decoded: SiteData = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(decoded, data);
    assert!(find_forbidden_key(&encoded).is_none());
}

#[test]
fn deny_unknown_fields_is_strict() {
    let json = serde_json::json!({
        "schema_version": SITE_SCHEMA_VERSION,
        "build": {
            "schema_version": SITE_SCHEMA_VERSION,
            "generated_at": "2026-08-11T12:00:00+00:00",
            "mode": "production",
            "source_commit_sha": "0000000000000000000000000000000000000000",
            "source_commit_short_sha": "0000000",
            "source_committed_at": "2026-08-11T11:59:00+00:00",
            "uncommitted_changes": false,
            "observed_toolchains": [],
            "adapters": [],
            "extra_key": "leak"
        },
        "site": {
            "title": "t", "description": "d", "language": "en"
        },
        "languages": [], "libraries": [], "solutions": []
    });
    let err = serde_json::from_value::<SiteData>(json).unwrap_err();
    assert!(
        err.to_string().contains("extra_key"),
        "unknown field must be rejected, got: {err}",
    );
}

#[test]
fn mixed_fixture_serializes_and_hides_private_paths() {
    let data = mixed_fixture();
    let encoded = serde_json::to_value(&data).unwrap();

    // Round-trip.
    let decoded: SiteData = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(decoded, data);

    // No forbidden keys.
    assert!(
        find_forbidden_key(&encoded).is_none(),
        "forbidden key leaked into fixture",
    );

    // No value contains the private library id/path or session token payload.
    let text = encoded.to_string();
    assert!(
        !text.contains("libraries/rust/private_helper.rs"),
        "private library id leaked into public projection",
    );
    assert!(
        !text.contains("SECRET_SESSION"),
        "raw OJ session leaked into public projection",
    );
}

#[test]
fn stale_evidence_carries_only_public_reason() {
    let data = mixed_fixture();
    let stale = data
        .libraries
        .iter()
        .find(|lib| lib.library_id == "libraries/rust/algebra/monoid.rs")
        .expect("monoid library present");
    let evidence = stale
        .verification
        .evidence
        .iter()
        .find(|e| e.status == EvidenceStatus::Stale)
        .expect("has stale evidence");
    let reason = evidence
        .stale_reason
        .as_deref()
        .expect("stale has a reason");
    assert!(!reason.contains('/'), "stale reason must not contain paths");
    assert!(
        !reason.contains("private"),
        "stale reason must not name privacy leaks"
    );
}

#[test]
fn library_with_no_verifiers_reports_never_not_verified() {
    let data = mixed_fixture();
    let orphan = data
        .libraries
        .iter()
        .find(|lib| lib.library_id == "libraries/cpp/data_structures/bit.hpp")
        .expect("cpp orphan library present");
    assert_eq!(
        orphan.verification.aggregate_status,
        LibraryVerificationStatus::Never,
    );
    assert!(orphan.verification.evidence.is_empty());
}

#[test]
fn solution_verifications_expose_status_variants() {
    let data = mixed_fixture();
    let statuses: Vec<SolutionVerificationStatus> = data
        .solutions
        .iter()
        .map(|s| s.verification.status)
        .collect();
    assert!(statuses.contains(&SolutionVerificationStatus::Verified));
    assert!(statuses.contains(&SolutionVerificationStatus::Rejected));
    assert!(statuses.contains(&SolutionVerificationStatus::Unavailable));
    assert!(statuses.contains(&SolutionVerificationStatus::Stale));
    assert!(statuses.contains(&SolutionVerificationStatus::Never));
    assert!(statuses.contains(&SolutionVerificationStatus::NotConfigured));
}

#[test]
fn diagnostic_severities_serialize_lowercase() {
    for (severity, expected) in [
        (DiagnosticSeverity::Info, "\"info\""),
        (DiagnosticSeverity::Warning, "\"warning\""),
        (DiagnosticSeverity::Error, "\"error\""),
    ] {
        assert_eq!(serde_json::to_string(&severity).unwrap(), expected);
    }
}

#[test]
fn analysis_state_serializes_lowercase() {
    for (state, expected) in [
        (AnalysisState::Complete, "\"complete\""),
        (AnalysisState::Partial, "\"partial\""),
        (AnalysisState::Failed, "\"failed\""),
    ] {
        assert_eq!(serde_json::to_string(&state).unwrap(), expected);
    }
}

#[test]
fn forbidden_key_denylist_covers_common_leaks() {
    for leak in [
        "private",
        "token",
        "session",
        "cookie",
        "authorization",
        "raw_oj_response",
        "raw_response",
        "absolute_path",
        "internal_path",
    ] {
        assert!(forbidden_key(leak), "expected `{leak}` in denylist");
    }
    for benign in ["dependencies", "solution_id", "verdict"] {
        assert!(!forbidden_key(benign), "`{benign}` must not be forbidden");
    }
}

// ─── Fixture builder ─────────────────────────────────────────────────────────

fn mixed_fixture() -> SiteData {
    let monoid = LibraryPageData {
        page_id: "library:libraries/rust/algebra/monoid.rs".into(),
        library_id: "libraries/rust/algebra/monoid.rs".into(),
        language: "rust".into(),
        title: "Monoid".into(),
        source_path: "libraries/rust/algebra/monoid.rs".into(),
        source: "pub trait Monoid {}\n".into(),
        syntax_highlight: "rust".into(),
        updated_at: "2026-08-10T09:00:00+09:00".into(),
        updated_by_commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        description: Some("Additive monoid trait.".into()),
        symbol_analysis: SymbolAnalysisPublic {
            state: AnalysisState::Complete,
            symbols: vec![SymbolPublic {
                kind: "trait".into(),
                name: "Monoid".into(),
                qualified_name: Some("algebra::Monoid".into()),
                search_names: vec!["Monoid".into(), "algebra::Monoid".into()],
                signature: Some("pub trait Monoid".into()),
                location: Some(PublicLocation {
                    start: LinePosition {
                        line: 1,
                        column: Some(1),
                    },
                    end: None,
                }),
            }],
        },
        dependency_analysis: DependencyAnalysisPublic {
            state: AnalysisState::Complete,
            direct: vec![LibraryLink {
                library_id: "libraries/rust/algebra/magma.rs".into(),
                language: "rust".into(),
                title: "Magma".into(),
                source_path: "libraries/rust/algebra/magma.rs".into(),
                manual: false,
            }],
            transitive: vec![],
            has_private_dependencies: false,
        },
        reverse_dependencies: vec![],
        relations: vec![RelationPublic {
            kind: "impl".into(),
            target: LibraryLink {
                library_id: "libraries/rust/algebra/magma.rs".into(),
                language: "rust".into(),
                title: "Magma".into(),
                source_path: "libraries/rust/algebra/magma.rs".into(),
                manual: false,
            },
            manual: false,
        }],
        verification: LibraryVerificationView {
            aggregate_status: LibraryVerificationStatus::Stale,
            evidence: vec![VerificationEvidence {
                solution_id: "abc999/a/monoid".into(),
                solution_page_id: "solution:abc999/a/monoid".into(),
                online_judge: "librarychecker".into(),
                status: EvidenceStatus::Stale,
                verdict: Some("Accepted".into()),
                judged_at: Some("2026-08-01T00:00:00+00:00".into()),
                oj_submission_url: Some("https://judge.yosupo.jp/submission/12345".into()),
                stale_reason: Some(
                    "Source or dependencies changed since the last submission.".into(),
                ),
            }],
        },
        diagnostics: vec![],
    };

    let bit_hpp = LibraryPageData {
        page_id: "library:libraries/cpp/data_structures/bit.hpp".into(),
        library_id: "libraries/cpp/data_structures/bit.hpp".into(),
        language: "cpp".into(),
        title: "Binary Indexed Tree".into(),
        source_path: "libraries/cpp/data_structures/bit.hpp".into(),
        source: "#pragma once\n".into(),
        syntax_highlight: "cpp".into(),
        updated_at: "2026-07-30T10:00:00+09:00".into(),
        updated_by_commit: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        description: None,
        symbol_analysis: SymbolAnalysisPublic {
            state: AnalysisState::Partial,
            symbols: vec![],
        },
        dependency_analysis: DependencyAnalysisPublic {
            state: AnalysisState::Complete,
            direct: vec![],
            transitive: vec![],
            has_private_dependencies: true,
        },
        reverse_dependencies: vec![],
        relations: vec![],
        verification: LibraryVerificationView {
            aggregate_status: LibraryVerificationStatus::Never,
            evidence: vec![],
        },
        diagnostics: vec![DiagnosticPublic {
            severity: DiagnosticSeverity::Warning,
            code: "cpp.symbol.partial".into(),
            message: "Some declarations were skipped.".into(),
            location: None,
        }],
    };

    let lean_theorem = LibraryPageData {
        page_id: "library:libraries/lean/Algebra/Group.lean".into(),
        library_id: "libraries/lean/Algebra/Group.lean".into(),
        language: "lean".into(),
        title: "Group".into(),
        source_path: "libraries/lean/Algebra/Group.lean".into(),
        source: "import Mathlib\n".into(),
        syntax_highlight: "lean".into(),
        updated_at: "2026-08-05T11:00:00+09:00".into(),
        updated_by_commit: "cccccccccccccccccccccccccccccccccccccccc".into(),
        description: Some("Group axioms.".into()),
        symbol_analysis: SymbolAnalysisPublic {
            state: AnalysisState::Complete,
            symbols: vec![],
        },
        dependency_analysis: DependencyAnalysisPublic {
            state: AnalysisState::Failed,
            direct: vec![],
            transitive: vec![],
            has_private_dependencies: false,
        },
        reverse_dependencies: vec![],
        relations: vec![],
        verification: LibraryVerificationView {
            aggregate_status: LibraryVerificationStatus::Rejected,
            evidence: vec![],
        },
        diagnostics: vec![DiagnosticPublic {
            severity: DiagnosticSeverity::Error,
            code: "lean.header.failed".into(),
            message: "Failed to parse module header.".into(),
            location: None,
        }],
    };

    let solutions = vec![
        SolutionPageData {
            page_id: "solution:abc999/a/monoid".into(),
            solution_id: "abc999/a/monoid".into(),
            contest_id: "abc999".into(),
            problem_code: "a".into(),
            solution_name: "monoid".into(),
            online_judge: "librarychecker".into(),
            language: "rust".into(),
            solved_at: "2026-08-01T14:30:00+09:00".into(),
            source_path: "solutions/abc999/a/monoid/src/main.rs".into(),
            source: "fn main() {}\n".into(),
            syntax_highlight: "rust".into(),
            has_preprocess: false,
            verifies: vec![LibraryLink {
                library_id: "libraries/rust/algebra/monoid.rs".into(),
                language: "rust".into(),
                title: "Monoid".into(),
                source_path: "libraries/rust/algebra/monoid.rs".into(),
                manual: false,
            }],
            direct_dependencies: vec![],
            has_private_dependencies: false,
            verification: SolutionVerificationView {
                status: SolutionVerificationStatus::Verified,
                result: Some(VerificationResultPublic {
                    attempt_id: "att-verified".into(),
                    verdict: Some("Accepted".into()),
                    judged_at: Some("2026-08-01T14:35:00+09:00".into()),
                    oj_submission_url: Some("https://judge.yosupo.jp/submission/12345".into()),
                    execution_time_ms: Some(120),
                    memory_kib: Some(2048),
                    submitted_source_hash:
                        "sha256:11111111111111111111111111111111111111111111111111111111111111\
                         11"
                        .into(),
                    verify_fingerprint:
                        "sha256:22222222222222222222222222222222222222222222222222222222222222\
                         22"
                        .into(),
                    stale_reason: None,
                    testcases: vec![TestcaseVerdictPublic {
                        name: "sample_01".into(),
                        verdict: "AC".into(),
                        execution_time_ms: Some(80),
                        memory_kib: Some(2048),
                    }],
                }),
            },
            dependency_analysis_state: AnalysisState::Complete,
            diagnostics: vec![],
        },
        SolutionPageData {
            page_id: "solution:abc999/b/broken".into(),
            solution_id: "abc999/b/broken".into(),
            contest_id: "abc999".into(),
            problem_code: "b".into(),
            solution_name: "broken".into(),
            online_judge: "librarychecker".into(),
            language: "rust".into(),
            solved_at: "2026-07-15T00:00:00+09:00".into(),
            source_path: "solutions/abc999/b/broken/src/main.rs".into(),
            source: "fn main() {}\n".into(),
            syntax_highlight: "rust".into(),
            has_preprocess: false,
            verifies: vec![],
            direct_dependencies: vec![],
            has_private_dependencies: false,
            verification: SolutionVerificationView {
                status: SolutionVerificationStatus::Rejected,
                result: Some(VerificationResultPublic {
                    attempt_id: "att-wa".into(),
                    verdict: Some("WrongAnswer".into()),
                    judged_at: Some("2026-07-15T00:05:00+09:00".into()),
                    oj_submission_url: Some("https://judge.yosupo.jp/submission/22".into()),
                    execution_time_ms: Some(90),
                    memory_kib: Some(1024),
                    submitted_source_hash:
                        "sha256:33333333333333333333333333333333333333333333333333333333333333\
                         33"
                        .into(),
                    verify_fingerprint:
                        "sha256:44444444444444444444444444444444444444444444444444444444444444\
                         44"
                        .into(),
                    stale_reason: None,
                    testcases: vec![],
                }),
            },
            dependency_analysis_state: AnalysisState::Complete,
            diagnostics: vec![],
        },
        SolutionPageData {
            page_id: "solution:abc999/c/interactive".into(),
            solution_id: "abc999/c/interactive".into(),
            contest_id: "abc999".into(),
            problem_code: "c".into(),
            solution_name: "interactive".into(),
            online_judge: "atcoder".into(),
            language: "cpp".into(),
            solved_at: "2026-07-01T00:00:00+09:00".into(),
            source_path: "solutions/abc999/c/interactive/main.cpp".into(),
            source: "#include <cstdio>\n".into(),
            syntax_highlight: "cpp".into(),
            has_preprocess: false,
            verifies: vec![],
            direct_dependencies: vec![],
            has_private_dependencies: false,
            verification: SolutionVerificationView {
                status: SolutionVerificationStatus::Unavailable,
                result: None,
            },
            dependency_analysis_state: AnalysisState::Complete,
            diagnostics: vec![],
        },
        SolutionPageData {
            page_id: "solution:abc999/d/stale".into(),
            solution_id: "abc999/d/stale".into(),
            contest_id: "abc999".into(),
            problem_code: "d".into(),
            solution_name: "stale".into(),
            online_judge: "librarychecker".into(),
            language: "cpp".into(),
            solved_at: "2026-06-01T00:00:00+09:00".into(),
            source_path: "solutions/abc999/d/stale/main.cpp".into(),
            source: "int main(){}\n".into(),
            syntax_highlight: "cpp".into(),
            has_preprocess: false,
            verifies: vec![],
            direct_dependencies: vec![],
            has_private_dependencies: false,
            verification: SolutionVerificationView {
                status: SolutionVerificationStatus::Stale,
                result: Some(VerificationResultPublic {
                    attempt_id: "att-stale".into(),
                    verdict: Some("Accepted".into()),
                    judged_at: Some("2026-06-01T00:05:00+09:00".into()),
                    oj_submission_url: Some("https://judge.yosupo.jp/submission/9".into()),
                    execution_time_ms: Some(300),
                    memory_kib: Some(4096),
                    submitted_source_hash:
                        "sha256:55555555555555555555555555555555555555555555555555555555555555\
                         55"
                        .into(),
                    verify_fingerprint:
                        "sha256:66666666666666666666666666666666666666666666666666666666666666\
                         66"
                        .into(),
                    stale_reason: Some(
                        "Source or dependencies changed since the last submission.".into(),
                    ),
                    testcases: vec![],
                }),
            },
            dependency_analysis_state: AnalysisState::Complete,
            diagnostics: vec![SolutionDiagnosticPublic {
                severity: DiagnosticSeverity::Info,
                code: "cpp.deps.info".into(),
                message: "Diagnostic is not in the displayed entry file.".into(),
                location: None,
                location_notice: Some("Location is in a non-displayed solution file.".into()),
            }],
        },
        SolutionPageData {
            page_id: "solution:abc999/e/never".into(),
            solution_id: "abc999/e/never".into(),
            contest_id: "abc999".into(),
            problem_code: "e".into(),
            solution_name: "never".into(),
            online_judge: "librarychecker".into(),
            language: "lean".into(),
            solved_at: "2026-05-01T00:00:00+09:00".into(),
            source_path: "solutions/abc999/e/never/Main.lean".into(),
            source: "def main : IO Unit := pure ()\n".into(),
            syntax_highlight: "lean".into(),
            has_preprocess: false,
            verifies: vec![LibraryLink {
                library_id: "libraries/lean/Algebra/Group.lean".into(),
                language: "lean".into(),
                title: "Group".into(),
                source_path: "libraries/lean/Algebra/Group.lean".into(),
                manual: false,
            }],
            direct_dependencies: vec![],
            has_private_dependencies: false,
            verification: SolutionVerificationView {
                status: SolutionVerificationStatus::Never,
                result: None,
            },
            dependency_analysis_state: AnalysisState::Complete,
            diagnostics: vec![],
        },
        SolutionPageData {
            page_id: "solution:abc999/f/manual".into(),
            solution_id: "abc999/f/manual".into(),
            contest_id: "abc999".into(),
            problem_code: "f".into(),
            solution_name: "manual".into(),
            online_judge: "atcoder".into(),
            language: "rust".into(),
            solved_at: "2026-04-01T00:00:00+09:00".into(),
            source_path: "solutions/abc999/f/manual/src/main.rs".into(),
            source: "fn main(){}\n".into(),
            syntax_highlight: "rust".into(),
            has_preprocess: true,
            verifies: vec![],
            direct_dependencies: vec![],
            has_private_dependencies: false,
            verification: SolutionVerificationView {
                status: SolutionVerificationStatus::NotConfigured,
                result: None,
            },
            dependency_analysis_state: AnalysisState::Complete,
            diagnostics: vec![],
        },
    ];

    SiteData {
        schema_version: SITE_SCHEMA_VERSION,
        build: BuildMetadata {
            schema_version: SITE_SCHEMA_VERSION,
            generated_at: "2026-08-11T12:00:00+00:00".into(),
            mode: BuildMode::Production,
            source_commit_sha: "deadbeef00000000000000000000000000000000".into(),
            source_commit_short_sha: "deadbee".into(),
            source_committed_at: "2026-08-11T11:59:00+00:00".into(),
            uncommitted_changes: false,
            observed_toolchains: vec![ToolchainIdentity {
                name: "rustc".into(),
                version: "1.92.0".into(),
                target: Some("aarch64-apple-darwin".into()),
            }],
            adapters: vec![AdapterIdentity {
                language: "rust".into(),
                name: "compro-env-rust-analyzer".into(),
                version: "1.0.0".into(),
            }],
        },
        site: SiteMetadata {
            title: "compro-env".into(),
            description: "Competitive programming libraries and solutions".into(),
            language: "en".into(),
            repository_url: Some("https://github.com/example/compro-env".into()),
        },
        languages: vec![
            LanguageSummary {
                id: "cpp".into(),
                display_name: "C++".into(),
                syntax_highlight: "cpp".into(),
                library_count: 1,
                verification_summary: VerificationCounts {
                    never: 1,
                    ..VerificationCounts::default()
                },
            },
            LanguageSummary {
                id: "lean".into(),
                display_name: "Lean".into(),
                syntax_highlight: "lean".into(),
                library_count: 1,
                verification_summary: VerificationCounts {
                    rejected: 1,
                    ..VerificationCounts::default()
                },
            },
            LanguageSummary {
                id: "rust".into(),
                display_name: "Rust".into(),
                syntax_highlight: "rust".into(),
                library_count: 1,
                verification_summary: VerificationCounts {
                    stale: 1,
                    ..VerificationCounts::default()
                },
            },
        ],
        libraries: vec![bit_hpp, lean_theorem, monoid],
        solutions,
    }
}
