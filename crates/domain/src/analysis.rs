//! Normalized analysis models used by the library platform.
//!
//! These types describe (a) the raw discovery output that the core hands to
//! adapters and (b) the immutable snapshot the pipeline produces after
//! normalizing adapter responses (spec §4, §6.4).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, FixedOffset};

use crate::library::{ExpectedToolchain, LanguageId, LibraryId};
use crate::solution::PublishedSolution;

// ─── Discovery output ───────────────────────────────────────────────────────

/// A managed library source file (spec §4.1).
///
/// `managed = true` covers every file that matches include/exclude even if it
/// is not published; only `published` files become Web pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryFile {
    pub id: LibraryId,
    pub language: LanguageId,
    pub source_path: String,
    pub description_path: Option<String>,
    pub published: bool,
    pub managed: bool,
    pub title: Option<String>,
}

/// Per-language discovery result stored inside a `DiscoveryManifest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredLanguage {
    pub id: LanguageId,
    pub root: String,
    pub display_name: String,
    pub description_path: Option<String>,
}

/// Non-fatal issues raised during discovery (per pre-decided default #3).
///
/// Kept as free-form so infrastructure can report zero-match include patterns,
/// stale sidecars, and similar warnings without failing the pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryDiagnostic {
    pub severity: DiscoverySeverity,
    pub code: String,
    pub message: String,
    pub language: Option<LanguageId>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoverySeverity {
    Warning,
    Error,
}

/// Immutable input passed to language adapters (spec §6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryManifest {
    pub languages: BTreeMap<LanguageId, DiscoveredLanguage>,
    /// Sorted by `LibraryId` (UTF-8 byte order).
    pub libraries: Vec<LibraryFile>,
    /// Only publish-eligible solutions (spec §6.2).
    pub solutions: Vec<PublishedSolution>,
    pub diagnostics: Vec<DiscoveryDiagnostic>,
}

// ─── Normalized analysis ─────────────────────────────────────────────────────

/// Independent dependency/symbol state stored per target (spec §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetAnalysisState {
    pub dependency_state: AnalysisState,
    pub symbol_state: AnalysisState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisState {
    Complete,
    Partial,
    Failed,
}

/// Direct internal edge between two managed sources (spec §4.3).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DirectEdge {
    pub from: LibraryId,
    pub to: LibraryId,
}

/// Normalized per-library analysis payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedLibraryAnalysis {
    pub id: LibraryId,
    pub state: TargetAnalysisState,
    /// Direct internal edges out of this file, sorted by target ID.
    pub direct_dependencies: Vec<LibraryId>,
    /// Symbols in declaration/source order, adapter-normalized.
    pub symbols: Vec<NormalizedSymbol>,
    pub diagnostics: Vec<NormalizedDiagnostic>,
}

/// Normalized per-solution analysis payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedSolutionAnalysis {
    pub solution_id: crate::library::SolutionId,
    pub dependency_state: AnalysisState,
    pub direct_dependencies: Vec<LibraryId>,
    pub diagnostics: Vec<NormalizedDiagnostic>,
}

/// Adapter-provided symbol metadata post-normalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedSymbol {
    pub name: String,
    pub kind: String,
    pub qualified_name: Option<String>,
    pub search_names: Vec<String>,
    pub signature: Option<String>,
    pub location: Option<SourceLocation>,
}

/// Post-normalization diagnostic (spec §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub location: Option<SourceLocation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// Source location shared by symbols, dependencies, and diagnostics (spec §6.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub path: String,
    pub start_line: u32,
    pub start_column: Option<u32>,
    pub end_line: Option<u32>,
    pub end_column: Option<u32>,
}

/// Aggregated per-language analysis state stored in an `AnalysisSnapshot`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedLanguageAnalysis {
    pub language: LanguageId,
    pub adapter_name: String,
    pub adapter_version: String,
    pub observed_toolchains: Vec<ExpectedToolchain>,
    pub libraries: BTreeMap<LibraryId, NormalizedLibraryAnalysis>,
    pub solutions: BTreeMap<crate::library::SolutionId, NormalizedSolutionAnalysis>,
}

/// Immutable normalized analysis output for one pipeline run (spec §6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisSnapshot {
    pub schema_version: u32,
    pub repository_revision: String,
    pub created_at: DateTime<FixedOffset>,
    pub discovery_hash: String,
    pub source_hashes: BTreeMap<String, String>,
    pub languages: BTreeMap<LanguageId, NormalizedLanguageAnalysis>,
    pub snapshot_hash: String,
}

impl AnalysisSnapshot {
    /// Returns the set of reverse edges (`to -> from`) derived from the direct
    /// dependency graph across all languages. Cycles are preserved.
    pub fn reverse_edges(&self) -> BTreeMap<LibraryId, BTreeSet<LibraryId>> {
        let mut rev: BTreeMap<LibraryId, BTreeSet<LibraryId>> = BTreeMap::new();
        for lang in self.languages.values() {
            for library in lang.libraries.values() {
                for target in &library.direct_dependencies {
                    rev.entry(target.clone())
                        .or_default()
                        .insert(library.id.clone());
                }
            }
        }
        rev
    }

    /// Returns the transitive closure of dependencies reachable from `start`
    /// via one or more direct edges, following edges across all languages.
    /// Cycles are safe: each node is visited at most once, and `start` itself
    /// is included in the result only when a cycle actually reaches back to
    /// it.
    pub fn transitive_closure(&self, start: &LibraryId) -> BTreeSet<LibraryId> {
        let mut edges: BTreeMap<LibraryId, Vec<LibraryId>> = BTreeMap::new();
        for lang in self.languages.values() {
            for library in lang.libraries.values() {
                edges.insert(library.id.clone(), library.direct_dependencies.clone());
            }
        }
        let mut visited: BTreeSet<LibraryId> = BTreeSet::new();
        let mut stack: Vec<LibraryId> = edges.get(start).cloned().unwrap_or_default();
        while let Some(node) = stack.pop() {
            if !visited.insert(node.clone()) {
                continue;
            }
            if let Some(next) = edges.get(&node) {
                for edge in next {
                    stack.push(edge.clone());
                }
            }
        }
        visited
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib(id: &str, language: &str) -> LibraryFile {
        LibraryFile {
            id: LibraryId::parse(id).unwrap(),
            language: LanguageId::parse(language).unwrap(),
            source_path: id.to_string(),
            description_path: None,
            published: true,
            managed: true,
            title: None,
        }
    }

    fn private_lib(id: &str, language: &str) -> LibraryFile {
        let mut f = lib(id, language);
        f.published = false;
        f
    }

    #[test]
    fn private_library_is_managed_but_not_published() {
        let file = private_lib("libraries/rust/private.rs", "rust");
        assert!(file.managed);
        assert!(!file.published);
    }

    #[test]
    fn dependency_and_symbol_states_are_independent() {
        let state = TargetAnalysisState {
            dependency_state: AnalysisState::Complete,
            symbol_state: AnalysisState::Partial,
        };
        assert_eq!(state.dependency_state, AnalysisState::Complete);
        assert_eq!(state.symbol_state, AnalysisState::Partial);
    }

    #[test]
    fn direct_dependencies_may_form_a_cycle() {
        let a = LibraryId::parse("libraries/rust/a.rs").unwrap();
        let b = LibraryId::parse("libraries/rust/b.rs").unwrap();
        let make = |id: &LibraryId, deps: Vec<LibraryId>| NormalizedLibraryAnalysis {
            id: id.clone(),
            state: TargetAnalysisState {
                dependency_state: AnalysisState::Complete,
                symbol_state: AnalysisState::Complete,
            },
            direct_dependencies: deps,
            symbols: vec![],
            diagnostics: vec![],
        };
        let mut libraries = BTreeMap::new();
        libraries.insert(a.clone(), make(&a, vec![b.clone()]));
        libraries.insert(b.clone(), make(&b, vec![a.clone()]));

        let lang = LanguageId::parse("rust").unwrap();
        let language = NormalizedLanguageAnalysis {
            language: lang.clone(),
            adapter_name: "test".into(),
            adapter_version: "0".into(),
            observed_toolchains: vec![],
            libraries,
            solutions: BTreeMap::new(),
        };
        let mut languages = BTreeMap::new();
        languages.insert(lang, language);

        let snapshot = AnalysisSnapshot {
            schema_version: 1,
            repository_revision: "rev".into(),
            created_at: DateTime::parse_from_rfc3339("2026-08-10T00:00:00+00:00").unwrap(),
            discovery_hash: "d".into(),
            source_hashes: BTreeMap::new(),
            languages,
            snapshot_hash: "h".into(),
        };

        let closure = snapshot.transitive_closure(&a);
        assert!(closure.contains(&b));
        assert!(closure.contains(&a));

        let reverse = snapshot.reverse_edges();
        assert!(reverse.get(&a).is_some_and(|s| s.contains(&b)));
        assert!(reverse.get(&b).is_some_and(|s| s.contains(&a)));
    }

    #[test]
    fn discovery_manifest_sorts_libraries_by_id_bytes() {
        let libs = vec![
            lib("libraries/rust/z.rs", "rust"),
            lib("libraries/rust/a.rs", "rust"),
            lib("libraries/cpp/a.hpp", "cpp"),
        ];
        let mut sorted = libs.clone();
        sorted.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(
            sorted.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
            vec![
                "libraries/cpp/a.hpp",
                "libraries/rust/a.rs",
                "libraries/rust/z.rs",
            ]
        );
    }
}
