# Rust Symbol Emit Wire-up Plan (#105)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the existing `analyze_symbols` walker into the `TargetDependencyAnalysis::Library` produced by `dependencies.rs`, so every consumer of the public `analyze_request` API — not just the `main.rs` binary — receives real symbol data. Ensure symbol-analysis failure produces a diagnostic without disturbing dependency-analysis state.

**Root cause:** `tools/library-analyzers/rust/src/dependencies.rs:65-69` pins `SymbolAnalysis { state: Partial, symbols: vec![] }` as a plan-043 placeholder. `main.rs::run_symbol_analysis` overwrites it after `analyze_request` returns, but the public `analyze_request` API — and every library-crate caller (including the integration test in `tests/symbols.rs`) — sees only the placeholder unless it duplicates the same overlay. That is a leak of a "TODO" placeholder into what is documented as a finished layer, and the `main.rs` overlay makes it impossible for a future caller to reach a Complete state through `analyze_request` alone.

**Architecture:**
- Move the source read + `analyze_symbols` call from `main.rs::run_symbol_analysis` into a private helper inside `dependencies.rs`. The helper is called once per library target inside `analyze_dependencies` (the function that builds `TargetDependencyAnalysis::Library`).
- The helper returns `(SymbolAnalysis, Vec<Diagnostic>)`. The library's diagnostics accumulator merges symbol-analysis diagnostics with dependency-analysis diagnostics; dependency-analysis state is untouched.
- Diagnostic-emission policy (new):
  - `Complete` → no symbol diagnostic.
  - `Partial` → one `rust.symbols.partial` warning diagnostic on the library entry (line 1). This is new; today `analyze_symbols` returns `Partial` silently for macro-item invocations and dropped spans.
  - `Failed` → one `rust.symbols.parse` warning diagnostic (parse error) or `rust.symbols.read` error diagnostic (I/O error).
- Public function signatures stay stable. `analyze_symbols(source, target_path, module_path) -> SymbolAnalysis` is unchanged.
- No new external crate. Only the already-declared `syn` / `proc-macro2` dependencies are used.
- Fingerprint impact per spec §4.4: symbol state is independent of dependency state; the site-data fingerprint pipeline (#104) already ignores symbol state. No fingerprint changes.

**Tech Stack:** Rust 1.92.0, `syn` full/visit, `proc-macro2` span-locations.

## Global Constraints

- **Branch:** `fix/105-rust-symbols-emit` (already checked out under `.worktrees/fix-105-rust-symbols-emit`).
- **Depends on:** current `main`. No cross-crate dep changes.
- **Do not add** any new cargo dependency to `tools/library-analyzers/rust/Cargo.toml` (per issue #108).
- **Do not change** the public `analyze_symbols` signature (it is documented in plan `2026-08-10-library-rust-symbols.md`).
- **Do not touch** dependency-analysis state, dependency edges, or the direct-dependency fixture.
- Rust code comments in English; commit message / PR body / review replies in Japanese; no emoji.
- Run `cargo test -p ce-library-rust-analyzer`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings` before pushing.

## File Structure

- Modify `tools/library-analyzers/rust/src/dependencies.rs`
  - Replace the pinned placeholder in the library branch of `analyze_dependencies`.
  - Add `fn run_symbol_analysis(&RustWorkspace, &str) -> (SymbolAnalysis, Vec<Diagnostic>)`.
  - Merge returned diagnostics into the library's diagnostics list.
- Modify `tools/library-analyzers/rust/src/main.rs`
  - Delete `run_symbol_analysis` and its call site.
  - Drop the now-unused imports (`analyze_symbols`, `AnalysisState`, `SymbolAnalysis`, `Location`, `Position`, `Diagnostic`, `Severity`).
- Modify `tools/library-analyzers/rust/tests/symbols.rs`
  - Simplify `resolve()`: no more manual overlay — `analyze_request` alone must produce the full response now.
  - Add a case for the `Partial` diagnostic on item-level macro invocation.
  - Add a case for the `Failed` diagnostic on a syn parse failure, confirming the library's dependency-analysis state remains untouched.
- Modify `tools/library-analyzers/protocol/fixtures/rust-symbols-response.json`
  - Refresh via `UPDATE_EXPECT=1 cargo test -p ce-library-rust-analyzer --test symbols fixture_matches_checked_in_response` — expected diff is limited to any new diagnostic entries on partial libraries, if the fixture library exhibits macro-item-level partial parse. (The current fixture `basic.rs` yields Complete symbols; only its dependency analysis is `partial`. If Complete stays Complete, the only diff is the elimination of the previous pinned `Partial` placeholder — which never made it into the checked-in fixture, so likely no diff at all. Verify.)
- Modify `tools/library-analyzers/protocol/fixtures/rust-dependencies-response.json`
  - The dependency-only fixture test (`tests/dependencies.rs`) already asserts against a fixture that includes `symbol_analysis`. After the wire-up, this fixture will hold real symbols for its libraries. Refresh with `UPDATE_EXPECT=1 cargo test -p ce-library-rust-analyzer --test dependencies` after tests compile and pass.
- Ensure `docs/spec.md` / `docs/commands/site-data.md` do not say "symbols are partial". Confirmed by grep: no such wording exists today. No spec edits required.

## `Partial` Definition (Rust adapter)

`SymbolAnalysis.state` transitions are defined per spec §4.4 and existing `symbols.rs` behavior:

- **Complete** — every top-level item in the file was reached, and every emitted symbol has a valid location (line ≥ 1, `end >= start`).
- **Partial** — `syn::parse_file` succeeded, but at least one item is either:
  - An item-position macro invocation `foo!(...)` that may generate items the visitor cannot see (already handled in `symbols.rs::visit_item::Item::Macro` branch → `mark_partial`), or
  - A syntactically valid item whose span was rejected because `start.line == 0`, `end.line == 0`, or `end < start` (already handled in `SymbolCollector::build_location`).
- **Failed** — `syn::parse_file` returned `Err`. No symbols emitted.

The wire-up adds a **diagnostic** for `Partial` and `Failed` (the current walker returns them silently). Diagnostic code strings match what `main.rs` used before removal:

- `rust.symbols.read` (Error severity) — filesystem read failed. `symbol_analysis.state = Failed`.
- `rust.symbols.parse` (Warning severity) — `syn::parse_file` returned Err. `symbol_analysis.state = Failed`.
- `rust.symbols.partial` (Warning severity) — walker marked itself Partial. `symbol_analysis.state = Partial` and `symbols` is non-empty (or empty for a file whose only content is an item-level macro invocation).

## Test Strategy

Three deterministic behaviors are locked in tests, all under `tools/library-analyzers/rust/tests/symbols.rs`:

1. **Complete path (fixture parity).** The existing `fixture_matches_checked_in_response` continues to pass with the simpler `resolve()` (no manual overlay). Proves `analyze_request` alone returns real symbols.
2. **Partial + diagnostic on item-level macro.** New test writes a temporary tree containing one library `libraries/rust/partial.rs` whose body is `lazy_static::x!{}\npub struct Kept;\n`, runs `analyze_request`, and asserts:
   - `symbol_analysis.state == Partial`
   - `symbol_analysis.symbols` contains `Kept`
   - `diagnostics` contains exactly one entry with `code == "rust.symbols.partial"`, severity `Warning`
   - `dependency_analysis.state` unaffected (`Complete` in this case).
3. **Failed + diagnostic on broken source; dep analysis isolated.** New test writes a temporary library `libraries/rust/broken.rs` with a body that trips `syn::parse_file` (e.g. `pub struct Broken {\n`), asserts:
   - `symbol_analysis.state == Failed`
   - `symbol_analysis.symbols` is empty
   - `diagnostics` contains `code == "rust.symbols.parse"`
   - `dependency_analysis.state` reflects what the dep pass produced on the same broken file (either `Failed` because entry file parse failed via `load_file`, or `Complete` if a stub is used — the test asserts the two states are computed independently, not that dep is Complete).

Since the dep pass also calls `load_file` (which calls `syn::parse_file`) on the entry file, a truly unparseable entry file yields `dependency_analysis.state = Failed` as well. To satisfy the DoD's spirit — proving symbol failure does not cascade into dep analysis — test 3 uses a library whose **entry file parses fine** for `analyze_dependencies` (which walks only item structure like `mod`/`use`), but whose body is arranged so a subsequent `syn::parse_file` from `analyze_symbols` still succeeds. Since both use the same parser, the only way to get symbol-only failure would be an I/O race — not deterministic. Instead we split the DoD into:

- Test 3a — broken source: both dep and symbol state become Failed; assert **the two failures produce independent diagnostics** (`rust.parse.entry_file` for dep, `rust.symbols.parse` for symbol). This proves the pipelines have separate diagnostic channels.
- Test 3b — I/O failure via a missing file (workspace lists a library path that vanishes between manifest build and symbol read): assert `rust.symbols.read` diagnostic with `state = Failed`. Because `RustWorkspace::from_request` currently validates paths at manifest time, this test uses `analyze_dependencies` directly with a workspace whose managed set includes a path whose parent directory is removed after workspace construction. If constructing this state proves impossible without adapter-internal hacks, fall back to unit-testing `run_symbol_analysis` directly with an absolute path pointing to a non-existent file.

The Partial + diagnostic behavior (test 2) is the primary "state: Partial with diagnostic" case called out in the issue acceptance criteria.

---

### Task 1: Add failing tests for the new wire-up behavior

**Files:**
- Modify: `tools/library-analyzers/rust/tests/symbols.rs`
- Create (test fixture tree): reuse `tempfile::TempDir` inside each test — no new committed fixtures.

**Interfaces produced:**
- Two new integration tests in `tests/symbols.rs`:
  - `analyze_request_emits_partial_with_diagnostic_on_item_level_macro`
  - `analyze_request_emits_failed_with_diagnostic_on_broken_source_without_cascading_into_dependencies`
- Simplification of the existing `resolve()` helper: remove the manual `analyze_symbols` overlay loop.

- [ ] **Step 1: Simplify `resolve()`**

Replace the current body of `resolve` (lines 58-71):

```rust
fn resolve(request: &AnalysisRequest) -> ResolvedResponse {
    let workspace = RustWorkspace::from_request(request).expect("workspace builds");
    let (libraries, solutions) = analyze_request(request, &workspace);
    ResolvedResponse { libraries, solutions }
}
```

Drop the now-unused `analyze_symbols` import from `tests/symbols.rs` if it becomes unused after later edits (keep it for the direct-parse unit tests further down — those still call `analyze_symbols` directly).

- [ ] **Step 2: Write the Partial-diagnostic test**

Add at the end of `tests/symbols.rs`:

```rust
// ─── Task issue #105: wire-up regressions ───────────────────────────────────

use tempfile::TempDir;

fn write_library_tree(files: &[(&str, &str)]) -> TempDir {
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
            title: None,
            description: None,
        }],
        solutions: vec![],
    }
}

#[test]
fn analyze_request_emits_partial_with_diagnostic_on_item_level_macro() {
    let tree = write_library_tree(&[(
        "libraries/rust/partial.rs",
        "lazy_static::x!{}\npub struct Kept;\n",
    )]);
    let request = request_with_library(tree.path(), "libraries/rust/partial.rs");
    let workspace = RustWorkspace::from_request(&request).expect("workspace builds");
    let (libraries, _solutions) = analyze_request(&request, &workspace);

    let lib = libraries.first().expect("one library analyzed");
    assert!(matches!(lib.symbol_analysis.state, AnalysisState::Partial));
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
        "exactly one partial-symbol diagnostic, got {:?}",
        lib.diagnostics,
    );
    assert!(matches!(
        symbol_diags[0].severity,
        library_adapter_protocol::Severity::Warning
    ));
}
```

- [ ] **Step 3: Write the Failed-diagnostic test**

```rust
#[test]
fn analyze_request_emits_failed_with_diagnostic_on_broken_source_without_cascading_into_dependencies() {
    let tree = write_library_tree(&[(
        "libraries/rust/broken.rs",
        // syn::parse_file rejects the unterminated struct body.
        "pub struct Broken {\n",
    )]);
    let request = request_with_library(tree.path(), "libraries/rust/broken.rs");
    let workspace = RustWorkspace::from_request(&request).expect("workspace builds");
    let (libraries, _solutions) = analyze_request(&request, &workspace);

    let lib = libraries.first().expect("one library analyzed");
    assert!(matches!(lib.symbol_analysis.state, AnalysisState::Failed));
    assert!(lib.symbol_analysis.symbols.is_empty());

    // Symbol failure produces its own diagnostic code, independent of any
    // dependency-analysis diagnostic.
    let symbol_diag = lib
        .diagnostics
        .iter()
        .find(|d| d.code == "rust.symbols.parse")
        .expect("symbols.parse diagnostic emitted");
    assert!(matches!(
        symbol_diag.severity,
        library_adapter_protocol::Severity::Warning
    ));

    // Dependency analysis has its own code (`rust.parse.entry_file`) — the two
    // pipelines emit independent diagnostics even when the same file is bad.
    let dep_diag = lib
        .diagnostics
        .iter()
        .find(|d| d.code == "rust.parse.entry_file");
    assert!(
        dep_diag.is_some(),
        "dependency pass still emits its own diagnostic on the same broken file",
    );
}
```

- [ ] **Step 4: Run tests to see them fail**

Run: `cargo test -p ce-library-rust-analyzer --test symbols -- analyze_request_emits`

Expected:
- `analyze_request_emits_partial_with_diagnostic_on_item_level_macro` — FAIL: `symbol_analysis.state` is still `Partial` from the pinned placeholder, but there is no `rust.symbols.partial` diagnostic.
- `analyze_request_emits_failed_with_diagnostic_on_broken_source_...` — FAIL: no `rust.symbols.parse` diagnostic today (only `rust.parse.entry_file` from dep pass).

- [ ] **Step 5: Commit the failing tests**

```bash
git add tools/library-analyzers/rust/tests/symbols.rs
git commit -m "test: lock #105 symbol wire-up behavior (failing)"
```

---

### Task 2: Move `run_symbol_analysis` into `dependencies.rs`

**Files:**
- Modify: `tools/library-analyzers/rust/src/dependencies.rs`
- Modify: `tools/library-analyzers/rust/src/main.rs`

**Interfaces:**
- Consumes: `analyze_symbols(source, target_path, module_path) -> SymbolAnalysis` (unchanged).
- Produces: private `fn run_symbol_analysis(workspace: &RustWorkspace, library_path: &str) -> (SymbolAnalysis, Vec<Diagnostic>)` inside `dependencies.rs`. Not re-exported.

- [ ] **Step 1: Replace the placeholder in `analyze_dependencies`**

In `dependencies.rs`, at the top of the library branch inside `analyze_dependencies` (currently lines 55-71), replace the entire library push with:

```rust
for target in &request.libraries {
    let target_id = target.path.clone();
    let entry_path = &target_id;
    let (deps, dep_state, mut diagnostics) =
        analyze_target(workspace, &target_id, entry_path);
    let (symbol_analysis, mut symbol_diagnostics) =
        run_symbol_analysis(workspace, entry_path);
    diagnostics.append(&mut symbol_diagnostics);
    out.push(TargetDependencyAnalysis::Library(LibraryAnalysis {
        path: target.path.clone(),
        dependency_analysis: DependencyAnalysis {
            state: dep_state,
            dependencies: deps,
        },
        symbol_analysis,
        diagnostics,
    }));
}
```

- [ ] **Step 2: Add the `run_symbol_analysis` helper**

Add near the other helpers (before `// ─── Helpers ───`), inside `dependencies.rs`:

```rust
/// Read `library_path` from disk and delegate to
/// [`crate::symbols::analyze_symbols`], attaching a diagnostic on any
/// non-`Complete` result. The dependency pass is untouched by any error
/// this helper reports.
fn run_symbol_analysis(
    workspace: &RustWorkspace,
    library_path: &str,
) -> (SymbolAnalysis, Vec<Diagnostic>) {
    let absolute = workspace.absolute(library_path);
    let source = match std::fs::read_to_string(&absolute) {
        Ok(s) => s,
        Err(err) => {
            return (
                SymbolAnalysis {
                    state: AnalysisState::Failed,
                    symbols: vec![],
                },
                vec![Diagnostic {
                    severity: Severity::Error,
                    code: "rust.symbols.read".into(),
                    message: format!("failed to read {library_path}: {err}"),
                    location: Some(entry_location(library_path)),
                }],
            );
        }
    };
    let analysis = crate::symbols::analyze_symbols(&source, library_path, &[]);
    let diagnostics = match analysis.state {
        AnalysisState::Complete => vec![],
        AnalysisState::Partial => vec![Diagnostic {
            severity: Severity::Warning,
            code: "rust.symbols.partial".into(),
            message: format!(
                "symbol analysis is partial for {library_path} (item-level macro or dropped span)"
            ),
            location: Some(entry_location(library_path)),
        }],
        AnalysisState::Failed => vec![Diagnostic {
            severity: Severity::Warning,
            code: "rust.symbols.parse".into(),
            message: format!("failed to parse {library_path} for symbol analysis"),
            location: Some(entry_location(library_path)),
        }],
    };
    (analysis, diagnostics)
}

fn entry_location(path: &str) -> Location {
    Location {
        path: path.to_string(),
        start: Position {
            line: 1,
            column: Some(1),
        },
        end: None,
    }
}
```

- [ ] **Step 3: Delete `run_symbol_analysis` and `entry_location` from `main.rs`**

- Remove `fn run_symbol_analysis`, `fn entry_location`, and the `for lib in &mut libraries { run_symbol_analysis(...) }` loop from `build_response`.
- After the loop is gone, `build_response` reduces to assembling `AnalysisResponse` from `analyze_request`'s output directly.
- Drop now-unused imports from `main.rs`: `analyze_symbols`, `AnalysisState`, `SymbolAnalysis`, `Location`, `Position`, `Diagnostic`, `Severity`, `LibraryAnalysis`. Keep the imports still used by tests (`ToolchainIdentity`, `AdapterIdentity`, `AnalysisRequest`, `AnalysisResponse`, `SCHEMA_VERSION`).

Verify with `cargo check -p ce-library-rust-analyzer` after the edit.

- [ ] **Step 4: Run the two new tests**

Run: `cargo test -p ce-library-rust-analyzer --test symbols -- analyze_request_emits`

Expected: both pass.

- [ ] **Step 5: Run every symbol test**

Run: `cargo test -p ce-library-rust-analyzer`

The `tests/symbols.rs::fixture_matches_checked_in_response` may fail because `analyze_request` now emits real symbols and possibly new diagnostics into the response the fixture expects. Diff the failure to confirm the drift is only:
- new symbols where the placeholder used to leave an empty list (already overwritten in the fixture — likely no diff), and
- new diagnostics only for genuinely non-Complete libraries.

- [ ] **Step 6: Refresh fixtures if necessary**

Only if step 5 fails with a drift consistent with the wire-up (no unexpected shape changes):

```bash
UPDATE_EXPECT=1 cargo test -p ce-library-rust-analyzer --test symbols fixture_matches_checked_in_response
UPDATE_EXPECT=1 cargo test -p ce-library-rust-analyzer --test dependencies fixture_matches_checked_in_response
```

Inspect the resulting diffs with `git diff tools/library-analyzers/protocol/fixtures/`. Any diff outside `symbol_analysis` / `diagnostics` fields is a bug — do not commit it.

- [ ] **Step 7: Commit the wire-up**

```bash
git add tools/library-analyzers/rust/src/dependencies.rs tools/library-analyzers/rust/src/main.rs tools/library-analyzers/protocol/fixtures/rust-symbols-response.json tools/library-analyzers/protocol/fixtures/rust-dependencies-response.json
git commit -m "fix(105): emit Rust symbol analysis from analyze_request"
```

---

### Task 3: Verify the whole workspace and lint

- [ ] **Step 1: Run the workspace test suite**

Run: `cargo test --workspace`

Expected: all green. Special attention to:
- `crates/usecases/tests/library_analysis.rs` — was written under the "symbols are partial" assumption; confirm it still passes.
- `crates/infrastructure/tests/site_data_generate.rs` — synthesizes its own analyzer output, not affected by wire-up.

- [ ] **Step 2: Run clippy with `-D warnings`**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: no warnings.

- [ ] **Step 3: Commit any lint fixes if needed**

If clippy surfaces trivial fixes (unused imports left by the `main.rs` cleanup), address inline and amend the previous commit:

```bash
git commit --amend --no-edit
```

- [ ] **Step 4: Push and open PR**

```bash
git push -u origin fix/105-rust-symbols-emit
```

Then invoke `skill://pr` with a Japanese PR body referencing #105, describing the wire-up move, the new diagnostic codes, and calling out that no external cargo dependency was added (issue #108 constraint honored).

- [ ] **Step 5: Cycle through `skill://pr-review claude`**

Address every review comment in Japanese. Loop until Claude returns no new comments.
