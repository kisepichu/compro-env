//! Deterministic next-candidate selection for scheduled verify runs (spec §15).
//!
//! [`select_next_candidate`] is a pure function that consumes the current
//! publication set, the persisted latest-record map from `automation/verify`,
//! and the freshly-computed fingerprints of every currently-`Completed`
//! solution. It returns at most one [`SolutionId`] per invocation so
//! `verify.yml`'s dispatcher can hand it to the worker as the `solution`
//! input. When no eligible solution exists the caller must translate the
//! `None` into `run_worker=false`.
//!
//! Eligibility rules — see the caller contract on
//! [`select_next_candidate`] for the details — are:
//! * no record for the solution yet;
//! * the latest record is an `InfrastructureFailure` whose retry deadline has
//!   elapsed (including the `None` "unscheduled" case that persisters emit
//!   today);
//! * the latest record is `Completed` but its `fingerprint` disagrees with
//!   the recomputed fingerprint (input drift).
//!
//! All other states — the five in-flight variants and terminal `Unavailable`
//! — are excluded. Non-retryable `InfrastructureFailure` records are also
//! excluded permanently.

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
/// * `records` maps every `SolutionId` for which the overlay branch
///   (`automation/verify`) has a latest record. Solutions absent from the map
///   are treated as "never verified" and are always eligible.
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

    // Order: retry-ready records with the earliest deadline first (with
    // `None` deadline sorting ahead of any scheduled retry), then all other
    // eligible candidates. Within a bucket, tie-break by the raw
    // `SolutionId` bytes so scheduling stays deterministic even when the
    // discovery layer returns solutions in a different order.
    eligible.sort();

    eligible.into_iter().next().map(|c| c.id.clone())
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Bucket {
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
            ..
        }) => match next_retry_at {
            Some(deadline) if *deadline > now => None,
            Some(deadline) => Some(Candidate {
                bucket: Bucket::RetryReady,
                deadline: *deadline,
                id,
            }),
            None => Some(Candidate {
                bucket: Bucket::RetryReady,
                deadline: min_datetime(),
                id,
            }),
        },
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
        // Terminal `Unavailable` and the five in-flight variants (Starting,
        // AcceptanceUnknown, Submitted, Queued, Judging) all fall through
        // here. Fail closed: any future non-terminal, non-`InfrastructureFailure`
        // variant introduced by later plans stays excluded until this arm
        // is updated.
        VerificationState::Unavailable(_)
        | VerificationState::Starting(_)
        | VerificationState::AcceptanceUnknown(_)
        | VerificationState::Submitted(_)
        | VerificationState::Queued(_)
        | VerificationState::Judging(_) => None,
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

    #[test]
    fn in_flight_variants_are_all_excluded() {
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
            let picked = select_next_candidate(now(), &[sol], &records, &fingerprints);
            assert_eq!(
                picked, None,
                "in-flight variant `{label}` must not be picked"
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
