//! Deterministic candidate picker for the `verify.yml` dispatcher
//! (plan 063, spec §15).
//!
//! This module is the secretless bridge between the on-disk publication set
//! (`config.toml` + `solutions/**/ce.toml`), the current
//! `automation/verify` overlay (`verification/results/**`), and the pure
//! [`usecases::verification::select_next_candidate`] rule that decides which
//! solution the worker chain should verify next.
//!
//! The dispatcher invokes `ce internal pick-candidate --root . --state
//! <automation-verify-worktree> [--now ...]` on every scheduled and eligible
//! push tick. The printed stdout line is the worker's `solution` input; an
//! empty line means "no candidate this tick" and the dispatcher must set
//! `run_worker=false`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, FixedOffset};
use domain::library::SolutionId;
use domain::solution::PublishedSolution;
use domain::verification::{VerificationRecord, VerificationState, VerifyFingerprint};

use crate::library_project::config::ProjectLibraryConfigLoader;
use crate::library_project::discovery::LibraryDiscovery;

/// Load the current publication set, overlay the `automation/verify`
/// verification records, recompute fingerprints for `Completed` records only,
/// and return the picker's decision.
///
/// Returns `Ok(None)` when nothing is eligible; the dispatcher must translate
/// that into `run_worker=false`. Errors bubble up for missing config,
/// unreadable overlay JSON, and symlinked `state` targets.
pub fn pick_candidate_with_io(
    root: &Path,
    state: &Path,
    now: DateTime<FixedOffset>,
) -> Result<Option<SolutionId>> {
    validate_state_dir(state)?;

    let config = ProjectLibraryConfigLoader::load(root)
        .with_context(|| format!("failed to load config.toml under {}", root.display()))?;

    let manifest = LibraryDiscovery::discover(root, &config)?;
    let published: Vec<PublishedSolution> = manifest
        .solutions
        .iter()
        .filter(|s| s.verify.is_some())
        .cloned()
        .collect();
    let known_ids: BTreeSet<SolutionId> = published.iter().map(|s| s.id.clone()).collect();

    let records = load_overlay_records(state, &known_ids)?;

    let fingerprints = compute_completed_fingerprints(root, &config, &records)?;

    Ok(usecases::verification::select_next_candidate(
        now,
        &published,
        &records,
        &fingerprints,
    ))
}

fn validate_state_dir(state: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(state)
        .with_context(|| format!("failed to stat --state directory {}", state.display()))?;
    if meta.file_type().is_symlink() {
        return Err(anyhow!(
            "--state {} is a symlink; symlinks are rejected (spec §6.1)",
            state.display()
        ));
    }
    if !meta.is_dir() {
        return Err(anyhow!("--state {} is not a directory", state.display()));
    }
    Ok(())
}

fn load_overlay_records(
    state: &Path,
    known_ids: &BTreeSet<SolutionId>,
) -> Result<BTreeMap<SolutionId, VerificationRecord>> {
    let results_dir = state.join("verification/results");
    let mut records = BTreeMap::new();
    if !results_dir.exists() {
        return Ok(records);
    }
    for entry in walkdir::WalkDir::new(&results_dir).follow_links(false) {
        let entry = entry.with_context(|| {
            format!(
                "failed to walk overlay results at {}",
                results_dir.display()
            )
        })?;
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            // Silently skip symlinks under the overlay; verify persist only
            // ever writes plain regular files.
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read overlay record {}", path.display()))?;
        let record: VerificationRecord = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "failed to deserialize overlay record {} as VerificationRecord",
                path.display()
            )
        })?;
        if !known_ids.contains(&record.solution_id) {
            // Overlay carries a record for a solution that has since been
            // unpublished. The runbook documents that these files are left
            // in place until an operator cleans them up; the picker only
            // needs to ignore them for scheduling purposes.
            continue;
        }
        records.insert(record.solution_id.clone(), record);
    }
    Ok(records)
}

fn compute_completed_fingerprints(
    root: &Path,
    config: &domain::library::LibraryProjectConfig,
    records: &BTreeMap<SolutionId, VerificationRecord>,
) -> Result<BTreeMap<SolutionId, VerifyFingerprint>> {
    let completed_ids: Vec<&SolutionId> = records
        .iter()
        .filter(|(_, r)| matches!(r.state, VerificationState::Completed(_)))
        .map(|(id, _)| id)
        .collect();
    if completed_ids.is_empty() {
        // Fast path: no need to spin up the analyzer / starter registry when
        // nothing needs a drift check. Every scheduler tick that only has
        // in-flight, retryable, or fresh solutions ends here.
        return Ok(BTreeMap::new());
    }

    let (manifest, snapshot) = crate::shell::build_analysis(root, config)?;
    let controller = crate::shell::build_verify_controller(root)?;

    let mut out = BTreeMap::new();
    for id in completed_ids {
        let fp = controller
            .compute_solution_fingerprint(
                root,
                config,
                &manifest,
                &snapshot,
                id,
                &usecases::clock::SystemClock,
                &usecases::id_generator::MonotonicAttemptIdGenerator::new(),
                &usecases::submission_lifecycle::RealSleeper,
                &usecases::submission_lifecycle::NoRetryHint,
                usecases::submission_lifecycle::PollingPolicy::verify_defaults(),
            )
            .with_context(|| {
                format!("failed to compute current fingerprint for completed solution {id}")
            })?;
        out.insert(id.clone(), fp);
    }
    Ok(out)
}
