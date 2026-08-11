//! Fixture-driven integration tests for the Rust symbol analyzer (plan 044).
//!
//! `fixture_matches_checked_in_response` loads `rust-symbols-request.json`,
//! substitutes the fixture tree's absolute path into `repository_root`, runs
//! `analyze_request` + `analyze_symbols`, and diffs the assembled response
//! against `rust-symbols-response.json` via `serde_json::Value` equality. Set
//! `UPDATE_EXPECT=1` to rewrite the fixture from a green run.

use std::fs;
use std::path::{Path, PathBuf};

use ce_library_rust_analyzer::dependencies::analyze_request;
use ce_library_rust_analyzer::module_graph::RustWorkspace;
use ce_library_rust_analyzer::symbols::analyze_symbols;
use library_adapter_protocol::{AnalysisRequest, AnalysisState, LibraryAnalysis, SolutionAnalysis};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn tree_root() -> PathBuf {
    fixture_root().join("tree")
}

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..3 {
        assert!(p.pop(), "unable to locate repo root");
    }
    p
}

fn load_request() -> AnalysisRequest {
    let protocol_fixture =
        repo_root().join("tools/library-analyzers/protocol/fixtures/rust-symbols-request.json");
    let raw = fs::read_to_string(&protocol_fixture)
        .unwrap_or_else(|e| panic!("read {}: {e}", protocol_fixture.display()));
    let substituted = raw.replace("<REPOSITORY_ROOT>", &tree_root().display().to_string());
    serde_json::from_str(&substituted).expect("request JSON parses")
}

fn expected_fixture_path() -> PathBuf {
    repo_root().join("tools/library-analyzers/protocol/fixtures/rust-symbols-response.json")
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

fn resolve(request: &AnalysisRequest) -> ResolvedResponse {
    let workspace = RustWorkspace::from_request(request).expect("workspace builds");
    let (mut libraries, solutions) = analyze_request(request, &workspace);
    for lib in &mut libraries {
        let absolute = workspace.absolute(&lib.path);
        let source = fs::read_to_string(&absolute)
            .unwrap_or_else(|e| panic!("read {}: {e}", absolute.display()));
        lib.symbol_analysis = analyze_symbols(&source, &lib.path, &[]);
    }
    ResolvedResponse {
        libraries,
        solutions,
    }
}

#[test]
fn fixture_matches_checked_in_response() {
    let request = load_request();
    let actual = resolve(&request);
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

// ─── Focused unit tests ────────────────────────────────────────────────────

fn symbol_names(source: &str) -> Vec<(String, String)> {
    let analysis = analyze_symbols(source, "test.rs", &[]);
    analysis
        .symbols
        .into_iter()
        .map(|s| (s.name, s.kind))
        .collect()
}

#[test]
fn struct_enum_union_are_extracted() {
    let src = "pub struct S; pub enum E { A } pub union U { a: u32 }";
    let names = symbol_names(src);
    assert!(names.contains(&("S".into(), "struct".into())));
    assert!(names.contains(&("E".into(), "enum".into())));
    assert!(names.contains(&("A".into(), "enum_variant".into())));
    assert!(names.contains(&("U".into(), "union".into())));
}

#[test]
fn trait_items_get_kind_projections() {
    let src = "pub trait T { fn m(); type X; const N: u32; }";
    let names = symbol_names(src);
    assert!(names.contains(&("T".into(), "trait".into())));
    assert!(names.contains(&("m".into(), "method".into())));
    assert!(names.contains(&("X".into(), "type".into())));
    assert!(names.contains(&("N".into(), "const".into())));
}

#[test]
fn impl_block_is_not_emitted_but_methods_are() {
    let src = "pub struct S; impl S { pub fn m(&self) {} } impl Clone for S { fn clone(&self) -> Self { S } }";
    let analysis = analyze_symbols(src, "t.rs", &[]);
    let kinds: Vec<&str> = analysis.symbols.iter().map(|s| s.kind.as_str()).collect();
    assert!(
        !kinds.contains(&"impl"),
        "impl blocks must not become symbols"
    );
    // Two `method` entries: the inherent `m` and the trait-impl `clone`.
    let method_count = kinds.iter().filter(|k| **k == "method").count();
    assert_eq!(method_count, 2, "got kinds {kinds:?}");
}

#[test]
fn impl_methods_qualify_under_target_type() {
    let src = "pub struct S; impl S { pub fn m() {} }";
    let analysis = analyze_symbols(src, "t.rs", &["algebra".into()]);
    let m = analysis
        .symbols
        .iter()
        .find(|s| s.name == "m")
        .expect("method m");
    assert_eq!(m.qualified_name.as_deref(), Some("algebra::S::m"));
    assert!(m.search_names.contains(&"m".to_string()));
    assert!(m.search_names.contains(&"algebra::S::m".to_string()));
}

#[test]
fn nested_mod_extends_qualified_name() {
    let src = "pub mod outer { pub mod inner { pub fn deep() {} } }";
    let analysis = analyze_symbols(src, "t.rs", &[]);
    let deep = analysis
        .symbols
        .iter()
        .find(|s| s.name == "deep")
        .expect("nested fn");
    assert_eq!(deep.qualified_name.as_deref(), Some("outer::inner::deep"));
}

#[test]
fn macro_declaration_emits_but_macro_invocation_marks_partial() {
    // `macro_rules! shout { ... }` is a declaration → emit macro symbol.
    let src = "macro_rules! shout { () => {} }";
    let analysis = analyze_symbols(src, "t.rs", &[]);
    assert!(
        analysis
            .symbols
            .iter()
            .any(|s| s.name == "shout" && s.kind == "macro"),
    );
    assert!(matches!(analysis.state, AnalysisState::Complete));

    // Item-level `foo!()` invocation may hide further items — partial.
    let src = "lazy_static::x! {}\npub struct Public;";
    let analysis = analyze_symbols(src, "t.rs", &[]);
    assert!(matches!(analysis.state, AnalysisState::Partial));
    assert!(analysis.symbols.iter().any(|s| s.name == "Public"));
}

#[test]
fn unicode_columns_are_unicode_scalar_values() {
    // `pub struct 東京 { ... }` — the identifier starts at USV column 12
    // (0-based col 11, plus 1 for 1-based reporting).
    let src = "pub struct 東京 { pub 人口: u64 }";
    let analysis = analyze_symbols(src, "t.rs", &[]);
    let tokyo = analysis
        .symbols
        .iter()
        .find(|s| s.name == "東京")
        .expect("struct 東京");
    let loc = tokyo.location.as_ref().expect("has location");
    assert_eq!(loc.start.line, 1);
    assert_eq!(loc.start.column, Some(1));
}
