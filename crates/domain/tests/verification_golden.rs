//! Golden-fixture round-trip and semantic tests for verification records
//! (spec §8, §10, §11).

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::DateTime;

use domain::library::{LanguageId, LibraryId, SolutionId};
use domain::online_judge::{RecoveryMode, ResultDetail, SubmissionCapabilities, SubmissionMode};
use domain::verification::{
    AttemptId, CompletedState, ContentHash, ErrorKind, FailureStage, LanguageBinding, PendingState,
    PublicExtraValue, SubmissionHandle, SubmissionSummary, TestCaseResult, UnavailableReason,
    Verdict, VerdictKind, VerificationRecord, VerificationState, VerifyFingerprint,
};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("verification")
}

fn load_fixture(name: &str) -> (String, VerificationRecord) {
    let path = fixture_dir().join(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));
    let parsed: VerificationRecord = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("failed to parse fixture {}: {e}", path.display()));
    (raw, parsed)
}

// ── Golden round-trip ────────────────────────────────────────────────────────

#[test]
fn accepted_fixture_round_trips() {
    let (_, parsed) = load_fixture("accepted.json");
    let reserialized = serde_json::to_string(&parsed).unwrap();
    let reparsed: VerificationRecord = serde_json::from_str(&reserialized).unwrap();
    assert_eq!(reparsed, parsed);
}

#[test]
fn rejected_fixture_round_trips() {
    let (_, parsed) = load_fixture("rejected.json");
    let reserialized = serde_json::to_string(&parsed).unwrap();
    let reparsed: VerificationRecord = serde_json::from_str(&reserialized).unwrap();
    assert_eq!(reparsed, parsed);
}

#[test]
fn pending_fixture_round_trips() {
    let (_, parsed) = load_fixture("pending.json");
    let reserialized = serde_json::to_string(&parsed).unwrap();
    let reparsed: VerificationRecord = serde_json::from_str(&reserialized).unwrap();
    assert_eq!(reparsed, parsed);
}

#[test]
fn unavailable_fixture_round_trips() {
    let (_, parsed) = load_fixture("unavailable.json");
    let reserialized = serde_json::to_string(&parsed).unwrap();
    let reparsed: VerificationRecord = serde_json::from_str(&reserialized).unwrap();
    assert_eq!(reparsed, parsed);
}

#[test]
fn infrastructure_failure_fixture_round_trips() {
    let (_, parsed) = load_fixture("infrastructure-failure.json");
    let reserialized = serde_json::to_string(&parsed).unwrap();
    let reparsed: VerificationRecord = serde_json::from_str(&reserialized).unwrap();
    assert_eq!(reparsed, parsed);
}

// ── Per-state semantic checks ────────────────────────────────────────────────

#[test]
fn accepted_state_has_details_and_matches_recomputed_summary() {
    let (_, parsed) = load_fixture("accepted.json");
    let VerificationState::Completed(state) = &parsed.state else {
        panic!("expected completed state, got {:?}", parsed.state);
    };
    assert_eq!(state.verdict.kind, VerdictKind::Accepted);
    let cases = state
        .test_cases
        .as_ref()
        .expect("accepted fixture must carry test cases");
    assert!(!cases.is_empty());
    let recomputed = state.recomputed_summary();
    assert_eq!(recomputed, state.summary);
    assert_eq!(recomputed.max_execution_time_ms, Some(20));
    assert_eq!(recomputed.max_memory_bytes, Some(2048));
}

#[test]
fn rejected_state_lacks_case_detail_but_keeps_summary() {
    let (_, parsed) = load_fixture("rejected.json");
    let VerificationState::Completed(state) = &parsed.state else {
        panic!("expected completed state, got {:?}", parsed.state);
    };
    assert_eq!(state.verdict.kind, VerdictKind::WrongAnswer);
    assert!(state.test_cases.is_none());
    assert_eq!(state.summary.max_execution_time_ms, Some(42));
    assert_eq!(state.summary.max_memory_bytes, Some(4096));
}

#[test]
fn pending_state_parses_and_exposes_handle() {
    let (_, parsed) = load_fixture("pending.json");
    let VerificationState::Queued(state) = &parsed.state else {
        panic!("expected queued state, got {:?}", parsed.state);
    };
    assert_eq!(state.handle.oj, "librarychecker");
    assert_eq!(state.handle.submission_id, "12347");
    assert_eq!(state.handle.locator.as_deref(), Some("queue-position:3"));
}

#[test]
fn judging_variant_shares_pending_state_shape() {
    let (_, mut parsed) = load_fixture("pending.json");
    // Swap the tag and confirm both variants share `PendingState` semantics.
    if let VerificationState::Queued(inner) = parsed.state.clone() {
        parsed.state = VerificationState::Judging(inner);
    }
    let json = serde_json::to_string(&parsed).unwrap();
    assert!(json.contains("\"kind\":\"judging\""));
    let reparsed: VerificationRecord = serde_json::from_str(&json).unwrap();
    let VerificationState::Judging(state) = &reparsed.state else {
        panic!("expected judging state, got {:?}", reparsed.state);
    };
    assert_eq!(state.handle.submission_id, "12347");
}

#[test]
fn unavailable_state_reports_interactive_untrackable() {
    let (_, parsed) = load_fixture("unavailable.json");
    let VerificationState::Unavailable(state) = &parsed.state else {
        panic!("expected unavailable state, got {:?}", parsed.state);
    };
    assert_eq!(state.reason, UnavailableReason::InteractiveUntrackable);
    assert_eq!(
        state.capabilities.submission_mode,
        SubmissionMode::InteractiveUntrackable,
    );
    assert_eq!(state.capabilities.recovery_mode, RecoveryMode::None);
}

#[test]
fn infrastructure_failure_state_is_retryable_network_poll_error() {
    let (_, parsed) = load_fixture("infrastructure-failure.json");
    let VerificationState::InfrastructureFailure(state) = &parsed.state else {
        panic!("expected infrastructure failure, got {:?}", parsed.state);
    };
    assert!(state.retryable);
    assert_eq!(state.stage, FailureStage::Poll);
    assert_eq!(state.error_kind, ErrorKind::Network);
    assert!(state.next_retry_at.is_some());
    assert!(state.handle.is_some());
    assert!(state.plan_hash.is_some());
}

// ── Verdict raw preservation ─────────────────────────────────────────────────

#[test]
fn verdict_other_preserves_raw_string() {
    let (_, mut parsed) = load_fixture("accepted.json");
    if let VerificationState::Completed(state) = &mut parsed.state {
        state.verdict = Verdict {
            kind: VerdictKind::Other,
            raw: "Unknown Verdict String".into(),
        };
    }
    let json = serde_json::to_string(&parsed).unwrap();
    let reparsed: VerificationRecord = serde_json::from_str(&json).unwrap();
    let VerificationState::Completed(state) = &reparsed.state else {
        panic!("expected completed state");
    };
    assert_eq!(state.verdict.kind, VerdictKind::Other);
    assert_eq!(state.verdict.raw, "Unknown Verdict String");
}

// ── Optional list distinctions ───────────────────────────────────────────────

#[test]
fn empty_and_null_test_cases_are_distinct_across_round_trip() {
    let (_, base) = load_fixture("accepted.json");

    // Explicit empty list stays Some(vec![]).
    let VerificationState::Completed(mut with_empty) = base.state.clone() else {
        panic!("expected completed state");
    };
    with_empty.test_cases = Some(vec![]);
    let record_empty = VerificationRecord {
        state: VerificationState::Completed(with_empty),
        ..base.clone()
    };
    let serialised = serde_json::to_string(&record_empty).unwrap();
    assert!(serialised.contains("\"test_cases\":[]"));
    let reparsed: VerificationRecord = serde_json::from_str(&serialised).unwrap();
    let VerificationState::Completed(round) = &reparsed.state else {
        panic!("expected completed state");
    };
    assert_eq!(round.test_cases.as_deref(), Some(&[][..]));

    // `null` deserialises to None and stays None.
    let VerificationState::Completed(mut with_null) = base.state.clone() else {
        panic!("expected completed state");
    };
    with_null.test_cases = None;
    let record_null = VerificationRecord {
        state: VerificationState::Completed(with_null),
        ..base
    };
    let serialised = serde_json::to_string(&record_null).unwrap();
    assert!(serialised.contains("\"test_cases\":null"));
    let reparsed: VerificationRecord = serde_json::from_str(&serialised).unwrap();
    let VerificationState::Completed(round) = &reparsed.state else {
        panic!("expected completed state");
    };
    assert!(round.test_cases.is_none());
}

#[test]
fn none_and_some_zero_metrics_are_distinct() {
    let none = SubmissionSummary {
        max_execution_time_ms: None,
        max_memory_bytes: None,
    };
    let zero = SubmissionSummary {
        max_execution_time_ms: Some(0),
        max_memory_bytes: Some(0),
    };
    assert_ne!(none, zero);
    let none_round: SubmissionSummary =
        serde_json::from_str(&serde_json::to_string(&none).unwrap()).unwrap();
    let zero_round: SubmissionSummary =
        serde_json::from_str(&serde_json::to_string(&zero).unwrap()).unwrap();
    assert_eq!(none_round, none);
    assert_eq!(zero_round, zero);
    assert_ne!(none_round, zero_round);
}

// ── Capability combinations ──────────────────────────────────────────────────

#[test]
fn capability_combinations_round_trip_through_json() {
    let a = SubmissionCapabilities {
        submission_mode: SubmissionMode::UnattendedTrackable,
        result_detail: ResultDetail::TestcaseDetails,
        recovery_mode: RecoveryMode::Exact,
    };
    let b = SubmissionCapabilities {
        submission_mode: SubmissionMode::InteractiveUntrackable,
        result_detail: ResultDetail::OverallOnly,
        recovery_mode: RecoveryMode::None,
    };
    for cap in [a, b] {
        let json = serde_json::to_string(&cap).unwrap();
        let round: SubmissionCapabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(round, cap);
    }
}

// ── Public extra allowlist ───────────────────────────────────────────────────

#[test]
fn public_extra_allowlist_round_trips_supported_variants() {
    for value in [
        PublicExtraValue::String("compiled ok".into()),
        PublicExtraValue::Integer(-7),
        PublicExtraValue::Bool(true),
    ] {
        let json = serde_json::to_string(&value).unwrap();
        let round: PublicExtraValue = serde_json::from_str(&json).unwrap();
        assert_eq!(round, value);
    }
}

#[test]
fn public_extra_rejects_disallowed_kinds() {
    let bad = r#"{"kind":"array","data":[]}"#;
    assert!(serde_json::from_str::<PublicExtraValue>(bad).is_err());
    let bad_object = r#"{"kind":"object","data":{"a":1}}"#;
    assert!(serde_json::from_str::<PublicExtraValue>(bad_object).is_err());
}

// ── Newtype validation ──────────────────────────────────────────────────────

#[test]
fn attempt_id_rejects_empty_string() {
    assert!(AttemptId::parse("").is_err());
}

#[test]
fn content_hash_rejects_non_hex_suffix() {
    assert!(ContentHash::parse("sha256:not-hex").is_err());
}

#[test]
fn verify_fingerprint_rejects_wrong_algorithm_prefix() {
    assert!(VerifyFingerprint::parse("sha1:0000000000000000000000000000000000000000").is_err());
}

// ── Integration helper: build a full record programmatically ─────────────────

#[test]
fn record_can_be_built_from_scratch_and_round_trips() {
    let handle = SubmissionHandle {
        oj: "librarychecker".into(),
        submission_id: "999".into(),
        submission_url: "https://judge.yosupo.jp/submission/999".into(),
        locator: None,
        submitted_at: DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap(),
    };
    let state = VerificationState::Queued(PendingState {
        handle,
        observed_at: DateTime::parse_from_rfc3339("2026-08-10T09:00:01+00:00").unwrap(),
    });
    let record = VerificationRecord {
        schema_version: 1,
        solution_id: SolutionId::parse("librarychecker-aplusb/aplusb/main").unwrap(),
        attempt_id: AttemptId::parse("attempt-999").unwrap(),
        replaces_attempt_id: None,
        fingerprint: VerifyFingerprint::parse(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap(),
        state,
        plan_context: None,
    };
    let json = serde_json::to_string(&record).unwrap();
    let round: VerificationRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(round, record);
}

#[test]
fn completed_state_input_hashes_are_ordered_by_path() {
    let (_, parsed) = load_fixture("accepted.json");
    let VerificationState::Completed(state) = &parsed.state else {
        panic!("expected completed state");
    };
    let keys: Vec<&str> = state.input_hashes.keys().map(|s| s.as_str()).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
}

// Silence unused-import warnings on optional helpers so the file stays lean.
#[allow(dead_code)]
fn _touch_unused(
    _: LibraryId,
    _: LanguageId,
    _: LanguageBinding,
    _: TestCaseResult,
    _: BTreeMap<String, ContentHash>,
    _: CompletedState,
) {
}
