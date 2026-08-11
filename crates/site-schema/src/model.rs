//! Public DTO model.
//!
//! `#[serde(deny_unknown_fields)]` is applied to every struct so both writers
//! (Rust core) and readers (Astro build) fail fast on unknown keys instead of
//! silently dropping data. Optional fields default to `None`, never to
//! redacted string placeholders.

use serde::{Deserialize, Serialize};

/// Root document written to `site-data.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SiteData {
    pub schema_version: u32,
    pub build: BuildMetadata,
    pub site: SiteMetadata,
    pub languages: Vec<LanguageSummary>,
    pub libraries: Vec<LibraryPageData>,
    pub solutions: Vec<SolutionPageData>,
}

/// Immutable evidence about how this build was produced (spec §12.14, §15.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildMetadata {
    pub schema_version: u32,
    pub generated_at: String,
    pub mode: BuildMode,
    pub source_commit_sha: String,
    pub source_commit_short_sha: String,
    pub source_committed_at: String,
    pub uncommitted_changes: bool,
    pub observed_toolchains: Vec<ToolchainIdentity>,
    pub adapters: Vec<AdapterIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BuildMode {
    Production,
    Preview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolchainIdentity {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdapterIdentity {
    pub language: String,
    pub name: String,
    pub version: String,
}

/// Repository-level publishing metadata (spec §6, `[library.site]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SiteMetadata {
    pub title: String,
    pub description: String,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub repository_url: Option<String>,
}

/// Public-only language card (spec §12.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LanguageSummary {
    pub id: String,
    pub display_name: String,
    pub syntax_highlight: String,
    pub library_count: u32,
    pub verification_summary: VerificationCounts,
}

/// Rolled-up counts per public verification status.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(deny_unknown_fields)]
pub struct VerificationCounts {
    pub verified: u32,
    pub rejected: u32,
    pub unavailable: u32,
    pub stale: u32,
    pub never: u32,
}

// ─── Library page ────────────────────────────────────────────────────────────

/// Canonical public projection of a `LibraryFile` (spec §12.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibraryPageData {
    pub page_id: String,
    pub library_id: String,
    pub language: String,
    pub title: String,
    pub source_path: String,
    pub source: String,
    pub syntax_highlight: String,
    pub updated_at: String,
    pub updated_by_commit: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    pub symbol_analysis: SymbolAnalysisPublic,
    pub dependency_analysis: DependencyAnalysisPublic,
    pub reverse_dependencies: Vec<LibraryLink>,
    pub relations: Vec<RelationPublic>,
    pub verification: LibraryVerificationView,
    pub diagnostics: Vec<DiagnosticPublic>,
}

/// A cross-reference to another public library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibraryLink {
    pub library_id: String,
    pub language: String,
    pub title: String,
    pub source_path: String,
    pub manual: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelationPublic {
    pub kind: String,
    pub target: LibraryLink,
    pub manual: bool,
}

/// Symbol analysis projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymbolAnalysisPublic {
    pub state: AnalysisState,
    pub symbols: Vec<SymbolPublic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AnalysisState {
    Complete,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymbolPublic {
    pub kind: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub qualified_name: Option<String>,
    pub search_names: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub location: Option<PublicLocation>,
}

/// Dependency analysis projection.
///
/// `has_private_dependencies` is a *count-free* boolean — spec §4.4 forbids
/// leaking names or counts of private targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DependencyAnalysisPublic {
    pub state: AnalysisState,
    pub direct: Vec<LibraryLink>,
    pub transitive: Vec<LibraryLink>,
    pub has_private_dependencies: bool,
}

/// Location within the current page's source. `path` is intentionally absent
/// because it is always the library's own source. The Web layer generates
/// `#L<line>` anchors from `start.line`; symbol locations without valid lines
/// are rendered under a `#symbols` anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublicLocation {
    pub start: LinePosition,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub end: Option<LinePosition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinePosition {
    pub line: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub column: Option<u32>,
}

/// Aggregate verification state on a library page (spec §12.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibraryVerificationView {
    pub aggregate_status: LibraryVerificationStatus,
    pub evidence: Vec<VerificationEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LibraryVerificationStatus {
    Verified,
    Rejected,
    Unavailable,
    Stale,
    Never,
}

/// Latest verification observation for a `[verify].libraries` link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationEvidence {
    pub solution_id: String,
    pub solution_page_id: String,
    pub online_judge: String,
    pub status: EvidenceStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub verdict: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub judged_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub oj_submission_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stale_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Verified,
    Rejected,
    Unavailable,
    Stale,
    Never,
}

/// Public diagnostic entry (spec §4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticPublic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub location: Option<PublicLocation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

// ─── Solution page ───────────────────────────────────────────────────────────

/// Canonical public projection of a `Solution` (spec §12.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolutionPageData {
    pub page_id: String,
    pub solution_id: String,
    pub contest_id: String,
    pub problem_code: String,
    pub solution_name: String,
    pub online_judge: String,
    pub language: String,
    pub solved_at: String,
    pub source_path: String,
    pub source: String,
    pub syntax_highlight: String,
    pub has_preprocess: bool,
    pub verifies: Vec<LibraryLink>,
    pub direct_dependencies: Vec<LibraryLink>,
    pub has_private_dependencies: bool,
    pub verification: SolutionVerificationView,
    pub dependency_analysis_state: AnalysisState,
    pub diagnostics: Vec<SolutionDiagnosticPublic>,
}

/// Verification view on a solution page (spec §12.7, §12.8).
///
/// `not_configured` is only ever emitted here — libraries collapse to `never`
/// when they have no direct verifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolutionVerificationView {
    pub status: SolutionVerificationStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<VerificationResultPublic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SolutionVerificationStatus {
    Verified,
    Rejected,
    Unavailable,
    Stale,
    Never,
    NotConfigured,
}

/// Latest terminal verification result attached to a solution page.
///
/// Only publishable fields exist: no session/token/cookie/raw OJ payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationResultPublic {
    pub attempt_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub verdict: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub judged_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub oj_submission_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub execution_time_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub memory_kib: Option<u64>,
    pub submitted_source_hash: String,
    pub verify_fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stale_reason: Option<String>,
    pub testcases: Vec<TestcaseVerdictPublic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestcaseVerdictPublic {
    pub name: String,
    pub verdict: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub execution_time_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub memory_kib: Option<u64>,
}

/// Solution diagnostic — public location is limited to the entry source file.
///
/// When the diagnostic originally referred to a non-entry file, the projection
/// strips the location and populates `location_notice` with a fixed message
/// (spec §12, §12.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolutionDiagnosticPublic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub location: Option<PublicLocation>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub location_notice: Option<String>,
}
