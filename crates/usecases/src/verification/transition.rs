//! Verification lifecycle transitions (spec §8, §8.2, §8.3, §10).
//!
//! [`apply_transition`] takes the current [`VerificationRecord`] and a
//! [`VerificationEvent`] and returns the next record (attempt ID and
//! fingerprint preserved) or an [`InvalidTransition`] error explaining which
//! source/event combination is forbidden. Every edge in the state machine is
//! matched exhaustively so backward moves or attempt-crossing events fail
//! loudly rather than silently.

use domain::verification::{
    AcceptanceUnknownState, AttemptId, CompletedState, InfrastructureFailure, PendingState,
    SubmissionHandle, SubmittedState, UnavailableState, VerificationRecord, VerificationState,
};
use thiserror::Error;

/// Events that may drive the state machine forward (spec §8, §8.2, §8.3).
///
/// `handle_recovery` variants preserve the same attempt when the poll job
/// wakes up on a stored handle; `unavailable` events are terminal per spec
/// §10.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationEvent {
    /// Submission request accepted; OJ returned a fresh handle.
    HandleAcquired(SubmittedState),
    /// Submission request may have been accepted but no handle was received
    /// (spec §8.2). Only reachable from `Starting`.
    AcceptanceLost(AcceptanceUnknownState),
    /// Handle recovered on a later run (spec §8.2), moving `AcceptanceUnknown`
    /// forward to `Submitted`.
    HandleRecovered(SubmittedState),
    /// OJ report: submission is queued.
    PollQueued(PendingState),
    /// OJ report: submission is under judgement.
    PollJudging(PendingState),
    /// OJ report: terminal judgement returned.
    PollCompleted(CompletedState),
    /// Adapter reports the attempt cannot be verified (spec §9, §10).
    UnavailableObserved(UnavailableState),
    /// Operational failure that must not be surfaced as a terminal result
    /// (spec §8.3).
    InfrastructureError(InfrastructureFailure),
}

/// Reasons [`apply_transition`] refuses to move the record forward.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum InvalidTransition {
    #[error("event `{event}` is not allowed from state `{from}`")]
    ForbiddenTransition {
        from: &'static str,
        event: &'static str,
    },
    #[error(
        "event handle `{event_handle}` differs from the record's stored handle `{record_handle}`"
    )]
    HandleMismatch {
        event_handle: String,
        record_handle: String,
    },
}

/// Compute the successor record for `current` after `event` (spec §8).
///
/// The returned record inherits `attempt_id`, `replaces_attempt_id`, and
/// `fingerprint` from `current`; only `state` changes. Callers persist the
/// new record with `VerificationRepository::compare_and_swap`.
pub fn apply_transition(
    current: &VerificationRecord,
    event: VerificationEvent,
) -> Result<VerificationRecord, InvalidTransition> {
    let next_state = next_state(&current.state, event)?;
    Ok(VerificationRecord {
        schema_version: current.schema_version,
        solution_id: current.solution_id.clone(),
        attempt_id: current.attempt_id.clone(),
        replaces_attempt_id: current.replaces_attempt_id.clone(),
        fingerprint: current.fingerprint.clone(),
        state: next_state,
    })
}

fn next_state(
    current: &VerificationState,
    event: VerificationEvent,
) -> Result<VerificationState, InvalidTransition> {
    match (current, event) {
        // ── from Starting ──────────────────────────────────────────────
        (VerificationState::Starting(_), VerificationEvent::HandleAcquired(state)) => {
            Ok(VerificationState::Submitted(state))
        }
        (VerificationState::Starting(_), VerificationEvent::AcceptanceLost(state)) => {
            Ok(VerificationState::AcceptanceUnknown(state))
        }
        (VerificationState::Starting(_), VerificationEvent::InfrastructureError(state)) => {
            Ok(VerificationState::InfrastructureFailure(state))
        }
        (VerificationState::Starting(_), VerificationEvent::UnavailableObserved(state)) => {
            Ok(VerificationState::Unavailable(state))
        }

        // ── from AcceptanceUnknown ─────────────────────────────────────
        (VerificationState::AcceptanceUnknown(_), VerificationEvent::HandleRecovered(state)) => {
            Ok(VerificationState::Submitted(state))
        }
        (
            VerificationState::AcceptanceUnknown(_),
            VerificationEvent::InfrastructureError(state),
        ) => Ok(VerificationState::InfrastructureFailure(state)),
        (
            VerificationState::AcceptanceUnknown(_),
            VerificationEvent::UnavailableObserved(state),
        ) => Ok(VerificationState::Unavailable(state)),

        // ── from Submitted ─────────────────────────────────────────────
        (VerificationState::Submitted(prev), VerificationEvent::PollQueued(state)) => {
            enforce_same_handle("Submitted", "PollQueued", &prev.handle, &state.handle)?;
            Ok(VerificationState::Queued(state))
        }
        (VerificationState::Submitted(prev), VerificationEvent::PollJudging(state)) => {
            enforce_same_handle("Submitted", "PollJudging", &prev.handle, &state.handle)?;
            Ok(VerificationState::Judging(state))
        }
        (VerificationState::Submitted(prev), VerificationEvent::PollCompleted(state)) => {
            enforce_same_handle("Submitted", "PollCompleted", &prev.handle, &state.handle)?;
            Ok(VerificationState::Completed(state))
        }
        (VerificationState::Submitted(_), VerificationEvent::InfrastructureError(state)) => {
            Ok(VerificationState::InfrastructureFailure(state))
        }
        (VerificationState::Submitted(_), VerificationEvent::UnavailableObserved(state)) => {
            Ok(VerificationState::Unavailable(state))
        }

        // ── from Queued ────────────────────────────────────────────────
        (VerificationState::Queued(prev), VerificationEvent::PollQueued(state)) => {
            enforce_same_handle("Queued", "PollQueued", &prev.handle, &state.handle)?;
            Ok(VerificationState::Queued(state))
        }
        (VerificationState::Queued(prev), VerificationEvent::PollJudging(state)) => {
            enforce_same_handle("Queued", "PollJudging", &prev.handle, &state.handle)?;
            Ok(VerificationState::Judging(state))
        }
        (VerificationState::Queued(prev), VerificationEvent::PollCompleted(state)) => {
            enforce_same_handle("Queued", "PollCompleted", &prev.handle, &state.handle)?;
            Ok(VerificationState::Completed(state))
        }
        (VerificationState::Queued(_), VerificationEvent::InfrastructureError(state)) => {
            Ok(VerificationState::InfrastructureFailure(state))
        }
        (VerificationState::Queued(_), VerificationEvent::UnavailableObserved(state)) => {
            Ok(VerificationState::Unavailable(state))
        }

        // ── from Judging ───────────────────────────────────────────────
        (VerificationState::Judging(prev), VerificationEvent::PollJudging(state)) => {
            enforce_same_handle("Judging", "PollJudging", &prev.handle, &state.handle)?;
            Ok(VerificationState::Judging(state))
        }
        (VerificationState::Judging(prev), VerificationEvent::PollCompleted(state)) => {
            enforce_same_handle("Judging", "PollCompleted", &prev.handle, &state.handle)?;
            Ok(VerificationState::Completed(state))
        }
        (VerificationState::Judging(_), VerificationEvent::InfrastructureError(state)) => {
            Ok(VerificationState::InfrastructureFailure(state))
        }
        (VerificationState::Judging(_), VerificationEvent::UnavailableObserved(state)) => {
            Ok(VerificationState::Unavailable(state))
        }

        // ── from InfrastructureFailure ─────────────────────────────────
        (VerificationState::InfrastructureFailure(prev), VerificationEvent::PollQueued(state)) => {
            enforce_prev_handle(
                "InfrastructureFailure",
                "PollQueued",
                prev.handle.as_ref(),
                &state.handle,
            )?;
            Ok(VerificationState::Queued(state))
        }
        (VerificationState::InfrastructureFailure(prev), VerificationEvent::PollJudging(state)) => {
            enforce_prev_handle(
                "InfrastructureFailure",
                "PollJudging",
                prev.handle.as_ref(),
                &state.handle,
            )?;
            Ok(VerificationState::Judging(state))
        }
        (
            VerificationState::InfrastructureFailure(prev),
            VerificationEvent::PollCompleted(state),
        ) => {
            enforce_prev_handle(
                "InfrastructureFailure",
                "PollCompleted",
                prev.handle.as_ref(),
                &state.handle,
            )?;
            Ok(VerificationState::Completed(state))
        }
        (
            VerificationState::InfrastructureFailure(_),
            VerificationEvent::InfrastructureError(state),
        ) => Ok(VerificationState::InfrastructureFailure(state)),
        (
            VerificationState::InfrastructureFailure(prev),
            VerificationEvent::HandleRecovered(state),
        ) => {
            // Handle recovery requires the recovered handle to match a
            // previously stored one, if any (spec §8.2).
            enforce_prev_handle(
                "InfrastructureFailure",
                "HandleRecovered",
                prev.handle.as_ref(),
                &state.handle,
            )?;
            Ok(VerificationState::Submitted(state))
        }
        (
            VerificationState::InfrastructureFailure(_),
            VerificationEvent::UnavailableObserved(state),
        ) => Ok(VerificationState::Unavailable(state)),

        // ── from terminal states — every event is forbidden ────────────
        (VerificationState::Completed(_), event) => Err(InvalidTransition::ForbiddenTransition {
            from: "Completed",
            event: event_label(&event),
        }),
        (VerificationState::Unavailable(_), event) => Err(InvalidTransition::ForbiddenTransition {
            from: "Unavailable",
            event: event_label(&event),
        }),

        // ── everything else is forbidden ───────────────────────────────
        (source, event) => Err(InvalidTransition::ForbiddenTransition {
            from: state_label(source),
            event: event_label(&event),
        }),
    }
}

fn enforce_same_handle(
    from: &'static str,
    event: &'static str,
    record: &SubmissionHandle,
    incoming: &SubmissionHandle,
) -> Result<(), InvalidTransition> {
    if record.submission_id == incoming.submission_id && record.oj == incoming.oj {
        Ok(())
    } else {
        let _ = from;
        let _ = event;
        Err(InvalidTransition::HandleMismatch {
            event_handle: incoming.submission_id.clone(),
            record_handle: record.submission_id.clone(),
        })
    }
}

fn enforce_prev_handle(
    from: &'static str,
    event: &'static str,
    prev: Option<&SubmissionHandle>,
    incoming: &SubmissionHandle,
) -> Result<(), InvalidTransition> {
    match prev {
        Some(h) => enforce_same_handle(from, event, h, incoming),
        None => Ok(()),
    }
}

fn state_label(state: &VerificationState) -> &'static str {
    match state {
        VerificationState::Starting(_) => "Starting",
        VerificationState::AcceptanceUnknown(_) => "AcceptanceUnknown",
        VerificationState::Submitted(_) => "Submitted",
        VerificationState::Queued(_) => "Queued",
        VerificationState::Judging(_) => "Judging",
        VerificationState::InfrastructureFailure(_) => "InfrastructureFailure",
        VerificationState::Completed(_) => "Completed",
        VerificationState::Unavailable(_) => "Unavailable",
    }
}

fn event_label(event: &VerificationEvent) -> &'static str {
    match event {
        VerificationEvent::HandleAcquired(_) => "HandleAcquired",
        VerificationEvent::AcceptanceLost(_) => "AcceptanceLost",
        VerificationEvent::HandleRecovered(_) => "HandleRecovered",
        VerificationEvent::PollQueued(_) => "PollQueued",
        VerificationEvent::PollJudging(_) => "PollJudging",
        VerificationEvent::PollCompleted(_) => "PollCompleted",
        VerificationEvent::UnavailableObserved(_) => "UnavailableObserved",
        VerificationEvent::InfrastructureError(_) => "InfrastructureError",
    }
}

// The `AttemptId` re-import keeps the trait boundary honest: the transition
// module never generates new attempt IDs — every event stays on the current
// attempt.
#[allow(dead_code)]
fn _preserve_attempt(_id: &AttemptId) {}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, FixedOffset};
    use domain::library::{LanguageId, SolutionId};
    use domain::online_judge::{
        RecoveryMode, ResultDetail, SubmissionCapabilities, SubmissionMode,
    };
    use domain::verification::{
        AttemptId, ContentHash, ErrorKind, FailureStage, LanguageBinding, StartingState,
        SubmissionSummary, UnavailableReason, Verdict, VerdictKind, VerifyFingerprint,
    };
    use std::collections::BTreeMap;

    fn now(offset_min: i64) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap()
            + chrono::Duration::minutes(offset_min)
    }

    fn handle(id: &str) -> SubmissionHandle {
        SubmissionHandle {
            oj: "librarychecker".into(),
            submission_id: id.into(),
            submission_url: format!("https://example.test/{id}"),
            locator: None,
            submitted_at: now(0),
        }
    }

    fn language() -> LanguageBinding {
        LanguageBinding {
            language_id: LanguageId::parse("rust").unwrap(),
            oj_language_id: "rust".into(),
        }
    }

    fn plan_hash() -> ContentHash {
        ContentHash::parse(
            "sha256:2222222222222222222222222222222222222222222222222222222222222222",
        )
        .unwrap()
    }

    fn source_hash() -> ContentHash {
        ContentHash::parse(
            "sha256:3333333333333333333333333333333333333333333333333333333333333333",
        )
        .unwrap()
    }

    fn fingerprint() -> VerifyFingerprint {
        VerifyFingerprint::parse(
            "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        )
        .unwrap()
    }

    fn capabilities() -> SubmissionCapabilities {
        SubmissionCapabilities {
            submission_mode: SubmissionMode::UnattendedTrackable,
            result_detail: ResultDetail::TestcaseDetails,
            recovery_mode: RecoveryMode::BestEffort,
        }
    }

    fn record(state: VerificationState) -> VerificationRecord {
        VerificationRecord {
            schema_version: 1,
            solution_id: SolutionId::parse("abc999/a/main").unwrap(),
            attempt_id: AttemptId::parse("attempt-1").unwrap(),
            replaces_attempt_id: None,
            fingerprint: fingerprint(),
            state,
        }
    }

    fn starting() -> VerificationRecord {
        record(VerificationState::Starting(StartingState {
            plan_hash: plan_hash(),
            submitted_source_hash: source_hash(),
            language: language(),
            started_at: now(0),
        }))
    }

    fn submitted(id: &str) -> VerificationRecord {
        record(VerificationState::Submitted(SubmittedState {
            handle: handle(id),
            submitted_at: now(1),
        }))
    }

    fn queued(id: &str) -> VerificationRecord {
        record(VerificationState::Queued(PendingState {
            handle: handle(id),
            observed_at: now(2),
        }))
    }

    fn judging(id: &str) -> VerificationRecord {
        record(VerificationState::Judging(PendingState {
            handle: handle(id),
            observed_at: now(3),
        }))
    }

    fn acceptance_unknown() -> VerificationRecord {
        record(VerificationState::AcceptanceUnknown(
            AcceptanceUnknownState {
                plan_hash: plan_hash(),
                submitted_source_hash: source_hash(),
                language: language(),
                started_at: now(0),
                observed_at: now(1),
                summary: "network drop after POST".into(),
            },
        ))
    }

    fn infrastructure_failure(with_handle: Option<&str>) -> VerificationRecord {
        record(VerificationState::InfrastructureFailure(
            InfrastructureFailure {
                stage: FailureStage::Poll,
                error_kind: ErrorKind::Network,
                retryable: true,
                retry_count: 1,
                next_retry_at: None,
                updated_at: now(5),
                summary: "poll timeout".into(),
                plan_hash: Some(plan_hash()),
                handle: with_handle.map(handle),
            },
        ))
    }

    fn completed_state() -> CompletedState {
        CompletedState {
            verdict: Verdict {
                kind: VerdictKind::Accepted,
                raw: "AC".into(),
            },
            verified_libraries: vec![],
            language: language(),
            verified_at: now(10),
            capabilities: capabilities(),
            submitted_source_hash: source_hash(),
            input_hashes: BTreeMap::new(),
            summary: SubmissionSummary {
                max_execution_time_ms: None,
                max_memory_bytes: None,
            },
            test_cases: None,
            handle: handle("s1"),
            extra: BTreeMap::new(),
        }
    }

    fn unavailable_state() -> UnavailableState {
        UnavailableState {
            reason: UnavailableReason::InteractiveUntrackable,
            capabilities: capabilities(),
            observed_at: now(1),
            summary: "browser only".into(),
        }
    }

    fn infra_state() -> InfrastructureFailure {
        InfrastructureFailure {
            stage: FailureStage::Start,
            error_kind: ErrorKind::Network,
            retryable: true,
            retry_count: 1,
            next_retry_at: None,
            updated_at: now(1),
            summary: "connect timeout".into(),
            plan_hash: None,
            handle: None,
        }
    }

    // ── attempt identity is preserved across every transition ─────────────

    #[test]
    fn transition_preserves_attempt_and_fingerprint() {
        let curr = starting();
        let submitted_state = SubmittedState {
            handle: handle("s1"),
            submitted_at: now(1),
        };
        let next =
            apply_transition(&curr, VerificationEvent::HandleAcquired(submitted_state)).unwrap();
        assert_eq!(next.attempt_id, curr.attempt_id);
        assert_eq!(next.fingerprint, curr.fingerprint);
        assert_eq!(next.replaces_attempt_id, curr.replaces_attempt_id);
        assert_eq!(next.solution_id, curr.solution_id);
    }

    // ── valid transitions cover every forward edge ────────────────────────

    #[test]
    fn starting_transitions() {
        assert!(
            apply_transition(
                &starting(),
                VerificationEvent::HandleAcquired(SubmittedState {
                    handle: handle("s1"),
                    submitted_at: now(1),
                }),
            )
            .is_ok()
        );
        assert!(
            apply_transition(
                &starting(),
                VerificationEvent::AcceptanceLost(AcceptanceUnknownState {
                    plan_hash: plan_hash(),
                    submitted_source_hash: source_hash(),
                    language: language(),
                    started_at: now(0),
                    observed_at: now(1),
                    summary: "disconnect".into(),
                }),
            )
            .is_ok()
        );
        assert!(
            apply_transition(
                &starting(),
                VerificationEvent::InfrastructureError(infra_state()),
            )
            .is_ok()
        );
        assert!(
            apply_transition(
                &starting(),
                VerificationEvent::UnavailableObserved(unavailable_state()),
            )
            .is_ok()
        );
    }

    #[test]
    fn acceptance_unknown_recovers_to_submitted() {
        let recovered = SubmittedState {
            handle: handle("s1"),
            submitted_at: now(3),
        };
        let next = apply_transition(
            &acceptance_unknown(),
            VerificationEvent::HandleRecovered(recovered),
        )
        .unwrap();
        assert!(matches!(next.state, VerificationState::Submitted(_)));
    }

    #[test]
    fn submitted_moves_through_pending_states() {
        assert!(matches!(
            apply_transition(
                &submitted("s1"),
                VerificationEvent::PollQueued(PendingState {
                    handle: handle("s1"),
                    observed_at: now(2),
                }),
            )
            .unwrap()
            .state,
            VerificationState::Queued(_)
        ));
        assert!(matches!(
            apply_transition(
                &submitted("s1"),
                VerificationEvent::PollJudging(PendingState {
                    handle: handle("s1"),
                    observed_at: now(2),
                }),
            )
            .unwrap()
            .state,
            VerificationState::Judging(_)
        ));
        assert!(matches!(
            apply_transition(
                &submitted("s1"),
                VerificationEvent::PollCompleted(completed_state()),
            )
            .unwrap()
            .state,
            VerificationState::Completed(_)
        ));
    }

    #[test]
    fn queued_and_judging_can_repeat_or_advance() {
        let q = queued("s1");
        // Queued → Queued (poll refresh) is allowed if handle matches.
        assert!(
            apply_transition(
                &q,
                VerificationEvent::PollQueued(PendingState {
                    handle: handle("s1"),
                    observed_at: now(3),
                }),
            )
            .is_ok()
        );
        // Queued → Judging.
        assert!(
            apply_transition(
                &q,
                VerificationEvent::PollJudging(PendingState {
                    handle: handle("s1"),
                    observed_at: now(3),
                }),
            )
            .is_ok()
        );
        let j = judging("s1");
        // Judging → Judging.
        assert!(
            apply_transition(
                &j,
                VerificationEvent::PollJudging(PendingState {
                    handle: handle("s1"),
                    observed_at: now(4),
                }),
            )
            .is_ok()
        );
        // Judging → Completed.
        assert!(apply_transition(&j, VerificationEvent::PollCompleted(completed_state())).is_ok());
    }

    #[test]
    fn handles_must_match_when_polling() {
        let q = queued("s1");
        let err = apply_transition(
            &q,
            VerificationEvent::PollJudging(PendingState {
                handle: handle("s2"),
                observed_at: now(3),
            }),
        )
        .unwrap_err();
        assert!(matches!(err, InvalidTransition::HandleMismatch { .. }));
    }

    // ── infrastructure failure preserves the handle across resume ─────────

    #[test]
    fn infrastructure_failure_resume_preserves_handle() {
        let base = infrastructure_failure(Some("s1"));
        // Poll resumes with the same handle.
        let next = apply_transition(
            &base,
            VerificationEvent::PollJudging(PendingState {
                handle: handle("s1"),
                observed_at: now(6),
            }),
        )
        .unwrap();
        assert!(matches!(next.state, VerificationState::Judging(_)));

        // Handle mismatch during resume is rejected.
        let err = apply_transition(
            &base,
            VerificationEvent::PollJudging(PendingState {
                handle: handle("s2"),
                observed_at: now(6),
            }),
        )
        .unwrap_err();
        assert!(matches!(err, InvalidTransition::HandleMismatch { .. }));
    }

    #[test]
    fn infrastructure_failure_without_handle_allows_recovery() {
        let base = infrastructure_failure(None);
        let recovered = SubmittedState {
            handle: handle("s1"),
            submitted_at: now(6),
        };
        assert!(apply_transition(&base, VerificationEvent::HandleRecovered(recovered)).is_ok());
    }

    // ── terminal states reject every event ────────────────────────────────

    #[test]
    fn completed_is_terminal() {
        let completed = record(VerificationState::Completed(completed_state()));
        for event in [
            VerificationEvent::HandleAcquired(SubmittedState {
                handle: handle("s1"),
                submitted_at: now(1),
            }),
            VerificationEvent::AcceptanceLost(AcceptanceUnknownState {
                plan_hash: plan_hash(),
                submitted_source_hash: source_hash(),
                language: language(),
                started_at: now(0),
                observed_at: now(1),
                summary: "x".into(),
            }),
            VerificationEvent::HandleRecovered(SubmittedState {
                handle: handle("s1"),
                submitted_at: now(1),
            }),
            VerificationEvent::PollQueued(PendingState {
                handle: handle("s1"),
                observed_at: now(1),
            }),
            VerificationEvent::PollJudging(PendingState {
                handle: handle("s1"),
                observed_at: now(1),
            }),
            VerificationEvent::PollCompleted(completed_state()),
            VerificationEvent::UnavailableObserved(unavailable_state()),
            VerificationEvent::InfrastructureError(infra_state()),
        ] {
            let err = apply_transition(&completed, event).unwrap_err();
            assert!(matches!(
                err,
                InvalidTransition::ForbiddenTransition {
                    from: "Completed",
                    ..
                }
            ));
        }
    }

    #[test]
    fn unavailable_is_terminal() {
        let unav = record(VerificationState::Unavailable(unavailable_state()));
        let err = apply_transition(
            &unav,
            VerificationEvent::HandleAcquired(SubmittedState {
                handle: handle("s1"),
                submitted_at: now(1),
            }),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            InvalidTransition::ForbiddenTransition {
                from: "Unavailable",
                ..
            }
        ));
    }

    // ── forbidden backward and attempt-crossing transitions ───────────────

    #[test]
    fn submitted_cannot_go_back_to_starting() {
        let s = submitted("s1");
        let err = apply_transition(
            &s,
            VerificationEvent::AcceptanceLost(AcceptanceUnknownState {
                plan_hash: plan_hash(),
                submitted_source_hash: source_hash(),
                language: language(),
                started_at: now(0),
                observed_at: now(2),
                summary: "x".into(),
            }),
        )
        .unwrap_err();
        assert!(matches!(err, InvalidTransition::ForbiddenTransition { .. }));
    }

    #[test]
    fn queued_cannot_start_a_new_submission() {
        let q = queued("s1");
        let err = apply_transition(
            &q,
            VerificationEvent::HandleAcquired(SubmittedState {
                handle: handle("s2"),
                submitted_at: now(3),
            }),
        )
        .unwrap_err();
        assert!(matches!(err, InvalidTransition::ForbiddenTransition { .. }));
    }

    #[test]
    fn judging_cannot_receive_acceptance_lost() {
        let j = judging("s1");
        let err = apply_transition(
            &j,
            VerificationEvent::AcceptanceLost(AcceptanceUnknownState {
                plan_hash: plan_hash(),
                submitted_source_hash: source_hash(),
                language: language(),
                started_at: now(0),
                observed_at: now(4),
                summary: "x".into(),
            }),
        )
        .unwrap_err();
        assert!(matches!(err, InvalidTransition::ForbiddenTransition { .. }));
    }

    #[test]
    fn acceptance_unknown_cannot_receive_poll_events() {
        let a = acceptance_unknown();
        let err = apply_transition(
            &a,
            VerificationEvent::PollJudging(PendingState {
                handle: handle("s1"),
                observed_at: now(1),
            }),
        )
        .unwrap_err();
        assert!(matches!(err, InvalidTransition::ForbiddenTransition { .. }));
    }

    #[test]
    fn starting_cannot_receive_handle_recovered() {
        // HandleRecovered comes from AcceptanceUnknown / InfrastructureFailure only.
        let err = apply_transition(
            &starting(),
            VerificationEvent::HandleRecovered(SubmittedState {
                handle: handle("s1"),
                submitted_at: now(2),
            }),
        )
        .unwrap_err();
        assert!(matches!(err, InvalidTransition::ForbiddenTransition { .. }));
    }
}
