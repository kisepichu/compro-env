//! Public projection of `AnalysisSnapshot` + verification state → `SiteData`.
//!
//! Every field on the produced `site_schema::SiteData` is a *public* view: this
//! module strips non-public libraries out of every dependency, relation,
//! diagnostic, and evidence link before the DTO is built (spec §12, §14).
//! Locations are only retained for symbols on the target file itself and for
//! diagnostics that land inside the entry source of the solution being
//! projected; anywhere else, the location is dropped and a `location_notice`
//! captures the fact that the referenced file is not displayed.
//!
//! The projection is pure: no filesystem, no clock, no git. Callers assemble
//! the immutable inputs into [`PublicProjectionInput`] and receive back a
//! deterministic [`SiteData`].

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, FixedOffset};
use thiserror::Error;

use domain::analysis::{
    AnalysisSnapshot, AnalysisState, DiscoveryManifest, LibraryFile, NormalizedDiagnostic,
    NormalizedLibraryAnalysis, NormalizedSolutionAnalysis, NormalizedSymbol,
};
use domain::library::{LanguageConfig, LanguageId, LibraryId, LibraryProjectConfig, SolutionId};
use domain::solution::PublishedSolution;
use domain::verification::{VerdictKind, VerificationRecord, VerificationState, VerifyFingerprint};
use site_schema::{
    AdapterIdentity, AnalysisState as PublicAnalysisState, BuildMetadata, BuildMode,
    DependencyAnalysisPublic, DiagnosticPublic, DiagnosticSeverity as PublicDiagnosticSeverity,
    EvidenceStatus, LanguageSummary, LibraryLink, LibraryPageData, LibraryVerificationStatus,
    LibraryVerificationView, LinePosition, PublicLocation, RelationPublic, SITE_SCHEMA_VERSION,
    SiteData, SiteMetadata, SolutionDiagnosticPublic, SolutionPageData, SolutionVerificationStatus,
    SolutionVerificationView, SymbolAnalysisPublic, SymbolPublic, TestcaseVerdictPublic,
    ToolchainIdentity, VerificationCounts, VerificationEvidence, VerificationResultPublic,
};

use crate::verification::fingerprint::FingerprintError;
use crate::verification::status::{
    VerificationStatus, classify_library_status, classify_solution_status,
};

// ─── Errors ─────────────────────────────────────────────────────────────────

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SiteDataError {
    #[error("production build requires a fully-populated site config")]
    MissingProductionSiteConfig,
    #[error("published library {id} is missing analysis data")]
    UnanalyzedPublishedLibrary { id: LibraryId },
    #[error("published solution {id} is missing analysis data")]
    UnanalyzedPublishedSolution { id: SolutionId },
    #[error("published library {id} has no source bytes registered")]
    MissingLibrarySource { id: LibraryId },
    #[error("published solution {id} has no source bytes registered")]
    MissingSolutionSource { id: SolutionId },
    #[error("library {id} has no git updated_at record")]
    MissingLibraryUpdate { id: LibraryId },
    #[error("verify spec on {solution} references unknown library {library}")]
    UnknownVerifierTarget {
        solution: SolutionId,
        library: LibraryId,
    },
    #[error("dependency override on {origin} references unknown library {target}")]
    UnknownDependencyTarget {
        origin: LibraryId,
        target: LibraryId,
    },
    #[error("language {id} referenced by discovery is missing from analysis")]
    LanguageMissingAnalysis { id: LanguageId },
    #[error(
        "source bytes for library {id} are not valid UTF-8; managed source must be UTF-8 per spec §6.3"
    )]
    NonUtf8LibrarySource { id: LibraryId },
    #[error(
        "source bytes for solution {id} are not valid UTF-8; managed source must be UTF-8 per spec §6.3"
    )]
    NonUtf8SolutionSource { id: SolutionId },
}

// ─── Inputs ─────────────────────────────────────────────────────────────────

/// Immutable inputs to a single projection run.
///
/// The projection is deterministic — every field is either derived from the
/// analysis snapshot or explicitly captured by the caller so that two
/// consecutive runs on the same repository state produce byte-identical
/// output.
pub struct PublicProjectionInput<'a> {
    pub config: &'a LibraryProjectConfig,
    pub manifest: &'a DiscoveryManifest,
    pub snapshot: &'a AnalysisSnapshot,
    pub verifications: &'a BTreeMap<SolutionId, VerificationRecord>,
    pub current_fingerprints: &'a BTreeMap<SolutionId, Result<VerifyFingerprint, FingerprintError>>,
    pub library_sources: &'a BTreeMap<LibraryId, Vec<u8>>,
    pub library_descriptions: &'a BTreeMap<LibraryId, String>,
    pub library_updates: &'a BTreeMap<LibraryId, LibraryGitUpdate>,
    pub solution_sources: &'a BTreeMap<SolutionId, Vec<u8>>,
    pub solution_has_preprocess: &'a BTreeMap<SolutionId, bool>,
    pub oj_by_contest: &'a BTreeMap<String, String>,
    pub relations: &'a BTreeMap<LibraryId, Vec<ProjectedRelation>>,
    pub manual_dependency_edges: &'a BTreeMap<LibraryId, BTreeSet<LibraryId>>,
    pub build: &'a BuildContext,
}

/// Git-derived update metadata for a single library file (spec §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryGitUpdate {
    pub updated_at: DateTime<FixedOffset>,
    pub source_commit_sha: String,
}

/// A relation declared via frontmatter (spec §4.3, §5.1). Cross-language
/// relations are allowed; targets must be published libraries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedRelation {
    pub kind: String,
    pub target: LibraryId,
    pub manual: bool,
}

/// Immutable build context supplied by the caller (spec §12.14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildContext {
    pub mode: BuildMode,
    pub generated_at: DateTime<FixedOffset>,
    pub source_commit_sha: String,
    pub source_commit_short_sha: String,
    pub source_committed_at: DateTime<FixedOffset>,
    pub uncommitted_changes: bool,
}

// ─── Entry point ────────────────────────────────────────────────────────────

/// Project an immutable analysis snapshot and its verification records into
/// the public [`SiteData`] DTO. See module docs for the projection rules.
pub fn project_site_data(input: PublicProjectionInput<'_>) -> Result<SiteData, SiteDataError> {
    let ctx = ProjectionContext::build(&input)?;

    let build_metadata = build_build_metadata(&input, &ctx)?;
    let site_metadata = build_site_metadata(&input, &ctx)?;
    let solution_statuses = classify_all_solutions(&input, &ctx);

    let libraries = project_libraries(&input, &ctx, &solution_statuses)?;
    let solutions = project_solutions(&input, &ctx, &solution_statuses)?;
    let languages = summarize_languages(&input, &ctx, &libraries);

    Ok(SiteData {
        schema_version: SITE_SCHEMA_VERSION,
        build: build_metadata,
        site: site_metadata,
        languages,
        libraries,
        solutions,
    })
}

// ─── Projection context ─────────────────────────────────────────────────────

struct ProjectionContext<'a> {
    /// Every managed library indexed by ID (published or not).
    libraries_by_id: BTreeMap<&'a LibraryId, &'a LibraryFile>,
    /// Every publishable solution indexed by ID.
    solutions_by_id: BTreeMap<&'a SolutionId, &'a PublishedSolution>,
    /// Per-language analysis lookup by library.
    library_analyses: BTreeMap<&'a LibraryId, &'a NormalizedLibraryAnalysis>,
    /// Per-language analysis lookup by solution.
    solution_analyses: BTreeMap<&'a SolutionId, &'a NormalizedSolutionAnalysis>,
    /// Effective direct dependency edges after manual overrides applied.
    effective_direct: BTreeMap<&'a LibraryId, Vec<LibraryId>>,
    /// Reverse dependency lookup limited to *public* target → public sources.
    public_reverse: BTreeMap<LibraryId, Vec<LibraryId>>,
    /// Publish check: `true` when a `LibraryFile` exists and is `published`.
    published: BTreeSet<LibraryId>,
}

impl<'a> ProjectionContext<'a> {
    fn build(input: &'a PublicProjectionInput<'a>) -> Result<Self, SiteDataError> {
        let mut libraries_by_id = BTreeMap::new();
        let mut published = BTreeSet::new();
        for lib in &input.manifest.libraries {
            libraries_by_id.insert(&lib.id, lib);
            if lib.published {
                published.insert(lib.id.clone());
            }
        }

        let mut solutions_by_id = BTreeMap::new();
        for sol in &input.manifest.solutions {
            solutions_by_id.insert(&sol.id, sol);
        }

        let mut library_analyses: BTreeMap<&LibraryId, &NormalizedLibraryAnalysis> =
            BTreeMap::new();
        let mut solution_analyses: BTreeMap<&SolutionId, &NormalizedSolutionAnalysis> =
            BTreeMap::new();
        for lang in input.snapshot.languages.values() {
            for (id, analysis) in &lang.libraries {
                library_analyses.insert(id, analysis);
            }
            for (id, analysis) in &lang.solutions {
                solution_analyses.insert(id, analysis);
            }
        }

        // Every discovered language must have analysis coverage; missing
        // coverage is a protocol/setup bug that should not silently degrade.
        for lang in input.manifest.languages.keys() {
            if !input.snapshot.languages.contains_key(lang) {
                return Err(SiteDataError::LanguageMissingAnalysis { id: lang.clone() });
            }
        }

        // Compute effective direct edges = adapter direct + manual add - manual remove.
        // We keep manual removals implicit: the ProjectedRelation type carries
        // only *effective* edges; the caller precomputed the effective set.
        let mut effective_direct: BTreeMap<&LibraryId, Vec<LibraryId>> = BTreeMap::new();
        for (id, analysis) in &library_analyses {
            let mut set: BTreeSet<LibraryId> =
                analysis.direct_dependencies.iter().cloned().collect();
            if let Some(extra) = input.manual_dependency_edges.get(*id) {
                for target in extra {
                    set.insert(target.clone());
                }
            }
            let mut vec: Vec<LibraryId> = set.into_iter().collect();
            vec.sort();
            effective_direct.insert(*id, vec);
        }

        // Validate manual override targets exist somewhere in the manifest.
        for (origin, targets) in input.manual_dependency_edges {
            for target in targets {
                if !libraries_by_id.contains_key(target) {
                    return Err(SiteDataError::UnknownDependencyTarget {
                        origin: origin.clone(),
                        target: target.clone(),
                    });
                }
            }
        }

        // Reverse edges → published-only.
        let mut public_reverse: BTreeMap<LibraryId, Vec<LibraryId>> = BTreeMap::new();
        for (source_id, targets) in &effective_direct {
            if !published.contains(source_id) {
                continue;
            }
            for target in targets {
                if !published.contains(target) {
                    continue;
                }
                public_reverse
                    .entry(target.clone())
                    .or_default()
                    .push((*source_id).clone());
            }
        }
        for list in public_reverse.values_mut() {
            list.sort();
            list.dedup();
        }

        Ok(Self {
            libraries_by_id,
            solutions_by_id,
            library_analyses,
            solution_analyses,
            effective_direct,
            public_reverse,
            published,
        })
    }
}

// ─── Build & site metadata ──────────────────────────────────────────────────

fn build_build_metadata(
    input: &PublicProjectionInput<'_>,
    _ctx: &ProjectionContext<'_>,
) -> Result<BuildMetadata, SiteDataError> {
    let mut adapter_map: BTreeMap<(String, String, String), AdapterIdentity> = BTreeMap::new();
    let mut toolchain_map: BTreeMap<(String, String, Option<String>), ToolchainIdentity> =
        BTreeMap::new();

    for (lang_id, lang_analysis) in &input.snapshot.languages {
        let key = (
            lang_id.as_str().to_string(),
            lang_analysis.adapter_name.clone(),
            lang_analysis.adapter_version.clone(),
        );
        adapter_map.entry(key).or_insert(AdapterIdentity {
            language: lang_id.as_str().to_string(),
            name: lang_analysis.adapter_name.clone(),
            version: lang_analysis.adapter_version.clone(),
        });
        for tc in &lang_analysis.observed_toolchains {
            let key = (tc.name.clone(), tc.version.clone(), None::<String>);
            toolchain_map
                .entry(key.clone())
                .or_insert(ToolchainIdentity {
                    name: tc.name.clone(),
                    version: tc.version.clone(),
                    target: None,
                });
        }
    }

    Ok(BuildMetadata {
        schema_version: SITE_SCHEMA_VERSION,
        generated_at: rfc3339(&input.build.generated_at),
        mode: input.build.mode,
        source_commit_sha: input.build.source_commit_sha.clone(),
        source_commit_short_sha: input.build.source_commit_short_sha.clone(),
        source_committed_at: rfc3339(&input.build.source_committed_at),
        uncommitted_changes: input.build.uncommitted_changes,
        observed_toolchains: toolchain_map.into_values().collect(),
        adapters: adapter_map.into_values().collect(),
    })
}

fn build_site_metadata(
    input: &PublicProjectionInput<'_>,
    _ctx: &ProjectionContext<'_>,
) -> Result<SiteMetadata, SiteDataError> {
    let site = match &input.config.site {
        Some(site) => site,
        None => {
            if matches!(input.build.mode, BuildMode::Production) {
                return Err(SiteDataError::MissingProductionSiteConfig);
            }
            return Ok(SiteMetadata {
                title: String::new(),
                description: String::new(),
                language: String::new(),
                repository_url: None,
            });
        }
    };
    Ok(SiteMetadata {
        title: site.title.clone(),
        description: site.description.clone(),
        language: site.language.clone(),
        repository_url: Some(site.repository_url.clone()),
    })
}

// ─── Solution classification ────────────────────────────────────────────────

fn classify_all_solutions(
    input: &PublicProjectionInput<'_>,
    ctx: &ProjectionContext<'_>,
) -> BTreeMap<SolutionId, VerificationStatus> {
    let mut out = BTreeMap::new();
    for (id, sol) in &ctx.solutions_by_id {
        let saved = input.verifications.get(*id);
        let status = match input.current_fingerprints.get(*id) {
            Some(Ok(fp)) => classify_solution_status(sol.verify.as_ref(), Ok(fp), saved),
            Some(Err(err)) => classify_solution_status(sol.verify.as_ref(), Err(err), saved),
            None => {
                // Fingerprint intentionally omitted (e.g. blocked analysis for
                // this solution). Feed a same-solution `UnknownSolution` error;
                // classifier reads it as "blocked" and falls back to Stale/Never.
                let sentinel = FingerprintError::UnknownSolution((*id).clone());
                classify_solution_status(sol.verify.as_ref(), Err(&sentinel), saved)
            }
        };
        out.insert((*id).clone(), status);
    }
    out
}

// ─── Libraries ──────────────────────────────────────────────────────────────

fn project_libraries(
    input: &PublicProjectionInput<'_>,
    ctx: &ProjectionContext<'_>,
    solution_statuses: &BTreeMap<SolutionId, VerificationStatus>,
) -> Result<Vec<LibraryPageData>, SiteDataError> {
    let mut pages = Vec::new();
    for lib in &input.manifest.libraries {
        if !lib.published {
            continue;
        }
        pages.push(project_library(input, ctx, lib, solution_statuses)?);
    }
    pages.sort_by(|a, b| a.library_id.cmp(&b.library_id));
    Ok(pages)
}

fn project_library(
    input: &PublicProjectionInput<'_>,
    ctx: &ProjectionContext<'_>,
    library: &LibraryFile,
    solution_statuses: &BTreeMap<SolutionId, VerificationStatus>,
) -> Result<LibraryPageData, SiteDataError> {
    let analysis = ctx.library_analyses.get(&library.id).ok_or_else(|| {
        SiteDataError::UnanalyzedPublishedLibrary {
            id: library.id.clone(),
        }
    })?;

    let language_id = &library.language;
    let language_config = input.config.languages.get(language_id).ok_or_else(|| {
        SiteDataError::LanguageMissingAnalysis {
            id: language_id.clone(),
        }
    })?;
    let source_bytes = input.library_sources.get(&library.id).ok_or_else(|| {
        SiteDataError::MissingLibrarySource {
            id: library.id.clone(),
        }
    })?;
    let source = std::str::from_utf8(source_bytes)
        .map_err(|_| SiteDataError::NonUtf8LibrarySource {
            id: library.id.clone(),
        })?
        .to_string();
    let git_update = input.library_updates.get(&library.id).ok_or_else(|| {
        SiteDataError::MissingLibraryUpdate {
            id: library.id.clone(),
        }
    })?;

    // Dependencies: direct → public only.
    // `manual_targets` are the edges the caller added via
    // `[library.dependency_overrides]`; every match flips `LibraryLink.manual`
    // on the corresponding link (spec §5.1).
    let empty_manual: BTreeSet<LibraryId> = BTreeSet::new();
    let manual_targets = input
        .manual_dependency_edges
        .get(&library.id)
        .unwrap_or(&empty_manual);
    let direct = build_library_links_marked(
        input,
        ctx,
        ctx.effective_direct.get(&library.id).into_iter().flatten(),
        manual_targets,
    );
    let has_private_direct = ctx
        .effective_direct
        .get(&library.id)
        .map(|edges| edges.iter().any(|to| !ctx.published.contains(to)))
        .unwrap_or(false);

    // Transitive closure (from the raw snapshot, since we compute per-language
    // closure via reverse-edges from adapter output). Only public targets.
    let transitive_ids = input.snapshot.transitive_closure(&library.id);
    let mut transitive_public: Vec<LibraryId> = transitive_ids
        .into_iter()
        .filter(|id| id != &library.id) // exclude self
        .filter(|id| ctx.published.contains(id))
        .collect();
    transitive_public.sort();
    // Direct set for exclusion.
    let direct_set: BTreeSet<LibraryId> = ctx
        .effective_direct
        .get(&library.id)
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    transitive_public.retain(|id| !direct_set.contains(id));
    let transitive = build_library_links_from_ids(input, ctx, transitive_public.iter());

    // Reverse dependencies (public only, from ctx).
    let reverse = build_library_links_from_ids(
        input,
        ctx,
        ctx.public_reverse.get(&library.id).into_iter().flatten(),
    );

    // Relations.
    let mut relations: Vec<RelationPublic> = input
        .relations
        .get(&library.id)
        .into_iter()
        .flatten()
        .filter_map(|rel| {
            if !ctx.published.contains(&rel.target) {
                return None;
            }
            let manual_edge = input
                .manual_dependency_edges
                .get(&library.id)
                .is_some_and(|set| set.contains(&rel.target));
            Some(RelationPublic {
                kind: rel.kind.clone(),
                target: build_library_link(input, ctx, &rel.target, manual_edge)?,
                manual: rel.manual,
            })
        })
        .collect();
    relations.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.target.library_id.cmp(&b.target.library_id))
    });

    // Symbols: keep locations only when they refer to this library's own path.
    let mut symbols: Vec<SymbolPublic> = analysis
        .symbols
        .iter()
        .map(|sym| project_symbol(sym, &library.source_path))
        .collect();
    symbols.sort_by(symbol_ordering);

    // Diagnostics.
    let mut diagnostics: Vec<DiagnosticPublic> = analysis
        .diagnostics
        .iter()
        .map(|diag| project_library_diagnostic(diag, &library.source_path))
        .collect();
    diagnostics.sort_by(diagnostic_ordering);

    // Verification.
    let direct_verifiers = collect_direct_verifiers(input, &library.id, solution_statuses);
    let aggregate_status = classify_library_status(&library.id, &direct_verifiers);
    let evidence = build_evidence(input, &library.id, &direct_verifiers);

    let description = input.library_descriptions.get(&library.id).cloned();

    Ok(LibraryPageData {
        page_id: format!("library:{}", library.id.as_str()),
        library_id: library.id.as_str().to_string(),
        language: language_id.as_str().to_string(),
        title: library
            .title
            .clone()
            .unwrap_or_else(|| default_title(&library.source_path)),
        source_path: library.source_path.clone(),
        source,
        syntax_highlight: language_config.effective_syntax_highlight().to_string(),
        updated_at: rfc3339(&git_update.updated_at),
        updated_by_commit: git_update.source_commit_sha.clone(),
        description,
        symbol_analysis: SymbolAnalysisPublic {
            state: map_state(analysis.state.symbol_state),
            symbols,
        },
        dependency_analysis: DependencyAnalysisPublic {
            state: map_state(analysis.state.dependency_state),
            direct,
            transitive,
            has_private_dependencies: has_private_direct
                || library_has_private_transitive_dep(ctx, &library.id),
        },
        reverse_dependencies: reverse,
        relations,
        verification: LibraryVerificationView {
            aggregate_status: aggregate_library_public(aggregate_status),
            evidence,
        },
        diagnostics,
    })
}

fn library_has_private_transitive_dep(ctx: &ProjectionContext<'_>, library: &LibraryId) -> bool {
    let mut stack: Vec<LibraryId> = ctx
        .effective_direct
        .get(library)
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    let mut visited: BTreeSet<LibraryId> = BTreeSet::new();
    while let Some(node) = stack.pop() {
        if !visited.insert(node.clone()) {
            continue;
        }
        if !ctx.published.contains(&node) {
            return true;
        }
        if let Some(next) = ctx.effective_direct.get(&node) {
            stack.extend(next.iter().cloned());
        }
    }
    false
}

fn collect_direct_verifiers(
    input: &PublicProjectionInput<'_>,
    library: &LibraryId,
    solution_statuses: &BTreeMap<SolutionId, VerificationStatus>,
) -> Vec<(SolutionId, VerificationStatus)> {
    let mut out = Vec::new();
    for sol in &input.manifest.solutions {
        let Some(spec) = sol.verify.as_ref() else {
            continue;
        };
        if spec.libraries.iter().any(|l| l == library)
            && let Some(status) = solution_statuses.get(&sol.id)
        {
            out.push((sol.id.clone(), *status));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn build_evidence(
    input: &PublicProjectionInput<'_>,
    library: &LibraryId,
    direct_verifiers: &[(SolutionId, VerificationStatus)],
) -> Vec<VerificationEvidence> {
    let mut out = Vec::new();
    for (solution_id, status) in direct_verifiers {
        let public = match to_evidence_status(*status) {
            Some(s) => s,
            None => continue,
        };
        let record = input.verifications.get(solution_id);
        let (verdict, judged_at, oj_url, oj) = extract_evidence_fields(record);
        let stale_reason = if matches!(public, EvidenceStatus::Stale) {
            Some(public_stale_reason())
        } else {
            None
        };
        // Fall back to the derived OJ from contest_id when no submission
        // record exists yet (`Never` above filters that out, but keep the
        // helper robust for future statuses).
        let oj_name = oj.unwrap_or_else(|| default_oj(input, solution_id));
        let _ = library;
        out.push(VerificationEvidence {
            solution_id: solution_id.as_str().to_string(),
            solution_page_id: format!("solution:{}", solution_id.as_str()),
            online_judge: oj_name,
            status: public,
            verdict,
            judged_at,
            oj_submission_url: oj_url,
            stale_reason,
        });
    }
    out
}

fn to_evidence_status(status: VerificationStatus) -> Option<EvidenceStatus> {
    match status {
        VerificationStatus::Verified => Some(EvidenceStatus::Verified),
        VerificationStatus::Rejected => Some(EvidenceStatus::Rejected),
        VerificationStatus::Unavailable => Some(EvidenceStatus::Unavailable),
        VerificationStatus::Stale => Some(EvidenceStatus::Stale),
        VerificationStatus::Never | VerificationStatus::NotConfigured => None,
        // In-flight statuses aren't public evidence yet; leave them off the page.
        VerificationStatus::Pending
        | VerificationStatus::Judging
        | VerificationStatus::InfrastructureError => None,
    }
}

fn extract_evidence_fields(
    record: Option<&VerificationRecord>,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let Some(record) = record else {
        return (None, None, None, None);
    };
    match &record.state {
        VerificationState::Completed(state) => (
            Some(format!("{:?}", state.verdict.kind)),
            Some(state.verified_at.to_rfc3339()),
            Some(state.handle.submission_url.clone()),
            Some(state.handle.oj.clone()),
        ),
        VerificationState::Unavailable(state) => {
            (None, Some(state.observed_at.to_rfc3339()), None, None)
        }
        _ => (None, None, None, None),
    }
}

fn public_stale_reason() -> String {
    "Source or dependencies changed since the last submission.".to_string()
}

fn default_oj(input: &PublicProjectionInput<'_>, solution: &SolutionId) -> String {
    input
        .oj_by_contest
        .get(solution.contest_id())
        .cloned()
        .unwrap_or_else(|| "unknown".to_string())
}

fn aggregate_library_public(status: VerificationStatus) -> LibraryVerificationStatus {
    match status {
        VerificationStatus::Verified => LibraryVerificationStatus::Verified,
        VerificationStatus::Rejected => LibraryVerificationStatus::Rejected,
        VerificationStatus::Unavailable => LibraryVerificationStatus::Unavailable,
        VerificationStatus::Stale => LibraryVerificationStatus::Stale,
        // Never, Pending, Judging, InfrastructureError, NotConfigured all
        // become the neutral `never` at the library level per spec §12.8:
        // libraries with zero direct verifiers or only in-flight evidence
        // show as "Never verified".
        _ => LibraryVerificationStatus::Never,
    }
}

// ─── Solutions ──────────────────────────────────────────────────────────────

fn project_solutions(
    input: &PublicProjectionInput<'_>,
    ctx: &ProjectionContext<'_>,
    solution_statuses: &BTreeMap<SolutionId, VerificationStatus>,
) -> Result<Vec<SolutionPageData>, SiteDataError> {
    let mut out = Vec::new();
    for sol in &input.manifest.solutions {
        out.push(project_solution(input, ctx, sol, solution_statuses)?);
    }
    out.sort_by(|a, b| a.solution_id.cmp(&b.solution_id));
    Ok(out)
}

fn project_solution(
    input: &PublicProjectionInput<'_>,
    ctx: &ProjectionContext<'_>,
    solution: &PublishedSolution,
    solution_statuses: &BTreeMap<SolutionId, VerificationStatus>,
) -> Result<SolutionPageData, SiteDataError> {
    let analysis = ctx.solution_analyses.get(&solution.id).ok_or_else(|| {
        SiteDataError::UnanalyzedPublishedSolution {
            id: solution.id.clone(),
        }
    })?;
    let language_config = input
        .config
        .languages
        .get(&solution.language)
        .ok_or_else(|| SiteDataError::LanguageMissingAnalysis {
            id: solution.language.clone(),
        })?;
    let source_bytes = input.solution_sources.get(&solution.id).ok_or_else(|| {
        SiteDataError::MissingSolutionSource {
            id: solution.id.clone(),
        }
    })?;
    let source = std::str::from_utf8(source_bytes)
        .map_err(|_| SiteDataError::NonUtf8SolutionSource {
            id: solution.id.clone(),
        })?
        .to_string();

    // Verify.libraries → LibraryLink.
    let verifies: Vec<LibraryLink> = match &solution.verify {
        Some(spec) => {
            let mut links = Vec::new();
            for target in &spec.libraries {
                if !ctx.libraries_by_id.contains_key(target) {
                    return Err(SiteDataError::UnknownVerifierTarget {
                        solution: solution.id.clone(),
                        library: target.clone(),
                    });
                }
                if let Some(link) = build_library_link(input, ctx, target, false) {
                    links.push(link);
                }
            }
            links.sort_by(|a, b| a.library_id.cmp(&b.library_id));
            links
        }
        None => Vec::new(),
    };

    // Direct dependencies (public only).
    let direct_public: Vec<LibraryId> = analysis
        .direct_dependencies
        .iter()
        .filter(|id| ctx.published.contains(id))
        .cloned()
        .collect();
    let direct_dependencies = build_library_links_from_ids(input, ctx, direct_public.iter());
    let has_private_dependencies = analysis
        .direct_dependencies
        .iter()
        .any(|id| !ctx.published.contains(id));

    // Diagnostics — keep location only when it matches the entry file.
    let entry_path = format!("{}/{}", solution.root, solution.entry);
    let mut diagnostics: Vec<SolutionDiagnosticPublic> = analysis
        .diagnostics
        .iter()
        .map(|d| project_solution_diagnostic(d, &entry_path))
        .collect();
    diagnostics.sort_by(solution_diagnostic_ordering);

    // Verification.
    let status = solution_statuses
        .get(&solution.id)
        .copied()
        .unwrap_or(VerificationStatus::Never);
    let public_status = to_solution_public_status(status);
    let record = input.verifications.get(&solution.id);
    let result = build_verification_result(
        record,
        matches!(public_status, SolutionVerificationStatus::Stale),
    );

    let has_preprocess = input
        .solution_has_preprocess
        .get(&solution.id)
        .copied()
        .unwrap_or(false);

    let oj_name = match record {
        Some(rec) => match &rec.state {
            VerificationState::Completed(state) => state.handle.oj.clone(),
            _ => default_oj(input, &solution.id),
        },
        None => default_oj(input, &solution.id),
    };
    let _ = language_config;
    Ok(SolutionPageData {
        page_id: format!("solution:{}", solution.id.as_str()),
        solution_id: solution.id.as_str().to_string(),
        contest_id: solution.id.contest_id().to_string(),
        problem_code: solution.id.problem_code().to_string(),
        solution_name: solution.id.solution_name().to_string(),
        online_judge: oj_name,
        language: solution.language.as_str().to_string(),
        solved_at: rfc3339(&solution.solved_at),
        source_path: entry_path,
        source,
        syntax_highlight: language_config.effective_syntax_highlight().to_string(),
        has_preprocess,
        verifies,
        direct_dependencies,
        has_private_dependencies,
        verification: SolutionVerificationView {
            status: public_status,
            result,
        },
        dependency_analysis_state: map_state(analysis.dependency_state),
        diagnostics,
    })
}

fn to_solution_public_status(status: VerificationStatus) -> SolutionVerificationStatus {
    match status {
        VerificationStatus::Verified => SolutionVerificationStatus::Verified,
        VerificationStatus::Rejected => SolutionVerificationStatus::Rejected,
        VerificationStatus::Unavailable => SolutionVerificationStatus::Unavailable,
        VerificationStatus::Stale => SolutionVerificationStatus::Stale,
        VerificationStatus::NotConfigured => SolutionVerificationStatus::NotConfigured,
        // In-flight statuses collapse to Never on the public site so the
        // frontend never has to render a bespoke pending state (spec §12.7).
        _ => SolutionVerificationStatus::Never,
    }
}

fn build_verification_result(
    record: Option<&VerificationRecord>,
    is_stale: bool,
) -> Option<VerificationResultPublic> {
    let record = record?;
    match &record.state {
        VerificationState::Completed(state) => Some(VerificationResultPublic {
            attempt_id: record.attempt_id.as_str().to_string(),
            verdict: Some(match state.verdict.kind {
                VerdictKind::Accepted => "Accepted".to_string(),
                _ => state.verdict.raw.clone(),
            }),
            judged_at: Some(state.verified_at.to_rfc3339()),
            oj_submission_url: Some(state.handle.submission_url.clone()),
            execution_time_ms: state.summary.max_execution_time_ms,
            memory_kib: state.summary.max_memory_bytes.map(|bytes| bytes / 1024),
            submitted_source_hash: state.submitted_source_hash.as_str().to_string(),
            verify_fingerprint: record.fingerprint.as_str().to_string(),
            stale_reason: if is_stale {
                Some(public_stale_reason())
            } else {
                None
            },
            testcases: state
                .test_cases
                .as_ref()
                .map(|cases| {
                    cases
                        .iter()
                        .map(|c| TestcaseVerdictPublic {
                            name: c.name.clone().unwrap_or_default(),
                            verdict: c.verdict.raw.clone(),
                            execution_time_ms: c.execution_time_ms,
                            memory_kib: c.memory_bytes.map(|b| b / 1024),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }),
        _ => None,
    }
}

// ─── Diagnostics & symbols helpers ──────────────────────────────────────────

fn project_symbol(symbol: &NormalizedSymbol, own_path: &str) -> SymbolPublic {
    let location = symbol.location.as_ref().and_then(|loc| {
        if loc.path == own_path {
            Some(PublicLocation {
                start: LinePosition {
                    line: loc.start_line,
                    column: loc.start_column,
                },
                end: match (loc.end_line, loc.end_column) {
                    (Some(line), col) => Some(LinePosition { line, column: col }),
                    _ => None,
                },
            })
        } else {
            None
        }
    });
    SymbolPublic {
        kind: symbol.kind.clone(),
        name: symbol.name.clone(),
        qualified_name: symbol.qualified_name.clone(),
        search_names: symbol.search_names.clone(),
        signature: symbol.signature.clone(),
        location,
    }
}

fn project_library_diagnostic(diag: &NormalizedDiagnostic, own_path: &str) -> DiagnosticPublic {
    let location = diag.location.as_ref().and_then(|loc| {
        if loc.path == own_path {
            Some(PublicLocation {
                start: LinePosition {
                    line: loc.start_line,
                    column: loc.start_column,
                },
                end: match (loc.end_line, loc.end_column) {
                    (Some(line), col) => Some(LinePosition { line, column: col }),
                    _ => None,
                },
            })
        } else {
            None
        }
    });
    DiagnosticPublic {
        severity: map_diagnostic_severity(diag.severity),
        code: diag.code.clone(),
        message: diag.message.clone(),
        location,
    }
}

fn project_solution_diagnostic(
    diag: &NormalizedDiagnostic,
    entry_path: &str,
) -> SolutionDiagnosticPublic {
    let (location, location_notice) = match &diag.location {
        Some(loc) if loc.path == entry_path => (
            Some(PublicLocation {
                start: LinePosition {
                    line: loc.start_line,
                    column: loc.start_column,
                },
                end: match (loc.end_line, loc.end_column) {
                    (Some(line), col) => Some(LinePosition { line, column: col }),
                    _ => None,
                },
            }),
            None,
        ),
        Some(_) => (
            None,
            Some("Location is in a non-displayed solution file.".to_string()),
        ),
        None => (None, None),
    };
    SolutionDiagnosticPublic {
        severity: map_diagnostic_severity(diag.severity),
        code: diag.code.clone(),
        message: diag.message.clone(),
        location,
        location_notice,
    }
}

fn symbol_ordering(a: &SymbolPublic, b: &SymbolPublic) -> std::cmp::Ordering {
    match (&a.location, &b.location) {
        (Some(la), Some(lb)) => la
            .start
            .line
            .cmp(&lb.start.line)
            .then_with(|| la.start.column.cmp(&lb.start.column))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.qualified_name.cmp(&b.qualified_name))
            .then_with(|| a.name.cmp(&b.name)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a
            .kind
            .cmp(&b.kind)
            .then_with(|| a.qualified_name.cmp(&b.qualified_name))
            .then_with(|| a.name.cmp(&b.name)),
    }
}

fn diagnostic_ordering(a: &DiagnosticPublic, b: &DiagnosticPublic) -> std::cmp::Ordering {
    severity_order(a.severity)
        .cmp(&severity_order(b.severity))
        .then_with(|| location_ordering(&a.location, &b.location))
        .then_with(|| a.code.cmp(&b.code))
        .then_with(|| a.message.cmp(&b.message))
}

fn solution_diagnostic_ordering(
    a: &SolutionDiagnosticPublic,
    b: &SolutionDiagnosticPublic,
) -> std::cmp::Ordering {
    severity_order(a.severity)
        .cmp(&severity_order(b.severity))
        .then_with(|| location_ordering(&a.location, &b.location))
        .then_with(|| a.code.cmp(&b.code))
        .then_with(|| a.message.cmp(&b.message))
}

fn location_ordering(a: &Option<PublicLocation>, b: &Option<PublicLocation>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a
            .start
            .line
            .cmp(&b.start.line)
            .then_with(|| a.start.column.cmp(&b.start.column)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn severity_order(s: PublicDiagnosticSeverity) -> u8 {
    match s {
        PublicDiagnosticSeverity::Error => 0,
        PublicDiagnosticSeverity::Warning => 1,
        PublicDiagnosticSeverity::Info => 2,
    }
}

fn map_diagnostic_severity(s: domain::analysis::DiagnosticSeverity) -> PublicDiagnosticSeverity {
    match s {
        domain::analysis::DiagnosticSeverity::Info => PublicDiagnosticSeverity::Info,
        domain::analysis::DiagnosticSeverity::Warning => PublicDiagnosticSeverity::Warning,
        domain::analysis::DiagnosticSeverity::Error => PublicDiagnosticSeverity::Error,
    }
}

fn map_state(state: AnalysisState) -> PublicAnalysisState {
    match state {
        AnalysisState::Complete => PublicAnalysisState::Complete,
        AnalysisState::Partial => PublicAnalysisState::Partial,
        AnalysisState::Failed => PublicAnalysisState::Failed,
    }
}

// ─── Language summaries ─────────────────────────────────────────────────────

fn summarize_languages(
    input: &PublicProjectionInput<'_>,
    _ctx: &ProjectionContext<'_>,
    libraries: &[LibraryPageData],
) -> Vec<LanguageSummary> {
    // Only include languages that (a) have >=1 published library, or
    // (b) have a description sidecar (`_index.md`). Spec §12.4 defers to
    // discovery for language visibility; we approximate here by including
    // any language mentioned in the config so private-only languages still
    // show `_index.md`-less pages. The Astro layer then filters visibility.
    let mut counts_by_language: BTreeMap<String, (u32, VerificationCounts)> = BTreeMap::new();
    for lib in libraries {
        let entry = counts_by_language
            .entry(lib.language.clone())
            .or_insert((0, VerificationCounts::default()));
        entry.0 += 1;
        match lib.verification.aggregate_status {
            LibraryVerificationStatus::Verified => entry.1.verified += 1,
            LibraryVerificationStatus::Rejected => entry.1.rejected += 1,
            LibraryVerificationStatus::Unavailable => entry.1.unavailable += 1,
            LibraryVerificationStatus::Stale => entry.1.stale += 1,
            LibraryVerificationStatus::Never => entry.1.never += 1,
        }
    }

    let mut out = Vec::new();
    for (language_id, language_config) in &input.config.languages {
        let (count, verification_summary) = counts_by_language
            .get(language_id.as_str())
            .cloned()
            .unwrap_or_default();
        out.push(LanguageSummary {
            id: language_id.as_str().to_string(),
            display_name: language_config.effective_display_name().to_string(),
            syntax_highlight: language_config.effective_syntax_highlight().to_string(),
            library_count: count,
            verification_summary,
        });
    }
    // BTreeMap gives byte-order; keep explicit sort to match spec §14.
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

// ─── Small helpers ──────────────────────────────────────────────────────────

fn build_library_links_from_ids<'a, I>(
    input: &PublicProjectionInput<'_>,
    ctx: &ProjectionContext<'_>,
    ids: I,
) -> Vec<LibraryLink>
where
    I: IntoIterator<Item = &'a LibraryId>,
{
    build_library_links_marked(input, ctx, ids, &BTreeSet::new())
}

fn build_library_links_marked<'a, I>(
    input: &PublicProjectionInput<'_>,
    ctx: &ProjectionContext<'_>,
    ids: I,
    manual_targets: &BTreeSet<LibraryId>,
) -> Vec<LibraryLink>
where
    I: IntoIterator<Item = &'a LibraryId>,
{
    let mut out: Vec<LibraryLink> = ids
        .into_iter()
        .filter(|id| ctx.published.contains(id))
        .filter_map(|id| build_library_link(input, ctx, id, manual_targets.contains(id)))
        .collect();
    out.sort_by(|a, b| a.library_id.cmp(&b.library_id));
    out.dedup_by(|a, b| a.library_id == b.library_id);
    out
}

fn build_library_link(
    _input: &PublicProjectionInput<'_>,
    ctx: &ProjectionContext<'_>,
    target: &LibraryId,
    manual: bool,
) -> Option<LibraryLink> {
    let file = ctx.libraries_by_id.get(target)?;
    if !file.published {
        return None;
    }
    Some(LibraryLink {
        library_id: target.as_str().to_string(),
        language: file.language.as_str().to_string(),
        title: file
            .title
            .clone()
            .unwrap_or_else(|| default_title(&file.source_path)),
        source_path: file.source_path.clone(),
        manual,
    })
}

fn default_title(source_path: &str) -> String {
    source_path
        .rsplit('/')
        .next()
        .unwrap_or(source_path)
        .to_string()
}

fn rfc3339(ts: &DateTime<FixedOffset>) -> String {
    ts.to_rfc3339()
}

// Unused reference silencer for language_config in library projection.
fn _touch_language_config(_c: &LanguageConfig) {}
