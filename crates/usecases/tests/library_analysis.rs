//! Integration tests for `normalize_analysis` (spec §6.4).

use std::collections::BTreeMap;

use chrono::DateTime;
use domain::analysis::{AnalysisState, DiscoveredLanguage, DiscoveryManifest, LibraryFile};
use domain::library::{LanguageId, LibraryId, SolutionId};
use domain::solution::{PublishedSolution, VerifySpec};
use library_adapter_protocol::AnalysisResponse;
use usecases::library_analysis::{SNAPSHOT_SCHEMA_VERSION, normalize_analysis};

const MIXED_RESPONSES: &str = include_str!("./fixtures/mixed-analysis-response.json");

fn build_manifest() -> DiscoveryManifest {
    let rust = LanguageId::parse("rust").unwrap();
    let cpp = LanguageId::parse("cpp").unwrap();
    let lean = LanguageId::parse("lean").unwrap();

    let mut languages = BTreeMap::new();
    languages.insert(
        rust.clone(),
        DiscoveredLanguage {
            id: rust.clone(),
            root: "libraries/rust".into(),
            display_name: "Rust".into(),
            description_path: None,
            analyzer_command: vec!["tools/library-analyzers/rust".into()],
        },
    );
    languages.insert(
        cpp.clone(),
        DiscoveredLanguage {
            id: cpp.clone(),
            root: "libraries/cpp".into(),
            display_name: "C++".into(),
            description_path: None,
            analyzer_command: vec!["tools/library-analyzers/cpp".into()],
        },
    );
    languages.insert(
        lean.clone(),
        DiscoveredLanguage {
            id: lean.clone(),
            root: "libraries/lean".into(),
            display_name: "Lean".into(),
            description_path: None,
            analyzer_command: vec!["tools/library-analyzers/lean".into()],
        },
    );

    let libraries = vec![
        lib("libraries/rust/a.rs", &rust),
        lib("libraries/rust/b.rs", &rust),
        lib("libraries/cpp/monoid.hpp", &cpp),
        lib("libraries/lean/Monoid.lean", &lean),
    ];

    let solved_at = DateTime::parse_from_rfc3339("2026-08-02T14:30:00+09:00").unwrap();
    let solutions = vec![PublishedSolution {
        id: SolutionId::parse("librarychecker-aplusb/aplusb/main").unwrap(),
        language: rust.clone(),
        root: "solutions/librarychecker-aplusb/aplusb/main".into(),
        entry: "src/main.rs".into(),
        solved_at,
        test_command: "./test.sh".into(),
        test_timeout_seconds: 600,
        verify: Some(VerifySpec {
            libraries: vec![LibraryId::parse("libraries/rust/a.rs").unwrap()],
            oj_language_id: "rust".into(),
        }),
    }];

    DiscoveryManifest {
        languages,
        libraries,
        solutions,
        diagnostics: vec![],
    }
}

fn lib(id: &str, lang: &LanguageId) -> LibraryFile {
    LibraryFile {
        id: LibraryId::parse(id).unwrap(),
        language: lang.clone(),
        source_path: id.into(),
        description_path: None,
        published: true,
        managed: true,
        title: None,
    }
}

fn build_responses() -> BTreeMap<LanguageId, AnalysisResponse> {
    let raw: BTreeMap<String, AnalysisResponse> = serde_json::from_str(MIXED_RESPONSES).unwrap();
    raw.into_iter()
        .map(|(k, v)| (LanguageId::parse(&k).unwrap(), v))
        .collect()
}

fn build_source_bytes() -> BTreeMap<String, Vec<u8>> {
    let mut m = BTreeMap::new();
    m.insert("libraries/rust/a.rs".to_string(), b"a".to_vec());
    m.insert("libraries/rust/b.rs".to_string(), b"b".to_vec());
    m.insert("libraries/cpp/monoid.hpp".to_string(), b"monoid".to_vec());
    m.insert("libraries/lean/Monoid.lean".to_string(), b"lean".to_vec());
    m.insert(
        "solutions/librarychecker-aplusb/aplusb/main/src/main.rs".to_string(),
        b"fn main() {}".to_vec(),
    );
    m
}

#[test]
fn three_language_fixture_normalizes_deterministically() {
    let manifest = build_manifest();
    let responses = build_responses();
    let source_bytes = build_source_bytes();

    let snapshot = normalize_analysis(&manifest, responses, "rev-abc", &source_bytes).unwrap();

    assert_eq!(snapshot.schema_version, SNAPSHOT_SCHEMA_VERSION);
    assert_eq!(snapshot.repository_revision, "rev-abc");

    // Source hashes cover every library the pipeline was told about.
    assert!(snapshot.source_hashes.contains_key("libraries/rust/a.rs"));
    assert!(snapshot.source_hashes.contains_key("libraries/rust/b.rs"));

    // Direct edges are deduplicated and sorted.
    let rust = LanguageId::parse("rust").unwrap();
    let rust_a = LibraryId::parse("libraries/rust/a.rs").unwrap();
    let rust_b = LibraryId::parse("libraries/rust/b.rs").unwrap();
    let rust_analysis = &snapshot.languages[&rust];
    let a_analysis = &rust_analysis.libraries[&rust_a];
    assert_eq!(a_analysis.direct_dependencies, vec![rust_b.clone()]);
    assert_eq!(a_analysis.state.dependency_state, AnalysisState::Complete);
    assert_eq!(a_analysis.state.symbol_state, AnalysisState::Partial);

    let b_analysis = &rust_analysis.libraries[&rust_b];
    assert_eq!(b_analysis.direct_dependencies, vec![rust_a.clone()]);
    assert_eq!(b_analysis.state.symbol_state, AnalysisState::Complete);

    // Cycles are safe: transitive closure includes both nodes.
    let closure = snapshot.transitive_closure(&rust_a);
    assert!(closure.contains(&rust_a));
    assert!(closure.contains(&rust_b));

    // Reverse edges are derived.
    let reverse = snapshot.reverse_edges();
    assert!(reverse[&rust_b].contains(&rust_a));
    assert!(reverse[&rust_a].contains(&rust_b));

    // Observed toolchains recorded, but not required to match `expected`.
    assert_eq!(rust_analysis.adapter_name, "example-rust-analyzer");
    assert_eq!(rust_analysis.observed_toolchains.len(), 2);

    // Lean library dependency_state = failed remains independent from
    // symbol_state = complete.
    let lean = LanguageId::parse("lean").unwrap();
    let lean_lib = LibraryId::parse("libraries/lean/Monoid.lean").unwrap();
    let lean_analysis = &snapshot.languages[&lean].libraries[&lean_lib];
    assert_eq!(lean_analysis.state.dependency_state, AnalysisState::Failed);
    assert_eq!(lean_analysis.state.symbol_state, AnalysisState::Complete);
    assert_eq!(lean_analysis.diagnostics.len(), 1);
}

#[test]
fn shuffled_response_order_produces_same_snapshot_hash() {
    let manifest = build_manifest();
    let source_bytes = build_source_bytes();

    let first = {
        let responses = build_responses();
        normalize_analysis(&manifest, responses, "rev", &source_bytes).unwrap()
    };
    let second = {
        // Rebuild the rust response with reversed library order.
        let mut responses = build_responses();
        let rust = LanguageId::parse("rust").unwrap();
        if let Some(r) = responses.get_mut(&rust) {
            r.libraries.reverse();
        }
        normalize_analysis(&manifest, responses, "rev", &source_bytes).unwrap()
    };

    assert_eq!(first.snapshot_hash, second.snapshot_hash);
}

#[test]
fn changing_only_adapter_identity_does_not_change_snapshot_hash() {
    let manifest = build_manifest();
    let source_bytes = build_source_bytes();

    let baseline = normalize_analysis(&manifest, build_responses(), "rev", &source_bytes).unwrap();

    let mut altered_responses = build_responses();
    for response in altered_responses.values_mut() {
        response.adapter.name = format!("{}-v2", response.adapter.name);
        response.adapter.version = format!("{}-suffix", response.adapter.version);
    }
    let altered = normalize_analysis(&manifest, altered_responses, "rev", &source_bytes).unwrap();

    assert_eq!(baseline.snapshot_hash, altered.snapshot_hash);
    assert_ne!(
        baseline.languages[&LanguageId::parse("rust").unwrap()].adapter_name,
        altered.languages[&LanguageId::parse("rust").unwrap()].adapter_name
    );
}

#[test]
fn rejects_response_missing_a_manifest_library() {
    let manifest = build_manifest();
    let mut responses = build_responses();
    responses
        .get_mut(&LanguageId::parse("rust").unwrap())
        .unwrap()
        .libraries
        .pop();
    let err = normalize_analysis(&manifest, responses, "rev", &build_source_bytes()).unwrap_err();
    assert!(format!("{err:#}").contains("missing libraries"), "{err:#}");
}

#[test]
fn rejects_response_with_extra_library() {
    let manifest = build_manifest();
    let mut responses = build_responses();
    let rust = LanguageId::parse("rust").unwrap();
    let extra = responses[&rust].libraries[0].clone();
    let mut extra_mut = extra;
    extra_mut.path = "libraries/rust/does-not-exist.rs".into();
    responses.get_mut(&rust).unwrap().libraries.push(extra_mut);
    let err = normalize_analysis(&manifest, responses, "rev", &build_source_bytes()).unwrap_err();
    assert!(
        format!("{err:#}").contains("not in the manifest"),
        "{err:#}"
    );
}

#[test]
fn rejects_duplicate_library_in_response() {
    let manifest = build_manifest();
    let mut responses = build_responses();
    let rust = LanguageId::parse("rust").unwrap();
    let dup = responses[&rust].libraries[0].clone();
    responses.get_mut(&rust).unwrap().libraries.push(dup);
    let err = normalize_analysis(&manifest, responses, "rev", &build_source_bytes()).unwrap_err();
    assert!(format!("{err:#}").contains("duplicate library"), "{err:#}");
}

#[test]
fn rejects_cross_language_internal_edge() {
    let manifest = build_manifest();
    let mut responses = build_responses();
    let rust = LanguageId::parse("rust").unwrap();
    let target = &mut responses.get_mut(&rust).unwrap().libraries[0];
    target
        .dependency_analysis
        .dependencies
        .push(library_adapter_protocol::Dependency::Internal {
            path: "libraries/cpp/monoid.hpp".into(),
            location: None,
        });
    let err = normalize_analysis(&manifest, responses, "rev", &build_source_bytes()).unwrap_err();
    assert!(format!("{err:#}").contains("same language"), "{err:#}");
}

#[test]
fn rejects_unsupported_schema_version() {
    let manifest = build_manifest();
    let mut responses = build_responses();
    responses
        .get_mut(&LanguageId::parse("rust").unwrap())
        .unwrap()
        .schema_version = 2;
    let err = normalize_analysis(&manifest, responses, "rev", &build_source_bytes()).unwrap_err();
    assert!(format!("{err:#}").contains("schema_version"), "{err:#}");
}

#[test]
fn rejects_language_set_mismatch() {
    let manifest = build_manifest();
    let mut responses = build_responses();
    responses.remove(&LanguageId::parse("cpp").unwrap());
    let err = normalize_analysis(&manifest, responses, "rev", &build_source_bytes()).unwrap_err();
    assert!(
        format!("{err:#}").contains("mismatches manifest"),
        "{err:#}"
    );
}

#[test]
fn symbol_only_failure_leaves_dependency_analysis_complete() {
    // plan 044 Task 2: A library whose symbol analysis fails must not knock
    // down its dependency analysis or block the snapshot / fingerprint that
    // downstream verification relies on.
    let manifest = build_manifest();
    let mut responses = build_responses();
    let rust = LanguageId::parse("rust").unwrap();
    // `libraries/rust/a.rs` in the fixture already has dependency_state =
    // complete; flip only its symbol side to failed.
    let a = responses
        .get_mut(&rust)
        .unwrap()
        .libraries
        .iter_mut()
        .find(|l| l.path == "libraries/rust/a.rs")
        .expect("rust/a in fixture");
    a.symbol_analysis.state = library_adapter_protocol::AnalysisState::Failed;
    a.symbol_analysis.symbols.clear();

    let snapshot = normalize_analysis(
        &manifest,
        responses,
        "rev-symbol-fail",
        &build_source_bytes(),
    )
    .unwrap();

    let rust_a = LibraryId::parse("libraries/rust/a.rs").unwrap();
    let analysis = &snapshot.languages[&rust].libraries[&rust_a];
    assert_eq!(
        analysis.state.dependency_state,
        AnalysisState::Complete,
        "symbol failure must not drag dependency state down",
    );
    assert_eq!(analysis.state.symbol_state, AnalysisState::Failed);

    // Verification fingerprinting depends on the transitive closure / source
    // hashes staying intact even when symbols failed.
    let closure = snapshot.transitive_closure(&rust_a);
    let rust_b = LibraryId::parse("libraries/rust/b.rs").unwrap();
    assert!(closure.contains(&rust_a));
    assert!(closure.contains(&rust_b));
    assert!(!snapshot.snapshot_hash.is_empty());
}

#[test]
fn cpp_partial_symbol_state_leaves_dependency_analysis_complete() {
    // plan 047 Task 2: The C++ symbol analyzer degrades to `partial` when a
    // declaration's location cannot be pinned (macro-expanded name, invalid
    // source range) or when parse recovery kicks in. Dependency completeness
    // is computed by the preprocess-only pass and must remain independent —
    // downstream verification only stalls when the dependency closure is
    // incomplete, so a partial symbol catalog is not a stall condition.
    let manifest = build_manifest();
    let mut responses = build_responses();
    let cpp = LanguageId::parse("cpp").unwrap();
    let monoid = responses
        .get_mut(&cpp)
        .unwrap()
        .libraries
        .iter_mut()
        .find(|l| l.path == "libraries/cpp/monoid.hpp")
        .expect("cpp/monoid.hpp in fixture");
    monoid.symbol_analysis.state = library_adapter_protocol::AnalysisState::Partial;
    monoid.symbol_analysis.symbols.clear();

    let snapshot = normalize_analysis(
        &manifest,
        responses,
        "rev-cpp-partial",
        &build_source_bytes(),
    )
    .unwrap();

    let cpp_lib = LibraryId::parse("libraries/cpp/monoid.hpp").unwrap();
    let analysis = &snapshot.languages[&cpp].libraries[&cpp_lib];
    assert_eq!(
        analysis.state.dependency_state,
        AnalysisState::Complete,
        "partial C++ symbol analysis must not drag dependency state down",
    );
    assert_eq!(analysis.state.symbol_state, AnalysisState::Partial);
    assert!(!snapshot.snapshot_hash.is_empty());
}

#[test]
fn rejects_duplicate_toolchain_name() {
    let manifest = build_manifest();
    let mut responses = build_responses();
    let rust = LanguageId::parse("rust").unwrap();
    let toolchain = responses[&rust].adapter.toolchains[0].clone();
    responses
        .get_mut(&rust)
        .unwrap()
        .adapter
        .toolchains
        .push(toolchain);
    let err = normalize_analysis(&manifest, responses, "rev", &build_source_bytes()).unwrap_err();
    assert!(
        format!("{err:#}").contains("duplicate toolchain"),
        "{err:#}"
    );
}
