//! Deterministic next-candidate selection for scheduled verify runs (spec §15).
//!
//! [`select_next_candidate`] is a pure function that consumes the current
//! publication set, the resolved latest-record map (`main` plus the in-flight
//! `automation/verify` overlay), and the freshly-computed fingerprints of
//! every currently-`Completed` solution. It returns at most one
//! [`SolutionId`] per invocation so `verify.yml`'s dispatcher can hand it to
//! the worker as the `solution` input. When no eligible solution exists the
//! caller must translate the `None` into `run_worker=false`.
//!
//! Eligibility rules — see the caller contract on
//! [`select_next_candidate`] for the details — are:
//! * no record for the solution yet;
//! * the latest record is one of the five in-flight variants (`Starting`,
//!   `AcceptanceUnknown`, `Submitted`, `Queued`, `Judging`). The worker
//!   resumes those instead of planning a new attempt, which is how spec
//!   §15.1 step 7 ("timeout や一時的障害では draft のまま残し、次回 worker が
//!   resume する") reaches CI at all;
//! * the latest record is an `InfrastructureFailure` whose retry deadline has
//!   elapsed (including the `None` "unscheduled" case that persisters emit
//!   today);
//! * the latest record is `Completed` but its `fingerprint` disagrees with
//!   the recomputed fingerprint (input drift).
//!
//! Terminal `Unavailable` and non-retryable `InfrastructureFailure` records
//! are excluded permanently — both need an operator.
//!
//! This module only answers *which* solution runs. *How* it runs — fresh plan
//! versus resume — is [`verify_action`], which the worker evaluates against
//! the record it re-reads at the head of its own run. Keeping the two in one
//! module means the picker can never offer a candidate the worker would
//! answer with a double submission.

use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, Utc};
use domain::library::SolutionId;
use domain::solution::PublishedSolution;
use domain::verification::{
    InfrastructureFailure, VerificationRecord, VerificationState, VerifyFingerprint,
};

/// Pick the next solution to verify on this tick, or `None` when nothing is
/// eligible.
///
/// # Contract
///
/// * `published` is the current publication set (post-discovery, post-verify
///   filter).
/// * `records` maps every `SolutionId` that has a latest record, resolved by
///   the caller from the records merged into `main` and the in-flight
///   `automation/verify` overlay. Solutions absent from the map are treated
///   as "never verified" and are always eligible.
/// * `fingerprints` MUST contain an entry for every `SolutionId` whose latest
///   record in `records` is [`VerificationState::Completed`]. Solutions in any
///   other state — no record, in-flight, `InfrastructureFailure`,
///   `Unavailable` — need no entry. This function indexes `fingerprints[&id]`
///   only when the record is `Completed`; a missing key for a `Completed` id
///   is a programmer error and will panic.
///
/// The returned choice is deterministic: given identical inputs, every
/// concurrent dispatcher tick collides on the same target, so the worker's
/// CAS is the sole race guard.
pub fn select_next_candidate(
    now: DateTime<FixedOffset>,
    published: &[PublishedSolution],
    records: &BTreeMap<SolutionId, VerificationRecord>,
    fingerprints: &BTreeMap<SolutionId, VerifyFingerprint>,
) -> Option<SolutionId> {
    let mut eligible: Vec<Candidate<'_>> = published
        .iter()
        .filter_map(|sol| eligibility(&sol.id, records, fingerprints, now))
        .collect();

    // Order: resume buckets first so an attempt that may already exist on
    // the OJ is driven to terminal before any new submission (spec §8.3
    // "未完了 handle が存在する場合は、その追跡を新規提出より先に行う",
    // §15.1 "worker は新規提出前に既存 draft PR を探し、保存済み pending
    // handle の追跡を優先する"). Then retry-ready records with the earliest
    // deadline first (with `None` deadline sorting ahead of any scheduled
    // retry), then everything else. Within a bucket, tie-break by the raw
    // `SolutionId` bytes so scheduling stays deterministic even when the
    // discovery layer returns solutions in a different order.
    eligible.sort();

    eligible.into_iter().next().map(|c| c.id.clone())
}

/// What the worker must do with the solution it was handed.
///
/// [`select_next_candidate`] decides *which* solution runs; this decides
/// *how*, from that solution's latest record alone. The worker re-reads the
/// record at the head of its run rather than trusting a flag computed by the
/// dispatcher, so a record that moved in between (an automation PR merging,
/// a concurrent operator dispatch) is answered from the fresh observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyAction {
    /// Freeze a new immutable plan and run the full start → poll chain.
    Fresh,
    /// Drive the stored attempt forward without planning anything: OJ
    /// recovery for `Starting` / `AcceptanceUnknown`, `poll_handle` for every
    /// state that carries a handle. Never calls the starter, so it cannot
    /// double-submit (spec §8.2, §8.3).
    Resume,
}

/// Classify a solution's latest record into the action the worker must take
/// (spec §8.2, §8.3, §15.1 step 7).
///
/// The safety invariant is one-directional: every state that may correspond
/// to a submission the OJ already accepted maps to [`VerifyAction::Resume`].
/// Only states that provably never reached the OJ — terminal results and a
/// start-stage `InfrastructureFailure` that never obtained a handle — may be
/// re-planned.
pub fn verify_action(state: &VerificationState) -> VerifyAction {
    match state {
        // A `Starting` record means the start request may have been sent
        // (spec §8.2: "`Starting` は、提出要求を送った可能性がある attempt を
        // 表す。直接再送してはならない"). `AcceptanceUnknown` is the same
        // uncertainty after a torn connection. Both go through the OJ's
        // recovery adapter, never through the starter.
        VerificationState::Starting(_)
        | VerificationState::AcceptanceUnknown(_)
        // Handle present: poll it to terminal.
        | VerificationState::Submitted(_)
        | VerificationState::Queued(_)
        | VerificationState::Judging(_) => VerifyAction::Resume,
        // Spec §8.3: "handle 取得後の failure では attempt ID と handle を
        // 維持し、start を呼ばず poll だけを再開する". A failure without a
        // handle either never sent the request or the OJ confirmed
        // non-acceptance, so re-planning is the documented recovery.
        VerificationState::InfrastructureFailure(f) => {
            if f.handle.is_some() {
                VerifyAction::Resume
            } else {
                VerifyAction::Fresh
            }
        }
        VerificationState::Completed(_) | VerificationState::Unavailable(_) => VerifyAction::Fresh,
    }
}

// ─── Internal ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate<'a> {
    bucket: Bucket,
    deadline: DateTime<FixedOffset>,
    id: &'a SolutionId,
}

impl Ord for Candidate<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.bucket
            .cmp(&other.bucket)
            .then_with(|| self.deadline.cmp(&other.deadline))
            .then_with(|| {
                self.id
                    .as_str()
                    .as_bytes()
                    .cmp(other.id.as_str().as_bytes())
            })
    }
}

impl PartialOrd for Candidate<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Scheduling priority, highest first. Mirrors the resume order
/// [`crate::submission_lifecycle::resume_pending`] applies locally so the CI
/// dispatcher and `ce verify` converge on the same target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Bucket {
    /// `Starting`: the start request may be in flight at the OJ.
    ResumeStarting,
    /// `AcceptanceUnknown`: a torn start whose acceptance is unproven.
    ResumeAcceptanceUnknown,
    /// A handle exists (`Submitted` / `Queued` / `Judging`, or an
    /// `InfrastructureFailure` that carries one): poll it to terminal.
    ResumePoll,
    /// Retryable `InfrastructureFailure` with no handle, past its deadline.
    RetryReady,
    Fresh,
}

fn eligibility<'a>(
    id: &'a SolutionId,
    records: &BTreeMap<SolutionId, VerificationRecord>,
    fingerprints: &BTreeMap<SolutionId, VerifyFingerprint>,
    now: DateTime<FixedOffset>,
) -> Option<Candidate<'a>> {
    let Some(record) = records.get(id) else {
        return Some(Candidate {
            bucket: Bucket::Fresh,
            deadline: min_datetime(),
            id,
        });
    };

    match &record.state {
        VerificationState::InfrastructureFailure(InfrastructureFailure {
            retryable: true,
            next_retry_at,
            handle,
            ..
        }) => {
            if let Some(deadline) = next_retry_at
                && *deadline > now
            {
                return None;
            }
            // A failure that already carries a handle must resume polling.
            // Routing it to `RetryReady` would make the worker freeze a new
            // plan and submit again for an attempt the OJ already accepted
            // (spec §8.3).
            let bucket = if handle.is_some() {
                Bucket::ResumePoll
            } else {
                Bucket::RetryReady
            };
            Some(Candidate {
                bucket,
                deadline: next_retry_at.unwrap_or_else(min_datetime),
                id,
            })
        }
        VerificationState::InfrastructureFailure(InfrastructureFailure {
            retryable: false,
            ..
        }) => None,
        VerificationState::Completed(_) => {
            let current = fingerprints.get(id).unwrap_or_else(|| {
                panic!(
                    "select_next_candidate: fingerprint missing for Completed solution `{id}` — the caller must populate the fingerprints map for every id whose latest record is Completed"
                );
            });
            if &record.fingerprint == current {
                None
            } else {
                Some(Candidate {
                    bucket: Bucket::Fresh,
                    deadline: min_datetime(),
                    id,
                })
            }
        }
        // The five in-flight variants are candidates so the worker can
        // resume them (spec §15.1 step 7). They carry no retry deadline, so
        // the bucket alone orders them.
        VerificationState::Starting(_) => Some(Candidate {
            bucket: Bucket::ResumeStarting,
            deadline: min_datetime(),
            id,
        }),
        VerificationState::AcceptanceUnknown(_) => Some(Candidate {
            bucket: Bucket::ResumeAcceptanceUnknown,
            deadline: min_datetime(),
            id,
        }),
        VerificationState::Submitted(_)
        | VerificationState::Queued(_)
        | VerificationState::Judging(_) => Some(Candidate {
            bucket: Bucket::ResumePoll,
            deadline: min_datetime(),
            id,
        }),
        // Terminal: never re-enters the picker (see the module docs on
        // `Unavailable` as a dead letter).
        VerificationState::Unavailable(_) => None,
    }
}

fn min_datetime() -> DateTime<FixedOffset> {
    // `chrono` only exposes `MAX_UTC` on `DateTime<Utc>`; project it into
    // the `FixedOffset` type the callers use.
    DateTime::<Utc>::MIN_UTC.fixed_offset()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use chrono::TimeZone;
    use domain::library::{LanguageId, LibraryId, SolutionId};
    use domain::online_judge::{
        RecoveryMode, ResultDetail, SubmissionCapabilities, SubmissionMode,
    };
    use domain::solution::{PublishedSolution, VerifySpec};
    use domain::verification::{
        AcceptanceUnknownState, AttemptId, CompletedState, ContentHash, ErrorKind, FailureStage,
        InfrastructureFailure, LanguageBinding, PendingState, PlanContext, StartingState,
        SubmissionHandle, SubmissionSummary, SubmittedState, UnavailableReason, UnavailableState,
        Verdict, VerdictKind, VerificationRecord, VerificationState, VerifyFingerprint,
    };

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<FixedOffset> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0)
            .unwrap()
            .fixed_offset()
    }

    fn now() -> DateTime<FixedOffset> {
        utc(2026, 8, 14, 12, 0)
    }

    fn solution(id: &str) -> PublishedSolution {
        PublishedSolution {
            id: SolutionId::parse(id).unwrap(),
            language: LanguageId::parse("rust").unwrap(),
            root: format!("solutions/{id}"),
            entry: "src/main.rs".into(),
            solved_at: now(),
            test_command: "./test.sh".into(),
            test_timeout_seconds: 600,
            verify: Some(VerifySpec {
                libraries: vec![LibraryId::parse("libraries/rust/algebra/monoid.rs").unwrap()],
                oj_language_id: "rust".into(),
            }),
        }
    }

    fn fingerprint(byte: u8) -> VerifyFingerprint {
        let hex: String = std::iter::repeat_n(byte, 32)
            .map(|b| format!("{b:02x}"))
            .collect();
        VerifyFingerprint::parse(&format!("sha256:{hex}")).unwrap()
    }

    fn content(byte: u8) -> ContentHash {
        let hex: String = std::iter::repeat_n(byte, 32)
            .map(|b| format!("{b:02x}"))
            .collect();
        ContentHash::parse(&format!("sha256:{hex}")).unwrap()
    }

    fn attempt(name: &str) -> AttemptId {
        AttemptId::parse(name).unwrap()
    }

    fn language_binding() -> LanguageBinding {
        LanguageBinding {
            language_id: LanguageId::parse("rust").unwrap(),
            oj_language_id: "rust".into(),
        }
    }

    fn plan_context() -> Option<PlanContext> {
        Some(PlanContext {
            language: language_binding(),
            submitted_source_hash: content(0x11),
            verify_libraries: Vec::new(),
        })
    }

    fn submission_handle() -> SubmissionHandle {
        SubmissionHandle {
            oj: "librarychecker".into(),
            submission_id: "1".into(),
            submission_url: "https://example.test/1".into(),
            locator: None,
            submitted_at: now(),
        }
    }

    fn capabilities() -> SubmissionCapabilities {
        SubmissionCapabilities {
            submission_mode: SubmissionMode::UnattendedTrackable,
            result_detail: ResultDetail::TestcaseDetails,
            recovery_mode: RecoveryMode::BestEffort,
        }
    }

    fn record(
        id: &SolutionId,
        fp: VerifyFingerprint,
        state: VerificationState,
    ) -> VerificationRecord {
        VerificationRecord {
            schema_version: 1,
            solution_id: id.clone(),
            attempt_id: attempt("attempt-1"),
            replaces_attempt_id: None,
            fingerprint: fp,
            state,
            plan_context: plan_context(),
        }
    }

    fn retryable_failure(next_retry_at: Option<DateTime<FixedOffset>>) -> VerificationState {
        VerificationState::InfrastructureFailure(InfrastructureFailure {
            stage: FailureStage::Poll,
            error_kind: ErrorKind::Network,
            retryable: true,
            retry_count: 1,
            next_retry_at,
            updated_at: now(),
            summary: "retryable network error".into(),
            plan_hash: None,
            handle: None,
        })
    }

    fn non_retryable_failure() -> VerificationState {
        VerificationState::InfrastructureFailure(InfrastructureFailure {
            stage: FailureStage::Prepare,
            error_kind: ErrorKind::SchemaError,
            retryable: false,
            retry_count: 0,
            next_retry_at: None,
            updated_at: now(),
            summary: "unrecoverable schema error".into(),
            plan_hash: None,
            handle: None,
        })
    }

    /// Retryable poll-stage failure that already holds a handle — spec §8.3
    /// forbids re-starting this attempt.
    fn retryable_failure_with_handle(
        next_retry_at: Option<DateTime<FixedOffset>>,
    ) -> VerificationState {
        VerificationState::InfrastructureFailure(InfrastructureFailure {
            handle: Some(submission_handle()),
            ..match retryable_failure(next_retry_at) {
                VerificationState::InfrastructureFailure(f) => f,
                _ => unreachable!("retryable_failure builds an InfrastructureFailure"),
            }
        })
    }

    /// The `HandleLost` shape from `poll_handle`: a handle is present but the
    /// failure is non-retryable, so an operator must confirm before anything
    /// automated touches it again.
    fn non_retryable_failure_with_handle() -> VerificationState {
        VerificationState::InfrastructureFailure(InfrastructureFailure {
            handle: Some(submission_handle()),
            ..match non_retryable_failure() {
                VerificationState::InfrastructureFailure(f) => f,
                _ => unreachable!("non_retryable_failure builds an InfrastructureFailure"),
            }
        })
    }

    fn completed() -> VerificationState {
        VerificationState::Completed(CompletedState {
            verdict: Verdict {
                kind: VerdictKind::Accepted,
                raw: "AC".into(),
            },
            verified_libraries: vec![],
            language: language_binding(),
            verified_at: now(),
            capabilities: capabilities(),
            submitted_source_hash: content(0x11),
            input_hashes: BTreeMap::new(),
            summary: SubmissionSummary {
                max_execution_time_ms: Some(0),
                max_memory_bytes: Some(0),
            },
            test_cases: None,
            handle: submission_handle(),
            extra: BTreeMap::new(),
        })
    }

    fn unavailable() -> VerificationState {
        VerificationState::Unavailable(UnavailableState {
            reason: UnavailableReason::OjUnsupported,
            capabilities: capabilities(),
            observed_at: now(),
            summary: "no adapter for this OJ".into(),
        })
    }

    fn starting() -> VerificationState {
        VerificationState::Starting(StartingState {
            plan_hash: content(0x22),
            submitted_source_hash: content(0x11),
            language: language_binding(),
            started_at: now(),
        })
    }

    fn acceptance_unknown() -> VerificationState {
        VerificationState::AcceptanceUnknown(AcceptanceUnknownState {
            plan_hash: content(0x22),
            submitted_source_hash: content(0x11),
            language: language_binding(),
            started_at: now(),
            observed_at: now(),
            summary: "start request timed out".into(),
        })
    }

    fn submitted() -> VerificationState {
        VerificationState::Submitted(SubmittedState {
            handle: submission_handle(),
            submitted_at: now(),
        })
    }

    fn queued() -> VerificationState {
        VerificationState::Queued(PendingState {
            handle: submission_handle(),
            observed_at: now(),
        })
    }

    fn judging() -> VerificationState {
        VerificationState::Judging(PendingState {
            handle: submission_handle(),
            observed_at: now(),
        })
    }

    // ─── Step 1: baseline selection tests ────────────────────────────────

    #[test]
    fn no_records_returns_smallest_solution_id() {
        let a = solution("abc999/a/main");
        let b = solution("abc999/b/main");
        let c = solution("abc999/c/main");
        let published = vec![c, a.clone(), b];
        let records = BTreeMap::new();
        let fingerprints = BTreeMap::new();

        assert_eq!(
            select_next_candidate(now(), &published, &records, &fingerprints),
            Some(a.id),
        );
    }

    #[test]
    fn retry_ready_beats_fresh() {
        let a = solution("abc999/a/main");
        let b = solution("abc999/b/main");
        let deadline = utc(2026, 8, 14, 10, 0);
        let mut records = BTreeMap::new();
        records.insert(
            b.id.clone(),
            record(&b.id, fingerprint(0xaa), retryable_failure(Some(deadline))),
        );
        let published = vec![a, b.clone()];
        let fingerprints = BTreeMap::new();

        assert_eq!(
            select_next_candidate(deadline, &published, &records, &fingerprints),
            Some(b.id),
        );
    }

    #[test]
    fn retry_not_ready_is_excluded() {
        let a = solution("abc999/a/main");
        let b = solution("abc999/b/main");
        let deadline = utc(2026, 8, 14, 14, 0); // future
        let mut records = BTreeMap::new();
        records.insert(
            b.id.clone(),
            record(&b.id, fingerprint(0xaa), retryable_failure(Some(deadline))),
        );
        let published = vec![a.clone(), b];
        let fingerprints = BTreeMap::new();

        assert_eq!(
            select_next_candidate(now(), &published, &records, &fingerprints),
            Some(a.id),
        );
    }

    /// The liveness contract (issue #130): a solution whose latest record is
    /// in-flight must stay a candidate, otherwise nothing ever hands it to a
    /// worker and it is never verified again.
    #[test]
    fn in_flight_variants_stay_candidates_so_the_worker_can_resume() {
        let states: [(&str, VerificationState); 5] = [
            ("starting", starting()),
            ("acceptance_unknown", acceptance_unknown()),
            ("submitted", submitted()),
            ("queued", queued()),
            ("judging", judging()),
        ];
        for (label, state) in states {
            let sol = solution("abc999/a/main");
            let mut records = BTreeMap::new();
            records.insert(sol.id.clone(), record(&sol.id, fingerprint(0xaa), state));
            let fingerprints = BTreeMap::new();
            let picked =
                select_next_candidate(now(), std::slice::from_ref(&sol), &records, &fingerprints);
            assert_eq!(
                picked,
                Some(sol.id.clone()),
                "in-flight variant `{label}` must remain a candidate to resume"
            );
        }
    }

    /// Spec §8.3 / §15.1: an attempt that may already exist at the OJ is
    /// tracked before any new submission starts.
    #[test]
    fn resume_buckets_outrank_retry_ready_and_fresh() {
        let starting_sol = solution("abc999/d/main");
        let au_sol = solution("abc999/c/main");
        let handle_sol = solution("abc999/b/main");
        let retry_sol = solution("abc999/a/main");
        let fresh_sol = solution("abc999/e/main");

        let mut records = BTreeMap::new();
        records.insert(
            starting_sol.id.clone(),
            record(&starting_sol.id, fingerprint(0xaa), starting()),
        );
        records.insert(
            au_sol.id.clone(),
            record(&au_sol.id, fingerprint(0xaa), acceptance_unknown()),
        );
        records.insert(
            handle_sol.id.clone(),
            record(&handle_sol.id, fingerprint(0xaa), queued()),
        );
        records.insert(
            retry_sol.id.clone(),
            record(&retry_sol.id, fingerprint(0xaa), retryable_failure(None)),
        );
        let fingerprints = BTreeMap::new();

        // Solution ids are deliberately reversed relative to the intended
        // priority, so an id-only ordering would fail this.
        let mut published = vec![
            retry_sol.clone(),
            handle_sol.clone(),
            au_sol.clone(),
            starting_sol.clone(),
            fresh_sol,
        ];

        for expected in [
            starting_sol.id.clone(),
            au_sol.id.clone(),
            handle_sol.id.clone(),
            retry_sol.id.clone(),
        ] {
            assert_eq!(
                select_next_candidate(now(), &published, &records, &fingerprints),
                Some(expected.clone()),
                "expected {expected} to be picked next",
            );
            // Drain the winner and re-run: the next bucket must surface.
            published.retain(|s| s.id != expected);
            records.remove(&expected);
        }
    }

    /// Spec §8.3: once a handle exists the attempt resumes by polling. If it
    /// landed in `RetryReady` the worker would freeze a fresh plan and submit
    /// the same source twice.
    #[test]
    fn poll_stage_failure_with_a_handle_resumes_instead_of_replanning() {
        let sol = solution("abc999/a/main");
        let state = retryable_failure_with_handle(None);
        assert_eq!(verify_action(&state), VerifyAction::Resume);

        let mut records = BTreeMap::new();
        records.insert(sol.id.clone(), record(&sol.id, fingerprint(0xaa), state));
        let fingerprints = BTreeMap::new();
        assert_eq!(
            select_next_candidate(now(), std::slice::from_ref(&sol), &records, &fingerprints),
            Some(sol.id.clone()),
        );

        // And it must outrank a handle-less retry-ready record.
        let other = solution("abc999/b/main");
        records.insert(
            other.id.clone(),
            record(&other.id, fingerprint(0xaa), retryable_failure(None)),
        );
        assert_eq!(
            select_next_candidate(now(), &[other, sol.clone()], &records, &fingerprints),
            Some(sol.id),
        );
    }

    /// A handle-bearing failure still honours its retry deadline (spec §8.3:
    /// "retryable failure は `next_retry_at` より前に OJ へ接続しない").
    #[test]
    fn handle_bearing_failure_before_its_deadline_is_excluded() {
        let sol = solution("abc999/a/main");
        let future = utc(2026, 8, 14, 14, 0);
        let mut records = BTreeMap::new();
        records.insert(
            sol.id.clone(),
            record(
                &sol.id,
                fingerprint(0xaa),
                retryable_failure_with_handle(Some(future)),
            ),
        );
        let fingerprints = BTreeMap::new();
        assert_eq!(
            select_next_candidate(now(), &[sol], &records, &fingerprints),
            None,
        );
    }

    /// `HandleLost` is the spec §8.3 defensive path: non-retryable even
    /// though a handle exists, so it waits for an operator.
    #[test]
    fn non_retryable_failure_with_a_handle_is_excluded() {
        let sol = solution("abc999/a/main");
        let mut records = BTreeMap::new();
        records.insert(
            sol.id.clone(),
            record(
                &sol.id,
                fingerprint(0xaa),
                non_retryable_failure_with_handle(),
            ),
        );
        let fingerprints = BTreeMap::new();
        assert_eq!(
            select_next_candidate(now(), &[sol], &records, &fingerprints),
            None,
        );
    }

    /// The double-submission guard. Every state that could correspond to a
    /// submission the OJ already holds must resume, never re-plan.
    #[test]
    fn verify_action_never_replans_a_state_that_may_have_reached_the_oj() {
        for (label, state) in [
            ("starting", starting()),
            ("acceptance_unknown", acceptance_unknown()),
            ("submitted", submitted()),
            ("queued", queued()),
            ("judging", judging()),
            (
                "poll_failure_with_handle",
                retryable_failure_with_handle(None),
            ),
            ("handle_lost", non_retryable_failure_with_handle()),
        ] {
            assert_eq!(
                verify_action(&state),
                VerifyAction::Resume,
                "state `{label}` must resume, not re-plan"
            );
        }
        for (label, state) in [
            ("start_failure_without_handle", retryable_failure(None)),
            ("non_retryable_without_handle", non_retryable_failure()),
            ("completed", completed()),
            ("unavailable", unavailable()),
        ] {
            assert_eq!(
                verify_action(&state),
                VerifyAction::Fresh,
                "state `{label}` provably never reached the OJ; it must re-plan"
            );
        }
    }

    #[test]
    fn stable_fingerprint_skips_completed() {
        let sol = solution("abc999/a/main");
        let fp = fingerprint(0xaa);
        let mut records = BTreeMap::new();
        records.insert(sol.id.clone(), record(&sol.id, fp.clone(), completed()));
        let mut fingerprints = BTreeMap::new();
        fingerprints.insert(sol.id.clone(), fp);
        assert_eq!(
            select_next_candidate(now(), &[sol], &records, &fingerprints),
            None,
        );
    }

    #[test]
    fn fingerprint_drift_makes_completed_eligible() {
        let sol = solution("abc999/a/main");
        let saved = fingerprint(0xaa);
        let current = fingerprint(0xbb);
        let mut records = BTreeMap::new();
        records.insert(sol.id.clone(), record(&sol.id, saved, completed()));
        let mut fingerprints = BTreeMap::new();
        fingerprints.insert(sol.id.clone(), current);
        assert_eq!(
            select_next_candidate(now(), std::slice::from_ref(&sol), &records, &fingerprints),
            Some(sol.id.clone()),
        );
    }

    #[test]
    fn non_retryable_failure_is_excluded() {
        let sol = solution("abc999/a/main");
        let mut records = BTreeMap::new();
        records.insert(
            sol.id.clone(),
            record(&sol.id, fingerprint(0xaa), non_retryable_failure()),
        );
        let fingerprints = BTreeMap::new();
        assert_eq!(
            select_next_candidate(now(), &[sol], &records, &fingerprints),
            None,
        );
    }

    #[test]
    fn retryable_without_deadline_is_immediately_eligible() {
        let sol = solution("abc999/a/main");
        let mut records = BTreeMap::new();
        records.insert(
            sol.id.clone(),
            record(&sol.id, fingerprint(0xaa), retryable_failure(None)),
        );
        let fingerprints = BTreeMap::new();
        assert_eq!(
            select_next_candidate(now(), std::slice::from_ref(&sol), &records, &fingerprints),
            Some(sol.id.clone()),
        );
    }

    #[test]
    fn unavailable_is_never_selected() {
        let sol = solution("abc999/a/main");
        let mut records = BTreeMap::new();
        records.insert(
            sol.id.clone(),
            record(&sol.id, fingerprint(0xaa), unavailable()),
        );
        let fingerprints = BTreeMap::new();
        assert_eq!(
            select_next_candidate(now(), &[sol], &records, &fingerprints),
            None,
        );
    }

    // ─── Step 4: stability + tie-break tests ─────────────────────────────

    #[test]
    fn tied_retry_deadlines_tie_break_by_solution_id() {
        let a = solution("abc999/a/main");
        let b = solution("abc999/b/main");
        let deadline = utc(2026, 8, 14, 10, 0);
        let mut records = BTreeMap::new();
        records.insert(
            a.id.clone(),
            record(&a.id, fingerprint(0xaa), retryable_failure(Some(deadline))),
        );
        records.insert(
            b.id.clone(),
            record(&b.id, fingerprint(0xbb), retryable_failure(Some(deadline))),
        );
        let published = vec![b, a.clone()];
        let fingerprints = BTreeMap::new();

        assert_eq!(
            select_next_candidate(deadline, &published, &records, &fingerprints),
            Some(a.id),
        );
    }

    #[test]
    fn retry_deadline_equal_to_now_is_eligible() {
        let sol = solution("abc999/a/main");
        let deadline = utc(2026, 8, 14, 12, 0);
        let mut records = BTreeMap::new();
        records.insert(
            sol.id.clone(),
            record(
                &sol.id,
                fingerprint(0xaa),
                retryable_failure(Some(deadline)),
            ),
        );
        let fingerprints = BTreeMap::new();
        assert_eq!(
            select_next_candidate(
                deadline,
                std::slice::from_ref(&sol),
                &records,
                &fingerprints
            ),
            Some(sol.id.clone()),
        );
    }

    #[test]
    fn input_order_does_not_change_output() {
        let a = solution("abc999/a/main");
        let b = solution("abc999/b/main");
        let c = solution("abc999/c/main");
        let records = BTreeMap::new();
        let fingerprints = BTreeMap::new();

        let forward = select_next_candidate(
            now(),
            &[a.clone(), b.clone(), c.clone()],
            &records,
            &fingerprints,
        );
        let reversed = select_next_candidate(now(), &[c, b, a.clone()], &records, &fingerprints);
        assert_eq!(forward, reversed);
        assert_eq!(forward, Some(a.id));
    }
}
