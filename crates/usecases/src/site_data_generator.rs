//! End-to-end orchestrator for `ce site-data generate` (spec §12, §14).
//!
//! Composes discovery, analyzer dispatch, source-byte capture, verification
//! record load, git history queries, current-fingerprint recomputation, and
//! the pure [`crate::site_data::project_site_data`] projection into one
//! deterministic pipeline. Writes are handed off to the caller-supplied
//! [`SiteDataRepository`] so the atomic-swap invariant lives in one place.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use domain::analysis::DiscoveryManifest;
use domain::library::{LibraryId, LibraryProjectConfig, SolutionId};
use domain::solution::PublishedSolution;
use site_schema::{BuildMode, SiteData};

use crate::git_history::GitHistory;
use crate::library_analysis::normalize_analysis;
use crate::library_analyzer::LibraryAnalyzer;
use crate::repository::site_data_repository::SiteDataRepository;
use crate::repository::verification_repository::VerificationRepository;
use crate::site_data::{
    BuildContext, LibraryGitUpdate, ProjectedRelation, PublicProjectionInput, project_site_data,
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

    // 7. Current fingerprints — leave empty for now; the classifier treats
    //    a missing entry as "blocked" which folds to Stale/Never per spec §11.
    //    Recomputing per-solution fingerprints requires the full closure walk
    //    across dependency states, which is delegated to the verify pipeline
    //    (plan 052) so we don't duplicate that logic here.
    let current_fingerprints = BTreeMap::new();

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

/// Convenience: derive the default output directory (`target/ce-site-data`) below
/// the repository root.
pub fn default_output_dir(repository_root: &Path) -> PathBuf {
    repository_root.join("target").join("ce-site-data")
}
