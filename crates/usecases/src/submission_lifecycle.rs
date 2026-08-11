//! Submission lifecycle orchestrator (spec §8, §8.2, §8.3, §10).
//!
//! Three entry points are shared by `ce verify` and future `ce submit --watch`:
//!
//! - [`resume_pending`] drives every stored non-terminal record for the
//!   selected solutions forward, in the priority order Starting →
//!   AcceptanceUnknown → Submitted/Queued/Judging → InfrastructureFailure.
//! - [`start_plan`] persists the `Starting` record BEFORE calling the OJ
//!   starter (spec §8.2 safety invariant), refuses duplicate starts on the
//!   same attempt, and enforces the "one in-flight submission per OJ" rule.
//! - [`poll_handle`] drives a `Submitted`/`Queued`/`Judging` record to a
//!   terminal state (`Completed`/`Unavailable`) or reports
//!   `BudgetExhausted` when the 15-minute wall-clock budget elapses.
//!
//! # `Retry-After` hint
//!
//! The `SubmissionPoller` port itself does not carry `Retry-After` (spec §8.3)
//! and we deliberately do NOT extend that trait here — the existing port
//! surface stays untouched. Instead, this module accepts an optional
//! [`RetryAfterHint`] alongside the pollers via [`SubmissionPorts`]. Adapters
//! that want to honour `Retry-After` implement `RetryAfterHint` on the same
//! value they hand out through the poller registry and refresh it inside
//! their `poll_submission` implementation; adapters that don't just leave it
//! at [`NoRetryHint`].

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use chrono::DateTime;

use domain::entity::OJKind;
use domain::library::SolutionId;
use domain::verification::{
    AcceptanceUnknownState, ErrorKind, FailureStage, InfrastructureFailure, PendingState,
    SubmissionHandle as DomainSubmissionHandle, SubmittedState, VerificationRecord,
    VerificationState,
};

use crate::clock::Clock;
use crate::repository::session_repository::SessionRepository;
use crate::repository::verification_repository::VerificationRepository;
use crate::submission::{
    InfrastructureErrorKind, PollObservation, PollSubmissionError, PollerRegistry,
    RecoverSubmissionError, RecoveryOutcome, RecoveryRegistry, RecoveryRequest,
    StartSubmissionError, StarterRegistry, SubmissionHandle as PortSubmissionHandle,
    SubmissionRequest, SubmissionStart, SubmissionStarter,
    UnavailableReason as PortUnavailableReason, sanitize_summary,
};
use crate::verification::plan::SubmissionPlan;
use crate::verification::transition::{VerificationEvent, apply_transition};

// ─── Injected dependencies ────────────────────────────────────────────────

/// Blocking sleeper injected so tests can record the wait durations without
/// actually pausing.
pub trait Sleeper {
    fn sleep(&self, dur: Duration);
}

/// Production sleeper wrapping `std::thread::sleep`.
pub struct RealSleeper;

impl Sleeper for RealSleeper {
    fn sleep(&self, dur: Duration) {
        std::thread::sleep(dur);
    }
}

/// Optional OJ-supplied "Retry-After" hint (spec §8.3). See module docs for
/// why this is a sibling port instead of a `SubmissionPoller` trait method.
pub trait RetryAfterHint {
    /// The `Retry-After` reported by the OJ for the last completed
    /// [`SubmissionPoller::poll_submission`] call, or `None` when no hint was
    /// present (or when the port never observed one).
    fn last_retry_after(&self, oj: &OJKind) -> Option<Duration>;
}

/// Default `Retry-After` port used when the caller has nothing better —
/// always reports `None`.
pub struct NoRetryHint;

impl RetryAfterHint for NoRetryHint {
    fn last_retry_after(&self, _oj: &OJKind) -> Option<Duration> {
        None
    }
}

/// Cadence + budget applied by [`poll_handle`] (spec §8.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollingPolicy {
    /// First wait between polls (spec: 2s).
    pub initial_interval: Duration,
    /// Ceiling reached after backoff on repeated pending observations
    /// (spec: 15s).
    pub max_interval: Duration,
    /// Ceiling on the exponential backoff applied to transient infrastructure
    /// errors (spec: 30s).
    pub max_error_backoff: Duration,
    /// Total wall-clock budget per invocation (spec: 15 minutes).
    pub total_budget: Duration,
}

impl PollingPolicy {
    /// The MVP verify defaults from spec §8.3.
    pub fn verify_defaults() -> Self {
        Self {
            initial_interval: Duration::from_secs(2),
            max_interval: Duration::from_secs(15),
            max_error_backoff: Duration::from_secs(30),
            total_budget: Duration::from_secs(15 * 60),
        }
    }
}

impl Default for PollingPolicy {
    fn default() -> Self {
        Self::verify_defaults()
    }
}

/// Bundle of all OJ-facing ports needed by the lifecycle orchestrator.
///
/// Intentionally narrow — this struct is internal glue for `ce verify` and
/// `ce submit --watch`. The `sessions` port is queried per OJ; the three
/// registries look up starter/poller/recovery adapters by [`OJKind`].
pub struct SubmissionPorts<'a> {
    pub starters: &'a StarterRegistry,
    pub pollers: &'a PollerRegistry,
    pub recovery: &'a RecoveryRegistry,
    pub sessions: &'a dyn SessionRepository,
    pub clock: &'a dyn Clock,
    pub sleeper: &'a dyn Sleeper,
    pub retry_hint: &'a dyn RetryAfterHint,
    pub policy: PollingPolicy,
}

/// Bundle of persistence ports. Kept as a struct so the shape can grow
/// (per-language or per-OJ split repositories) without churning every call
/// site.
pub struct VerificationRepositories<'a> {
    pub records: &'a dyn VerificationRepository,
    /// Full solution ID space the caller intends to touch. Used to enumerate
    /// stored records via [`VerificationRepository::load_all`] and to run the
    /// "one in-flight submission per OJ" check.
    pub known_solutions: BTreeSet<SolutionId>,
}

/// Which solutions the current call should resume / drive forward.
#[derive(Debug, Clone)]
pub struct VerifySelection {
    pub solutions: Vec<SolutionId>,
}

// ─── Result shapes ────────────────────────────────────────────────────────

/// Item in [`ResumeSummary::operator_actions`] – state the orchestrator could
/// not drive further on its own and that needs an operator to look at
/// (spec §8.2, §8.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorAction {
    pub solution_id: SolutionId,
    pub summary: String,
}

/// Outcome of [`resume_pending`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSummary {
    /// Solutions whose state advanced during this call.
    pub advanced: Vec<SolutionId>,
    /// Records recovery reported as never-accepted; safe to discard and
    /// re-plan on the next tick.
    pub replan_candidates: Vec<SolutionId>,
    /// Records that need operator attention before further automated moves.
    pub operator_actions: Vec<OperatorAction>,
    /// OJs that still have a non-terminal record after resume finished; the
    /// caller must not start new plans on any OJ in this set. Sorted by the
    /// OJ's stable slug so tests see a deterministic order.
    pub in_flight_ojs: Vec<OJKind>,
    /// Solutions that finished this call in a terminal state.
    pub terminal: Vec<SolutionId>,
}

/// Outcome of a single [`start_plan`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartEvent {
    /// OJ accepted the submission; record persisted as `Submitted`.
    Trackable { record: VerificationRecord },
    /// Interactive OJ or unsupported combination; record persisted as
    /// `Unavailable`.
    Unavailable { record: VerificationRecord },
    /// Post-send transport failure — spec §8.2 safety invariant. Record
    /// persisted as `AcceptanceUnknown`.
    AcceptanceUnknown { record: VerificationRecord },
    /// OJ actively refused the request before acceptance. Caller may re-plan
    /// with a fresh attempt ID. `record` still reflects the `Starting` state
    /// this attempt was persisted in; the caller decides how to clean up.
    ConfirmedNotAccepted { record: VerificationRecord },
    /// Operational failure at the start stage; record persisted as
    /// `InfrastructureFailure`.
    InfrastructureError { record: VerificationRecord },
}

/// Outcome of a single [`poll_handle`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollEvent {
    /// Terminal successful/rejected judgement (spec §10 verified/rejected).
    Completed { record: VerificationRecord },
    /// Terminal capability failure (spec §10 unavailable). Only reachable
    /// when a poller adapter surfaces an unavailable observation via a
    /// `HandleNotFound` style signal that this module maps to
    /// `AcceptanceUnknown` (see below).
    Unavailable { record: VerificationRecord },
    /// The 15-minute wall-clock budget elapsed; the record was persisted at
    /// its last observed pending state and will resume on the next tick.
    BudgetExhausted { record: VerificationRecord },
    /// Operational failure at the poll stage (spec §8.3).
    InfrastructureError { record: VerificationRecord },
    /// The OJ reports the handle no longer exists. Spec §8.3 defensive path:
    /// map to `AcceptanceUnknown` so operator confirms before any re-plan.
    HandleLost { record: VerificationRecord },
}

// ─── Public API ───────────────────────────────────────────────────────────

/// Resume every stored non-terminal record for the selected solutions
/// (spec §8, §8.2, §10).
pub fn resume_pending(
    repositories: &VerificationRepositories,
    ports: &SubmissionPorts,
    selection: &VerifySelection,
) -> Result<ResumeSummary> {
    let stored = repositories
        .records
        .load_all(&repositories.known_solutions)?;
    let mut summary = ResumeSummary {
        advanced: vec![],
        replan_candidates: vec![],
        operator_actions: vec![],
        in_flight_ojs: vec![],
        terminal: vec![],
    };

    let selected: BTreeSet<&SolutionId> = selection.solutions.iter().collect();

    // Priority buckets (spec §8.2, §8.3): Starting first, then AcceptanceUnknown,
    // then handles present, then InfrastructureFailure.
    let mut starting: Vec<&SolutionId> = vec![];
    let mut acceptance_unknown: Vec<&SolutionId> = vec![];
    let mut with_handle: Vec<&SolutionId> = vec![];
    let mut infra_failure: Vec<&SolutionId> = vec![];

    for (id, rec) in &stored {
        if !selected.contains(id) {
            continue;
        }
        match &rec.state {
            VerificationState::Starting(_) => starting.push(id),
            VerificationState::AcceptanceUnknown(_) => acceptance_unknown.push(id),
            VerificationState::Submitted(_)
            | VerificationState::Queued(_)
            | VerificationState::Judging(_) => with_handle.push(id),
            VerificationState::InfrastructureFailure(_) => infra_failure.push(id),
            VerificationState::Completed(_) | VerificationState::Unavailable(_) => {
                // Terminal — already done, nothing to resume.
            }
        }
    }

    for id in starting
        .into_iter()
        .chain(acceptance_unknown)
        .chain(with_handle)
        .chain(infra_failure)
    {
        let rec = stored
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("record disappeared for {}", id.as_str()))?;
        resume_one(repositories, ports, &rec, &mut summary)?;
    }

    // Recompute in-flight OJs from persisted state after all moves.
    let updated = repositories
        .records
        .load_all(&repositories.known_solutions)?;
    for rec in updated.values() {
        if !is_terminal_state(&rec.state)
            && let Some(oj) = oj_for_record(rec)
            && !summary.in_flight_ojs.contains(&oj)
        {
            summary.in_flight_ojs.push(oj);
        }
    }
    // Stable order for tests.
    summary.in_flight_ojs.sort_by_key(|k| k.as_str());
    Ok(summary)
}

/// Start a submission for the given plan (spec §8.1, §8.2).
///
/// Persistence order is load-bearing:
/// 1. Persist `Starting` via `compare_and_swap` – if that fails, the starter
///    is NEVER called.
/// 2. Call [`SubmissionStarter::start_submission`].
/// 3. Persist the resulting `Submitted`/`AcceptanceUnknown`/`Unavailable`
///    record immediately, before any polling.
pub fn start_plan(
    repositories: &VerificationRepositories,
    ports: &SubmissionPorts,
    plan: &SubmissionPlan,
) -> Result<StartEvent> {
    let solution_id = plan.body.solution_id.clone();
    let oj = OJKind::from_str(&plan.body.oj)
        .map_err(|e| anyhow!("unknown OJ {} in plan: {e}", plan.body.oj))?;

    // Defensive one-in-flight-per-OJ check: another solution on the same OJ
    // with a non-terminal record blocks new starts (spec §8.3).
    let stored = repositories
        .records
        .load_all(&repositories.known_solutions)?;
    for (id, rec) in stored.iter() {
        if id == &solution_id {
            continue;
        }
        if !is_terminal_state(&rec.state) && oj_for_record(rec).as_ref() == Some(&oj) {
            bail!(
                "OJ {} already has an in-flight attempt for solution {}",
                oj.as_str(),
                id.as_str()
            );
        }
    }

    // Duplicate-start check against the same solution's current record.
    let current = stored.get(&solution_id).cloned();
    let expected_attempt = match &current {
        Some(rec) if rec.attempt_id == plan.body.attempt_id => match &rec.state {
            VerificationState::Starting(_) => {
                bail!(
                    "start_plan called twice for attempt {}",
                    plan.body.attempt_id
                );
            }
            _ => {
                bail!(
                    "record for attempt {} is already past Starting",
                    plan.body.attempt_id
                );
            }
        },
        Some(rec) => {
            if !is_terminal_state(&rec.state) {
                bail!(
                    "existing in-flight attempt {} for solution {}",
                    rec.attempt_id,
                    solution_id.as_str()
                );
            }
            Some(rec.attempt_id.clone())
        }
        None => None,
    };

    // Step 1: persist Starting before any OJ contact (spec §8.2 boundary).
    let starting_record = plan.as_starting_record();
    repositories.records.compare_and_swap(
        &solution_id,
        expected_attempt.as_ref(),
        &starting_record,
    )?;

    // Step 2: call the starter.
    let starter = ports.starters.get(&oj)?;
    let session = ports.sessions.get(&oj)?;
    let request = plan_to_request(plan, &oj);
    let outcome = starter.start_submission(&request, session.as_ref());

    // Step 3: persist the terminal-or-forwarded state.
    match outcome {
        Ok(SubmissionStart::Trackable { handle }) => {
            let domain_handle = to_domain_handle(&handle);
            let submitted_state = SubmittedState {
                handle: domain_handle,
                submitted_at: handle.submitted_at.fixed_offset(),
            };
            let next = apply_transition(
                &starting_record,
                VerificationEvent::HandleAcquired(submitted_state),
            )?;
            repositories.records.compare_and_swap(
                &solution_id,
                Some(&plan.body.attempt_id),
                &next,
            )?;
            Ok(StartEvent::Trackable { record: next })
        }
        Ok(SubmissionStart::UserActionRequired { url: _ }) => {
            let observed_at = ports.clock.now();
            let unavailable = domain::verification::UnavailableState {
                reason: domain::verification::UnavailableReason::InteractiveUntrackable,
                capabilities: capabilities_of_starter(starter),
                observed_at,
                summary: sanitize_summary("interactive-only OJ requires user action"),
            };
            let next = apply_transition(
                &starting_record,
                VerificationEvent::UnavailableObserved(unavailable),
            )?;
            repositories.records.compare_and_swap(
                &solution_id,
                Some(&plan.body.attempt_id),
                &next,
            )?;
            Ok(StartEvent::Unavailable { record: next })
        }
        Ok(SubmissionStart::Unavailable { reason }) => {
            let observed_at = ports.clock.now();
            let unavailable = domain::verification::UnavailableState {
                reason: map_unavailable_reason(&reason),
                capabilities: capabilities_of_starter(starter),
                observed_at,
                summary: sanitize_summary(&format!("{reason:?}")),
            };
            let next = apply_transition(
                &starting_record,
                VerificationEvent::UnavailableObserved(unavailable),
            )?;
            repositories.records.compare_and_swap(
                &solution_id,
                Some(&plan.body.attempt_id),
                &next,
            )?;
            Ok(StartEvent::Unavailable { record: next })
        }
        Err(StartSubmissionError::AcceptanceUnknown { summary }) => {
            let observed_at = ports.clock.now();
            let acceptance_unknown = AcceptanceUnknownState {
                plan_hash: starting_record_plan_hash(&starting_record),
                submitted_source_hash: starting_record_source_hash(&starting_record),
                language: plan.body.language.clone(),
                started_at: starting_record_started_at(&starting_record),
                observed_at,
                summary: sanitize_summary(&summary),
            };
            let next = apply_transition(
                &starting_record,
                VerificationEvent::AcceptanceLost(acceptance_unknown),
            )?;
            repositories.records.compare_and_swap(
                &solution_id,
                Some(&plan.body.attempt_id),
                &next,
            )?;
            Ok(StartEvent::AcceptanceUnknown { record: next })
        }
        Err(StartSubmissionError::ConfirmedNotAccepted { summary: _ }) => {
            // Spec §8.2: caller may re-plan with a fresh attempt ID. This
            // module does NOT remove_if_attempt — it reports the outcome and
            // leaves the record in place. The caller (Task 2) decides.
            Ok(StartEvent::ConfirmedNotAccepted {
                record: starting_record,
            })
        }
        Err(StartSubmissionError::Unavailable { reason }) => {
            let observed_at = ports.clock.now();
            let unavailable = domain::verification::UnavailableState {
                reason: map_unavailable_reason(&reason),
                capabilities: capabilities_of_starter(starter),
                observed_at,
                summary: sanitize_summary(&format!("{reason:?}")),
            };
            let next = apply_transition(
                &starting_record,
                VerificationEvent::UnavailableObserved(unavailable),
            )?;
            repositories.records.compare_and_swap(
                &solution_id,
                Some(&plan.body.attempt_id),
                &next,
            )?;
            Ok(StartEvent::Unavailable { record: next })
        }
        Err(StartSubmissionError::Infrastructure { kind, summary }) => {
            let updated_at = ports.clock.now();
            let failure = InfrastructureFailure {
                stage: FailureStage::Start,
                error_kind: map_infra_kind(&kind),
                retryable: is_retryable_kind(&kind),
                retry_count: 1,
                next_retry_at: None,
                updated_at,
                summary: sanitize_summary(&summary),
                plan_hash: Some(starting_record_plan_hash(&starting_record)),
                handle: None,
            };
            let next = apply_transition(
                &starting_record,
                VerificationEvent::InfrastructureError(failure),
            )?;
            repositories.records.compare_and_swap(
                &solution_id,
                Some(&plan.body.attempt_id),
                &next,
            )?;
            Ok(StartEvent::InfrastructureError { record: next })
        }
    }
}

/// Poll a `Submitted`/`Queued`/`Judging`/`InfrastructureFailure` record until
/// terminal or the budget elapses (spec §8, §8.3, §10).
pub fn poll_handle(
    repositories: &VerificationRepositories,
    ports: &SubmissionPorts,
    record: &VerificationRecord,
) -> Result<PollEvent> {
    // Refuse terminal states loudly — callers must not re-poll them
    // (spec §10 "変更なしの結果を明示的に再実行する command は MVP に設けない").
    match &record.state {
        VerificationState::Completed(_) | VerificationState::Unavailable(_) => {
            bail!(
                "cannot poll a terminal record (attempt {})",
                record.attempt_id
            );
        }
        VerificationState::Starting(_) | VerificationState::AcceptanceUnknown(_) => {
            bail!("poll_handle requires a record with a submission handle");
        }
        _ => {}
    }

    let domain_handle = current_handle(&record.state)
        .ok_or_else(|| anyhow!("record has no submission handle to poll"))?
        .clone();
    let oj = OJKind::from_str(&domain_handle.oj)
        .map_err(|e| anyhow!("unknown OJ {} on handle: {e}", domain_handle.oj))?;
    let port_handle = to_port_handle(&domain_handle, &oj);
    let poller = ports.pollers.get(&oj)?;
    let session = ports.sessions.get(&oj)?;

    let start_instant = ports.clock.now();
    let policy = &ports.policy;
    let mut wait = policy.initial_interval;
    let mut error_backoff = Duration::from_secs(1);
    let mut current = record.clone();

    loop {
        // Budget check happens BEFORE the sleep so we don't pay a final
        // 15s wait after already using the budget.
        let now = ports.clock.now();
        let elapsed = (now - start_instant)
            .to_std()
            .unwrap_or(Duration::from_secs(0));
        if elapsed >= policy.total_budget {
            return Ok(PollEvent::BudgetExhausted { record: current });
        }

        match poller.poll_submission(&port_handle, session.as_ref()) {
            Ok(PollObservation::Queued) => {
                let observed_at = ports.clock.now();
                let next = apply_transition(
                    &current,
                    VerificationEvent::PollQueued(PendingState {
                        handle: domain_handle.clone(),
                        observed_at,
                    }),
                )?;
                repositories.records.compare_and_swap(
                    &current.solution_id,
                    Some(&current.attempt_id),
                    &next,
                )?;
                current = next;
                sleep_with_hint(ports, &oj, wait);
                wait = std::cmp::min(policy.max_interval, wait.saturating_mul(2));
                // Reset the error backoff after a healthy observation.
                error_backoff = Duration::from_secs(1);
            }
            Ok(PollObservation::Judging) => {
                let observed_at = ports.clock.now();
                let next = apply_transition(
                    &current,
                    VerificationEvent::PollJudging(PendingState {
                        handle: domain_handle.clone(),
                        observed_at,
                    }),
                )?;
                repositories.records.compare_and_swap(
                    &current.solution_id,
                    Some(&current.attempt_id),
                    &next,
                )?;
                current = next;
                sleep_with_hint(ports, &oj, wait);
                wait = std::cmp::min(policy.max_interval, wait.saturating_mul(2));
                error_backoff = Duration::from_secs(1);
            }
            Ok(PollObservation::Completed(result)) => {
                let verified_at = ports.clock.now();
                let completed =
                    build_completed_state(&current, &port_handle, &result, verified_at)?;
                let next = apply_transition(&current, VerificationEvent::PollCompleted(completed))?;
                repositories.records.compare_and_swap(
                    &current.solution_id,
                    Some(&current.attempt_id),
                    &next,
                )?;
                return Ok(PollEvent::Completed { record: next });
            }
            Err(PollSubmissionError::HandleNotFound { summary }) => {
                // Spec §8.3 defensive path: treat as AcceptanceUnknown so an
                // operator can confirm before any re-plan. We fabricate a
                // synthetic AcceptanceUnknown from the last known handle
                // context; the record's fingerprint and attempt_id are
                // preserved by apply_transition.
                let observed_at = ports.clock.now();
                // Move Submitted/Queued/Judging → InfrastructureFailure with
                // a clear operator-attention summary. The state machine
                // allows this; we choose it over faking a fresh
                // AcceptanceUnknown because AcceptanceUnknown is only
                // reachable from Starting.
                let failure = InfrastructureFailure {
                    stage: FailureStage::Poll,
                    error_kind: ErrorKind::InvalidResponse,
                    retryable: false,
                    retry_count: 1,
                    next_retry_at: None,
                    updated_at: observed_at,
                    summary: sanitize_summary(&format!("handle no longer known: {summary}")),
                    plan_hash: None,
                    handle: Some(domain_handle.clone()),
                };
                let next =
                    apply_transition(&current, VerificationEvent::InfrastructureError(failure))?;
                repositories.records.compare_and_swap(
                    &current.solution_id,
                    Some(&current.attempt_id),
                    &next,
                )?;
                return Ok(PollEvent::HandleLost { record: next });
            }
            Err(PollSubmissionError::Infrastructure { kind, summary }) => {
                let updated_at = ports.clock.now();
                let failure = InfrastructureFailure {
                    stage: FailureStage::Poll,
                    error_kind: map_infra_kind(&kind),
                    retryable: is_retryable_kind(&kind),
                    retry_count: 1,
                    next_retry_at: None,
                    updated_at,
                    summary: sanitize_summary(&summary),
                    plan_hash: None,
                    handle: None,
                };
                if is_retryable_kind(&kind) {
                    // Persist the infra failure snapshot for observability
                    // and continue after exponential backoff (capped).
                    let next = apply_transition(
                        &current,
                        VerificationEvent::InfrastructureError(failure),
                    )?;
                    repositories.records.compare_and_swap(
                        &current.solution_id,
                        Some(&current.attempt_id),
                        &next,
                    )?;
                    current = next;
                    sleep_with_hint(ports, &oj, error_backoff);
                    error_backoff =
                        std::cmp::min(policy.max_error_backoff, error_backoff.saturating_mul(2));
                    continue;
                }
                let next =
                    apply_transition(&current, VerificationEvent::InfrastructureError(failure))?;
                repositories.records.compare_and_swap(
                    &current.solution_id,
                    Some(&current.attempt_id),
                    &next,
                )?;
                return Ok(PollEvent::InfrastructureError { record: next });
            }
        }
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────

fn resume_one(
    repositories: &VerificationRepositories,
    ports: &SubmissionPorts,
    rec: &VerificationRecord,
    summary: &mut ResumeSummary,
) -> Result<()> {
    match &rec.state {
        VerificationState::Starting(_) | VerificationState::AcceptanceUnknown(_) => {
            resume_via_recovery(repositories, ports, rec, summary)?;
        }
        VerificationState::Submitted(_)
        | VerificationState::Queued(_)
        | VerificationState::Judging(_) => {
            let event = poll_handle(repositories, ports, rec)?;
            classify_poll_event(rec.solution_id.clone(), &event, summary);
        }
        VerificationState::InfrastructureFailure(f) => {
            if f.handle.is_some() {
                let event = poll_handle(repositories, ports, rec)?;
                classify_poll_event(rec.solution_id.clone(), &event, summary);
            } else {
                summary.operator_actions.push(OperatorAction {
                    solution_id: rec.solution_id.clone(),
                    summary: f.summary.clone(),
                });
            }
        }
        VerificationState::Completed(_) | VerificationState::Unavailable(_) => {
            summary.terminal.push(rec.solution_id.clone());
        }
    }
    Ok(())
}

fn resume_via_recovery(
    repositories: &VerificationRepositories,
    ports: &SubmissionPorts,
    rec: &VerificationRecord,
    summary: &mut ResumeSummary,
) -> Result<()> {
    let oj = oj_for_record(rec).ok_or_else(|| {
        anyhow!(
            "cannot resolve OJ for solution {}",
            rec.solution_id.as_str()
        )
    })?;
    let recovery = ports.recovery.get(&oj)?;
    let session = ports.sessions.get(&oj)?;
    let request = recovery_request_from_record(rec, &oj)?;

    match recovery.recover_submission(&request, session.as_ref()) {
        Ok(RecoveryOutcome::Recovered { handle }) => {
            let submitted_state = SubmittedState {
                handle: to_domain_handle(&handle),
                submitted_at: handle.submitted_at.fixed_offset(),
            };
            let event = match &rec.state {
                VerificationState::Starting(_) => {
                    VerificationEvent::HandleAcquired(submitted_state)
                }
                VerificationState::AcceptanceUnknown(_) => {
                    VerificationEvent::HandleRecovered(submitted_state)
                }
                _ => unreachable!("resume_via_recovery only called for Starting/AU"),
            };
            let next = apply_transition(rec, event)?;
            repositories.records.compare_and_swap(
                &rec.solution_id,
                Some(&rec.attempt_id),
                &next,
            )?;
            summary.advanced.push(rec.solution_id.clone());
            // Continue polling immediately.
            let poll_event = poll_handle(repositories, ports, &next)?;
            classify_poll_event(rec.solution_id.clone(), &poll_event, summary);
        }
        Ok(RecoveryOutcome::ConfirmedNotAccepted) => {
            summary.replan_candidates.push(rec.solution_id.clone());
        }
        Ok(RecoveryOutcome::AcceptanceUnknown) => {
            match &rec.state {
                VerificationState::Starting(_) => {
                    let observed_at = ports.clock.now();
                    let acceptance_unknown = AcceptanceUnknownState {
                        plan_hash: starting_record_plan_hash(rec),
                        submitted_source_hash: starting_record_source_hash(rec),
                        language: starting_record_language(rec),
                        started_at: starting_record_started_at(rec),
                        observed_at,
                        summary: "recovery ambiguous".into(),
                    };
                    let next = apply_transition(
                        rec,
                        VerificationEvent::AcceptanceLost(acceptance_unknown),
                    )?;
                    repositories.records.compare_and_swap(
                        &rec.solution_id,
                        Some(&rec.attempt_id),
                        &next,
                    )?;
                    summary.advanced.push(rec.solution_id.clone());
                }
                VerificationState::AcceptanceUnknown(_) => {
                    // Already AU — leave state, record operator action.
                }
                _ => {}
            }
            summary.operator_actions.push(OperatorAction {
                solution_id: rec.solution_id.clone(),
                summary: "recovery could not confirm acceptance; operator must intervene".into(),
            });
        }
        Ok(RecoveryOutcome::Unsupported) => {
            summary.operator_actions.push(OperatorAction {
                solution_id: rec.solution_id.clone(),
                summary: "recovery is not supported for this OJ".into(),
            });
        }
        Err(RecoverSubmissionError::Infrastructure { kind, summary: msg }) => {
            let updated_at = ports.clock.now();
            let failure = InfrastructureFailure {
                stage: FailureStage::Prepare,
                error_kind: map_infra_kind(&kind),
                retryable: is_retryable_kind(&kind),
                retry_count: 1,
                next_retry_at: None,
                updated_at,
                summary: sanitize_summary(&msg),
                plan_hash: Some(starting_record_plan_hash(rec)),
                handle: None,
            };
            let next = apply_transition(rec, VerificationEvent::InfrastructureError(failure))?;
            repositories.records.compare_and_swap(
                &rec.solution_id,
                Some(&rec.attempt_id),
                &next,
            )?;
            summary.advanced.push(rec.solution_id.clone());
            summary.operator_actions.push(OperatorAction {
                solution_id: rec.solution_id.clone(),
                summary: "recovery infrastructure failure".into(),
            });
        }
    }
    Ok(())
}

fn classify_poll_event(id: SolutionId, event: &PollEvent, summary: &mut ResumeSummary) {
    match event {
        PollEvent::Completed { .. } => {
            summary.advanced.push(id.clone());
            summary.terminal.push(id);
        }
        PollEvent::Unavailable { .. } => {
            summary.advanced.push(id.clone());
            summary.terminal.push(id);
        }
        PollEvent::BudgetExhausted { .. } => {
            summary.advanced.push(id);
        }
        PollEvent::HandleLost { .. } => {
            summary.advanced.push(id.clone());
            summary.operator_actions.push(OperatorAction {
                solution_id: id,
                summary: "handle no longer known on OJ".into(),
            });
        }
        PollEvent::InfrastructureError { .. } => {
            summary.advanced.push(id);
        }
    }
}

fn sleep_with_hint(ports: &SubmissionPorts, oj: &OJKind, wait: Duration) {
    let effective = match ports.retry_hint.last_retry_after(oj) {
        Some(hint) if hint > wait => hint,
        _ => wait,
    };
    ports.sleeper.sleep(effective);
}

fn current_handle(state: &VerificationState) -> Option<&DomainSubmissionHandle> {
    match state {
        VerificationState::Submitted(s) => Some(&s.handle),
        VerificationState::Queued(p) | VerificationState::Judging(p) => Some(&p.handle),
        VerificationState::InfrastructureFailure(f) => f.handle.as_ref(),
        _ => None,
    }
}

fn oj_for_record(rec: &VerificationRecord) -> Option<OJKind> {
    if let Some(h) = current_handle(&rec.state) {
        return OJKind::from_str(&h.oj).ok();
    }
    // Starting / AcceptanceUnknown: derive from contest_id.
    let contest_id = rec.solution_id.contest_id();
    OJKind::detect(contest_id).map(|(o, _)| o)
}

fn is_terminal_state(state: &VerificationState) -> bool {
    matches!(
        state,
        VerificationState::Completed(_) | VerificationState::Unavailable(_)
    )
}

fn to_domain_handle(handle: &PortSubmissionHandle) -> DomainSubmissionHandle {
    DomainSubmissionHandle {
        oj: handle.online_judge.as_str().to_string(),
        submission_id: handle.submission_id.clone(),
        submission_url: handle.submission_url.clone(),
        locator: handle.locator.clone(),
        submitted_at: handle.submitted_at.fixed_offset(),
    }
}

fn to_port_handle(handle: &DomainSubmissionHandle, oj: &OJKind) -> PortSubmissionHandle {
    PortSubmissionHandle {
        online_judge: oj.clone(),
        submission_id: handle.submission_id.clone(),
        submission_url: handle.submission_url.clone(),
        locator: handle.locator.clone(),
        submitted_at: handle.submitted_at.with_timezone(&chrono::Utc),
    }
}

fn plan_to_request(plan: &SubmissionPlan, oj: &OJKind) -> SubmissionRequest {
    let source = String::from_utf8(plan.body.submitted_source_bytes.clone()).unwrap_or_else(|_| {
        String::from_utf8_lossy(&plan.body.submitted_source_bytes).into_owned()
    });
    SubmissionRequest {
        online_judge: oj.clone(),
        contest_id: plan.body.contest_id.clone(),
        problem_id: plan.body.problem_code.clone(),
        lang_id: plan.body.language.oj_language_id.clone(),
        source,
    }
}

fn recovery_request_from_record(rec: &VerificationRecord, oj: &OJKind) -> Result<RecoveryRequest> {
    let (source_hash, started_at, oj_lang_id) = match &rec.state {
        VerificationState::Starting(s) => (
            s.submitted_source_hash.as_str().to_string(),
            s.started_at,
            s.language.oj_language_id.clone(),
        ),
        VerificationState::AcceptanceUnknown(s) => (
            s.submitted_source_hash.as_str().to_string(),
            s.started_at,
            s.language.oj_language_id.clone(),
        ),
        _ => bail!("recovery_request_from_record requires Starting/AcceptanceUnknown"),
    };
    Ok(RecoveryRequest {
        online_judge: oj.clone(),
        contest_id: rec.solution_id.contest_id().to_string(),
        problem_id: rec.solution_id.problem_code().to_string(),
        lang_id: oj_lang_id,
        source_hash,
        submitted_at_lower_bound: Some(started_at.with_timezone(&chrono::Utc)),
    })
}

fn starting_record_plan_hash(rec: &VerificationRecord) -> domain::verification::ContentHash {
    match &rec.state {
        VerificationState::Starting(s) => s.plan_hash.clone(),
        VerificationState::AcceptanceUnknown(s) => s.plan_hash.clone(),
        _ => panic!("plan hash requested from non-Starting/AU state"),
    }
}

fn starting_record_source_hash(rec: &VerificationRecord) -> domain::verification::ContentHash {
    match &rec.state {
        VerificationState::Starting(s) => s.submitted_source_hash.clone(),
        VerificationState::AcceptanceUnknown(s) => s.submitted_source_hash.clone(),
        _ => panic!("source hash requested from non-Starting/AU state"),
    }
}

fn starting_record_language(rec: &VerificationRecord) -> domain::verification::LanguageBinding {
    match &rec.state {
        VerificationState::Starting(s) => s.language.clone(),
        VerificationState::AcceptanceUnknown(s) => s.language.clone(),
        _ => panic!("language requested from non-Starting/AU state"),
    }
}

fn starting_record_started_at(rec: &VerificationRecord) -> DateTime<chrono::FixedOffset> {
    match &rec.state {
        VerificationState::Starting(s) => s.started_at,
        VerificationState::AcceptanceUnknown(s) => s.started_at,
        _ => panic!("started_at requested from non-Starting/AU state"),
    }
}

fn map_infra_kind(kind: &InfrastructureErrorKind) -> ErrorKind {
    match kind {
        InfrastructureErrorKind::Network => ErrorKind::Network,
        InfrastructureErrorKind::RateLimited => ErrorKind::RateLimited,
        InfrastructureErrorKind::ServiceUnavailable => ErrorKind::ServiceUnavailable,
        InfrastructureErrorKind::CredentialsMissing => ErrorKind::CredentialsMissing,
        InfrastructureErrorKind::AuthenticationRejected => ErrorKind::AuthenticationRejected,
        InfrastructureErrorKind::InvalidResponse => ErrorKind::InvalidResponse,
        InfrastructureErrorKind::SchemaError => ErrorKind::SchemaError,
        InfrastructureErrorKind::Other => ErrorKind::Other,
    }
}

fn is_retryable_kind(kind: &InfrastructureErrorKind) -> bool {
    // Spec §8.3: network / rate-limit / 5xx are retryable; the rest are
    // operator-action-required.
    matches!(
        kind,
        InfrastructureErrorKind::Network
            | InfrastructureErrorKind::RateLimited
            | InfrastructureErrorKind::ServiceUnavailable
    )
}

fn map_unavailable_reason(
    reason: &PortUnavailableReason,
) -> domain::verification::UnavailableReason {
    match reason {
        PortUnavailableReason::InteractiveUntrackable => {
            domain::verification::UnavailableReason::InteractiveUntrackable
        }
        PortUnavailableReason::UnsupportedProblemOrLanguage { .. } => {
            domain::verification::UnavailableReason::UnsupportedMode
        }
        PortUnavailableReason::Unsupported => {
            domain::verification::UnavailableReason::UnsupportedMode
        }
    }
}

fn capabilities_of_starter(
    starter: &dyn SubmissionStarter,
) -> domain::online_judge::SubmissionCapabilities {
    let d = starter.descriptor();
    use crate::submission::{
        RecoveryMode as PortRecoveryMode, ResultDetailLevel as PortResultDetail,
        SubmissionMode as PortMode,
    };
    use domain::online_judge::{
        RecoveryMode as DomRecoveryMode, ResultDetail as DomResultDetail,
        SubmissionCapabilities as DomCaps, SubmissionMode as DomMode,
    };
    DomCaps {
        submission_mode: match d.submission_mode {
            PortMode::UnattendedTrackable => DomMode::UnattendedTrackable,
            PortMode::InteractiveTrackable => DomMode::InteractiveTrackable,
            PortMode::InteractiveUntrackable => DomMode::InteractiveUntrackable,
            PortMode::Unsupported => DomMode::Unsupported,
        },
        result_detail: match d.result_detail {
            PortResultDetail::OverallOnly => DomResultDetail::OverallOnly,
            PortResultDetail::SummaryMetrics => DomResultDetail::SummaryMetrics,
            PortResultDetail::TestcaseDetails => DomResultDetail::TestcaseDetails,
        },
        recovery_mode: match d.recovery_mode {
            PortRecoveryMode::Exact => DomRecoveryMode::Exact,
            PortRecoveryMode::BestEffort => DomRecoveryMode::BestEffort,
            PortRecoveryMode::None => DomRecoveryMode::None,
        },
    }
}

fn build_completed_state(
    current: &VerificationRecord,
    handle: &PortSubmissionHandle,
    result: &crate::submission::JudgeResult,
    verified_at: DateTime<chrono::FixedOffset>,
) -> Result<domain::verification::CompletedState> {
    use crate::submission::JudgeVerdict as PortVerdict;
    use domain::verification::{Verdict, VerdictKind};
    let (kind, raw) = match &result.verdict {
        PortVerdict::Accepted => (VerdictKind::Accepted, "AC".to_string()),
        PortVerdict::WrongAnswer => (VerdictKind::WrongAnswer, "WA".to_string()),
        PortVerdict::TimeLimitExceeded => (VerdictKind::TimeLimitExceeded, "TLE".to_string()),
        PortVerdict::MemoryLimitExceeded => (VerdictKind::MemoryLimitExceeded, "MLE".to_string()),
        PortVerdict::RuntimeError => (VerdictKind::RuntimeError, "RE".to_string()),
        PortVerdict::CompilationError => (VerdictKind::CompileError, "CE".to_string()),
        PortVerdict::InternalError => (VerdictKind::JudgeError, "IE".to_string()),
        PortVerdict::Other(raw) => (VerdictKind::Other, raw.clone()),
    };

    // Derive language + capabilities from the current record if it carries
    // them; else fall back to a minimal binding. Callers running the full
    // verify pipeline supply language on Starting; poll_handle is invoked
    // after start_plan so we always have context.
    let language = match &current.state {
        VerificationState::Starting(s) => s.language.clone(),
        VerificationState::AcceptanceUnknown(s) => s.language.clone(),
        _ => domain::verification::LanguageBinding {
            language_id: domain::library::LanguageId::parse("unknown")
                .expect("static language id is valid"),
            oj_language_id: "unknown".into(),
        },
    };

    let capabilities = domain::online_judge::SubmissionCapabilities {
        submission_mode: domain::online_judge::SubmissionMode::UnattendedTrackable,
        result_detail: if result.testcases.is_empty() {
            domain::online_judge::ResultDetail::OverallOnly
        } else {
            domain::online_judge::ResultDetail::TestcaseDetails
        },
        recovery_mode: domain::online_judge::RecoveryMode::BestEffort,
    };

    Ok(domain::verification::CompletedState {
        verdict: Verdict { kind, raw },
        verified_libraries: vec![],
        language,
        verified_at,
        capabilities,
        submitted_source_hash: match &current.state {
            VerificationState::Starting(s) => s.submitted_source_hash.clone(),
            VerificationState::AcceptanceUnknown(s) => s.submitted_source_hash.clone(),
            _ => domain::verification::ContentHash::parse(
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            )
            .expect("static hash"),
        },
        input_hashes: BTreeMap::new(),
        summary: domain::verification::SubmissionSummary {
            max_execution_time_ms: result
                .testcases
                .iter()
                .filter_map(|c| c.time_ms)
                .map(|t| t as u64)
                .max(),
            max_memory_bytes: result
                .testcases
                .iter()
                .filter_map(|c| c.memory_kib)
                .map(|m| m as u64 * 1024)
                .max(),
        },
        test_cases: if result.testcases.is_empty() {
            None
        } else {
            Some(
                result
                    .testcases
                    .iter()
                    .map(|c| domain::verification::TestCaseResult {
                        name: Some(c.name.clone()),
                        verdict: verdict_from_port(&c.verdict),
                        execution_time_ms: c.time_ms.map(|t| t as u64),
                        memory_bytes: c.memory_kib.map(|m| m as u64 * 1024),
                    })
                    .collect(),
            )
        },
        handle: to_domain_handle(handle),
        extra: BTreeMap::new(),
    })
}

fn verdict_from_port(v: &crate::submission::JudgeVerdict) -> domain::verification::Verdict {
    use crate::submission::JudgeVerdict as PortVerdict;
    use domain::verification::{Verdict, VerdictKind};
    let (kind, raw) = match v {
        PortVerdict::Accepted => (VerdictKind::Accepted, "AC".to_string()),
        PortVerdict::WrongAnswer => (VerdictKind::WrongAnswer, "WA".to_string()),
        PortVerdict::TimeLimitExceeded => (VerdictKind::TimeLimitExceeded, "TLE".to_string()),
        PortVerdict::MemoryLimitExceeded => (VerdictKind::MemoryLimitExceeded, "MLE".to_string()),
        PortVerdict::RuntimeError => (VerdictKind::RuntimeError, "RE".to_string()),
        PortVerdict::CompilationError => (VerdictKind::CompileError, "CE".to_string()),
        PortVerdict::InternalError => (VerdictKind::JudgeError, "IE".to_string()),
        PortVerdict::Other(raw) => (VerdictKind::Other, raw.clone()),
    };
    Verdict { kind, raw }
}
