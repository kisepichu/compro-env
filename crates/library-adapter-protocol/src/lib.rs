//! Strict adapter protocol v1 shared by the Rust core and every language analyzer.
//!
//! The types in this crate form the JSON contract that flows in and out of every
//! adapter process. They are intentionally focused on serialization: business
//! rules (discovery, normalization, override) live in the core, not here.

pub mod schema;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Adapter protocol version implemented by this crate.
pub const SCHEMA_VERSION: u32 = 1;

/// Raised when a response reports a `schema_version` the core cannot serve.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("unsupported adapter protocol schema_version {actual}; expected {expected}")]
pub struct ProtocolVersionError {
    pub actual: u32,
    pub expected: u32,
}

/// Rejects any `schema_version` other than the one this crate implements.
pub fn validate_version(actual: u32) -> Result<(), ProtocolVersionError> {
    if actual == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(ProtocolVersionError {
            actual,
            expected: SCHEMA_VERSION,
        })
    }
}

// ─── Request ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisRequest {
    pub schema_version: u32,
    pub repository_root: String,
    pub language: String,
    pub libraries: Vec<LibraryTarget>,
    pub solutions: Vec<SolutionTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibraryTarget {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolutionTarget {
    pub id: String,
    pub root: String,
    pub entry: String,
}

// ─── Response ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisResponse {
    pub schema_version: u32,
    pub adapter: AdapterIdentity,
    pub libraries: Vec<LibraryAnalysis>,
    pub solutions: Vec<SolutionAnalysis>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdapterIdentity {
    pub name: String,
    pub version: String,
    pub toolchains: Vec<ToolchainIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolchainIdentity {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibraryAnalysis {
    pub path: String,
    pub dependency_analysis: DependencyAnalysis,
    pub symbol_analysis: SymbolAnalysis,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolutionAnalysis {
    pub id: String,
    pub dependency_analysis: DependencyAnalysis,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

// ─── Analysis payloads ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DependencyAnalysis {
    pub state: AnalysisState,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymbolAnalysis {
    pub state: AnalysisState,
    #[serde(default)]
    pub symbols: Vec<Symbol>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisState {
    Complete,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Dependency {
    Internal {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<Location>,
    },
    External {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<Location>,
    },
    Unresolved {
        key: String,
        display: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<Location>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Symbol {
    pub name: String,
    /// Adapter-defined kind token (see spec §6.3).
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub path: String,
    pub start: Position,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Position>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}
