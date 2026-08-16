//! Fixture-driven integration tests for the Rust symbol analyzer (plan 044).
//!
//! `fixture_matches_checked_in_response` loads `rust-symbols-request.json`,
//! substitutes the fixture tree's absolute path into `repository_root`, runs
//! `analyze_request`, and diffs the assembled response against
//! `rust-symbols-response.json` via `serde_json::Value` equality. Set
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
    let (libraries, solutions) = analyze_request(request, &workspace);
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
    // The `pub struct` item starts at column 1 — verify the walker uses
    // `ItemStruct::span()` (item start) rather than the identifier position,
    // and that the emitted column is 1-based even when the identifier itself
    // is multi-byte Unicode.
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

// ─── Task 2: validate locations and partial states ─────────────────────────

#[test]
fn line_and_column_are_one_based_at_top_of_file() {
    let src = "pub struct Head;\npub fn body() {}\n";
    let analysis = analyze_symbols(src, "t.rs", &[]);
    let head = analysis
        .symbols
        .iter()
        .find(|s| s.name == "Head")
        .expect("struct Head");
    let loc = head.location.as_ref().expect("has location");
    assert_eq!(loc.start.line, 1, "first line is 1-based");
    assert_eq!(loc.start.column, Some(1), "first column is 1-based");

    let body = analysis
        .symbols
        .iter()
        .find(|s| s.name == "body")
        .expect("fn body");
    let loc = body.location.as_ref().expect("has location");
    assert_eq!(loc.start.line, 2);
    assert_eq!(loc.start.column, Some(1));
}

#[test]
fn crlf_line_endings_count_as_one_line_each() {
    // CRLF between items — proc-macro2 treats "\r\n" as one line ending, so
    // the second item should still show up on line 2, not line 3.
    let src = "pub struct A;\r\npub struct B;\r\n";
    let analysis = analyze_symbols(src, "t.rs", &[]);
    let b = analysis
        .symbols
        .iter()
        .find(|s| s.name == "B")
        .expect("struct B");
    assert_eq!(b.location.as_ref().unwrap().start.line, 2);
}

#[test]
fn duplicate_names_are_kept_and_differentiated_by_qualified_name() {
    let src = "mod a { pub struct X; } mod b { pub struct X; }";
    let analysis = analyze_symbols(src, "t.rs", &[]);
    let xs: Vec<&library_adapter_protocol::Symbol> =
        analysis.symbols.iter().filter(|s| s.name == "X").collect();
    assert_eq!(xs.len(), 2, "both X symbols preserved");
    let qualifieds: Vec<&str> = xs
        .iter()
        .filter_map(|s| s.qualified_name.as_deref())
        .collect();
    assert!(qualifieds.contains(&"a::X"));
    assert!(qualifieds.contains(&"b::X"));
}

#[test]
fn malformed_source_yields_failed_and_no_symbols() {
    // Missing closing brace — syn's parser cannot recover.
    let src = "pub struct Broken {";
    let analysis = analyze_symbols(src, "t.rs", &[]);
    assert!(matches!(analysis.state, AnalysisState::Failed));
    assert!(analysis.symbols.is_empty());
}

#[test]
fn public_and_private_visibilities_are_both_extracted() {
    // The adapter does not filter by visibility — the core layer decides
    // what to publish. Verify both come through with kind + name intact.
    let src = "pub struct P; struct Q; pub(crate) fn r() {} fn s() {}";
    let analysis = analyze_symbols(src, "t.rs", &[]);
    let names: Vec<&str> = analysis.symbols.iter().map(|s| s.name.as_str()).collect();
    for expected in ["P", "Q", "r", "s"] {
        assert!(names.contains(&expected), "missing {expected} in {names:?}");
    }
}

#[test]
fn file_backed_mod_declarations_do_not_leak_other_files_symbols() {
    // `mod other;` without content — the analyzer must not open `other.rs`.
    // Only the `mod` declaration itself is emitted; children stay silent.
    let src = "mod other;\npub struct Local;\n";
    let analysis = analyze_symbols(src, "t.rs", &[]);
    let names: Vec<&str> = analysis.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"other"), "file-backed mod is emitted");
    assert!(names.contains(&"Local"), "sibling items still appear");
    // No off-file symbol from a hypothetical `other.rs` may appear.
    assert_eq!(analysis.symbols.len(), 2);
}

#[test]
fn locations_are_never_reversed() {
    // Every emitted location must have end >= start.
    let src = include_str!("fixtures/tree/libraries/rust/symbols/basic.rs");
    let analysis = analyze_symbols(src, "basic.rs", &[]);
    for symbol in &analysis.symbols {
        let loc = symbol
            .location
            .as_ref()
            .expect("all basic.rs symbols located");
        let end = loc.end.as_ref().expect("basic.rs spans have end");
        assert!(
            (end.line, end.column) >= (loc.start.line, loc.start.column),
            "reversed span for {:?}",
            symbol.name
        );
    }
}

// ─── Issue #105: analyze_request wire-up ───────────────────────────────────

fn write_library_tree(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, body) in files {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).expect("mkdir -p");
        std::fs::write(&abs, body).expect("write library file");
    }
    dir
}

fn request_with_library(repo_root: &std::path::Path, library_path: &str) -> AnalysisRequest {
    AnalysisRequest {
        schema_version: library_adapter_protocol::SCHEMA_VERSION,
        language: "rust".into(),
        repository_root: repo_root.display().to_string(),
        libraries: vec![library_adapter_protocol::LibraryTarget {
            path: library_path.into(),
        }],
        solutions: vec![],
    }
}

#[test]
fn analyze_request_emits_partial_with_diagnostic_on_item_level_macro() {
    // `lazy_static::x!{}` is an item-level macro invocation the syn walker
    // cannot expand. The walker marks the analysis Partial and the wire-up
    // must surface a diagnostic so downstream UI can flag the gap.
    let tree = write_library_tree(&[(
        "libraries/rust/partial.rs",
        "lazy_static::x!{}\npub struct Kept;\n",
    )]);
    let request = request_with_library(tree.path(), "libraries/rust/partial.rs");
    let workspace = RustWorkspace::from_request(&request).expect("workspace builds");
    let (libraries, _solutions) = analyze_request(&request, &workspace);

    let lib = libraries.first().expect("one library analyzed");
    assert!(
        matches!(lib.symbol_analysis.state, AnalysisState::Partial),
        "state was {:?}",
        lib.symbol_analysis.state
    );
    assert!(
        lib.symbol_analysis.symbols.iter().any(|s| s.name == "Kept"),
        "walker still emits items it could see: {:?}",
        lib.symbol_analysis.symbols,
    );
    let symbol_diags: Vec<&library_adapter_protocol::Diagnostic> = lib
        .diagnostics
        .iter()
        .filter(|d| d.code == "rust.symbols.partial")
        .collect();
    assert_eq!(
        symbol_diags.len(),
        1,
        "exactly one rust.symbols.partial diagnostic expected, got {:?}",
        lib.diagnostics,
    );
    assert!(matches!(
        symbol_diags[0].severity,
        library_adapter_protocol::Severity::Warning
    ));
}

#[test]
fn analyze_request_emits_failed_with_diagnostic_on_broken_source_without_cascading_into_dependencies()
 {
    // An unterminated struct body trips `syn::parse_file`. Both the
    // dependency pass and the symbol pass see the same failure, but each
    // reports its own diagnostic — proving the two pipelines are wired
    // independently rather than sharing one failure state.
    let tree = write_library_tree(&[(
        "libraries/rust/broken.rs",
        "pub struct Broken {\n",
    )]);
    let request = request_with_library(tree.path(), "libraries/rust/broken.rs");
    let workspace = RustWorkspace::from_request(&request).expect("workspace builds");
    let (libraries, _solutions) = analyze_request(&request, &workspace);

    let lib = libraries.first().expect("one library analyzed");
    assert!(
        matches!(lib.symbol_analysis.state, AnalysisState::Failed),
        "state was {:?}",
        lib.symbol_analysis.state
    );
    assert!(lib.symbol_analysis.symbols.is_empty());

    let symbol_diag = lib
        .diagnostics
        .iter()
        .find(|d| d.code == "rust.symbols.parse")
        .unwrap_or_else(|| {
            panic!(
                "expected rust.symbols.parse diagnostic, got {:?}",
                lib.diagnostics
            )
        });
    assert!(matches!(
        symbol_diag.severity,
        library_adapter_protocol::Severity::Warning
    ));

    // The dependency pass already emits its own diagnostic on the same
    // broken file — assert both codes coexist on the same LibraryAnalysis.
    assert!(
        lib.diagnostics
            .iter()
            .any(|d| d.code == "rust.parse.entry_file"),
        "dependency pass still emits its own diagnostic: {:?}",
        lib.diagnostics,
    );
}
