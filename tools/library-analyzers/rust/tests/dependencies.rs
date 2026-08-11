//! Fixture-driven integration test for the direct-dependency resolver
//! (plan 043 Task 2).
//!
//! Loads the checked-in `rust-dependencies-request.json`, substitutes the
//! fixture tree's absolute path into `repository_root`, runs
//! `analyze_dependencies`, and compares the response against the checked-in
//! `rust-dependencies-response.json` byte-for-byte after canonical
//! serialization. Any drift in the resolver's output surfaces as a diff
//! against the checked-in expected fixture.

use std::fs;
use std::path::{Path, PathBuf};

use ce_library_rust_analyzer::dependencies::analyze_request;
use ce_library_rust_analyzer::module_graph::RustWorkspace;
use library_adapter_protocol::{AnalysisRequest, LibraryAnalysis, SolutionAnalysis};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn tree_root() -> PathBuf {
    fixture_root().join("tree")
}

fn repo_root() -> PathBuf {
    // <manifest>/tools/library-analyzers/rust → repo root is three parents up.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..3 {
        assert!(p.pop(), "unable to locate repo root");
    }
    p
}

fn load_request() -> AnalysisRequest {
    let protocol_fixture = repo_root()
        .join("tools/library-analyzers/protocol/fixtures/rust-dependencies-request.json");
    let raw = fs::read_to_string(&protocol_fixture)
        .unwrap_or_else(|e| panic!("read {}: {e}", protocol_fixture.display()));
    let substituted = raw.replace("<REPOSITORY_ROOT>", &tree_root().display().to_string());
    serde_json::from_str(&substituted).expect("request JSON parses")
}

fn expected_fixture_path() -> PathBuf {
    repo_root().join("tools/library-analyzers/protocol/fixtures/rust-dependencies-response.json")
}

fn load_expected() -> Option<serde_json::Value> {
    let path = expected_fixture_path();
    let raw = fs::read_to_string(&path).ok()?;
    Some(serde_json::from_str(&raw).expect("expected response JSON parses"))
}

#[derive(serde::Serialize)]
struct ResolvedResponse {
    libraries: Vec<LibraryAnalysis>,
    solutions: Vec<SolutionAnalysis>,
}

#[test]
fn fixture_matches_checked_in_response() {
    let request = load_request();
    let workspace = RustWorkspace::from_request(&request).expect("workspace builds");
    let (libraries, solutions) = analyze_request(&request, &workspace);
    let actual = ResolvedResponse {
        libraries,
        solutions,
    };
    let actual_value = serde_json::to_value(&actual).expect("serialize");

    if std::env::var_os("UPDATE_EXPECT").is_some() {
        let pretty = serde_json::to_string_pretty(&actual_value).unwrap() + "\n";
        fs::write(expected_fixture_path(), pretty).expect("write updated fixture");
        return;
    }

    let expected = load_expected().unwrap_or_else(|| {
        panic!(
            "expected fixture missing at {}. Run with UPDATE_EXPECT=1 to create it.",
            expected_fixture_path().display()
        )
    });

    if actual_value != expected {
        let actual_pretty = serde_json::to_string_pretty(&actual_value).unwrap();
        let expected_pretty = serde_json::to_string_pretty(&expected).unwrap();
        panic!(
            "response fixture drift.\n\n--- expected ---\n{expected_pretty}\n\n--- actual ---\n{actual_pretty}\n"
        );
    }
}
