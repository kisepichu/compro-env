//! Rust symbol projection for the ce-rust adapter (plan 044 Task 1).
//!
//! The walker parses a single source file and returns every declaration item
//! as a language-neutral [`Symbol`]. It never opens other files: caller-side
//! logic (usually `main.rs`) picks the library's source text, hands it in, and
//! keeps dependency analysis on the module-graph resolver.
//!
//! Kinds are lowercase adapter tokens the core does not interpret:
//! `mod`, `struct`, `enum`, `enum_variant`, `union`, `trait`, `impl`, `type`,
//! `fn`, `method`, `const`, `static`, `macro`. Trait items become their
//! declaration kind (`method`, `type`, `const`); impl items become `method`,
//! `type`, `const`. Impl blocks are containers — no impl-level symbol is
//! fabricated, per plan 044 Task 1.
//!
//! Location fields follow the shared protocol: 1-based line/column in Unicode
//! scalar values. `proc-macro2` with `span-locations` already reports
//! `LineColumn` in USV units, so the walker forwards them verbatim (column +1).
//!
//! Location validation (plan 044 Task 2): spans with `line == 0` (macro
//! call-site placeholders) or with `end < start` are rejected before
//! serialization. The symbol is still emitted — search still benefits from
//! the name — but the analysis state degrades to `partial` so callers can
//! see the catalog is incomplete. Full parse failure produces `failed` with
//! no symbols; item-level macro invocations produce `partial` with the items
//! syn *could* see.

use library_adapter_protocol::{AnalysisState, Location, Position, Symbol, SymbolAnalysis};
use syn::{
    ImplItem, Item, ItemEnum, ItemImpl, ItemMod, ItemStruct, ItemTrait, ItemUnion, TraitItem, Type,
    spanned::Spanned,
};

/// Analyze one file's declarations.
///
/// * `source` — full UTF-8 body of the library file.
/// * `target_path` — repository-relative POSIX path emitted into every
///   `Location.path`.
/// * `module_path` — the caller's module prefix. Empty for MVP: symbols are
///   qualified starting from the file itself.
pub fn analyze_symbols(source: &str, target_path: &str, module_path: &[String]) -> SymbolAnalysis {
    let file = match syn::parse_file(source) {
        Ok(f) => f,
        Err(_) => {
            return SymbolAnalysis {
                state: AnalysisState::Failed,
                symbols: vec![],
            };
        }
    };
    let mut collector = SymbolCollector {
        target_path,
        symbols: Vec::new(),
        state: AnalysisState::Complete,
    };
    for item in &file.items {
        collector.visit_item(item, module_path);
    }
    SymbolAnalysis {
        state: collector.state,
        symbols: collector.symbols,
    }
}

struct SymbolCollector<'a> {
    target_path: &'a str,
    symbols: Vec<Symbol>,
    state: AnalysisState,
}

impl SymbolCollector<'_> {
    fn mark_partial(&mut self) {
        if let AnalysisState::Complete = self.state {
            self.state = AnalysisState::Partial;
        }
    }

    fn visit_item(&mut self, item: &Item, module_path: &[String]) {
        match item {
            Item::Mod(m) => self.visit_item_mod(m, module_path),
            Item::Struct(s) => self.visit_item_struct(s, module_path),
            Item::Enum(e) => self.visit_item_enum(e, module_path),
            Item::Union(u) => self.visit_item_union(u, module_path),
            Item::Trait(t) => self.visit_item_trait(t, module_path),
            Item::Impl(i) => self.visit_item_impl(i, module_path),
            Item::Fn(f) => {
                self.emit(&f.sig.ident.to_string(), "fn", f.span(), module_path);
            }
            Item::Type(t) => {
                self.emit(&t.ident.to_string(), "type", t.span(), module_path);
            }
            Item::Const(c) => {
                self.emit(&c.ident.to_string(), "const", c.span(), module_path);
            }
            Item::Static(s) => {
                self.emit(&s.ident.to_string(), "static", s.span(), module_path);
            }
            Item::Macro(m) => {
                // `macro_rules! name { ... }` is a declaration; `foo!(...)` at
                // item position is an invocation that may generate further
                // items the visitor cannot see. Emit the declaration form and
                // mark partial for the invocation form.
                if let Some(ident) = &m.ident {
                    self.emit(&ident.to_string(), "macro", m.span(), module_path);
                } else {
                    self.mark_partial();
                }
            }
            _ => {}
        }
    }

    fn visit_item_mod(&mut self, m: &ItemMod, module_path: &[String]) {
        let name = m.ident.to_string();
        self.emit(&name, "mod", m.span(), module_path);
        if let Some((_, items)) = &m.content {
            let nested = extend(module_path, &name);
            for it in items {
                self.visit_item(it, &nested);
            }
        }
        // `mod foo;` without content is resolved through the dependency
        // resolver; no further symbols to emit here.
    }

    fn visit_item_struct(&mut self, s: &ItemStruct, module_path: &[String]) {
        self.emit(&s.ident.to_string(), "struct", s.span(), module_path);
    }

    fn visit_item_enum(&mut self, e: &ItemEnum, module_path: &[String]) {
        let name = e.ident.to_string();
        self.emit(&name, "enum", e.span(), module_path);
        let inside = extend(module_path, &name);
        for variant in &e.variants {
            self.emit(
                &variant.ident.to_string(),
                "enum_variant",
                variant.span(),
                &inside,
            );
        }
    }

    fn visit_item_union(&mut self, u: &ItemUnion, module_path: &[String]) {
        self.emit(&u.ident.to_string(), "union", u.span(), module_path);
    }

    fn visit_item_trait(&mut self, t: &ItemTrait, module_path: &[String]) {
        let name = t.ident.to_string();
        self.emit(&name, "trait", t.span(), module_path);
        let inside = extend(module_path, &name);
        for tr_item in &t.items {
            match tr_item {
                TraitItem::Fn(f) => {
                    self.emit(&f.sig.ident.to_string(), "method", f.span(), &inside);
                }
                TraitItem::Type(ty) => {
                    self.emit(&ty.ident.to_string(), "type", ty.span(), &inside);
                }
                TraitItem::Const(c) => {
                    self.emit(&c.ident.to_string(), "const", c.span(), &inside);
                }
                _ => {}
            }
        }
    }

    fn visit_item_impl(&mut self, i: &ItemImpl, module_path: &[String]) {
        // `impl Type { ... }` and `impl Trait for Type { ... }` are containers
        // that scope their inner method / type / const declarations to `Type`.
        // The impl block itself does not become a fabricated symbol (plan 044
        // constraint), but its items do — qualified under the target type when
        // the type can be named.
        let inside = match impl_target_name(&i.self_ty) {
            Some(target) => extend(module_path, &target),
            None => module_path.to_vec(),
        };
        for impl_item in &i.items {
            match impl_item {
                ImplItem::Fn(f) => {
                    self.emit(&f.sig.ident.to_string(), "method", f.span(), &inside);
                }
                ImplItem::Type(ty) => {
                    self.emit(&ty.ident.to_string(), "type", ty.span(), &inside);
                }
                ImplItem::Const(c) => {
                    self.emit(&c.ident.to_string(), "const", c.span(), &inside);
                }
                _ => {}
            }
        }
    }

    fn emit(&mut self, name: &str, kind: &str, span: proc_macro2::Span, module_path: &[String]) {
        let name = name.to_string();
        let qualified_name = qualified(module_path, &name);
        let mut search_names = vec![name.clone()];
        if let Some(q) = &qualified_name
            && q != &name
        {
            search_names.push(q.clone());
        }
        let (location, dropped) = self.build_location(span);
        if dropped {
            // A syntactically valid item whose span is unusable (macro-generated
            // token, reversed range, zero-line placeholder). Keep the symbol —
            // its name/kind are still useful for search — but degrade to
            // `partial` so callers know the location catalog is incomplete.
            self.mark_partial();
        }
        self.symbols.push(Symbol {
            name,
            kind: kind.into(),
            qualified_name,
            search_names,
            signature: None,
            location,
        });
    }

    /// Convert a proc-macro2 span into a protocol `Location`, or `None` when
    /// the span carries no usable coordinates. The boolean flag distinguishes
    /// "the item has no span metadata at all" (no dropping — trailing items
    /// never had one) from "we rejected a bogus span" (drop → mark partial).
    fn build_location(&self, span: proc_macro2::Span) -> (Option<Location>, bool) {
        let start = span.start();
        let end = span.end();
        // `proc-macro2` returns line = 0 for spans that carry no origin info
        // (call-site placeholders, macro-generated tokens). That is not a
        // representable location and must not be serialized.
        if start.line == 0 || end.line == 0 {
            return (None, true);
        }
        // Reversed `end < start` is a protocol error per spec §6.3 — drop it
        // instead of shipping an out-of-order span.
        if (end.line, end.column) < (start.line, start.column) {
            return (None, true);
        }
        (
            Some(Location {
                path: self.target_path.to_string(),
                start: Position {
                    line: start.line as u32,
                    column: Some((start.column as u32).saturating_add(1)),
                },
                end: Some(Position {
                    line: end.line as u32,
                    column: Some((end.column as u32).saturating_add(1)),
                }),
            }),
            false,
        )
    }
}

fn qualified(module_path: &[String], name: &str) -> Option<String> {
    if module_path.is_empty() {
        None
    } else {
        let mut segments: Vec<String> = module_path.to_vec();
        segments.push(name.into());
        Some(segments.join("::"))
    }
}

fn extend(module_path: &[String], segment: &str) -> Vec<String> {
    let mut v: Vec<String> = module_path.to_vec();
    v.push(segment.into());
    v
}

/// Extract a stable display name from an impl target type.
///
/// Only handles path types like `Foo` or `crate::Foo` — the last segment's
/// identifier becomes the qualifier. Complex types (`&Foo<T>`, tuples,
/// references) collapse to `None`, so their impl items fall back to the
/// containing module's qualified name space.
fn impl_target_name(ty: &Type) -> Option<String> {
    if let Type::Path(p) = ty {
        let last = p.path.segments.last()?;
        return Some(last.ident.to_string());
    }
    None
}
