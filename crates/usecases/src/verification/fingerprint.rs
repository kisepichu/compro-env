//! Verification closure and canonical fingerprints (spec §11).
//!
//! [`verification_closure`] gathers the transitive `internal` dependencies of
//! the direct verify targets plus the solution's own internal dependencies.
//! [`calculate_fingerprint`] hashes the field-framed canonical projection so
//! source bytes, IDs, OJ identity, adapter capabilities, and per-input
//! hashes each participate independently and can be reported as stale
//! reasons.
//!
//! Failures in dependency analysis (`partial` or `failed`) block the
//! fingerprint; symbol analysis failure does not, per spec §4.4 and §11.

use std::collections::{BTreeMap, BTreeSet};

use domain::analysis::{AnalysisSnapshot, AnalysisState};
use domain::library::{LanguageId, LibraryId, SolutionId};
use domain::online_judge::SubmissionCapabilities;
use domain::verification::{ContentHash, VerifyFingerprint};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Fingerprint schema version stamped into every [`VerifyFingerprint`]
/// computation (spec §11).
pub const FINGERPRINT_SCHEMA_VERSION: u32 = 1;

/// Errors that block fingerprint construction.
///
/// Every variant maps to a spec-defined refusal path: dependency-analysis
/// failure (§4.4, §11), missing solution or library entries in the analysis
/// snapshot (§6.4), and empty verify targets when the caller expected at
/// least one (§11).
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum FingerprintError {
    #[error("solution `{0}` is not present in the analysis snapshot")]
    UnknownSolution(SolutionId),
    #[error("library `{0}` is not present in the analysis snapshot")]
    UnknownLibrary(LibraryId),
    #[error("solution `{solution}` dependency analysis is `{state}`")]
    SolutionDependencyBlocked {
        solution: SolutionId,
        state: &'static str,
    },
    #[error("library `{library}` dependency analysis is `{state}`")]
    LibraryDependencyBlocked {
        library: LibraryId,
        state: &'static str,
    },
    #[error("solution `{0}` has no source bytes registered in the snapshot")]
    MissingSolutionSource(SolutionId),
    #[error("library `{0}` has no source bytes registered in the snapshot")]
    MissingLibrarySource(LibraryId),
}

/// Aggregate binding between the internal language ID and the OJ-specific
/// submission language ID used for fingerprinting (spec §11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OjBinding {
    pub oj: String,
    pub problem_id: String,
    pub language_id: LanguageId,
    pub oj_language_id: String,
}

/// Adapter identity contributing to the fingerprint (spec §11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterIdentity {
    pub name: String,
    pub version: String,
    pub capabilities: SubmissionCapabilities,
}

/// Source-bytes anchor for a single path (spec §11 "各入力の content hash").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintSource {
    pub path: String,
    pub bytes: Vec<u8>,
}

impl FingerprintSource {
    pub fn hash(&self) -> ContentHash {
        ContentHash::parse(&sha256_hex(&self.bytes))
            .expect("sha256_hex always emits sha256:<64-hex-lowercase>")
    }
}

/// Full input material to [`calculate_fingerprint`] (spec §11).
///
/// `verified_libraries` are the direct `[verify].libraries` entries after
/// override application; `dependency_library_sources` carries the closure of
/// internal dependencies (direct verifiers plus solution deps plus their
/// transitive closure) with source bytes so the hash can prove which files
/// contributed. `verify_config_hash` isolates the effective verify block so
/// a change to the config re-triggers verification without leaking the raw
/// TOML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintMaterial {
    pub solution_id: SolutionId,
    pub submitted_source: FingerprintSource,
    pub verified_libraries: BTreeSet<LibraryId>,
    pub dependency_library_sources: BTreeMap<LibraryId, FingerprintSource>,
    pub binding: OjBinding,
    pub adapter: AdapterIdentity,
    pub verify_config_hash: ContentHash,
}

/// Compute the verification closure for a solution (spec §11).
///
/// The closure combines:
/// - the explicit `[verify].libraries` entries;
/// - the solution's own direct internal dependencies;
/// - and every internal library reachable from either set through the
///   direct dependency graph.
///
/// Cycles are safe: each library appears at most once. Private (unpublished)
/// libraries participate; only the `internal` edges from
/// [`NormalizedLibraryAnalysis::direct_dependencies`] are followed. Any
/// closure member whose dependency state is `partial` or `failed` aborts
/// with [`FingerprintError::LibraryDependencyBlocked`], per spec §11.
///
/// [`NormalizedLibraryAnalysis::direct_dependencies`]:
/// domain::analysis::NormalizedLibraryAnalysis::direct_dependencies
pub fn verification_closure(
    solution_id: &SolutionId,
    explicit_verify_libraries: &BTreeSet<LibraryId>,
    snapshot: &AnalysisSnapshot,
) -> Result<BTreeSet<LibraryId>, FingerprintError> {
    let (solution_analysis, _lang) = find_solution(snapshot, solution_id)
        .ok_or_else(|| FingerprintError::UnknownSolution(solution_id.clone()))?;

    match solution_analysis.dependency_state {
        AnalysisState::Complete => {}
        AnalysisState::Partial => {
            return Err(FingerprintError::SolutionDependencyBlocked {
                solution: solution_id.clone(),
                state: "partial",
            });
        }
        AnalysisState::Failed => {
            return Err(FingerprintError::SolutionDependencyBlocked {
                solution: solution_id.clone(),
                state: "failed",
            });
        }
    }

    let mut library_edges: BTreeMap<LibraryId, Vec<LibraryId>> = BTreeMap::new();
    let mut library_states: BTreeMap<LibraryId, AnalysisState> = BTreeMap::new();
    for lang in snapshot.languages.values() {
        for (id, analysis) in &lang.libraries {
            library_edges.insert(id.clone(), analysis.direct_dependencies.clone());
            library_states.insert(id.clone(), analysis.state.dependency_state);
        }
    }

    let mut roots: BTreeSet<LibraryId> = BTreeSet::new();
    for lib in explicit_verify_libraries {
        if !library_edges.contains_key(lib) {
            return Err(FingerprintError::UnknownLibrary(lib.clone()));
        }
        roots.insert(lib.clone());
    }
    for lib in &solution_analysis.direct_dependencies {
        if !library_edges.contains_key(lib) {
            return Err(FingerprintError::UnknownLibrary(lib.clone()));
        }
        roots.insert(lib.clone());
    }

    let mut closure: BTreeSet<LibraryId> = BTreeSet::new();
    let mut stack: Vec<LibraryId> = roots.into_iter().collect();
    while let Some(node) = stack.pop() {
        if !closure.insert(node.clone()) {
            continue;
        }
        match library_states.get(&node) {
            Some(AnalysisState::Complete) => {}
            Some(AnalysisState::Partial) => {
                return Err(FingerprintError::LibraryDependencyBlocked {
                    library: node,
                    state: "partial",
                });
            }
            Some(AnalysisState::Failed) => {
                return Err(FingerprintError::LibraryDependencyBlocked {
                    library: node,
                    state: "failed",
                });
            }
            None => {
                return Err(FingerprintError::UnknownLibrary(node));
            }
        }
        if let Some(next) = library_edges.get(&node) {
            for edge in next {
                if !closure.contains(edge) {
                    stack.push(edge.clone());
                }
            }
        }
    }
    Ok(closure)
}

fn find_solution<'a>(
    snapshot: &'a AnalysisSnapshot,
    solution_id: &SolutionId,
) -> Option<(
    &'a domain::analysis::NormalizedSolutionAnalysis,
    &'a LanguageId,
)> {
    for (lang_id, lang) in &snapshot.languages {
        if let Some(analysis) = lang.solutions.get(solution_id) {
            return Some((analysis, lang_id));
        }
    }
    None
}

/// Compute the canonical fingerprint bytes for `material` and wrap them in a
/// [`VerifyFingerprint`] (spec §11).
///
/// The hash is stable across re-orderings of the input maps and does not
/// normalize the source bytes (no newline conversion, no encoding change).
/// Per-input content hashes are exposed on
/// [`FingerprintSource::hash`] so consumers can persist them alongside the
/// aggregate fingerprint to explain stale results.
pub fn calculate_fingerprint(
    material: &FingerprintMaterial,
) -> Result<VerifyFingerprint, FingerprintError> {
    let submitted_hash = material.submitted_source.hash();
    let mut library_hashes: BTreeMap<String, String> = BTreeMap::new();
    for (id, source) in &material.dependency_library_sources {
        library_hashes.insert(id.to_string(), source.hash().to_string());
    }
    // Every direct verify target must have a matching entry in
    // `dependency_library_sources` (the closure). The caller assembles the
    // closure via `verification_closure`, then attaches the corresponding
    // source bytes; if a direct verify target has no anchor, the aggregate
    // fingerprint would ignore its source and the stale-reason report
    // would be misleading. Additional closure entries beyond the direct
    // verifiers stay in `library_hashes` above so transitive changes still
    // shift the fingerprint.
    for lib in &material.verified_libraries {
        if !material.dependency_library_sources.contains_key(lib) {
            return Err(FingerprintError::MissingLibrarySource(lib.clone()));
        }
    }

    let payload = serde_json::json!({
        "schema_version": FINGERPRINT_SCHEMA_VERSION,
        "solution_id": material.solution_id.as_str(),
        "submitted_source_hash": submitted_hash.as_str(),
        "verified_libraries": material
            .verified_libraries
            .iter()
            .map(|l| l.as_str().to_string())
            .collect::<Vec<_>>(),
        "library_hashes": library_hashes,
        "verify_config_hash": material.verify_config_hash.as_str(),
        "binding": {
            "oj": material.binding.oj,
            "problem_id": material.binding.problem_id,
            "language_id": material.binding.language_id.as_str(),
            "oj_language_id": material.binding.oj_language_id,
        },
        "adapter": {
            "name": material.adapter.name,
            "version": material.adapter.version,
            "capabilities": material.adapter.capabilities,
        },
    });
    let bytes = canonical_json(&payload);
    let hex = sha256_hex(&bytes);
    Ok(VerifyFingerprint::parse(&hex).expect("sha256_hex emits a valid fingerprint string"))
}

/// Canonical hash of the resolved `[verify]` block used by
/// [`FingerprintMaterial::verify_config_hash`] (spec §11).
///
/// The hash covers the sorted library IDs plus the resolved
/// `oj_language_id`. Kept in one place so the verify pipeline (which
/// persists records) and the site-data generator (which recomputes the
/// current fingerprint for staleness detection) agree byte-for-byte.
pub fn hash_verify_config(verify: &domain::solution::VerifySpec) -> ContentHash {
    let mut libs: Vec<String> = verify.libraries.iter().map(|l| l.to_string()).collect();
    libs.sort();
    let json = serde_json::json!({
        "libraries": libs,
        "oj_language_id": verify.oj_language_id,
    });
    let text = serde_json::to_string(&json).expect("serializes");
    let hex = sha256_hex(text.as_bytes());
    ContentHash::parse(&hex).expect("static hash")
}

/// Convert a submission-port adapter descriptor into the domain-level
/// [`SubmissionCapabilities`] used inside [`AdapterIdentity`].
///
/// Shared between the verify pipeline (`build_plan_context`) and the
/// site-data current-fingerprint recomputation so both callers reach the
/// same capability set for the same starter.
pub fn capabilities_from_descriptor(
    descriptor: &crate::submission::SubmissionAdapterDescriptor,
) -> SubmissionCapabilities {
    use crate::submission::{
        RecoveryMode as PortRecoveryMode, ResultDetailLevel as PortResultDetail,
        SubmissionMode as PortMode,
    };
    use domain::online_judge::{
        RecoveryMode as DomRecoveryMode, ResultDetail as DomResultDetail, SubmissionMode as DomMode,
    };
    SubmissionCapabilities {
        submission_mode: match descriptor.submission_mode {
            PortMode::UnattendedTrackable => DomMode::UnattendedTrackable,
            PortMode::InteractiveTrackable => DomMode::InteractiveTrackable,
            PortMode::InteractiveUntrackable => DomMode::InteractiveUntrackable,
            PortMode::Unsupported => DomMode::Unsupported,
        },
        result_detail: match descriptor.result_detail {
            PortResultDetail::OverallOnly => DomResultDetail::OverallOnly,
            PortResultDetail::SummaryMetrics => DomResultDetail::SummaryMetrics,
            PortResultDetail::TestcaseDetails => DomResultDetail::TestcaseDetails,
        },
        recovery_mode: match descriptor.recovery_mode {
            PortRecoveryMode::Exact => DomRecoveryMode::Exact,
            PortRecoveryMode::BestEffort => DomRecoveryMode::BestEffort,
            PortRecoveryMode::None => DomRecoveryMode::None,
        },
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn canonical_json(value: &serde_json::Value) -> Vec<u8> {
    let text = serde_json::to_string(value).expect("Value serializes as JSON");
    let reparsed: serde_json::Value = serde_json::from_str(&text).expect("just serialized JSON");
    let mut out = Vec::with_capacity(text.len());
    serde_json::to_writer(&mut out, &reparsed).expect("compact serializer never fails");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use domain::analysis::{
        AnalysisSnapshot, DirectEdge, NormalizedLanguageAnalysis, NormalizedLibraryAnalysis,
        NormalizedSolutionAnalysis, TargetAnalysisState,
    };
    use domain::online_judge::{RecoveryMode, ResultDetail, SubmissionMode};

    fn language_id() -> LanguageId {
        LanguageId::parse("rust").unwrap()
    }

    fn library(
        id: &str,
        deps: Vec<&str>,
        state: AnalysisState,
        symbol: AnalysisState,
    ) -> NormalizedLibraryAnalysis {
        NormalizedLibraryAnalysis {
            id: LibraryId::parse(id).unwrap(),
            state: TargetAnalysisState {
                dependency_state: state,
                symbol_state: symbol,
            },
            direct_dependencies: deps
                .into_iter()
                .map(|d| LibraryId::parse(d).unwrap())
                .collect(),
            symbols: vec![],
            diagnostics: vec![],
        }
    }

    fn solution(id: &str, deps: Vec<&str>, state: AnalysisState) -> NormalizedSolutionAnalysis {
        NormalizedSolutionAnalysis {
            solution_id: SolutionId::parse(id).unwrap(),
            dependency_state: state,
            direct_dependencies: deps
                .into_iter()
                .map(|d| LibraryId::parse(d).unwrap())
                .collect(),
            diagnostics: vec![],
        }
    }

    fn snapshot_with(
        libraries: Vec<NormalizedLibraryAnalysis>,
        solutions: Vec<NormalizedSolutionAnalysis>,
    ) -> AnalysisSnapshot {
        let lang = language_id();
        let mut libs = BTreeMap::new();
        for l in libraries {
            libs.insert(l.id.clone(), l);
        }
        let mut sols = BTreeMap::new();
        for s in solutions {
            sols.insert(s.solution_id.clone(), s);
        }
        let mut languages = BTreeMap::new();
        languages.insert(
            lang.clone(),
            NormalizedLanguageAnalysis {
                language: lang,
                adapter_name: "test".into(),
                adapter_version: "0".into(),
                observed_toolchains: vec![],
                analyzer_command: vec!["test-analyzer".into()],
                libraries: libs,
                solutions: sols,
            },
        );
        AnalysisSnapshot {
            schema_version: 1,
            repository_revision: "rev".into(),
            created_at: DateTime::parse_from_rfc3339("2026-08-10T00:00:00+00:00").unwrap(),
            discovery_hash: "d".into(),
            source_hashes: BTreeMap::new(),
            languages,
            snapshot_hash: "h".into(),
        }
    }

    // Silence dead-code warning for the type alias while we prove the module
    // works stand-alone.
    #[allow(dead_code)]
    fn _use_direct_edge(_: DirectEdge) {}

    fn capabilities() -> SubmissionCapabilities {
        SubmissionCapabilities {
            submission_mode: SubmissionMode::UnattendedTrackable,
            result_detail: ResultDetail::TestcaseDetails,
            recovery_mode: RecoveryMode::BestEffort,
        }
    }

    fn binding() -> OjBinding {
        OjBinding {
            oj: "librarychecker".into(),
            problem_id: "aplusb".into(),
            language_id: LanguageId::parse("rust").unwrap(),
            oj_language_id: "rust".into(),
        }
    }

    fn adapter() -> AdapterIdentity {
        AdapterIdentity {
            name: "librarychecker".into(),
            version: "1.0.0".into(),
            capabilities: capabilities(),
        }
    }

    fn config_hash() -> ContentHash {
        ContentHash::parse(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap()
    }

    fn source(path: &str, body: &[u8]) -> FingerprintSource {
        FingerprintSource {
            path: path.into(),
            bytes: body.to_vec(),
        }
    }

    // ─── verification_closure ──────────────────────────────────────────────

    #[test]
    fn closure_walks_transitive_internal_edges_through_cycles() {
        let a = "libraries/rust/a.rs";
        let b = "libraries/rust/b.rs";
        let c = "libraries/rust/c.rs";
        let snapshot = snapshot_with(
            vec![
                library(a, vec![b], AnalysisState::Complete, AnalysisState::Complete),
                library(
                    b,
                    vec![c, a],
                    AnalysisState::Complete,
                    AnalysisState::Complete,
                ),
                library(c, vec![b], AnalysisState::Complete, AnalysisState::Complete),
            ],
            vec![solution("abc999/a/main", vec![], AnalysisState::Complete)],
        );
        let mut explicit = BTreeSet::new();
        explicit.insert(LibraryId::parse(a).unwrap());
        let closure = verification_closure(
            &SolutionId::parse("abc999/a/main").unwrap(),
            &explicit,
            &snapshot,
        )
        .unwrap();
        assert_eq!(closure.len(), 3);
        assert!(closure.contains(&LibraryId::parse(a).unwrap()));
        assert!(closure.contains(&LibraryId::parse(b).unwrap()));
        assert!(closure.contains(&LibraryId::parse(c).unwrap()));
    }

    #[test]
    fn closure_includes_private_transitive_libraries() {
        let a = "libraries/rust/public.rs";
        let private = "libraries/rust/private.rs";
        let snapshot = snapshot_with(
            vec![
                library(
                    a,
                    vec![private],
                    AnalysisState::Complete,
                    AnalysisState::Failed,
                ),
                library(
                    private,
                    vec![],
                    AnalysisState::Complete,
                    AnalysisState::Complete,
                ),
            ],
            vec![solution("abc999/a/main", vec![], AnalysisState::Complete)],
        );
        let mut explicit = BTreeSet::new();
        explicit.insert(LibraryId::parse(a).unwrap());
        let closure = verification_closure(
            &SolutionId::parse("abc999/a/main").unwrap(),
            &explicit,
            &snapshot,
        )
        .unwrap();
        assert!(closure.contains(&LibraryId::parse(private).unwrap()));
    }

    #[test]
    fn closure_symbol_failure_does_not_block() {
        let a = "libraries/rust/a.rs";
        let snapshot = snapshot_with(
            vec![library(
                a,
                vec![],
                AnalysisState::Complete,
                AnalysisState::Failed,
            )],
            vec![solution("abc999/a/main", vec![], AnalysisState::Complete)],
        );
        let mut explicit = BTreeSet::new();
        explicit.insert(LibraryId::parse(a).unwrap());
        assert!(
            verification_closure(
                &SolutionId::parse("abc999/a/main").unwrap(),
                &explicit,
                &snapshot,
            )
            .is_ok()
        );
    }

    #[test]
    fn closure_dependency_failed_blocks() {
        let a = "libraries/rust/a.rs";
        let snapshot = snapshot_with(
            vec![library(
                a,
                vec![],
                AnalysisState::Failed,
                AnalysisState::Complete,
            )],
            vec![solution("abc999/a/main", vec![], AnalysisState::Complete)],
        );
        let mut explicit = BTreeSet::new();
        explicit.insert(LibraryId::parse(a).unwrap());
        let err = verification_closure(
            &SolutionId::parse("abc999/a/main").unwrap(),
            &explicit,
            &snapshot,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            FingerprintError::LibraryDependencyBlocked { .. }
        ));
    }

    #[test]
    fn closure_dependency_partial_blocks() {
        let sol = SolutionId::parse("abc999/a/main").unwrap();
        let snapshot = snapshot_with(
            vec![],
            vec![solution(sol.as_str(), vec![], AnalysisState::Partial)],
        );
        let err = verification_closure(&sol, &BTreeSet::new(), &snapshot).unwrap_err();
        assert!(matches!(
            err,
            FingerprintError::SolutionDependencyBlocked { .. }
        ));
    }

    #[test]
    fn closure_unknown_solution_errors() {
        let snapshot = snapshot_with(vec![], vec![]);
        let err = verification_closure(
            &SolutionId::parse("abc999/a/main").unwrap(),
            &BTreeSet::new(),
            &snapshot,
        )
        .unwrap_err();
        assert!(matches!(err, FingerprintError::UnknownSolution(_)));
    }

    #[test]
    fn closure_unknown_library_errors() {
        let snapshot = snapshot_with(
            vec![],
            vec![solution("abc999/a/main", vec![], AnalysisState::Complete)],
        );
        let mut explicit = BTreeSet::new();
        explicit.insert(LibraryId::parse("libraries/rust/missing.rs").unwrap());
        let err = verification_closure(
            &SolutionId::parse("abc999/a/main").unwrap(),
            &explicit,
            &snapshot,
        )
        .unwrap_err();
        assert!(matches!(err, FingerprintError::UnknownLibrary(_)));
    }

    // ─── calculate_fingerprint ─────────────────────────────────────────────

    fn material(
        submitted: FingerprintSource,
        verified: Vec<&str>,
        deps: Vec<FingerprintSource>,
    ) -> FingerprintMaterial {
        let mut verified_libs: BTreeSet<LibraryId> = BTreeSet::new();
        for v in verified {
            verified_libs.insert(LibraryId::parse(v).unwrap());
        }
        let mut sources = BTreeMap::new();
        for s in deps {
            sources.insert(LibraryId::parse(&s.path).unwrap(), s);
        }
        FingerprintMaterial {
            solution_id: SolutionId::parse("abc999/a/main").unwrap(),
            submitted_source: submitted,
            verified_libraries: verified_libs,
            dependency_library_sources: sources,
            binding: binding(),
            adapter: adapter(),
            verify_config_hash: config_hash(),
        }
    }

    #[test]
    fn fingerprint_is_stable_across_insertion_order() {
        let libs_forward = vec![
            source("libraries/rust/a.rs", b"body-a"),
            source("libraries/rust/b.rs", b"body-b"),
        ];
        let libs_reverse = vec![
            source("libraries/rust/b.rs", b"body-b"),
            source("libraries/rust/a.rs", b"body-a"),
        ];
        let m1 = material(
            source("solutions/abc999/a/main/src/main.rs", b"fn main() {}"),
            vec!["libraries/rust/a.rs", "libraries/rust/b.rs"],
            libs_forward,
        );
        let m2 = material(
            source("solutions/abc999/a/main/src/main.rs", b"fn main() {}"),
            vec!["libraries/rust/b.rs", "libraries/rust/a.rs"],
            libs_reverse,
        );
        assert_eq!(
            calculate_fingerprint(&m1).unwrap(),
            calculate_fingerprint(&m2).unwrap()
        );
    }

    #[test]
    fn fingerprint_uses_source_bytes_without_newline_normalization() {
        let lf = material(
            source("solutions/abc999/a/main/src/main.rs", b"a\nb\n"),
            vec![],
            vec![],
        );
        let crlf = material(
            source("solutions/abc999/a/main/src/main.rs", b"a\r\nb\r\n"),
            vec![],
            vec![],
        );
        assert_ne!(
            calculate_fingerprint(&lf).unwrap(),
            calculate_fingerprint(&crlf).unwrap()
        );
    }

    #[test]
    fn fingerprint_changes_on_rename() {
        let m1 = material(
            source("solutions/abc999/a/main/src/main.rs", b"body"),
            vec!["libraries/rust/a.rs"],
            vec![source("libraries/rust/a.rs", b"lib")],
        );
        let mut m2 = m1.clone();
        m2.verified_libraries.clear();
        m2.verified_libraries
            .insert(LibraryId::parse("libraries/rust/renamed.rs").unwrap());
        m2.dependency_library_sources.clear();
        m2.dependency_library_sources.insert(
            LibraryId::parse("libraries/rust/renamed.rs").unwrap(),
            source("libraries/rust/renamed.rs", b"lib"),
        );
        assert_ne!(
            calculate_fingerprint(&m1).unwrap(),
            calculate_fingerprint(&m2).unwrap()
        );
    }

    #[test]
    fn fingerprint_changes_on_content() {
        let m1 = material(
            source("solutions/abc999/a/main/src/main.rs", b"body"),
            vec!["libraries/rust/a.rs"],
            vec![source("libraries/rust/a.rs", b"lib")],
        );
        let m2 = material(
            source("solutions/abc999/a/main/src/main.rs", b"body"),
            vec!["libraries/rust/a.rs"],
            vec![source("libraries/rust/a.rs", b"lib-different")],
        );
        assert_ne!(
            calculate_fingerprint(&m1).unwrap(),
            calculate_fingerprint(&m2).unwrap()
        );
    }

    #[test]
    fn fingerprint_changes_on_mapping() {
        let mut m1 = material(
            source("solutions/abc999/a/main/src/main.rs", b"body"),
            vec![],
            vec![],
        );
        let mut m2 = m1.clone();
        m2.binding.oj_language_id = "rust-2024".into();
        assert_ne!(
            calculate_fingerprint(&m1).unwrap(),
            calculate_fingerprint(&m2).unwrap()
        );

        // Also verify: internal language ID change flips fingerprint.
        m1.binding.language_id = LanguageId::parse("rust").unwrap();
        m2 = m1.clone();
        m2.binding.language_id = LanguageId::parse("cpp").unwrap();
        assert_ne!(
            calculate_fingerprint(&m1).unwrap(),
            calculate_fingerprint(&m2).unwrap()
        );
    }

    #[test]
    fn fingerprint_changes_on_capabilities() {
        let m1 = material(
            source("solutions/abc999/a/main/src/main.rs", b"body"),
            vec![],
            vec![],
        );
        let mut m2 = m1.clone();
        m2.adapter.capabilities.result_detail = ResultDetail::OverallOnly;
        assert_ne!(
            calculate_fingerprint(&m1).unwrap(),
            calculate_fingerprint(&m2).unwrap()
        );

        let mut m3 = m1.clone();
        m3.adapter.version = "2.0.0".into();
        assert_ne!(
            calculate_fingerprint(&m1).unwrap(),
            calculate_fingerprint(&m3).unwrap()
        );
    }

    #[test]
    fn fingerprint_verified_library_without_source_errors() {
        let m = material(
            source("solutions/abc999/a/main/src/main.rs", b"body"),
            vec!["libraries/rust/a.rs"],
            vec![],
        );
        let err = calculate_fingerprint(&m).unwrap_err();
        assert!(matches!(err, FingerprintError::MissingLibrarySource(_)));
    }
}
