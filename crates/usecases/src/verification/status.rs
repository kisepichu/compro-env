//! Public verification statuses (spec §10, §11).
//!
//! [`classify_solution_status`] maps the current fingerprint plus the saved
//! [`VerificationRecord`] to the outcome that the CLI and Web display. Order
//! of precedence, in the classification rules below, matches spec §10:
//! `not_configured` overrides everything else; `stale` overrides a saved
//! success once inputs change; `pending`/`judging`/`infrastructure_error`
//! remain resume targets even when a newer fingerprint arrives; and
//! `unavailable` sticks to the current fingerprint as a terminal decision.
//!
//! [`classify_library_status`] aggregates per-solution outcomes into the
//! representative library status (spec §7.2). Only solutions that name the
//! library in their direct `[verify].libraries` count as evidence; transitive
//! closure membership never promotes a library.
//!
//! [`VerificationRecord`]: domain::verification::VerificationRecord

use std::collections::BTreeSet;

use domain::library::{LibraryId, SolutionId};
use domain::solution::VerifySpec;
use domain::verification::{VerdictKind, VerificationRecord, VerificationState, VerifyFingerprint};

use crate::verification::fingerprint::FingerprintError;

/// Public verification status for a single solution (spec §10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VerificationStatus {
    Verified,
    Rejected,
    Unavailable,
    InfrastructureError,
    Pending,
    Judging,
    Stale,
    Never,
    NotConfigured,
}

/// Classify a solution's current status (spec §10, §11).
///
/// * `verify_spec` — the effective `[verify]` block on the solution
///   (`None` ⇒ `NotConfigured`).
/// * `current_fingerprint` — the fingerprint recomputed for the working tree.
///   `Err` means dependency analysis blocked fingerprinting; per spec §11 the
///   solution stays `stale` (or `never` when no record exists) so callers
///   still exit non-zero without erasing evidence.
/// * `saved` — the persisted latest [`VerificationRecord`], if any.
pub fn classify_solution_status(
    verify_spec: Option<&VerifySpec>,
    current_fingerprint: Result<&VerifyFingerprint, &FingerprintError>,
    saved: Option<&VerificationRecord>,
) -> VerificationStatus {
    if verify_spec.is_none() {
        return VerificationStatus::NotConfigured;
    }

    // Fingerprint blocked → still surface every in-flight record as a
    // resume target (spec §8.1: saved Starting / AcceptanceUnknown / handle
    // records must be picked up before new processing) and only fall back
    // to `stale` / `never` for records with a terminal (`Completed` /
    // `Unavailable`) state.
    match (current_fingerprint, saved) {
        (Err(_), None) => VerificationStatus::Never,
        (Err(_), Some(record)) => match &record.state {
            VerificationState::InfrastructureFailure(_) => VerificationStatus::InfrastructureError,
            VerificationState::Queued(_) => VerificationStatus::Pending,
            VerificationState::Judging(_) => VerificationStatus::Judging,
            VerificationState::Starting(_)
            | VerificationState::Submitted(_)
            | VerificationState::AcceptanceUnknown(_) => VerificationStatus::Pending,
            VerificationState::Completed(_) | VerificationState::Unavailable(_) => {
                VerificationStatus::Stale
            }
        },
        (Ok(current), None) => {
            let _ = current;
            VerificationStatus::Never
        }
        (Ok(current), Some(record)) => classify_with_fingerprint(current, record),
    }
}

fn classify_with_fingerprint(
    current: &VerifyFingerprint,
    record: &VerificationRecord,
) -> VerificationStatus {
    let matches = record.fingerprint == *current;
    match &record.state {
        VerificationState::Completed(state) => match state.verdict.kind {
            VerdictKind::Accepted if matches => VerificationStatus::Verified,
            VerdictKind::Accepted => VerificationStatus::Stale,
            _ if matches => VerificationStatus::Rejected,
            _ => VerificationStatus::Stale,
        },
        VerificationState::Unavailable(_) if matches => VerificationStatus::Unavailable,
        VerificationState::Unavailable(_) => VerificationStatus::Stale,
        VerificationState::InfrastructureFailure(_) => VerificationStatus::InfrastructureError,
        VerificationState::Queued(_) => VerificationStatus::Pending,
        VerificationState::Judging(_) => VerificationStatus::Judging,
        VerificationState::Submitted(_)
        | VerificationState::Starting(_)
        | VerificationState::AcceptanceUnknown(_) => VerificationStatus::Pending,
    }
}

/// Aggregate the representative status of a library from the set of
/// solutions that directly verify it (spec §7.2).
///
/// * `library` — the library whose representative status we want.
/// * `direct_verifier_status` — pairs `(solution_id, status)` for every
///   solution that names `library` in its direct `[verify].libraries`.
///
/// Solutions that merely include `library` via transitive closure are
/// deliberately excluded by the caller; passing them here would falsely
/// promote the library to `Verified`.
///
/// Rules:
/// - No direct verifiers → `Never`.
/// - Any direct verifier is `Verified` → `Verified`.
/// - Otherwise the highest-severity active status wins in this order:
///   `InfrastructureError` > `Pending` > `Judging` > `Rejected` >
///   `Unavailable` > `Stale`. `NotConfigured` never applies to a library.
pub fn classify_library_status(
    library: &LibraryId,
    direct_verifier_status: &[(SolutionId, VerificationStatus)],
) -> VerificationStatus {
    if direct_verifier_status.is_empty() {
        return VerificationStatus::Never;
    }
    let seen: BTreeSet<&VerificationStatus> =
        direct_verifier_status.iter().map(|(_, s)| s).collect();
    if seen.contains(&VerificationStatus::Verified) {
        return VerificationStatus::Verified;
    }
    for candidate in [
        VerificationStatus::InfrastructureError,
        VerificationStatus::Pending,
        VerificationStatus::Judging,
        VerificationStatus::Rejected,
        VerificationStatus::Unavailable,
        VerificationStatus::Stale,
        VerificationStatus::Never,
    ] {
        if seen.contains(&candidate) {
            let _ = library;
            return candidate;
        }
    }
    VerificationStatus::Never
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use domain::library::{LanguageId, LibraryId, SolutionId};
    use domain::online_judge::{
        RecoveryMode, ResultDetail, SubmissionCapabilities, SubmissionMode,
    };
    use domain::solution::VerifySpec;
    use domain::verification::{
        AttemptId, CompletedState, ContentHash, ErrorKind, FailureStage, InfrastructureFailure,
        LanguageBinding, PendingState, SubmissionHandle, SubmissionSummary, UnavailableReason,
        UnavailableState, Verdict, VerdictKind, VerificationRecord, VerificationState,
        VerifyFingerprint,
    };
    use std::collections::BTreeMap;

    fn fingerprint(byte: u8) -> VerifyFingerprint {
        let hex = std::iter::repeat_n(format!("{byte:02x}"), 32).collect::<String>();
        VerifyFingerprint::parse(&format!("sha256:{hex}")).unwrap()
    }

    fn attempt() -> AttemptId {
        AttemptId::parse("attempt-1").unwrap()
    }

    fn language() -> LanguageBinding {
        LanguageBinding {
            language_id: LanguageId::parse("rust").unwrap(),
            oj_language_id: "rust".into(),
        }
    }

    fn handle() -> SubmissionHandle {
        SubmissionHandle {
            oj: "librarychecker".into(),
            submission_id: "1".into(),
            submission_url: "https://example.test/1".into(),
            locator: None,
            submitted_at: DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap(),
        }
    }

    fn capabilities() -> SubmissionCapabilities {
        SubmissionCapabilities {
            submission_mode: SubmissionMode::UnattendedTrackable,
            result_detail: ResultDetail::TestcaseDetails,
            recovery_mode: RecoveryMode::BestEffort,
        }
    }

    fn record_with(state: VerificationState, fp: VerifyFingerprint) -> VerificationRecord {
        VerificationRecord {
            schema_version: 1,
            solution_id: SolutionId::parse("abc999/a/main").unwrap(),
            attempt_id: attempt(),
            replaces_attempt_id: None,
            fingerprint: fp,
            state,
        }
    }

    fn completed(kind: VerdictKind) -> VerificationState {
        VerificationState::Completed(CompletedState {
            verdict: Verdict {
                kind,
                raw: format!("{kind:?}"),
            },
            verified_libraries: vec![],
            language: language(),
            verified_at: DateTime::parse_from_rfc3339("2026-08-10T10:00:00+00:00").unwrap(),
            capabilities: capabilities(),
            submitted_source_hash: ContentHash::parse(
                "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            )
            .unwrap(),
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

    fn unavailable() -> VerificationState {
        VerificationState::Unavailable(UnavailableState {
            reason: UnavailableReason::InteractiveUntrackable,
            capabilities: capabilities(),
            observed_at: DateTime::parse_from_rfc3339("2026-08-10T09:30:00+00:00").unwrap(),
            summary: "browser only".into(),
        })
    }

    fn spec() -> VerifySpec {
        VerifySpec {
            libraries: vec![],
            oj_language_id: "rust".into(),
        }
    }

    #[test]
    fn not_configured_takes_precedence_even_over_saved_record() {
        let fp = fingerprint(1);
        let record = record_with(completed(VerdictKind::Accepted), fp.clone());
        assert_eq!(
            classify_solution_status(None, Ok(&fp), Some(&record)),
            VerificationStatus::NotConfigured,
        );
    }

    #[test]
    fn never_when_no_saved_record() {
        assert_eq!(
            classify_solution_status(Some(&spec()), Ok(&fingerprint(1)), None),
            VerificationStatus::Never,
        );
    }

    #[test]
    fn verified_only_when_fingerprint_matches_and_verdict_accepted() {
        let fp = fingerprint(1);
        let record = record_with(completed(VerdictKind::Accepted), fp.clone());
        assert_eq!(
            classify_solution_status(Some(&spec()), Ok(&fp), Some(&record)),
            VerificationStatus::Verified,
        );

        let stale_fp = fingerprint(2);
        assert_eq!(
            classify_solution_status(Some(&spec()), Ok(&stale_fp), Some(&record)),
            VerificationStatus::Stale,
        );
    }

    #[test]
    fn rejected_persists_at_current_fingerprint_only() {
        let fp = fingerprint(3);
        let record = record_with(completed(VerdictKind::WrongAnswer), fp.clone());
        assert_eq!(
            classify_solution_status(Some(&spec()), Ok(&fp), Some(&record)),
            VerificationStatus::Rejected,
        );
        assert_eq!(
            classify_solution_status(Some(&spec()), Ok(&fingerprint(4)), Some(&record)),
            VerificationStatus::Stale,
        );
    }

    #[test]
    fn unavailable_terminal_but_becomes_stale_on_fingerprint_change() {
        let fp = fingerprint(5);
        let record = record_with(unavailable(), fp.clone());
        assert_eq!(
            classify_solution_status(Some(&spec()), Ok(&fp), Some(&record)),
            VerificationStatus::Unavailable,
        );
        assert_eq!(
            classify_solution_status(Some(&spec()), Ok(&fingerprint(6)), Some(&record)),
            VerificationStatus::Stale,
        );
    }

    #[test]
    fn infrastructure_failure_survives_fingerprint_mismatch() {
        let fp = fingerprint(7);
        let state = VerificationState::InfrastructureFailure(InfrastructureFailure {
            stage: FailureStage::Prepare,
            error_kind: ErrorKind::Network,
            retryable: true,
            retry_count: 1,
            next_retry_at: None,
            updated_at: DateTime::parse_from_rfc3339("2026-08-10T09:30:00+00:00").unwrap(),
            summary: "timeout".into(),
            plan_hash: None,
            handle: None,
        });
        let record = record_with(state, fp);
        assert_eq!(
            classify_solution_status(Some(&spec()), Ok(&fingerprint(8)), Some(&record)),
            VerificationStatus::InfrastructureError,
        );
    }

    #[test]
    fn pending_and_judging_are_resume_targets() {
        let fp = fingerprint(9);
        let queued = record_with(
            VerificationState::Queued(PendingState {
                handle: handle(),
                observed_at: DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap(),
            }),
            fp.clone(),
        );
        let judging = record_with(
            VerificationState::Judging(PendingState {
                handle: handle(),
                observed_at: DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap(),
            }),
            fp,
        );
        assert_eq!(
            classify_solution_status(Some(&spec()), Ok(&fingerprint(10)), Some(&queued)),
            VerificationStatus::Pending,
        );
        assert_eq!(
            classify_solution_status(Some(&spec()), Ok(&fingerprint(10)), Some(&judging)),
            VerificationStatus::Judging,
        );
    }

    #[test]
    fn blocked_fingerprint_falls_back_to_stale_or_never() {
        let err = FingerprintError::SolutionDependencyBlocked {
            solution: SolutionId::parse("abc999/a/main").unwrap(),
            state: "partial",
        };
        assert_eq!(
            classify_solution_status(Some(&spec()), Err(&err), None),
            VerificationStatus::Never,
        );

        let fp = fingerprint(11);
        let record = record_with(completed(VerdictKind::Accepted), fp);
        assert_eq!(
            classify_solution_status(Some(&spec()), Err(&err), Some(&record)),
            VerificationStatus::Stale,
        );
    }

    #[test]
    fn blocked_fingerprint_preserves_active_resume_targets() {
        let err = FingerprintError::SolutionDependencyBlocked {
            solution: SolutionId::parse("abc999/a/main").unwrap(),
            state: "failed",
        };
        let fp = fingerprint(12);
        let state = VerificationState::Queued(PendingState {
            handle: handle(),
            observed_at: DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap(),
        });
        let record = record_with(state, fp);
        assert_eq!(
            classify_solution_status(Some(&spec()), Err(&err), Some(&record)),
            VerificationStatus::Pending,
        );
    }

    #[test]
    fn blocked_fingerprint_treats_pre_handle_states_as_resume_targets() {
        let err = FingerprintError::SolutionDependencyBlocked {
            solution: SolutionId::parse("abc999/a/main").unwrap(),
            state: "partial",
        };
        let fp = fingerprint(13);
        let starting = record_with(
            VerificationState::Starting(domain::verification::StartingState {
                plan_hash: ContentHash::parse(
                    "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                )
                .unwrap(),
                submitted_source_hash: ContentHash::parse(
                    "sha256:3333333333333333333333333333333333333333333333333333333333333333",
                )
                .unwrap(),
                language: language(),
                started_at: DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap(),
            }),
            fp.clone(),
        );
        assert_eq!(
            classify_solution_status(Some(&spec()), Err(&err), Some(&starting)),
            VerificationStatus::Pending,
        );

        let submitted = record_with(
            VerificationState::Submitted(domain::verification::SubmittedState {
                handle: handle(),
                submitted_at: DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap(),
            }),
            fp.clone(),
        );
        assert_eq!(
            classify_solution_status(Some(&spec()), Err(&err), Some(&submitted)),
            VerificationStatus::Pending,
        );

        let unknown = record_with(
            VerificationState::AcceptanceUnknown(domain::verification::AcceptanceUnknownState {
                plan_hash: ContentHash::parse(
                    "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                )
                .unwrap(),
                submitted_source_hash: ContentHash::parse(
                    "sha256:3333333333333333333333333333333333333333333333333333333333333333",
                )
                .unwrap(),
                language: language(),
                started_at: DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap(),
                observed_at: DateTime::parse_from_rfc3339("2026-08-10T09:30:00+00:00").unwrap(),
                summary: "disconnect".into(),
            }),
            fp,
        );
        assert_eq!(
            classify_solution_status(Some(&spec()), Err(&err), Some(&unknown)),
            VerificationStatus::Pending,
        );
    }

    // ─── library aggregation ───────────────────────────────────────────────

    fn sol(id: &str) -> SolutionId {
        SolutionId::parse(id).unwrap()
    }

    #[test]
    fn library_never_when_no_direct_verifiers() {
        let lib = LibraryId::parse("libraries/rust/a.rs").unwrap();
        assert_eq!(
            classify_library_status(&lib, &[]),
            VerificationStatus::Never,
        );
    }

    #[test]
    fn library_verified_when_any_direct_verifier_passes() {
        let lib = LibraryId::parse("libraries/rust/a.rs").unwrap();
        let evidence = vec![
            (sol("abc999/a/main"), VerificationStatus::Rejected),
            (sol("abc999/b/main"), VerificationStatus::Verified),
            (sol("abc999/c/main"), VerificationStatus::Stale),
        ];
        assert_eq!(
            classify_library_status(&lib, &evidence),
            VerificationStatus::Verified,
        );
    }

    #[test]
    fn library_priority_order_when_no_success() {
        let lib = LibraryId::parse("libraries/rust/a.rs").unwrap();
        // InfrastructureError > Pending > Judging > Rejected > Unavailable > Stale
        let evidence = vec![
            (sol("abc999/a/main"), VerificationStatus::Stale),
            (sol("abc999/b/main"), VerificationStatus::Rejected),
            (
                sol("abc999/c/main"),
                VerificationStatus::InfrastructureError,
            ),
        ];
        assert_eq!(
            classify_library_status(&lib, &evidence),
            VerificationStatus::InfrastructureError,
        );

        let evidence = vec![
            (sol("abc999/a/main"), VerificationStatus::Stale),
            (sol("abc999/b/main"), VerificationStatus::Unavailable),
        ];
        assert_eq!(
            classify_library_status(&lib, &evidence),
            VerificationStatus::Unavailable,
        );
    }
}
