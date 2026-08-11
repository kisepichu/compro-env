//! Integration tests for the submission lifecycle ports.
//!
//! These cover the contract that the future `ce verify` and `ce submit --watch`
//! share with today's `ce submit`: serializable handles, all start/poll/recovery
//! outcomes, sanitized errors, capability consistency, and the
//! confirmed-not-accepted vs. acceptance-unknown boundary.
//!
//! Behaviour lives in the port trait + registries; adapter migration is Task 2.

use chrono::{TimeZone, Utc};
use domain::entity::{OJKind, Session};
use usecases::submission::{
    JudgeResult, JudgeVerdict, PollObservation, PollSubmissionError, PollerRegistry,
    RecoverSubmissionError, RecoveryOutcome, RecoveryRegistry, RecoveryRequest, ResultDetailLevel,
    StartSubmissionError, StarterRegistry, SubmissionAdapterDescriptor, SubmissionHandle,
    SubmissionMode, SubmissionPoller, SubmissionRecovery, SubmissionRequest, SubmissionStart,
    SubmissionStarter, TestcaseOutcome, UnavailableReason,
};

// ── Handles ─────────────────────────────────────────────────────────────────

/// A `SubmissionHandle` round-trips through JSON so resume across processes is safe.
#[test]
fn submission_handle_json_round_trip() {
    let handle = SubmissionHandle {
        online_judge: OJKind::LibraryChecker,
        submission_id: "42".to_string(),
        submission_url: "https://judge.yosupo.jp/submission/42".to_string(),
        locator: Some("aplusb:cpp:abc123".to_string()),
        submitted_at: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
    };
    let json = serde_json::to_string(&handle).expect("handle serializes");
    let decoded: SubmissionHandle = serde_json::from_str(&json).expect("handle deserializes");
    assert_eq!(handle, decoded);
}

/// `SubmissionHandle` refuses to deserialize without required fields, so a stored
/// handle cannot silently drop the OJ or submission id.
#[test]
fn submission_handle_requires_core_fields() {
    let missing_oj =
        r#"{"submission_id":"1","submission_url":"u","submitted_at":"2026-01-02T03:04:05Z"}"#;
    assert!(serde_json::from_str::<SubmissionHandle>(missing_oj).is_err());
    let missing_id =
        r#"{"online_judge":"AtCoder","submission_url":"u","submitted_at":"2026-01-02T03:04:05Z"}"#;
    assert!(serde_json::from_str::<SubmissionHandle>(missing_id).is_err());
}

// ── Capability descriptors ─────────────────────────────────────────────────

/// Adapter descriptors document their submission mode + recovery mode + detail
/// level as a single value so registries can filter callers by capability.
#[test]
fn descriptor_captures_declared_capabilities() {
    let d = SubmissionAdapterDescriptor {
        name: "librarychecker".to_string(),
        version: "1".to_string(),
        submission_mode: SubmissionMode::UnattendedTrackable,
        result_detail: ResultDetailLevel::TestcaseDetails,
        recovery_mode: usecases::submission::RecoveryMode::BestEffort,
    };
    assert!(d.supports_unattended_verify());
    let interactive = SubmissionAdapterDescriptor {
        name: "atcoder".to_string(),
        version: "1".to_string(),
        submission_mode: SubmissionMode::InteractiveUntrackable,
        result_detail: ResultDetailLevel::OverallOnly,
        recovery_mode: usecases::submission::RecoveryMode::None,
    };
    assert!(!interactive.supports_unattended_verify());
}

// ── Adapters ────────────────────────────────────────────────────────────────

struct StubStarter {
    descriptor: SubmissionAdapterDescriptor,
    outcome: SubmissionStart,
}
impl SubmissionStarter for StubStarter {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        self.descriptor.clone()
    }
    fn start_submission(
        &self,
        _request: &SubmissionRequest,
        _session: Option<&Session>,
    ) -> Result<SubmissionStart, StartSubmissionError> {
        Ok(self.outcome.clone())
    }
}

struct FailingStarter {
    descriptor: SubmissionAdapterDescriptor,
    error: StartSubmissionError,
}
impl SubmissionStarter for FailingStarter {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        self.descriptor.clone()
    }
    fn start_submission(
        &self,
        _request: &SubmissionRequest,
        _session: Option<&Session>,
    ) -> Result<SubmissionStart, StartSubmissionError> {
        Err(self.error.clone())
    }
}

struct StubPoller {
    descriptor: SubmissionAdapterDescriptor,
    observation: PollObservation,
}
impl SubmissionPoller for StubPoller {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        self.descriptor.clone()
    }
    fn poll_submission(
        &self,
        _handle: &SubmissionHandle,
        _session: Option<&Session>,
    ) -> Result<PollObservation, PollSubmissionError> {
        Ok(self.observation.clone())
    }
}

struct StubRecovery {
    descriptor: SubmissionAdapterDescriptor,
    outcome: RecoveryOutcome,
}
impl SubmissionRecovery for StubRecovery {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        self.descriptor.clone()
    }
    fn recover_submission(
        &self,
        _request: &RecoveryRequest,
        _session: Option<&Session>,
    ) -> Result<RecoveryOutcome, RecoverSubmissionError> {
        Ok(self.outcome.clone())
    }
}

fn sample_request() -> SubmissionRequest {
    SubmissionRequest {
        online_judge: OJKind::LibraryChecker,
        contest_id: "librarychecker-aplusb".to_string(),
        problem_id: "aplusb".to_string(),
        lang_id: "cpp".to_string(),
        source: "int main(){}".to_string(),
    }
}

fn sample_handle() -> SubmissionHandle {
    SubmissionHandle {
        online_judge: OJKind::LibraryChecker,
        submission_id: "42".to_string(),
        submission_url: "https://judge.yosupo.jp/submission/42".to_string(),
        locator: None,
        submitted_at: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
    }
}

fn lc_descriptor() -> SubmissionAdapterDescriptor {
    SubmissionAdapterDescriptor {
        name: "librarychecker".to_string(),
        version: "1".to_string(),
        submission_mode: SubmissionMode::UnattendedTrackable,
        result_detail: ResultDetailLevel::TestcaseDetails,
        recovery_mode: usecases::submission::RecoveryMode::BestEffort,
    }
}

fn atcoder_descriptor() -> SubmissionAdapterDescriptor {
    SubmissionAdapterDescriptor {
        name: "atcoder".to_string(),
        version: "1".to_string(),
        submission_mode: SubmissionMode::InteractiveUntrackable,
        result_detail: ResultDetailLevel::OverallOnly,
        recovery_mode: usecases::submission::RecoveryMode::None,
    }
}

// ── SubmissionStart outcomes ────────────────────────────────────────────────

#[test]
fn start_returns_trackable_handle_for_unattended_adapter() {
    let starter = StubStarter {
        descriptor: lc_descriptor(),
        outcome: SubmissionStart::Trackable {
            handle: sample_handle(),
        },
    };
    let start = starter
        .start_submission(&sample_request(), None)
        .expect("start");
    match start {
        SubmissionStart::Trackable { handle } => {
            assert_eq!(handle.online_judge, OJKind::LibraryChecker);
        }
        other => panic!("expected Trackable, got {other:?}"),
    }
}

#[test]
fn start_returns_user_action_required_for_interactive_untrackable_adapter() {
    let starter = StubStarter {
        descriptor: atcoder_descriptor(),
        outcome: SubmissionStart::UserActionRequired {
            url: "https://atcoder.jp/contests/abc001/submit#ce=X".to_string(),
        },
    };
    let start = starter
        .start_submission(&sample_request(), None)
        .expect("start");
    match start {
        SubmissionStart::UserActionRequired { url } => {
            assert!(url.starts_with("https://atcoder.jp"));
        }
        other => panic!("expected UserActionRequired, got {other:?}"),
    }
}

#[test]
fn start_returns_unavailable_when_adapter_cannot_serve_request() {
    let starter = StubStarter {
        descriptor: atcoder_descriptor(),
        outcome: SubmissionStart::Unavailable {
            reason: UnavailableReason::InteractiveUntrackable,
        },
    };
    let start = starter
        .start_submission(&sample_request(), None)
        .expect("start");
    assert!(matches!(start, SubmissionStart::Unavailable { .. }));
}

// ── PollObservation outcomes ────────────────────────────────────────────────

#[test]
fn poll_reports_queued_judging_and_completed() {
    let handle = sample_handle();
    let queued = StubPoller {
        descriptor: lc_descriptor(),
        observation: PollObservation::Queued,
    };
    assert!(matches!(
        queued.poll_submission(&handle, None).unwrap(),
        PollObservation::Queued
    ));

    let judging = StubPoller {
        descriptor: lc_descriptor(),
        observation: PollObservation::Judging,
    };
    assert!(matches!(
        judging.poll_submission(&handle, None).unwrap(),
        PollObservation::Judging
    ));

    let completed = StubPoller {
        descriptor: lc_descriptor(),
        observation: PollObservation::Completed(JudgeResult {
            verdict: JudgeVerdict::Accepted,
            testcases: vec![TestcaseOutcome {
                name: "example_00".to_string(),
                verdict: JudgeVerdict::Accepted,
                time_ms: Some(3),
                memory_kib: Some(1024),
            }],
        }),
    };
    match completed.poll_submission(&handle, None).unwrap() {
        PollObservation::Completed(result) => {
            assert_eq!(result.verdict, JudgeVerdict::Accepted);
            assert_eq!(result.testcases.len(), 1);
        }
        other => panic!("expected Completed, got {other:?}"),
    }
}

// ── RecoveryOutcome — confirmed-not-accepted vs acceptance-unknown ─────────

/// `Recovered` returns a fresh handle that the caller can hand back to the poller.
#[test]
fn recover_returns_recovered_handle_when_uniquely_identified() {
    let recovery = StubRecovery {
        descriptor: lc_descriptor(),
        outcome: RecoveryOutcome::Recovered {
            handle: sample_handle(),
        },
    };
    let out = recovery
        .recover_submission(&RecoveryRequest::from_request(&sample_request()), None)
        .expect("recovery ok");
    assert!(matches!(out, RecoveryOutcome::Recovered { .. }));
}

/// `ConfirmedNotAccepted` proves the OJ never received the attempt so it is safe
/// to discard the `Starting` and re-plan on the next tick.
#[test]
fn recover_confirmed_not_accepted_is_safe_to_discard() {
    let recovery = StubRecovery {
        descriptor: lc_descriptor(),
        outcome: RecoveryOutcome::ConfirmedNotAccepted,
    };
    let out = recovery
        .recover_submission(&RecoveryRequest::from_request(&sample_request()), None)
        .expect("recovery ok");
    assert!(
        out.is_safe_to_discard_attempt(),
        "ConfirmedNotAccepted must allow discarding the attempt"
    );
}

/// `AcceptanceUnknown` never permits automatic retry — the attempt keeps its
/// `Starting`/handle-less state until an operator decides.
#[test]
fn recover_acceptance_unknown_blocks_safe_discard() {
    let recovery = StubRecovery {
        descriptor: lc_descriptor(),
        outcome: RecoveryOutcome::AcceptanceUnknown,
    };
    let out = recovery
        .recover_submission(&RecoveryRequest::from_request(&sample_request()), None)
        .expect("recovery ok");
    assert!(
        !out.is_safe_to_discard_attempt(),
        "AcceptanceUnknown must NOT allow discarding — that would risk double submission"
    );
}

/// `Unsupported` is returned by adapters whose recovery_mode is `None`.
#[test]
fn recover_unsupported_from_no_recovery_adapters() {
    let recovery = StubRecovery {
        descriptor: atcoder_descriptor(),
        outcome: RecoveryOutcome::Unsupported,
    };
    let out = recovery
        .recover_submission(&RecoveryRequest::from_request(&sample_request()), None)
        .expect("recovery ok");
    assert!(!out.is_safe_to_discard_attempt());
}

// ── StartSubmissionError — sanitized + acceptance-unknown boundary ──────────

/// The `AcceptanceUnknown` start error must NEVER expose credential/cookie/token
/// substrings — the caller writes it to a draft PR summary.
#[test]
fn start_acceptance_unknown_error_is_sanitized() {
    let err = StartSubmissionError::AcceptanceUnknown {
        summary: "connection reset while awaiting submit response".to_string(),
    };
    assert!(!err.is_safe_to_retry());
    assert!(!err.summary().to_lowercase().contains("cookie"));
    assert!(!err.summary().to_lowercase().contains("bearer"));
    assert!(!err.summary().to_lowercase().contains("password"));
}

/// The `ConfirmedNotAccepted` start error IS safe to treat as pre-acceptance and
/// re-plan the attempt on the next tick.
#[test]
fn start_confirmed_not_accepted_is_safe_to_retry() {
    let err = StartSubmissionError::ConfirmedNotAccepted {
        summary: "OJ returned 400 for invalid payload before accepting".to_string(),
    };
    assert!(err.is_safe_to_retry());
}

/// A transport failure that could have been transmitted MUST be reported as
/// `AcceptanceUnknown`, never as a retryable error — this is the safety invariant
/// spec §8.2 pins down.
#[test]
fn transport_failure_after_send_is_acceptance_unknown() {
    let err = StartSubmissionError::from_transport_after_send("write failed after headers sent");
    match err {
        StartSubmissionError::AcceptanceUnknown { .. } => {}
        other => {
            panic!("transport failure after send must map to AcceptanceUnknown, got: {other:?}")
        }
    }
}

/// The `Sanitized` constructor strips credential-like substrings automatically,
/// so adapters that forget to sanitize cannot leak by accident. The dummy
/// values here (`TOKEN123`, `RS_abc123`, `hunter2`) are deliberately obvious
/// non-secrets that still exercise the same code path.
#[test]
fn sanitized_summary_strips_credential_substrings() {
    let raw = "auth failed: Bearer TOKEN123; cookie=REVEL_SESSION=RS_abc123; password=hunter2";
    let summary = usecases::submission::sanitize_summary(raw);
    let lower = summary.to_lowercase();
    // Keywords are scrubbed:
    assert!(
        !lower.contains("bearer"),
        "summary still contains 'bearer': {summary}"
    );
    assert!(
        !lower.contains("revel_session"),
        "summary still contains 'revel_session': {summary}"
    );
    assert!(
        !lower.contains("password"),
        "summary still contains 'password': {summary}"
    );
    // AND the value fragments trailing each keyword are scrubbed:
    assert!(
        !summary.contains("TOKEN123"),
        "summary still leaks Bearer value: {summary}"
    );
    assert!(
        !summary.contains("RS_abc123"),
        "summary still leaks cookie value: {summary}"
    );
    assert!(
        !summary.contains("hunter2"),
        "summary still leaks password value: {summary}"
    );
}

/// Regression: `sanitize_summary` must never panic on multi-byte UTF-8 input.
/// An earlier version indexed the full-Unicode-lowercased string by input byte
/// offsets, which panics whenever `to_lowercase()` changed the string's length.
#[test]
fn sanitize_summary_handles_multibyte_utf8_without_panic() {
    // Non-ASCII text bracketing a keyword-value pair.
    let raw = "認証エラー: Bearer TOKEN123 が拒否されました 🚫";
    let summary = usecases::submission::sanitize_summary(raw);
    assert!(!summary.to_lowercase().contains("bearer"));
    assert!(!summary.contains("TOKEN123"));
    // Surrounding non-ASCII is preserved verbatim.
    assert!(summary.contains("認証エラー"));
    assert!(summary.contains("拒否されました"));
    assert!(summary.contains("🚫"));
}

// ── Registries and capability consistency ──────────────────────────────────

/// A starter registry can register per-OJ starters and return them by kind.
#[test]
fn starter_registry_dispatches_by_oj_kind() {
    let mut reg = StarterRegistry::new();
    reg.register(
        OJKind::LibraryChecker,
        Box::new(StubStarter {
            descriptor: lc_descriptor(),
            outcome: SubmissionStart::Trackable {
                handle: sample_handle(),
            },
        }),
    );
    let starter = reg.get(&OJKind::LibraryChecker).expect("registered");
    let start = starter
        .start_submission(&sample_request(), None)
        .expect("start ok");
    assert!(matches!(start, SubmissionStart::Trackable { .. }));

    let missing = reg.get(&OJKind::AtCoder);
    assert!(missing.is_err(), "unknown OJ should fail lookup");
}

/// Registering an `unattended_trackable` starter but a recovery adapter that
/// declares `recovery_mode = None` is rejected: the pair would leak
/// `AcceptanceUnknown` attempts forever with no way to recover.
#[test]
fn registries_reject_capability_mismatch_between_starter_and_recovery() {
    let mut starters = StarterRegistry::new();
    starters.register(
        OJKind::LibraryChecker,
        Box::new(StubStarter {
            descriptor: lc_descriptor(),
            outcome: SubmissionStart::Trackable {
                handle: sample_handle(),
            },
        }),
    );
    let mut pollers = PollerRegistry::new();
    pollers.register(
        OJKind::LibraryChecker,
        Box::new(StubPoller {
            descriptor: lc_descriptor(),
            observation: PollObservation::Queued,
        }),
    );
    let mut recovery = RecoveryRegistry::new();
    recovery.register(
        OJKind::LibraryChecker,
        Box::new(StubRecovery {
            descriptor: SubmissionAdapterDescriptor {
                name: "librarychecker".to_string(),
                version: "1".to_string(),
                // Mismatched capabilities: unattended_trackable but recovery None.
                submission_mode: SubmissionMode::UnattendedTrackable,
                result_detail: ResultDetailLevel::TestcaseDetails,
                recovery_mode: usecases::submission::RecoveryMode::None,
            },
            outcome: RecoveryOutcome::Unsupported,
        }),
    );
    let err = usecases::submission::verify_registry_consistency(
        &starters,
        Some(&pollers),
        Some(&recovery),
    )
    .expect_err("mismatch should fail verification");
    assert!(
        err.to_string().to_lowercase().contains("recovery"),
        "error should mention recovery mismatch: {err}"
    );
}

/// A well-configured trio (starter + poller + recovery) with consistent
/// capabilities passes verification.
#[test]
fn registries_accept_consistent_capabilities() {
    let mut starters = StarterRegistry::new();
    starters.register(
        OJKind::LibraryChecker,
        Box::new(StubStarter {
            descriptor: lc_descriptor(),
            outcome: SubmissionStart::Trackable {
                handle: sample_handle(),
            },
        }),
    );
    let mut pollers = PollerRegistry::new();
    pollers.register(
        OJKind::LibraryChecker,
        Box::new(StubPoller {
            descriptor: lc_descriptor(),
            observation: PollObservation::Queued,
        }),
    );
    let mut recovery = RecoveryRegistry::new();
    recovery.register(
        OJKind::LibraryChecker,
        Box::new(StubRecovery {
            descriptor: lc_descriptor(),
            outcome: RecoveryOutcome::AcceptanceUnknown,
        }),
    );
    usecases::submission::verify_registry_consistency(&starters, Some(&pollers), Some(&recovery))
        .expect("consistent registries verify");
}

/// AtCoder registers `interactive_untrackable`: only a starter is legal, and it
/// need not (indeed, must not) come with a poller.
#[test]
fn interactive_untrackable_starter_requires_no_poller() {
    let mut starters = StarterRegistry::new();
    starters.register(
        OJKind::AtCoder,
        Box::new(StubStarter {
            descriptor: atcoder_descriptor(),
            outcome: SubmissionStart::UserActionRequired {
                url: "https://atcoder.jp".to_string(),
            },
        }),
    );
    let pollers = PollerRegistry::new();
    let recovery = RecoveryRegistry::new();
    usecases::submission::verify_registry_consistency(&starters, Some(&pollers), Some(&recovery))
        .expect("interactive_untrackable needs neither poller nor recovery");
}

/// Registering an `unattended_trackable` starter without a poller is a
/// misconfiguration: verify would have no way to observe the result.
#[test]
fn registries_reject_trackable_starter_without_poller() {
    let mut starters = StarterRegistry::new();
    starters.register(
        OJKind::LibraryChecker,
        Box::new(StubStarter {
            descriptor: lc_descriptor(),
            outcome: SubmissionStart::Trackable {
                handle: sample_handle(),
            },
        }),
    );
    let pollers = PollerRegistry::new();
    let err = usecases::submission::verify_registry_consistency(&starters, Some(&pollers), None)
        .expect_err("missing poller should fail verification");
    assert!(err.to_string().to_lowercase().contains("poll"));
}

// ── FailingStarter path (unused paths verify) ──────────────────────────────

#[test]
fn failing_starter_propagates_error() {
    let starter = FailingStarter {
        descriptor: lc_descriptor(),
        error: StartSubmissionError::AcceptanceUnknown {
            summary: "socket closed".to_string(),
        },
    };
    let err = starter
        .start_submission(&sample_request(), None)
        .expect_err("should fail");
    assert!(matches!(
        err,
        StartSubmissionError::AcceptanceUnknown { .. }
    ));
}
