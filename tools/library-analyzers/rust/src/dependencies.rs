//! Direct-dependency resolution for the ce-rust adapter (plan 043 Task 2).
//!
//! Each target (library file or solution entry) is treated as a synthetic
//! crate. The analyzer walks its item tree with `syn`, resolves `mod` /
//! `#[path]` / grouped `use` / `crate` `self` `super` paths against the
//! workspace's managed file set, and emits every source reference as one
//! `Dependency` variant. Anything the resolver cannot classify uniquely
//! yields a stable `Unresolved` entry plus flips the target's analysis
//! state to `partial`.
//!
//! What this pass intentionally does *not* do:
//!
//! * Rank or dedupe edges by construction; duplicates are legitimate signal
//!   for downstream normalization (see `usecases::library_analysis`).
//! * Track cfg activation. Cfg-inactive branches are folded into the
//!   syntactic union so their references still surface — the analysis is
//!   `partial` because we cannot prove which branch a build would pick.
//! * Expand macros or execute build scripts. Any `foo!(...)` invocation at
//!   item position is recorded as an unresolved macro-generated path.

use std::collections::BTreeSet;
use std::iter;

use library_adapter_protocol::{
    AnalysisRequest, AnalysisState, Dependency, DependencyAnalysis, Diagnostic, LibraryAnalysis,
    Location, Position, Severity, SolutionAnalysis, SymbolAnalysis,
};
use syn::{
    AttrStyle, Attribute, Expr, ExprLit, Item, ItemMod, ItemUse, Lit, Meta, UseTree,
    spanned::Spanned,
};

use crate::module_graph::{
    CrateKind, ModResolution, ParsedFile, RustCrate, RustWorkspace, load_file, resolve_mod,
};

/// Per-target result returned by [`analyze_dependencies`].
///
/// A `TargetDependencyAnalysis` fits directly into an `AnalysisResponse`: it
/// exposes either the library shape (with dependency **and** symbol analysis
/// populated) or the solution shape (dependency analysis only), never both.
#[derive(Debug, Clone)]
pub enum TargetDependencyAnalysis {
    Library(LibraryAnalysis),
    Solution(SolutionAnalysis),
}

/// Analyze every target in `request` under `workspace`. Order follows the
/// request order for each list, libraries first then solutions.
pub fn analyze_dependencies(
    request: &AnalysisRequest,
    workspace: &RustWorkspace,
) -> Vec<TargetDependencyAnalysis> {
    let mut out = Vec::with_capacity(request.libraries.len() + request.solutions.len());
    for target in &request.libraries {
        let target_id = target.path.clone();
        let entry_path = &target_id;
        let (deps, dep_state, mut diagnostics) = analyze_target(workspace, &target_id, entry_path);
        let (symbol_analysis, mut symbol_diagnostics) = run_symbol_analysis(workspace, entry_path);
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
    for target in &request.solutions {
        let target_id = target.id.clone();
        let entry_path = &workspace
            .crates
            .get(&target_id)
            .expect("solution crate was registered in workspace")
            .root_file;
        let (deps, state, diagnostics) = analyze_target(workspace, &target_id, entry_path);
        out.push(TargetDependencyAnalysis::Solution(SolutionAnalysis {
            id: target.id.clone(),
            dependency_analysis: DependencyAnalysis {
                state,
                dependencies: deps,
            },
            diagnostics,
        }));
    }
    out
}

/// Read `library_path` from disk and delegate to
/// [`crate::symbols::analyze_symbols`], attaching a diagnostic whenever the
/// walker returns a non-`Complete` result. The dependency pass is untouched
/// by any error this helper reports — the two pipelines emit independent
/// diagnostic codes on the same `LibraryAnalysis`.
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
        AnalysisState::Complete => Vec::new(),
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

fn analyze_target(
    workspace: &RustWorkspace,
    target_id: &str,
    entry_path: &str,
) -> (Vec<Dependency>, AnalysisState, Vec<Diagnostic>) {
    let cr = match workspace.crates.get(target_id) {
        Some(c) => c,
        None => {
            return (
                vec![],
                AnalysisState::Failed,
                vec![Diagnostic {
                    severity: Severity::Error,
                    code: "rust.workspace.missing_target".into(),
                    message: format!("no crate registered for target {target_id:?}"),
                    location: None,
                }],
            );
        }
    };
    let mut collector = DependencyCollector::new(workspace, cr);
    match load_file(workspace, entry_path) {
        Ok(root) => collector.visit_crate(root),
        Err(err) => {
            collector.state = AnalysisState::Failed;
            collector.diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "rust.parse.entry_file".into(),
                message: format!("failed to load entry file: {err}"),
                location: Some(Location {
                    path: entry_path.to_string(),
                    start: Position {
                        line: 1,
                        column: Some(1),
                    },
                    end: None,
                }),
            });
        }
    }
    let state = collector.state;
    let diagnostics = std::mem::take(&mut collector.diagnostics);
    let deps = collector.finish();
    (deps, state, diagnostics)
}

// ─── Collector ──────────────────────────────────────────────────────────────

struct DependencyCollector<'w> {
    workspace: &'w RustWorkspace,
    krate: &'w RustCrate,
    dependencies: Vec<Dependency>,
    diagnostics: Vec<Diagnostic>,
    visited: BTreeSet<String>,
    state: AnalysisState,
}

impl<'w> DependencyCollector<'w> {
    fn new(workspace: &'w RustWorkspace, krate: &'w RustCrate) -> Self {
        Self {
            workspace,
            krate,
            dependencies: Vec::new(),
            diagnostics: Vec::new(),
            visited: BTreeSet::new(),
            state: AnalysisState::Complete,
        }
    }

    fn finish(self) -> Vec<Dependency> {
        self.dependencies
    }

    fn mark_partial(&mut self) {
        if let AnalysisState::Complete = self.state {
            self.state = AnalysisState::Partial;
        }
    }

    fn visit_crate(&mut self, parsed: ParsedFile) {
        self.visit_file(parsed, ModulePath::default());
    }

    fn visit_file(&mut self, parsed: ParsedFile, module_path: ModulePath) {
        if !self.visited.insert(parsed.repo_relative.clone()) {
            // Cycle-safe: a module the collector already walked is skipped so
            // the traversal terminates on `mod a; mod b;` cycles where both
            // sides `use crate::…` each other. `mod` items add Internal edges
            // *before* the recursion, so no data is lost when we short-circuit.
            return;
        }
        for item in &parsed.file.items {
            self.visit_item(item, &parsed, &module_path);
        }
    }

    /// True when the collector should follow file-backed `mod` items into
    /// their child files. Libraries are single-file by contract; solutions
    /// are multi-file crates whose entire mod tree contributes direct edges.
    fn should_recurse_into_file_mod(&self) -> bool {
        matches!(self.krate.kind, CrateKind::Solution)
    }

    fn visit_item(&mut self, item: &Item, containing: &ParsedFile, module_path: &ModulePath) {
        match item {
            Item::Mod(m) => self.visit_item_mod(m, containing, module_path),
            Item::Use(u) => self.visit_item_use(u, containing),
            Item::ExternCrate(ec) => {
                self.emit_extern_crate(&ec.ident.to_string(), containing, ec.span())
            }
            Item::Macro(_) => {
                // Item-level macro invocations may generate arbitrary items
                // (including further `mod`/`use` statements). Mark partial
                // and record a stable unresolved key so overrides can steer.
                self.emit_macro_unresolved(item, containing);
            }
            _ => {}
        }
    }

    fn visit_item_mod(
        &mut self,
        item: &ItemMod,
        containing: &ParsedFile,
        module_path: &ModulePath,
    ) {
        let name = item.ident.to_string();
        let attr_path = read_path_attribute(&item.attrs);
        match &item.content {
            // Inline `mod name { ... }` — no external file, but descend for
            // nested `use` / `mod` items.
            Some((_, items)) => {
                let nested = module_path.child(&name);
                for nested_item in items {
                    self.visit_item(nested_item, containing, &nested);
                }
            }
            // File-backed `mod name;` — resolve to disk and emit an edge.
            None => {
                let resolution = resolve_mod(
                    self.workspace,
                    &containing.repo_relative,
                    &name,
                    attr_path.as_deref(),
                );
                let span = item.span();
                match resolution {
                    ModResolution::ManagedFile { repo_relative } => {
                        let is_library = self.workspace.is_library_target(&repo_relative);
                        if is_library {
                            // Library targets are true dependencies. Emit
                            // Internal and stop — a.rs owns its own
                            // analysis and will surface its inner uses.
                            self.push_dependency(Dependency::Internal {
                                path: repo_relative.clone(),
                                location: Some(location_of(containing, span)),
                            });
                        } else if self.should_recurse_into_file_mod()
                            && let Ok(parsed) = self.load(containing, &repo_relative, span)
                        {
                            // Solution-internal submodule: no dependency
                            // edge for the crate itself, but its uses may
                            // reference libraries transitively — recurse.
                            let nested = module_path.child(&name);
                            self.visit_file(parsed, nested);
                        }
                    }
                    ModResolution::UnmanagedFile { repo_relative } => {
                        self.mark_partial();
                        self.push_dependency(Dependency::Unresolved {
                            key: format!("mod:{}::{name}", module_path.display()),
                            display: format!("mod {name} → {repo_relative} (unmanaged)"),
                            location: Some(location_of(containing, span)),
                        });
                    }
                    ModResolution::Missing { candidates } => {
                        self.mark_partial();
                        self.push_dependency(Dependency::Unresolved {
                            key: format!("mod:{}::{name}", module_path.display()),
                            display: format!(
                                "mod {name} not found (looked at {})",
                                candidates.join(", ")
                            ),
                            location: Some(location_of(containing, span)),
                        });
                    }
                    ModResolution::Ambiguous { candidates } => {
                        self.mark_partial();
                        self.push_dependency(Dependency::Unresolved {
                            key: format!("mod:{}::{name}", module_path.display()),
                            display: format!("mod {name} is ambiguous ({})", candidates.join(", ")),
                            location: Some(location_of(containing, span)),
                        });
                    }
                }
            }
        }
    }

    fn load(
        &mut self,
        containing: &ParsedFile,
        repo_relative: &str,
        span: proc_macro2::Span,
    ) -> Result<ParsedFile, ()> {
        match load_file(self.workspace, repo_relative) {
            Ok(f) => Ok(f),
            Err(err) => {
                self.mark_partial();
                self.diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    code: "rust.parse.child_module".into(),
                    message: format!("failed to parse {repo_relative}: {err}"),
                    location: Some(location_of(containing, span)),
                });
                Err(())
            }
        }
    }

    fn visit_item_use(&mut self, item: &ItemUse, containing: &ParsedFile) {
        let base_span = item.span();
        for path in collect_use_paths(&item.tree) {
            self.classify_path(&path, containing, base_span);
        }
    }

    fn classify_path(&mut self, path: &UsePath, containing: &ParsedFile, span: proc_macro2::Span) {
        // Absolute / rooted paths (`::foo::bar`) are canonical; the resolver
        // treats them identically to `foo::bar` here because we do not know
        // the fully-qualified crate graph.
        let (first, rest) = match path.segments.split_first() {
            Some(v) => v,
            None => return,
        };
        let display = path.display();
        if path.glob {
            self.mark_partial();
            self.push_dependency(Dependency::Unresolved {
                key: format!("glob:{display}"),
                display: format!("use {display}"),
                location: Some(location_of(containing, span)),
            });
            return;
        }
        match first.as_str() {
            "crate" | "self" | "super" => {
                // Internal to this crate. Try to resolve to a specific file.
                // If any segment leads to a managed `.rs` file, that's the
                // edge; otherwise Unresolved (the target may be a
                // symbol-inside-file that the analyzer cannot address).
                self.classify_internal(first, rest, containing, span, &display);
            }
            _ if first.as_str() == self.krate.package_name.as_str() => {
                // Absolute use of the crate's own name — treat like `crate::`.
                self.classify_internal("crate", rest, containing, span, &display);
            }
            _ => {
                // External. We emit the full qualified path so downstream
                // consumers (site render, override matching) can index by
                // exact reference. Duplicates are normalized away later.
                self.push_dependency(Dependency::External {
                    name: display,
                    location: Some(location_of(containing, span)),
                });
            }
        }
    }

    fn classify_internal(
        &mut self,
        root: &str,
        rest: &[String],
        containing: &ParsedFile,
        span: proc_macro2::Span,
        display: &str,
    ) {
        // Attempt to walk `rest` segment by segment, mapping the deepest
        // matching prefix to a file. `crate::a::b::Foo` should emit an
        // Internal edge to the file that defines module `a::b`.
        let resolution = walk_crate_relative(self.workspace, self.krate, root, rest);
        match resolution {
            Some(path) if self.workspace.is_library_target(&path) => {
                self.push_dependency(Dependency::Internal {
                    path,
                    location: Some(location_of(containing, span)),
                });
            }
            Some(_) => {
                // Resolved to a solution-internal file. Not a "dependency"
                // for site rendering — drop it silently so state stays
                // `complete` when nothing else is unresolved.
            }
            None => {
                self.mark_partial();
                self.push_dependency(Dependency::Unresolved {
                    key: format!("internal:{display}"),
                    display: format!("use {display}"),
                    location: Some(location_of(containing, span)),
                });
            }
        }
    }

    fn emit_extern_crate(&mut self, name: &str, containing: &ParsedFile, span: proc_macro2::Span) {
        self.push_dependency(Dependency::External {
            name: name.to_string(),
            location: Some(location_of(containing, span)),
        });
    }

    fn emit_macro_unresolved(&mut self, item: &Item, containing: &ParsedFile) {
        self.mark_partial();
        let span = item.span();
        let key = match item {
            Item::Macro(m) => {
                let display = m
                    .mac
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect::<Vec<_>>()
                    .join("::");
                format!("macro:{display}")
            }
            _ => "macro:unknown".to_string(),
        };
        self.push_dependency(Dependency::Unresolved {
            key,
            display: "item-level macro invocation may hide use / mod statements".into(),
            location: Some(location_of(containing, span)),
        });
    }

    fn push_dependency(&mut self, dep: Dependency) {
        self.dependencies.push(dep);
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Fully-qualified module path (relative to the crate root). Used only for
/// stable Unresolved keys, never emitted directly.
#[derive(Debug, Default, Clone)]
struct ModulePath {
    segments: Vec<String>,
}

impl ModulePath {
    fn child(&self, name: &str) -> Self {
        let mut next = self.segments.clone();
        next.push(name.to_string());
        Self { segments: next }
    }

    fn display(&self) -> String {
        if self.segments.is_empty() {
            "crate".into()
        } else {
            iter::once("crate".to_string())
                .chain(self.segments.iter().cloned())
                .collect::<Vec<_>>()
                .join("::")
        }
    }
}

/// Flattened representation of one `use` leaf.
#[derive(Debug, Clone)]
struct UsePath {
    /// Path segments (excluding any renamed final ident). `crate` / `self` /
    /// `super` remain as the leading segment.
    segments: Vec<String>,
    /// True when the leaf was a glob `*` — the caller emits Unresolved.
    glob: bool,
}

impl UsePath {
    fn display(&self) -> String {
        let mut s = self.segments.join("::");
        if self.glob {
            if s.is_empty() {
                s.push('*');
            } else {
                s.push_str("::*");
            }
        }
        s
    }
}

fn collect_use_paths(tree: &UseTree) -> Vec<UsePath> {
    fn walk(tree: &UseTree, prefix: &[String], out: &mut Vec<UsePath>) {
        match tree {
            UseTree::Path(p) => {
                let mut next: Vec<String> = prefix.to_vec();
                next.push(p.ident.to_string());
                walk(&p.tree, &next, out);
            }
            UseTree::Name(n) => {
                let mut segments: Vec<String> = prefix.to_vec();
                segments.push(n.ident.to_string());
                out.push(UsePath {
                    segments,
                    glob: false,
                });
            }
            UseTree::Rename(r) => {
                // `use foo::bar as baz;` — the reference is still `foo::bar`.
                let mut segments: Vec<String> = prefix.to_vec();
                segments.push(r.ident.to_string());
                out.push(UsePath {
                    segments,
                    glob: false,
                });
            }
            UseTree::Glob(_) => {
                out.push(UsePath {
                    segments: prefix.to_vec(),
                    glob: true,
                });
            }
            UseTree::Group(g) => {
                for item in &g.items {
                    walk(item, prefix, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(tree, &[], &mut out);
    out
}

fn read_path_attribute(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if !matches!(attr.style, AttrStyle::Outer) {
            continue;
        }
        if !attr.path().is_ident("path") {
            continue;
        }
        if let Meta::NameValue(nv) = &attr.meta
            && let Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            }) = &nv.value
        {
            return Some(s.value());
        }
    }
    None
}

/// Walk a `crate`/`self`/`super`-relative use path against the on-disk
/// module tree, honoring `#[path]` attributes on `mod` items. Returns the
/// repo-relative path of the deepest file the reference resolves to, or
/// `None` when nothing matched.
///
/// Because the walker parses each intermediate file, it stops as soon as
/// a segment fails to resolve — the deepest previously-resolved file is
/// still emitted so downstream consumers see the coarsest-known
/// dependency instead of losing the whole edge.
fn walk_crate_relative(
    workspace: &RustWorkspace,
    krate: &RustCrate,
    _root: &str,
    rest: &[String],
) -> Option<String> {
    let mut current_file = krate.root_file.clone();
    let mut resolved_something = false;
    for segment in rest {
        let explicit = mod_path_attribute(workspace, &current_file, segment);
        let step = resolve_mod(workspace, &current_file, segment, explicit.as_deref());
        match step {
            ModResolution::ManagedFile { repo_relative } => {
                current_file = repo_relative;
                resolved_something = true;
                // Library files terminate the walk — they are their own
                // synthetic crates and their inner mod tree does not
                // contribute to the caller's dependency edges.
                if workspace.is_library_target(&current_file) {
                    break;
                }
            }
            _ => break,
        }
    }
    if resolved_something && workspace.is_managed(&current_file) {
        Some(current_file)
    } else {
        None
    }
}

/// Look up an explicit `#[path="…"]` attribute for a `mod <name>;` inside
/// `containing_file`. Returns `None` when the file cannot be parsed or does
/// not declare that module. Best-effort; the resolver falls back to the
/// filename-based lookup when this returns `None`.
fn mod_path_attribute(
    workspace: &RustWorkspace,
    containing_file: &str,
    module_name: &str,
) -> Option<String> {
    let parsed = load_file(workspace, containing_file).ok()?;
    for item in &parsed.file.items {
        if let Item::Mod(m) = item
            && m.ident == module_name
            && let Some(explicit) = read_path_attribute(&m.attrs)
        {
            return Some(explicit);
        }
    }
    None
}

fn location_of(containing: &ParsedFile, span: proc_macro2::Span) -> Location {
    let start = span.start();
    let end = span.end();
    Location {
        path: containing.repo_relative.clone(),
        start: Position {
            line: start.line as u32,
            column: Some((start.column as u32).saturating_add(1)),
        },
        end: Some(Position {
            line: end.line as u32,
            column: Some((end.column as u32).saturating_add(1)),
        }),
    }
}

// ─── Utility exposed to callers ─────────────────────────────────────────────

/// Analyze a request end-to-end and return a `library_adapter_protocol`
/// response's `libraries` and `solutions` vectors.
///
/// This is the shape `main.rs` needs: consume `AnalysisRequest`, produce two
/// vectors that can be assembled into an `AnalysisResponse`.
pub fn analyze_request(
    request: &AnalysisRequest,
    workspace: &RustWorkspace,
) -> (Vec<LibraryAnalysis>, Vec<SolutionAnalysis>) {
    let analyses = analyze_dependencies(request, workspace);
    let mut libraries = Vec::new();
    let mut solutions = Vec::new();
    for a in analyses {
        match a {
            TargetDependencyAnalysis::Library(l) => libraries.push(l),
            TargetDependencyAnalysis::Solution(s) => solutions.push(s),
        }
    }
    (libraries, solutions)
}
