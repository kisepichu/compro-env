//! `ce verify` orchestration (spec §7.2, §8, §8.1, §8.2, §8.3, §9, §10, §11).
//!
//! Composes discovery, dependency analysis (via the caller-provided
//! `AnalysisSnapshot`), preprocess, planning, and the shared submission
//! lifecycle (`start_plan` / `poll_handle` / `resume_pending`) into a single
//! resumable verify run.
//!
//! The layering rule stays intact: this module never imports anything from
//! `infrastructure`. The discovery manifest, the analysis snapshot, the
//! preprocess command, and the repository root are all handed in.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use domain::analysis::{AnalysisSnapshot, DiscoveryManifest};
use domain::entity::OJKind;
use domain::library::{LanguageId, LibraryProjectConfig, SolutionId};
use domain::solution::PublishedSolution;
use domain::verification::{
    AttemptId, LanguageBinding, VerdictKind, VerificationRecord, VerificationState,
    VerifyFingerprint,
};
use sha2::{Digest, Sha256};

use crate::check::{CheckSelection, run_checks};
use crate::clock::Clock;
use crate::command_runner::{CommandRequest, CommandRunner};
use crate::id_generator::AttemptIdGenerator;
use crate::repository::session_repository::SessionRepository;
use crate::repository::verification_repository::VerificationRepository;
use crate::submission::{PollerRegistry, RecoveryRegistry, StarterRegistry, SubmissionStarter};
use crate::submission_lifecycle::{
    PollEvent, PollingPolicy, RetryAfterHint, Sleeper, StartEvent, SubmissionPorts,
    VerificationRepositories, VerifySelection as LifecycleSelection, poll_handle, resume_pending,
    start_plan, submit_prepared_plan,
};
use crate::verification::fingerprint::{
    AdapterIdentity, FingerprintMaterial, FingerprintSource, OjBinding, calculate_fingerprint,
    capabilities_from_descriptor, hash_verify_config, verification_closure,
};
use crate::verification::plan::{PrepareVerificationInput, SubmissionPlan, build_submission_plan};

// ─── Public API ────────────────────────────────────────────────────────────

/// User-facing selection of which solutions to plan new starts for.
#[derive(Debug, Clone)]
pub enum VerifySelection {
    /// Every published solution that has a `[verify]` configuration.
    All,
    /// A single solution by ID. Non-configured targets still emit a status
    /// line but do not fail the run on their own.
    Single(SolutionId),
}

/// Per-solution outcome of a verify run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyStatus {
    /// Solution has no `[verify]` block; skipped silently in bulk mode,
    /// reported as `not_configured` when explicitly named.
    NotConfigured,
    /// Analyzer / snapshot could not produce a fingerprint for this solution.
    /// Does NOT count as a verify pass; the run exits 1.
    AnalyzerBlocked { detail: String },
    /// The language check for this solution's language failed; new starts are
    /// suppressed by the aggregate barrier.
    LanguageCheckFailed {
        language: LanguageId,
        detail: String,
    },
    /// The solution's own `test_command` failed; new starts are suppressed.
    TestFailed { detail: String },
    /// Existing record is `Completed(Accepted)` and the fingerprint matches;
    /// re-execution is skipped (spec §10, §11).
    Verified,
    /// Existing record is `Completed(rejected)` — spec §10 does NOT
    /// auto-re-run this on the same tick, but the CLI still exits 1.
    Rejected { verdict: String, summary: String },
    /// OJ cannot serve the submission (interactive-untrackable, unsupported).
    Unavailable { summary: String },
    /// Non-terminal state after the run (Queued/Judging/AcceptanceUnknown/
    /// Starting/InfrastructureFailure without a handle, budget exhausted).
    Pending { state: String, summary: String },
    /// Skipped due to another attempt on the same OJ being in-flight.
    OjBlocked { oj: String },
    /// Post-resume infrastructure error at either the start or poll stage.
    InfraError { summary: String },
    /// The verify-target set was narrowed to an unknown solution.
    UnknownSolution,
}

/// One status line emitted by [`run_verify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyStatusLine {
    pub solution_id: SolutionId,
    pub status: VerifyStatus,
}

/// Aggregate outcome of a verify run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyOutcome {
    /// Status lines in ascending [`SolutionId`] order (spec §10 stable output).
    pub statuses: Vec<VerifyStatusLine>,
}

impl VerifyOutcome {
    /// Exit code per spec §10: 0 iff every status is a verified/not-configured
    /// success, 1 otherwise.
    pub fn exit_code(&self) -> i32 {
        for line in &self.statuses {
            match &line.status {
                VerifyStatus::Verified | VerifyStatus::NotConfigured => {}
                _ => return 1,
            }
        }
        0
    }
}

/// Target state of the long-lived automation PR after a verify record has
/// been persisted (spec §15.1, §15 PrTarget policy).
///
/// A single long-lived PR from `automation/verify` → `main` flips between
/// these two states over the lifetime of an attempt: mid-flight persists
/// keep it `Draft`, terminal-mergeable persists flip it to
/// `ReadyAutoMerge`. Indeterminate terminal verdicts (`Cancelled`, `Other`)
/// stay `Draft` so a human can decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrTarget {
    /// Keep (or restore) the PR as a draft.
    Draft,
    /// Mark the PR ready for review and enable auto-merge — the terminal
    /// merge lets `pages.yml` fire on the follow-up push to `main`.
    ReadyAutoMerge,
}

/// Map a persisted [`VerificationState`] to the target state of the
/// automation PR (spec §15.1, §優先度6).
///
/// Terminal-mergeable states — `Completed` with a normalized verdict kind
/// plus `Unavailable` — flip the PR to ready + auto-merge. `Cancelled` /
/// `Other` verdicts stay draft (indeterminate: a human should decide).
/// Every non-terminal state (`Starting`, `AcceptanceUnknown`, `Submitted`,
/// `Queued`, `Judging`, `InfrastructureFailure`) keeps the PR draft.
pub fn compute_pr_target(state: &VerificationState) -> PrTarget {
    match state {
        VerificationState::Completed(c) => match c.verdict.kind {
            VerdictKind::Accepted
            | VerdictKind::WrongAnswer
            | VerdictKind::TimeLimitExceeded
            | VerdictKind::MemoryLimitExceeded
            | VerdictKind::RuntimeError
            | VerdictKind::CompileError
            | VerdictKind::OutputLimitExceeded
            | VerdictKind::JudgeError => PrTarget::ReadyAutoMerge,
            VerdictKind::Cancelled | VerdictKind::Other => PrTarget::Draft,
        },
        VerificationState::Unavailable(_) => PrTarget::ReadyAutoMerge,
        VerificationState::Starting(_)
        | VerificationState::AcceptanceUnknown(_)
        | VerificationState::Submitted(_)
        | VerificationState::Queued(_)
        | VerificationState::Judging(_)
        | VerificationState::InfrastructureFailure(_) => PrTarget::Draft,
    }
}

/// Bundle of ports needed by [`run_verify`] and the internal helpers.
///
/// Kept as a struct so call sites do not have to spell out ten arguments.
pub struct VerifyPorts<'a> {
    pub verifications: &'a dyn VerificationRepository,
    pub runner: &'a dyn CommandRunner,
    pub starters: &'a StarterRegistry,
    pub pollers: &'a PollerRegistry,
    pub recovery: &'a RecoveryRegistry,
    pub sessions: &'a dyn SessionRepository,
    pub clock: &'a dyn Clock,
    pub ids: &'a dyn AttemptIdGenerator,
    pub sleeper: &'a dyn Sleeper,
    pub retry_hint: &'a dyn RetryAfterHint,
    pub policy: PollingPolicy,
}

/// Immutable inputs (data, not ports).
pub struct VerifyInputs<'a> {
    pub repository_root: &'a Path,
    pub library_config: &'a LibraryProjectConfig,
    pub manifest: &'a DiscoveryManifest,
    pub snapshot: &'a AnalysisSnapshot,
    pub selection: VerifySelection,
    pub submit_preprocess: Option<String>,
}

/// Wraps an inner [`VerificationRepository`] so every `compare_and_swap`
/// call has `next.replaces_attempt_id == expected` regardless of what
/// `apply_transition` set it to.
///
/// The persisted-repo invariant (spec §11) treats `replaces_attempt_id` as a
/// CAS token, but `apply_transition` inherits it from the current record. The
/// two views are reconciled here so callers can compose the transition module
/// with the strict on-disk repo without leaking that mismatch outward.
struct RepoNormalizer<'a> {
    inner: &'a dyn VerificationRepository,
}

impl<'a> VerificationRepository for RepoNormalizer<'a> {
    fn load(&self, id: &SolutionId) -> Result<Option<VerificationRecord>> {
        self.inner.load(id)
    }
    fn load_all(
        &self,
        discovered: &BTreeSet<SolutionId>,
    ) -> Result<BTreeMap<SolutionId, VerificationRecord>> {
        self.inner.load_all(discovered)
    }
    fn compare_and_swap(
        &self,
        id: &SolutionId,
        expected: Option<&AttemptId>,
        next: &VerificationRecord,
    ) -> Result<()> {
        let mut fixed = next.clone();
        fixed.replaces_attempt_id = expected.cloned();
        self.inner.compare_and_swap(id, expected, &fixed)
    }
    fn remove_if_attempt(&self, id: &SolutionId, expected: &AttemptId) -> Result<()> {
        self.inner.remove_if_attempt(id, expected)
    }
}

/// Run the full `ce verify` pipeline.
pub fn run_verify(inputs: VerifyInputs<'_>, ports: VerifyPorts<'_>) -> Result<VerifyOutcome> {
    let normalizer = RepoNormalizer {
        inner: ports.verifications,
    };
    // ── 1. Enumerate published solutions and validate target ───────────
    let all_published = collect_published(inputs.manifest);
    let known_solutions: BTreeSet<SolutionId> = all_published.keys().cloned().collect();

    let mut statuses: BTreeMap<SolutionId, VerifyStatus> = BTreeMap::new();

    // Determine the set of solutions the CLI is directly targeting for a
    // potential NEW start. Non-configured entries (verify = None) still get
    // a status line in single-target mode but never move.
    let (targeted, targeted_explicit) = match &inputs.selection {
        VerifySelection::All => (
            all_published.keys().cloned().collect::<BTreeSet<_>>(),
            false,
        ),
        VerifySelection::Single(id) => {
            if !all_published.contains_key(id) {
                let mut out = BTreeMap::new();
                out.insert(id.clone(), VerifyStatus::UnknownSolution);
                return Ok(VerifyOutcome {
                    statuses: to_sorted_vec(out),
                });
            }
            let mut set = BTreeSet::new();
            set.insert(id.clone());
            (set, true)
        }
    };

    // Pre-resume snapshot: which OJs had a non-terminal record before this
    // tick? Every OJ in that set is spent for the run — resume may drive its
    // record to terminal, but we still deny new starts on that OJ so a single
    // invocation never chains "resume A → start B" on the same slot
    // (spec §8.3, verify test scenarios 5, 10).
    let pre_resume: BTreeMap<SolutionId, VerificationRecord> =
        ports.verifications.load_all(&known_solutions)?;
    let mut pre_resume_in_flight_ojs: HashSet<OJKind> = HashSet::new();
    for rec in pre_resume.values() {
        if !is_terminal(&rec.state)
            && let Some(oj) = OJKind::detect(rec.solution_id.contest_id()).map(|(o, _)| o)
        {
            pre_resume_in_flight_ojs.insert(oj);
        }
    }
    // Solutions that already have a stored record before resume runs.
    let resume_touched: BTreeSet<SolutionId> = pre_resume
        .iter()
        .filter(|(_, r)| !is_terminal(&r.state))
        .map(|(id, _)| id.clone())
        .collect();

    // ── 2. Resume any pending work across the entire published set ─────
    let resume_selection = LifecycleSelection {
        solutions: known_solutions.iter().cloned().collect(),
    };
    let _resume_summary = resume_pending(
        &VerificationRepositories {
            records: &normalizer,
            known_solutions: known_solutions.clone(),
        },
        &submission_ports(&ports),
        &resume_selection,
    )?;

    // ── 3. Load stored records after resume to classify each target ────
    let post_resume: BTreeMap<SolutionId, VerificationRecord> =
        ports.verifications.load_all(&known_solutions)?;

    // ── 4. Emit status for solutions in the published set NOT targeted
    // by the CLI (bulk mode). In single mode we only report the target.
    // Non-configured targets emit their status early.
    for id in &targeted {
        let published = &all_published[id];
        if published.verify.is_none() {
            statuses.insert(id.clone(), VerifyStatus::NotConfigured);
        }
    }

    // ── 5. Compute analyzer-derived fingerprints for each target that has
    // [verify] and is not already terminal-verified with matching print.
    // Any analyzer failure marks the solution AnalyzerBlocked.
    let mut fingerprints: BTreeMap<SolutionId, VerifyFingerprint> = BTreeMap::new();
    let mut plan_ready: BTreeMap<SolutionId, PlanBuildContext> = BTreeMap::new();
    for id in &targeted {
        if statuses.contains_key(id) {
            continue;
        }
        let published = &all_published[id];
        let verify = published
            .verify
            .as_ref()
            .expect("checked above: only Some(verify) solutions reach here");

        let starter = match ports.starters.get(&oj_for_solution(id)?) {
            Ok(s) => s,
            Err(e) => {
                statuses.insert(
                    id.clone(),
                    VerifyStatus::AnalyzerBlocked {
                        detail: format!("no starter registered: {e}"),
                    },
                );
                continue;
            }
        };

        match build_plan_context(
            inputs.repository_root,
            inputs.submit_preprocess.as_deref(),
            inputs.snapshot,
            inputs.manifest,
            published,
            verify,
            starter,
        ) {
            Ok(ctx) => {
                fingerprints.insert(id.clone(), ctx.fingerprint.clone());
                plan_ready.insert(id.clone(), ctx);
            }
            Err(detail) => {
                statuses.insert(id.clone(), VerifyStatus::AnalyzerBlocked { detail });
            }
        }
    }

    // ── 6. Classify already-terminal targets against the new fingerprint.
    // If resume touched a solution in this tick, use its current state as-is
    // (fingerprint is not re-checked, since the resumed attempt just moved
    // forward and we do not re-plan on the same tick).
    let mut needs_check_and_test: BTreeSet<SolutionId> = BTreeSet::new();
    for id in &targeted {
        if statuses.contains_key(id) {
            continue;
        }
        let fingerprint = match fingerprints.get(id) {
            Some(fp) => fp,
            None => continue, // AnalyzerBlocked already recorded
        };
        if let Some(rec) = post_resume.get(id) {
            let touched_by_resume = resume_touched.contains(id);
            match &rec.state {
                VerificationState::Completed(c) => {
                    if touched_by_resume || rec.fingerprint == *fingerprint {
                        if matches!(c.verdict.kind, VerdictKind::Accepted) {
                            statuses.insert(id.clone(), VerifyStatus::Verified);
                        } else {
                            // Rejected: do not auto-re-run in the same tick
                            // (spec §10). CLI still exits 1.
                            statuses.insert(
                                id.clone(),
                                VerifyStatus::Rejected {
                                    verdict: c.verdict.raw.clone(),
                                    summary: format!(
                                        "{:?}: not re-executed on this tick",
                                        c.verdict.kind
                                    ),
                                },
                            );
                        }
                        continue;
                    }
                }
                VerificationState::Unavailable(u) => {
                    if touched_by_resume || rec.fingerprint == *fingerprint {
                        statuses.insert(
                            id.clone(),
                            VerifyStatus::Unavailable {
                                summary: u.summary.clone(),
                            },
                        );
                        continue;
                    }
                }
                VerificationState::Starting(_)
                | VerificationState::AcceptanceUnknown(_)
                | VerificationState::Submitted(_)
                | VerificationState::Queued(_)
                | VerificationState::Judging(_)
                | VerificationState::InfrastructureFailure(_) => {
                    // Resume advanced (or attempted to) but the record is
                    // still non-terminal. Report the current state and do
                    // NOT launch a fresh plan; the record occupies the OJ
                    // slot until the next tick.
                    statuses.insert(
                        id.clone(),
                        VerifyStatus::Pending {
                            state: state_label(&rec.state).to_string(),
                            summary: "attempt still in progress".into(),
                        },
                    );
                    continue;
                }
            }
        }
        needs_check_and_test.insert(id.clone());
    }

    // ── 7. Check + test barrier for the remaining targets ──────────────
    // Distinct languages to run one check per language.
    let mut languages_to_check: BTreeSet<LanguageId> = BTreeSet::new();
    for id in &needs_check_and_test {
        languages_to_check.insert(all_published[id].language.clone());
    }
    let check_pass: BTreeMap<LanguageId, bool> = if languages_to_check.is_empty() {
        BTreeMap::new()
    } else {
        run_language_checks(
            inputs.library_config,
            &languages_to_check,
            ports.runner,
            inputs.repository_root,
        )?
    };

    // Run each solution's own test_command.
    let mut test_pass: BTreeMap<SolutionId, TestOutcome> = BTreeMap::new();
    for id in &needs_check_and_test {
        let published = &all_published[id];
        let outcome = run_solution_test(id, published, inputs.repository_root, ports.runner)?;
        test_pass.insert(id.clone(), outcome);
    }

    // Aggregate barrier: if any language check failed OR any test failed,
    // block all new starts.
    let mut any_check_failed = false;
    for pass in check_pass.values() {
        if !*pass {
            any_check_failed = true;
            break;
        }
    }
    let mut any_test_failed = false;
    for outcome in test_pass.values() {
        if !outcome.passed {
            any_test_failed = true;
            break;
        }
    }
    let new_starts_barrier = any_check_failed || any_test_failed;

    // Record barrier-driven statuses per solution.
    for id in &needs_check_and_test {
        if statuses.contains_key(id) {
            continue;
        }
        let published = &all_published[id];
        let lang_check_ok = check_pass.get(&published.language).copied().unwrap_or(true);
        let solution_test = &test_pass[id];
        if !lang_check_ok {
            statuses.insert(
                id.clone(),
                VerifyStatus::LanguageCheckFailed {
                    language: published.language.clone(),
                    detail: "check_command failed".into(),
                },
            );
        } else if !solution_test.passed {
            statuses.insert(
                id.clone(),
                VerifyStatus::TestFailed {
                    detail: solution_test.detail.clone(),
                },
            );
        }
    }

    // ── 8. Plan + start remaining targets that survived the barrier ────
    if !new_starts_barrier {
        let in_flight = &pre_resume_in_flight_ojs;
        for id in &needs_check_and_test {
            if statuses.contains_key(id) {
                continue;
            }
            let ctx = &plan_ready[id];
            if in_flight.contains(&ctx.oj) {
                statuses.insert(
                    id.clone(),
                    VerifyStatus::OjBlocked {
                        oj: ctx.oj.as_str().to_string(),
                    },
                );
                continue;
            }

            let previous_attempt_id = post_resume.get(id).map(|r| r.attempt_id.clone());
            let plan_input = PrepareVerificationInput {
                solution_id: id,
                oj: ctx.oj.as_str().to_string(),
                contest_id: id.contest_id().to_string(),
                problem_code: id.problem_code().to_string(),
                language: ctx.binding.clone(),
                submitted_source: ctx.submitted_source.clone(),
                fingerprint: ctx.fingerprint.clone(),
                verifies: ctx.verify_libraries.clone(),
                previous_attempt_id,
            };
            let plan = build_submission_plan(plan_input, ports.clock, ports.ids)
                .map_err(|e| anyhow!("planning failed for {}: {e}", id.as_str()))?;

            // Persist the Starting record + call the starter atomically via
            // the lifecycle helper. It also enforces one-in-flight-per-OJ
            // defensively against the same solution's prior state.
            let repos_bundle = VerificationRepositories {
                records: &normalizer,
                known_solutions: known_solutions.clone(),
            };
            let start_event = start_plan(&repos_bundle, &submission_ports(&ports), &plan)?;

            let mut poll_result: Option<PollEvent> = None;
            if let StartEvent::Trackable { record } = &start_event {
                let ev = poll_handle(&repos_bundle, &submission_ports(&ports), record)?;
                poll_result = Some(ev);
            }

            let status = classify_start_and_poll(id, &start_event, poll_result.as_ref());
            statuses.insert(id.clone(), status);
        }
    }

    // ── 9. Ensure every targeted solution has a status line ────────────
    if !targeted_explicit {
        // Bulk mode: report only solutions the user actually targeted, not
        // the whole published set. Non-targeted solutions with a completed
        // record stay silent.
        // (No-op: we only populated statuses for `targeted`.)
    }
    for id in &targeted {
        statuses.entry(id.clone()).or_insert_with(|| {
            // Fallback: should be unreachable, but degrade gracefully.
            VerifyStatus::Pending {
                state: "unknown".into(),
                summary: "run reached no terminal branch".into(),
            }
        });
    }

    Ok(VerifyOutcome {
        statuses: to_sorted_vec(statuses),
    })
}

/// Freeze the submission plan for a single solution and return it. Used by
/// `internal verify-prepare`. This does NOT persist the `Starting` record;
/// `start_plan` owns the Starting-before-OJ-contact invariant (spec §8.2).
pub fn prepare_solution(
    inputs: &VerifyInputs<'_>,
    ports: &VerifyPorts<'_>,
    solution_id: &SolutionId,
) -> Result<SubmissionPlan> {
    let all_published = collect_published(inputs.manifest);
    let published = all_published
        .get(solution_id)
        .ok_or_else(|| anyhow!("solution {} is not in the discovery manifest", solution_id))?;
    let verify = published
        .verify
        .as_ref()
        .ok_or_else(|| anyhow!("solution {} has no [verify] block", solution_id))?;
    let oj = oj_for_solution(solution_id)?;
    let starter = ports.starters.get(&oj)?;
    let ctx = build_plan_context(
        inputs.repository_root,
        inputs.submit_preprocess.as_deref(),
        inputs.snapshot,
        inputs.manifest,
        published,
        verify,
        starter,
    )
    .map_err(|e| anyhow!("fingerprint blocked for {}: {e}", solution_id))?;

    let known: BTreeSet<SolutionId> = all_published.keys().cloned().collect();
    let normalizer = RepoNormalizer {
        inner: ports.verifications,
    };
    let repos_bundle = VerificationRepositories {
        records: &normalizer,
        known_solutions: known,
    };
    let previous = ports.verifications.load(solution_id)?;
    let previous_attempt_id = previous.map(|r| r.attempt_id);
    let plan_input = PrepareVerificationInput {
        solution_id,
        oj: ctx.oj.as_str().to_string(),
        contest_id: solution_id.contest_id().to_string(),
        problem_code: solution_id.problem_code().to_string(),
        language: ctx.binding.clone(),
        submitted_source: ctx.submitted_source.clone(),
        fingerprint: ctx.fingerprint.clone(),
        verifies: ctx.verify_libraries.clone(),
        previous_attempt_id: previous_attempt_id.clone(),
    };
    let plan = build_submission_plan(plan_input, ports.clock, ports.ids)
        .map_err(|e| anyhow!("planning failed for {}: {e}", solution_id))?;

    // Intentionally do NOT persist a Starting record here — `start_plan`
    // owns the Starting-before-OJ-contact invariant end-to-end (spec §8.2).
    // Writing Starting from both prepare and start would trip the
    // duplicate-start guard inside `start_plan`.
    let _ = repos_bundle; // keep the binding so reviewers see the intent
    Ok(plan)
}

/// Recompute the current [`VerifyFingerprint`] for one published solution
/// (spec §11, plan 063).
///
/// Reuses [`build_plan_context`] so the fingerprint stays byte-identical to
/// what `verify-prepare` produces for the same tree. The `pick-candidate`
/// dispatcher calls this for every `VerificationState::Completed` overlay
/// record to detect input drift; other states never need a fingerprint.
///
/// The target is read from `inputs.selection`, which must be
/// [`VerifySelection::Single`] — the bulk-verify variant has no meaning for
/// a fingerprint call and is rejected up front.
pub fn compute_solution_fingerprint(
    inputs: &VerifyInputs<'_>,
    ports: &VerifyPorts<'_>,
) -> Result<VerifyFingerprint> {
    let solution_id = match &inputs.selection {
        VerifySelection::Single(id) => id,
        VerifySelection::All => {
            return Err(anyhow!(
                "compute_solution_fingerprint requires VerifySelection::Single; got All"
            ));
        }
    };
    let all_published = collect_published(inputs.manifest);
    let published = all_published
        .get(solution_id)
        .ok_or_else(|| anyhow!("solution {} is not in the discovery manifest", solution_id))?;
    let verify = published
        .verify
        .as_ref()
        .ok_or_else(|| anyhow!("solution {} has no [verify] block", solution_id))?;
    let oj = oj_for_solution(solution_id)?;
    let starter = ports.starters.get(&oj)?;
    let ctx = build_plan_context(
        inputs.repository_root,
        inputs.submit_preprocess.as_deref(),
        inputs.snapshot,
        inputs.manifest,
        published,
        verify,
        starter,
    )
    .map_err(|e| anyhow!("fingerprint blocked for {}: {e}", solution_id))?;
    Ok(ctx.fingerprint)
}

/// Drive a previously-prepared plan through `submit_prepared_plan`. Used by
/// `internal verify-start` in the credential-separated worker (spec §15.4):
/// the `persist_starting` App-only job has already written the `Starting`
/// record to `automation/verify`, so `verify-start` must skip a second
/// `Starting` persist and go straight to the OJ starter.
pub fn start_prepared_plan(
    plan: &SubmissionPlan,
    inputs: &VerifyInputs<'_>,
    ports: &VerifyPorts<'_>,
) -> Result<StartEvent> {
    // Verify the plan JSON round-trips to the same hash so a caller cannot
    // sneak a modified plan past the boundary.
    let bytes = plan.to_canonical_json_bytes();
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let recomputed_hex = format!("sha256:{:x}", hasher.finalize());
    if plan.plan_hash.as_str() != recomputed_hex {
        return Err(anyhow!(
            "plan hash mismatch: expected {}, computed {recomputed_hex}",
            plan.plan_hash
        ));
    }

    let all_published = collect_published(inputs.manifest);
    let known: BTreeSet<SolutionId> = all_published.keys().cloned().collect();
    let normalizer = RepoNormalizer {
        inner: ports.verifications,
    };
    let repos_bundle = VerificationRepositories {
        records: &normalizer,
        known_solutions: known,
    };
    submit_prepared_plan(&repos_bundle, &submission_ports(ports), plan)
}

/// Drive the current record for `solution_id` forward via `poll_handle`.
pub fn poll_current(
    solution_id: &SolutionId,
    inputs: &VerifyInputs<'_>,
    ports: &VerifyPorts<'_>,
) -> Result<PollEvent> {
    let all_published = collect_published(inputs.manifest);
    let known: BTreeSet<SolutionId> = all_published.keys().cloned().collect();
    let record = ports
        .verifications
        .load(solution_id)?
        .ok_or_else(|| anyhow!("no verification record stored for {}", solution_id))?;
    let normalizer = RepoNormalizer {
        inner: ports.verifications,
    };
    let repos_bundle = VerificationRepositories {
        records: &normalizer,
        known_solutions: known,
    };
    poll_handle(&repos_bundle, &submission_ports(ports), &record)
}

// ─── Internal helpers ───────────────────────────────────────────────────

fn submission_ports<'a>(ports: &'a VerifyPorts<'a>) -> SubmissionPorts<'a> {
    SubmissionPorts {
        starters: ports.starters,
        pollers: ports.pollers,
        recovery: ports.recovery,
        sessions: ports.sessions,
        clock: ports.clock,
        sleeper: ports.sleeper,
        retry_hint: ports.retry_hint,
        policy: ports.policy.clone(),
    }
}

fn collect_published(manifest: &DiscoveryManifest) -> BTreeMap<SolutionId, PublishedSolution> {
    manifest
        .solutions
        .iter()
        .cloned()
        .map(|s| (s.id.clone(), s))
        .collect()
}

fn to_sorted_vec(map: BTreeMap<SolutionId, VerifyStatus>) -> Vec<VerifyStatusLine> {
    map.into_iter()
        .map(|(solution_id, status)| VerifyStatusLine {
            solution_id,
            status,
        })
        .collect()
}

fn oj_for_solution(id: &SolutionId) -> Result<OJKind> {
    OJKind::detect(id.contest_id())
        .map(|(oj, _)| oj)
        .ok_or_else(|| anyhow!("cannot determine OJ from contest id {}", id.contest_id()))
}

fn is_terminal(state: &VerificationState) -> bool {
    matches!(
        state,
        VerificationState::Completed(_) | VerificationState::Unavailable(_)
    )
}

fn state_label(state: &VerificationState) -> &'static str {
    match state {
        VerificationState::Starting(_) => "starting",
        VerificationState::AcceptanceUnknown(_) => "acceptance_unknown",
        VerificationState::Submitted(_) => "submitted",
        VerificationState::Queued(_) => "queued",
        VerificationState::Judging(_) => "judging",
        VerificationState::InfrastructureFailure(_) => "infra_failure",
        VerificationState::Completed(_) => "completed",
        VerificationState::Unavailable(_) => "unavailable",
    }
}

/// Everything needed to freeze a submission plan for one solution.
struct PlanBuildContext {
    oj: OJKind,
    binding: LanguageBinding,
    submitted_source: FingerprintSource,
    verify_libraries: Vec<domain::library::LibraryId>,
    fingerprint: VerifyFingerprint,
}

/// Compute fingerprint + freeze submission bytes for one solution.
///
/// Returns a stringly-typed error so the caller can put the detail in the
/// status line without needing to plumb `thiserror` through the boundary.
fn build_plan_context(
    repository_root: &Path,
    submit_preprocess: Option<&str>,
    snapshot: &AnalysisSnapshot,
    manifest: &DiscoveryManifest,
    published: &PublishedSolution,
    verify: &domain::solution::VerifySpec,
    starter: &dyn SubmissionStarter,
) -> std::result::Result<PlanBuildContext, String> {
    let solution_id = &published.id;
    let oj = OJKind::detect(solution_id.contest_id())
        .map(|(o, _)| o)
        .ok_or_else(|| format!("unknown OJ prefix for {}", solution_id.contest_id()))?;

    // Explicit verify targets → BTreeSet for the closure API.
    let mut explicit: BTreeSet<domain::library::LibraryId> = BTreeSet::new();
    for lib in &verify.libraries {
        explicit.insert(lib.clone());
    }
    let closure = verification_closure(solution_id, &explicit, snapshot)
        .map_err(|e| format!("dependency closure blocked: {e}"))?;

    // Look up on-disk source_path for each library in the closure and read
    // its bytes.
    let mut lib_by_id: BTreeMap<&domain::library::LibraryId, &str> = BTreeMap::new();
    for lib in &manifest.libraries {
        lib_by_id.insert(&lib.id, lib.source_path.as_str());
    }
    let mut dependency_library_sources: BTreeMap<domain::library::LibraryId, FingerprintSource> =
        BTreeMap::new();
    for id in &closure {
        let path = lib_by_id
            .get(id)
            .ok_or_else(|| format!("library {} not present in manifest", id))?;
        let bytes = std::fs::read(repository_root.join(path))
            .map_err(|e| format!("failed to read library {path}: {e}"))?;
        dependency_library_sources.insert(
            id.clone(),
            FingerprintSource {
                path: (*path).to_string(),
                bytes,
            },
        );
    }

    // Read the solution's entry file.
    let mut entry_rel = published.root.clone();
    if !entry_rel.ends_with('/') {
        entry_rel.push('/');
    }
    entry_rel.push_str(&published.entry);
    let entry_bytes_raw = std::fs::read(repository_root.join(&entry_rel))
        .map_err(|e| format!("failed to read solution entry {entry_rel}: {e}"))?;

    // Run the global preprocess hook if configured (Unix only).
    #[cfg(unix)]
    let entry_bytes = match submit_preprocess {
        Some(cmd) if !cmd.trim().is_empty() => {
            match run_preprocess(
                cmd,
                &entry_bytes_raw,
                solution_id,
                &published.language,
                &oj,
                repository_root,
                &entry_rel,
            ) {
                Ok(out) => out,
                Err(e) => return Err(format!("preprocess hook failed: {e}")),
            }
        }
        _ => entry_bytes_raw,
    };
    #[cfg(not(unix))]
    let entry_bytes = {
        let _ = submit_preprocess;
        entry_bytes_raw
    };

    let submitted_source = FingerprintSource {
        path: entry_rel.clone(),
        bytes: entry_bytes,
    };

    // Adapter identity from the starter's descriptor.
    let descriptor = starter.descriptor();
    let adapter = AdapterIdentity {
        name: descriptor.name.clone(),
        version: descriptor.version.clone(),
        capabilities: capabilities_from_descriptor(&descriptor),
    };

    let binding = LanguageBinding {
        language_id: published.language.clone(),
        oj_language_id: verify.oj_language_id.clone(),
    };
    let ojb = OjBinding {
        oj: oj.as_str().to_string(),
        problem_id: solution_id.problem_code().to_string(),
        language_id: published.language.clone(),
        oj_language_id: verify.oj_language_id.clone(),
    };

    let verify_config_hash = hash_verify_config(verify);

    let mut verified_libraries: BTreeSet<domain::library::LibraryId> = BTreeSet::new();
    for lib in &verify.libraries {
        verified_libraries.insert(lib.clone());
    }
    let material = FingerprintMaterial {
        solution_id: solution_id.clone(),
        submitted_source: submitted_source.clone(),
        verified_libraries,
        dependency_library_sources,
        binding: ojb,
        adapter,
        verify_config_hash,
    };
    let fingerprint =
        calculate_fingerprint(&material).map_err(|e| format!("fingerprint failed: {e}"))?;

    // Verify libraries fed back into the plan must be strictly sorted (spec §8.1).
    let mut verify_libraries = verify.libraries.clone();
    verify_libraries.sort();
    verify_libraries.dedup();

    Ok(PlanBuildContext {
        oj,
        binding,
        submitted_source,
        verify_libraries,
        fingerprint,
    })
}

fn classify_start_and_poll(
    _solution_id: &SolutionId,
    start: &StartEvent,
    poll: Option<&PollEvent>,
) -> VerifyStatus {
    match start {
        StartEvent::Unavailable { record } => VerifyStatus::Unavailable {
            summary: match &record.state {
                VerificationState::Unavailable(u) => u.summary.clone(),
                _ => "unavailable".into(),
            },
        },
        StartEvent::AcceptanceUnknown { .. } => VerifyStatus::Pending {
            state: "acceptance_unknown".into(),
            summary: "post-send transport failure; operator confirmation needed".into(),
        },
        StartEvent::ConfirmedNotAccepted { .. } => VerifyStatus::InfraError {
            summary: "OJ actively refused submission before acceptance".into(),
        },
        StartEvent::InfrastructureError { record } => VerifyStatus::InfraError {
            summary: match &record.state {
                VerificationState::InfrastructureFailure(f) => f.summary.clone(),
                _ => "infrastructure error at start".into(),
            },
        },
        StartEvent::Trackable { record } => {
            let Some(poll_event) = poll else {
                return VerifyStatus::Pending {
                    state: state_label(&record.state).to_string(),
                    summary: "submitted; not polled yet".into(),
                };
            };
            match poll_event {
                PollEvent::Completed { record } => match &record.state {
                    VerificationState::Completed(c) => {
                        if matches!(c.verdict.kind, VerdictKind::Accepted) {
                            VerifyStatus::Verified
                        } else {
                            VerifyStatus::Rejected {
                                verdict: c.verdict.raw.clone(),
                                summary: format!("{:?}", c.verdict.kind),
                            }
                        }
                    }
                    _ => VerifyStatus::Pending {
                        state: "completed_missing_body".into(),
                        summary: String::new(),
                    },
                },
                PollEvent::Unavailable { record } => VerifyStatus::Unavailable {
                    summary: match &record.state {
                        VerificationState::Unavailable(u) => u.summary.clone(),
                        _ => "unavailable at poll".into(),
                    },
                },
                PollEvent::BudgetExhausted { record } => VerifyStatus::Pending {
                    state: state_label(&record.state).to_string(),
                    summary: "15-minute poll budget exhausted".into(),
                },
                PollEvent::InfrastructureError { record } => VerifyStatus::InfraError {
                    summary: match &record.state {
                        VerificationState::InfrastructureFailure(f) => f.summary.clone(),
                        _ => "infrastructure error at poll".into(),
                    },
                },
                PollEvent::HandleLost { .. } => VerifyStatus::InfraError {
                    summary: "handle no longer known to the OJ".into(),
                },
            }
        }
    }
}

fn run_language_checks(
    library_config: &LibraryProjectConfig,
    languages: &BTreeSet<LanguageId>,
    runner: &dyn CommandRunner,
    repository_root: &Path,
) -> Result<BTreeMap<LanguageId, bool>> {
    let mut out = BTreeMap::new();
    for lang in languages {
        if !library_config.languages.contains_key(lang) {
            // A solution language must be declared under [library.languages]
            // for the analyzer to work; skip the check gracefully but treat
            // it as passed so the barrier does not fire on a config gap.
            out.insert(lang.clone(), true);
            continue;
        }
        let summary = run_checks(
            library_config,
            &CheckSelection::Language(lang.clone()),
            runner,
            repository_root,
        )?;
        let ok = summary.aggregate_success();
        out.insert(lang.clone(), ok);
    }
    Ok(out)
}

struct TestOutcome {
    passed: bool,
    detail: String,
}

fn run_solution_test(
    solution_id: &SolutionId,
    published: &PublishedSolution,
    repository_root: &Path,
    runner: &dyn CommandRunner,
) -> Result<TestOutcome> {
    let command = published.test_command.trim();
    if command.is_empty() {
        return Ok(TestOutcome {
            passed: true,
            detail: "no test_command configured".into(),
        });
    }
    let working_dir: PathBuf = repository_root.join(&published.root);
    let mut env = BTreeMap::new();
    for key in ["PATH", "HOME", "TERM"] {
        if let Some(value) = std::env::var_os(key) {
            env.insert(OsString::from(key), value);
        }
    }
    env.insert(
        OsString::from("CE_REPOSITORY_ROOT"),
        OsString::from(repository_root.as_os_str()),
    );
    env.insert(
        OsString::from("CE_SOLUTION_ID"),
        OsString::from(solution_id.as_str()),
    );
    let request = CommandRequest {
        program: OsString::from("sh"),
        arguments: vec![OsString::from("-c"), OsString::from(command)],
        current_dir: working_dir,
        environment: env,
        timeout: Duration::from_secs(u64::from(published.test_timeout_seconds)),
    };
    let outcome = runner.run_streaming(&request)?;
    if outcome.timed_out {
        return Ok(TestOutcome {
            passed: false,
            detail: format!(
                "test_command timed out after {}s",
                published.test_timeout_seconds
            ),
        });
    }
    match outcome.exit_code {
        Some(0) => Ok(TestOutcome {
            passed: true,
            detail: "test_command exited 0".into(),
        }),
        Some(code) => Ok(TestOutcome {
            passed: false,
            detail: format!("test_command exited with code {code}"),
        }),
        None => Ok(TestOutcome {
            passed: false,
            detail: "test_command killed by signal".into(),
        }),
    }
}

#[cfg(unix)]
fn run_preprocess(
    command: &str,
    source: &[u8],
    solution_id: &SolutionId,
    language: &LanguageId,
    oj: &OJKind,
    repository_root: &Path,
    entry_rel: &str,
) -> Result<Vec<u8>> {
    use std::io::{Read as _, Write as _};
    use std::process::{Command, Stdio};

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(repository_root)
        .env("CE_LANGUAGE", language.as_str())
        .env("CE_OJ", oj.as_str())
        .env("CE_CONTEST_ID", solution_id.contest_id())
        .env("CE_PROBLEM_CODE", solution_id.problem_code())
        .env("CE_SOLUTION_ID", solution_id.as_str())
        .env("CE_SOLUTION_ENTRY", entry_rel)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| "failed to launch preprocess hook via sh")?;

    let mut stdin = child.stdin.take().expect("stdin piped");
    let source_bytes: Vec<u8> = source.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&source_bytes);
    });
    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .expect("stdout piped")
        .read_to_end(&mut stdout)?;
    let status = child.wait()?;
    let _ = writer.join();
    if !status.success() {
        return Err(anyhow!(
            "preprocess hook failed with exit code {}",
            status.code().unwrap_or(1)
        ));
    }
    Ok(stdout)
}

#[cfg(test)]
mod pr_target_tests {
    use super::*;
    use chrono::{DateTime, FixedOffset};
    use domain::online_judge::{
        RecoveryMode, ResultDetail, SubmissionCapabilities, SubmissionMode,
    };
    use domain::verification::{
        AcceptanceUnknownState, CompletedState, ContentHash, ErrorKind, FailureStage,
        InfrastructureFailure, PendingState, StartingState, SubmissionHandle, SubmissionSummary,
        SubmittedState, UnavailableReason, UnavailableState, Verdict, VerdictKind,
    };

    fn ts() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-08-10T00:00:00+00:00").unwrap()
    }

    fn language() -> LanguageBinding {
        LanguageBinding {
            language_id: LanguageId::parse("rust").unwrap(),
            oj_language_id: "rust".into(),
        }
    }

    fn hash() -> ContentHash {
        ContentHash::parse(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap()
    }

    fn handle() -> SubmissionHandle {
        SubmissionHandle {
            oj: "librarychecker".into(),
            submission_id: "sub".into(),
            submission_url: "https://example.test/sub".into(),
            locator: None,
            submitted_at: ts(),
        }
    }

    fn capabilities() -> SubmissionCapabilities {
        SubmissionCapabilities {
            submission_mode: SubmissionMode::UnattendedTrackable,
            result_detail: ResultDetail::TestcaseDetails,
            recovery_mode: RecoveryMode::BestEffort,
        }
    }

    fn completed_with(kind: VerdictKind) -> VerificationState {
        VerificationState::Completed(CompletedState {
            verdict: Verdict {
                kind,
                raw: "raw".into(),
            },
            verified_libraries: Vec::new(),
            language: language(),
            verified_at: ts(),
            capabilities: capabilities(),
            submitted_source_hash: hash(),
            input_hashes: BTreeMap::new(),
            summary: SubmissionSummary {
                max_execution_time_ms: None,
                max_memory_bytes: None,
            },
            test_cases: None,
            handle: handle(),
            extra: BTreeMap::new(),
        })
    }

    #[test]
    fn terminal_mergeable_verdicts_map_to_ready_auto_merge() {
        for kind in [
            VerdictKind::Accepted,
            VerdictKind::WrongAnswer,
            VerdictKind::TimeLimitExceeded,
            VerdictKind::MemoryLimitExceeded,
            VerdictKind::RuntimeError,
            VerdictKind::CompileError,
            VerdictKind::OutputLimitExceeded,
            VerdictKind::JudgeError,
        ] {
            assert_eq!(
                compute_pr_target(&completed_with(kind)),
                PrTarget::ReadyAutoMerge,
                "verdict {kind:?} should be ReadyAutoMerge",
            );
        }
    }

    #[test]
    fn indeterminate_verdicts_stay_draft() {
        for kind in [VerdictKind::Cancelled, VerdictKind::Other] {
            assert_eq!(
                compute_pr_target(&completed_with(kind)),
                PrTarget::Draft,
                "verdict {kind:?} should stay Draft",
            );
        }
    }

    #[test]
    fn unavailable_state_maps_to_ready_auto_merge() {
        let state = VerificationState::Unavailable(UnavailableState {
            reason: UnavailableReason::InteractiveUntrackable,
            capabilities: capabilities(),
            observed_at: ts(),
            summary: "n/a".into(),
        });
        assert_eq!(compute_pr_target(&state), PrTarget::ReadyAutoMerge);
    }

    #[test]
    fn non_terminal_states_map_to_draft() {
        let starting = VerificationState::Starting(StartingState {
            plan_hash: hash(),
            submitted_source_hash: hash(),
            language: language(),
            started_at: ts(),
        });
        let acceptance_unknown = VerificationState::AcceptanceUnknown(AcceptanceUnknownState {
            plan_hash: hash(),
            submitted_source_hash: hash(),
            language: language(),
            started_at: ts(),
            observed_at: ts(),
            summary: "n/a".into(),
        });
        let submitted = VerificationState::Submitted(SubmittedState {
            handle: handle(),
            submitted_at: ts(),
        });
        let queued = VerificationState::Queued(PendingState {
            handle: handle(),
            observed_at: ts(),
        });
        let judging = VerificationState::Judging(PendingState {
            handle: handle(),
            observed_at: ts(),
        });
        let infra = VerificationState::InfrastructureFailure(InfrastructureFailure {
            stage: FailureStage::Poll,
            error_kind: ErrorKind::Network,
            retryable: true,
            retry_count: 0,
            next_retry_at: None,
            updated_at: ts(),
            summary: "net".into(),
            plan_hash: None,
            handle: None,
        });
        for state in [
            starting,
            acceptance_unknown,
            submitted,
            queued,
            judging,
            infra,
        ] {
            assert_eq!(
                compute_pr_target(&state),
                PrTarget::Draft,
                "non-terminal state {state:?} should map to Draft",
            );
        }
    }
}
