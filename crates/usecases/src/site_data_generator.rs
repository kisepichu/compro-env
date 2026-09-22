//! End-to-end orchestrator for `ce site-data generate` (spec §12, §14).
//!
//! Composes discovery, analyzer dispatch, source-byte capture, verification
//! record load, git history queries, current-fingerprint recomputation, and
//! the pure [`crate::site_data::project_site_data`] projection into one
//! deterministic pipeline. The current-fingerprint pass reuses the verify
//! pipeline's [`verification_closure`] + [`calculate_fingerprint`] so a
//! saved `Completed` record surfaces as `Verified` whenever the working
//! tree matches the record and as `Stale` when any hashed input differs
//! (spec §11). Writes are handed off to the caller-supplied
//! [`SiteDataRepository`] so the atomic-swap invariant lives in one place.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use domain::analysis::{AnalysisSnapshot, DiscoveryManifest};
use domain::entity::OJKind;
use domain::library::{LibraryId, LibraryProjectConfig, SolutionId};
use domain::solution::PublishedSolution;
use domain::verification::VerifyFingerprint;
use site_schema::{BuildMode, SiteData};

use crate::git_history::GitHistory;
use crate::library_analysis::normalize_analysis;
use crate::library_analyzer::LibraryAnalyzer;
use crate::repository::site_data_repository::SiteDataRepository;
use crate::repository::verification_repository::VerificationRepository;
use crate::site_data::{
    BuildContext, LibraryGitUpdate, ProjectedRelation, PublicProjectionInput, project_site_data,
};
use crate::submission::StarterRegistry;
use crate::verification::fingerprint::{
    AdapterIdentity, FingerprintError, FingerprintMaterial, FingerprintSource, OjBinding,
    calculate_fingerprint, capabilities_from_descriptor, hash_verify_config, verification_closure,
};

/// External inputs the caller must supply so the generator stays free of
/// filesystem, clock, and shell dependencies. `oj_by_contest` is the config
/// callers derive from `contests.toml` / project config; empty maps degrade
/// gracefully to `"unknown"`.
pub struct GenerateSiteData<'a> {
    pub repository_root: &'a Path,
    pub config: &'a LibraryProjectConfig,
    pub manifest: &'a DiscoveryManifest,
    pub analyzer: &'a dyn LibraryAnalyzer,
    pub verifications: &'a dyn VerificationRepository,
    pub git_history: &'a dyn GitHistory,
    pub oj_by_contest: &'a BTreeMap<String, String>,
    pub relations: &'a BTreeMap<LibraryId, Vec<ProjectedRelation>>,
    pub manual_dependency_edges: &'a BTreeMap<LibraryId, BTreeSet<LibraryId>>,
    pub solution_has_preprocess: &'a BTreeMap<SolutionId, bool>,
    pub library_descriptions: &'a BTreeMap<LibraryId, String>,
    pub starters: &'a StarterRegistry,
    pub mode: BuildMode,
}

/// Runs the pipeline, returns the projected `SiteData`. Callers usually then
/// call [`write_site_data`] to persist it.
pub fn generate_site_data(spec: &GenerateSiteData<'_>) -> Result<SiteData> {
    // 1. Analyzer fan-out.
    let responses = spec
        .analyzer
        .analyze_all(spec.repository_root, spec.manifest)
        .context("analyzer dispatch failed")?;

    // 2. Read every source byte the projection or fingerprints will need.
    let source_bytes =
        collect_source_bytes(spec.repository_root, spec.manifest).context("source read failed")?;

    // 3. Normalize adapter responses.
    let head = spec
        .git_history
        .head_snapshot()
        .context("git head lookup failed")?;
    let snapshot = normalize_analysis(spec.manifest, responses, &head.commit_sha, &source_bytes)
        .context("normalization failed")?;

    // 4. Load latest verification records.
    let discovered_solutions: BTreeSet<SolutionId> = spec
        .manifest
        .solutions
        .iter()
        .map(|s| s.id.clone())
        .collect();
    let verifications = spec
        .verifications
        .load_all(&discovered_solutions)
        .context("verification repository load failed")?;

    // 5. Query git history for every managed source path.
    let library_paths: Vec<&str> = spec
        .manifest
        .libraries
        .iter()
        .filter(|lib| lib.published)
        .map(|lib| lib.source_path.as_str())
        .collect();
    let path_updates = spec
        .git_history
        .last_touched(&library_paths)
        .context("git history lookup failed")?;
    let mut library_updates: BTreeMap<LibraryId, LibraryGitUpdate> = BTreeMap::new();
    for lib in &spec.manifest.libraries {
        if !lib.published {
            continue;
        }
        let update = path_updates.get(&lib.source_path).ok_or_else(|| {
            anyhow!(
                "no git history recorded for published library {}",
                lib.source_path
            )
        })?;
        library_updates.insert(
            lib.id.clone(),
            LibraryGitUpdate {
                updated_at: update.committer_time,
                source_commit_sha: update.commit_sha.clone(),
            },
        );
    }

    // 6. Split source bytes into library / solution maps for the projection.
    let (library_sources, solution_sources) = split_source_bytes(source_bytes, spec.manifest);

    // 7. Current fingerprints. Reuses the verify pipeline helpers so a
    //    saved `Completed` record classifies as `Verified` when the working
    //    tree still matches the record (spec §11). Any per-solution failure
    //    (unknown OJ, missing starter, blocked closure, missing source
    //    bytes) is captured as `Err(FingerprintError)`; the classifier
    //    treats every `Err(_)` uniformly and folds it to `Stale`/`Never`
    //    against a saved record so evidence links survive.
    let current_fingerprints = compute_current_fingerprints(
        spec.manifest,
        &snapshot,
        spec.starters,
        &library_sources,
        &solution_sources,
    );

    // 8. Build context.
    let build_context = BuildContext {
        mode: spec.mode,
        generated_at: Utc::now().fixed_offset(),
        source_commit_sha: head.commit_sha.clone(),
        source_commit_short_sha: head.short_sha.clone(),
        source_committed_at: head.committed_at,
        uncommitted_changes: head.uncommitted_changes,
    };

    if matches!(spec.mode, BuildMode::Production) && head.uncommitted_changes {
        bail!(
            "site-data generate --mode production requires a clean working tree; \
             re-run with --mode preview or commit your changes"
        );
    }

    // 9. Project.
    let input = PublicProjectionInput {
        config: spec.config,
        manifest: spec.manifest,
        snapshot: &snapshot,
        verifications: &verifications,
        current_fingerprints: &current_fingerprints,
        library_sources: &library_sources,
        library_descriptions: spec.library_descriptions,
        library_updates: &library_updates,
        solution_sources: &solution_sources,
        solution_has_preprocess: spec.solution_has_preprocess,
        oj_by_contest: spec.oj_by_contest,
        relations: spec.relations,
        manual_dependency_edges: spec.manual_dependency_edges,
        build: &build_context,
    };
    let data = project_site_data(input).map_err(anyhow::Error::from)?;
    Ok(data)
}

/// Persist a projected [`SiteData`] via the repository port.
pub fn write_site_data(
    repository: &dyn SiteDataRepository,
    output_dir: &Path,
    data: &SiteData,
) -> Result<()> {
    repository.write_atomically(output_dir, data)
}

fn collect_source_bytes(
    repo: &Path,
    manifest: &DiscoveryManifest,
) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut out = BTreeMap::new();
    for lib in &manifest.libraries {
        insert_source(&mut out, repo, &lib.source_path)?;
    }
    for sol in &manifest.solutions {
        let entry_path = solution_entry_path(sol);
        insert_source(&mut out, repo, &entry_path)?;
    }
    Ok(out)
}

fn solution_entry_path(sol: &PublishedSolution) -> String {
    format!("{}/{}", sol.root, sol.entry)
}

fn insert_source(out: &mut BTreeMap<String, Vec<u8>>, repo: &Path, path: &str) -> Result<()> {
    if out.contains_key(path) {
        return Ok(());
    }
    let full = repo.join(path);
    let bytes = std::fs::read(&full)
        .with_context(|| format!("failed to read managed source {}", full.display()))?;
    out.insert(path.to_string(), bytes);
    Ok(())
}

fn split_source_bytes(
    all: BTreeMap<String, Vec<u8>>,
    manifest: &DiscoveryManifest,
) -> (BTreeMap<LibraryId, Vec<u8>>, BTreeMap<SolutionId, Vec<u8>>) {
    let mut libraries = BTreeMap::new();
    let mut library_paths_by_id: BTreeMap<&str, &LibraryId> = BTreeMap::new();
    for lib in &manifest.libraries {
        library_paths_by_id.insert(lib.source_path.as_str(), &lib.id);
    }
    let mut solutions = BTreeMap::new();
    let mut solution_paths_by_id: BTreeMap<String, &SolutionId> = BTreeMap::new();
    for sol in &manifest.solutions {
        solution_paths_by_id.insert(solution_entry_path(sol), &sol.id);
    }
    for (path, bytes) in all {
        if let Some(id) = library_paths_by_id.get(path.as_str()) {
            libraries.insert((*id).clone(), bytes);
        } else if let Some(id) = solution_paths_by_id.get(path.as_str()) {
            solutions.insert((*id).clone(), bytes);
        }
    }
    (libraries, solutions)
}

/// Recompute per-solution current fingerprints for staleness classification
/// (spec §11).
///
/// Iterates every discovered solution that has a resolved `[verify]` block,
/// reproduces the same [`FingerprintMaterial`] the verify pipeline builds
/// (`build_plan_context` in `service/verify.rs`), and hashes it with
/// [`calculate_fingerprint`]. The result map only carries solutions whose
/// `verify` is configured; the projection layer treats a missing entry as
/// blocked and falls back to `Stale`/`Never` via the sentinel error path.
///
/// Errors are captured inline (never propagated) so a single bad solution
/// cannot poison the whole site-data build. The classifier treats every
/// `Err(_)` uniformly.
///
/// Preprocess hooks are intentionally not invoked: site-data generation is
/// offline, and the verify pipeline already re-hashes raw source bytes at
/// record time when no `[submit].preprocess` is configured (which is the
/// documented default for library-platform records).
fn compute_current_fingerprints(
    manifest: &DiscoveryManifest,
    snapshot: &AnalysisSnapshot,
    starters: &StarterRegistry,
    library_sources: &BTreeMap<LibraryId, Vec<u8>>,
    solution_sources: &BTreeMap<SolutionId, Vec<u8>>,
) -> BTreeMap<SolutionId, Result<VerifyFingerprint, FingerprintError>> {
    let mut library_paths_by_id: BTreeMap<&LibraryId, &str> = BTreeMap::new();
    for lib in &manifest.libraries {
        library_paths_by_id.insert(&lib.id, lib.source_path.as_str());
    }

    let mut out: BTreeMap<SolutionId, Result<VerifyFingerprint, FingerprintError>> =
        BTreeMap::new();
    for sol in &manifest.solutions {
        let Some(verify) = sol.verify.as_ref() else {
            continue;
        };
        let result = fingerprint_for_solution(
            sol,
            verify,
            snapshot,
            starters,
            &library_paths_by_id,
            library_sources,
            solution_sources,
        );
        out.insert(sol.id.clone(), result);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn fingerprint_for_solution(
    sol: &PublishedSolution,
    verify: &domain::solution::VerifySpec,
    snapshot: &AnalysisSnapshot,
    starters: &StarterRegistry,
    library_paths_by_id: &BTreeMap<&LibraryId, &str>,
    library_sources: &BTreeMap<LibraryId, Vec<u8>>,
    solution_sources: &BTreeMap<SolutionId, Vec<u8>>,
) -> Result<VerifyFingerprint, FingerprintError> {
    // Sentinel used whenever site-data cannot even reach `calculate_fingerprint`
    // (unknown OJ, missing starter, missing source bytes). The classifier
    // treats every `Err(_)` identically, so any `FingerprintError` variant
    // is a valid fallback; keeping the solution id inside the error aids
    // debugging.
    let sentinel = || FingerprintError::UnknownSolution(sol.id.clone());

    let oj = OJKind::detect(sol.id.contest_id())
        .map(|(o, _)| o)
        .ok_or_else(sentinel)?;
    let starter = starters.get(&oj).map_err(|_| sentinel())?;

    let mut explicit: BTreeSet<LibraryId> = BTreeSet::new();
    for lib in &verify.libraries {
        explicit.insert(lib.clone());
    }
    let closure = verification_closure(&sol.id, &explicit, snapshot)?;

    let mut dependency_library_sources: BTreeMap<LibraryId, FingerprintSource> = BTreeMap::new();
    for lib_id in &closure {
        let path = library_paths_by_id
            .get(lib_id)
            .ok_or_else(|| FingerprintError::UnknownLibrary(lib_id.clone()))?;
        let bytes = library_sources
            .get(lib_id)
            .ok_or_else(|| FingerprintError::MissingLibrarySource(lib_id.clone()))?;
        dependency_library_sources.insert(
            lib_id.clone(),
            FingerprintSource {
                path: (*path).to_string(),
                bytes: bytes.clone(),
            },
        );
    }

    let entry_path = solution_entry_path(sol);
    let entry_bytes = solution_sources
        .get(&sol.id)
        .ok_or_else(|| FingerprintError::MissingSolutionSource(sol.id.clone()))?
        .clone();
    // site-data is offline and never runs preprocess, so the working-tree
    // bytes are already the raw source that the verify pipeline hashed.
    let raw_source = FingerprintSource {
        path: entry_path,
        bytes: entry_bytes,
    };

    let descriptor = starter.descriptor();
    let adapter = AdapterIdentity {
        name: descriptor.name.clone(),
        version: descriptor.version.clone(),
        capabilities: capabilities_from_descriptor(&descriptor),
    };

    let binding = OjBinding {
        oj: oj.as_str().to_string(),
        problem_id: sol.id.problem_code().to_string(),
        language_id: sol.language.clone(),
        oj_language_id: verify.oj_language_id.clone(),
    };

    let verify_config_hash = hash_verify_config(verify);
    let verified_libraries: BTreeSet<LibraryId> = verify.libraries.iter().cloned().collect();

    let material = FingerprintMaterial {
        solution_id: sol.id.clone(),
        raw_source,
        verified_libraries,
        dependency_library_sources,
        binding,
        adapter,
        verify_config_hash,
    };
    calculate_fingerprint(&material)
}

/// Convenience: derive the default output directory (`target/ce-site-data`) below
/// the repository root.
pub fn default_output_dir(repository_root: &Path) -> PathBuf {
    repository_root.join("target").join("ce-site-data")
}
