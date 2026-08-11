/**
 * TypeScript projection of the site-data v1 JSON Schema.
 *
 * These types mirror `web/schema/site-data-v1.schema.json` and the
 * `site-schema` Rust crate. Every optional field is `null | undefined`
 * because the schema uses `anyOf` with `"type": "null"` rather than making
 * the key itself absent.
 */

export type AnalysisState = "complete" | "partial" | "failed";
export type BuildMode = "production" | "preview";
export type DiagnosticSeverity = "info" | "warning" | "error";
export type EvidenceStatus =
  | "verified"
  | "rejected"
  | "unavailable"
  | "stale"
  | "never";
export type LibraryVerificationStatus =
  | "verified"
  | "rejected"
  | "unavailable"
  | "stale"
  | "never";
export type SolutionVerificationStatus =
  | LibraryVerificationStatus
  | "not_configured";

export interface ToolchainIdentity {
  name: string;
  version: string;
  target?: string | null;
}

export interface AdapterIdentity {
  language: string;
  name: string;
  version: string;
}

export interface BuildMetadata {
  schema_version: number;
  generated_at: string;
  mode: BuildMode;
  source_commit_sha: string;
  source_commit_short_sha: string;
  source_committed_at: string;
  uncommitted_changes: boolean;
  observed_toolchains: ToolchainIdentity[];
  adapters: AdapterIdentity[];
}

export interface SiteMetadata {
  title: string;
  description: string;
  language: string;
  repository_url?: string | null;
}

export interface VerificationCounts {
  verified: number;
  rejected: number;
  unavailable: number;
  stale: number;
  never: number;
}

export interface LanguageSummary {
  id: string;
  display_name: string;
  syntax_highlight: string;
  library_count: number;
  verification_summary: VerificationCounts;
}

export interface LibraryLink {
  library_id: string;
  language: string;
  title: string;
  source_path: string;
  manual: boolean;
}

export interface RelationPublic {
  kind: string;
  target: LibraryLink;
  manual: boolean;
}

export interface LinePosition {
  line: number;
  column?: number | null;
}

export interface PublicLocation {
  start: LinePosition;
  end?: LinePosition | null;
}

export interface SymbolPublic {
  kind: string;
  name: string;
  qualified_name?: string | null;
  search_names: string[];
  signature?: string | null;
  location?: PublicLocation | null;
}

export interface SymbolAnalysisPublic {
  state: AnalysisState;
  symbols: SymbolPublic[];
}

export interface DependencyAnalysisPublic {
  state: AnalysisState;
  direct: LibraryLink[];
  transitive: LibraryLink[];
  has_private_dependencies: boolean;
}

export interface VerificationEvidence {
  solution_id: string;
  solution_page_id: string;
  online_judge: string;
  status: EvidenceStatus;
  verdict?: string | null;
  judged_at?: string | null;
  oj_submission_url?: string | null;
  stale_reason?: string | null;
}

export interface LibraryVerificationView {
  aggregate_status: LibraryVerificationStatus;
  evidence: VerificationEvidence[];
}

export interface DiagnosticPublic {
  severity: DiagnosticSeverity;
  code: string;
  message: string;
  location?: PublicLocation | null;
}

export interface LibraryPageData {
  page_id: string;
  library_id: string;
  language: string;
  title: string;
  source_path: string;
  source: string;
  syntax_highlight: string;
  updated_at: string;
  updated_by_commit: string;
  description?: string | null;
  symbol_analysis: SymbolAnalysisPublic;
  dependency_analysis: DependencyAnalysisPublic;
  reverse_dependencies: LibraryLink[];
  relations: RelationPublic[];
  verification: LibraryVerificationView;
  diagnostics: DiagnosticPublic[];
}

export interface TestcaseVerdictPublic {
  name: string;
  verdict: string;
  execution_time_ms?: number | null;
  memory_kib?: number | null;
}

export interface VerificationResultPublic {
  attempt_id: string;
  verdict?: string | null;
  judged_at?: string | null;
  oj_submission_url?: string | null;
  execution_time_ms?: number | null;
  memory_kib?: number | null;
  submitted_source_hash: string;
  verify_fingerprint: string;
  stale_reason?: string | null;
  testcases: TestcaseVerdictPublic[];
}

export interface SolutionVerificationView {
  status: SolutionVerificationStatus;
  result?: VerificationResultPublic | null;
}

export interface SolutionDiagnosticPublic {
  severity: DiagnosticSeverity;
  code: string;
  message: string;
  location?: PublicLocation | null;
  location_notice?: string | null;
}

export interface SolutionPageData {
  page_id: string;
  solution_id: string;
  contest_id: string;
  problem_code: string;
  solution_name: string;
  online_judge: string;
  language: string;
  solved_at: string;
  source_path: string;
  source: string;
  syntax_highlight: string;
  has_preprocess: boolean;
  verifies: LibraryLink[];
  direct_dependencies: LibraryLink[];
  has_private_dependencies: boolean;
  verification: SolutionVerificationView;
  dependency_analysis_state: AnalysisState;
  diagnostics: SolutionDiagnosticPublic[];
}

export interface SiteData {
  schema_version: number;
  build: BuildMetadata;
  site: SiteMetadata;
  languages: LanguageSummary[];
  libraries: LibraryPageData[];
  solutions: SolutionPageData[];
}
