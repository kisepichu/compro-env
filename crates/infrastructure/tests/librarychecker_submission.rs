//! Fixture-based integration tests for `LibraryCheckerPoller` and
//! `LibraryCheckerRecovery`.
//!
//! Each test spins a local `tiny_http` server so no real network is needed.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use domain::entity::{OJKind, Session};
use infrastructure::online_judge_impl::librarychecker::submission::{
    LibraryCheckerPoller, LibraryCheckerRecovery,
};
use infrastructure::submission_impl::poller::build_poller_registry;
use infrastructure::submission_impl::recovery::build_recovery_registry;
use tiny_http::{Header, Response, Server};
use usecases::submission::{
    InfrastructureErrorKind, JudgeVerdict, PollObservation, PollSubmissionError,
    RecoverSubmissionError, RecoveryOutcome, RecoveryRequest, SubmissionHandle, SubmissionPoller,
    SubmissionRecovery, SubmissionRequest,
};

// ─── Fixture server ──────────────────────────────────────────────────────────

struct FixtureServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl FixtureServer {
    fn start<F>(handler: F) -> Self
    where
        F: FnMut(tiny_http::Request) + Send + 'static,
    {
        let server = Server::http("127.0.0.1:0").expect("bind test server");
        let addr = server.server_addr().to_ip().expect("ip addr");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let mut handler = handler;
        let handle = thread::spawn(move || {
            while !stop_thread.load(Ordering::SeqCst) {
                match server.recv_timeout(Duration::from_millis(100)) {
                    Ok(Some(request)) => handler(request),
                    Ok(None) => continue,
                    Err(_) => break,
                }
            }
        });
        Self {
            addr,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn test_handle(id: &str) -> SubmissionHandle {
    SubmissionHandle {
        online_judge: OJKind::LibraryChecker,
        submission_id: id.to_string(),
        submission_url: format!("https://judge.yosupo.jp/submission/{id}"),
        locator: None,
        submitted_at: Utc::now(),
    }
}

/// Minimal valid `SubmissionInfoResponse` JSON with the given status and no
/// case results.
fn info_json(status: &str) -> String {
    format!(
        r#"{{"overview":{{"id":9999,"problem_name":"aplusb","lang":"rust","is_latest":true,"status":"{status}","time":0.0,"memory":0}},"source":"","can_rejudge":false}}"#
    )
}

/// `SubmissionInfoResponse` JSON with case results.
fn info_json_with_cases(status: &str, cases: &str) -> String {
    format!(
        r#"{{"overview":{{"id":9999,"problem_name":"aplusb","lang":"rust","is_latest":true,"status":"{status}","time":0.0,"memory":0}},"source":"","can_rejudge":false,"case_results":{cases}}}"#
    )
}

fn poller_for(server: &FixtureServer) -> LibraryCheckerPoller {
    LibraryCheckerPoller::with_base_url(format!("http://{}", server.addr))
        .expect("poller constructs")
}

fn serve_body(body: String) -> impl FnMut(tiny_http::Request) {
    move |req| {
        let _ = req.respond(Response::from_data(body.as_bytes().to_vec()));
    }
}

// ─── Tests 1-2: pending states ───────────────────────────────────────────────

#[test]
fn poll_queued_maps_to_queued() {
    let body = include_str!("fixtures/librarychecker/submission-pending.json").to_string();
    let server = FixtureServer::start(serve_body(body));
    let obs = poller_for(&server)
        .poll_submission(&test_handle("1236"), None)
        .expect("poll ok");
    assert!(matches!(obs, PollObservation::Queued), "{obs:?}");
}

#[test]
fn poll_judging_maps_to_judging() {
    let body = info_json("Judging");
    let server = FixtureServer::start(serve_body(body));
    let obs = poller_for(&server)
        .poll_submission(&test_handle("1"), None)
        .expect("poll ok");
    assert!(matches!(obs, PollObservation::Judging), "{obs:?}");
}

// ─── Test 3: accepted with testcase metrics ───────────────────────────────────

#[test]
fn poll_accepted_maps_verdict_and_testcases() {
    let body = include_str!("fixtures/librarychecker/submission-accepted.json").to_string();
    let server = FixtureServer::start(serve_body(body));
    let obs = poller_for(&server)
        .poll_submission(&test_handle("1234"), None)
        .expect("poll ok");
    let result = match obs {
        PollObservation::Completed(r) => r,
        other => panic!("expected Completed, got {other:?}"),
    };
    assert_eq!(result.verdict, JudgeVerdict::Accepted);
    assert_eq!(result.testcases.len(), 3);
    // time in fixture is seconds; memory is bytes
    let t0 = &result.testcases[0];
    assert_eq!(t0.name, "example_00");
    assert_eq!(t0.verdict, JudgeVerdict::Accepted);
    assert_eq!(t0.time_ms, Some(4000)); // 4.0 s * 1000
    assert_eq!(t0.memory_kib, Some(2)); // 2048 bytes / 1024
    let t2 = &result.testcases[2];
    assert_eq!(t2.name, "random_00");
    assert_eq!(t2.time_ms, Some(12500)); // 12.5 s * 1000
    assert_eq!(t2.memory_kib, Some(4)); // 4096 bytes / 1024
}

// ─── Test 4: each known verdict ───────────────────────────────────────────────

#[test]
fn poll_maps_each_known_verdict() {
    let pairs: &[(&str, JudgeVerdict)] = &[
        ("WA", JudgeVerdict::WrongAnswer),
        ("TLE", JudgeVerdict::TimeLimitExceeded),
        ("MLE", JudgeVerdict::MemoryLimitExceeded),
        ("RE", JudgeVerdict::RuntimeError),
        ("CE", JudgeVerdict::CompilationError),
        ("IE", JudgeVerdict::InternalError),
    ];
    for (status_str, expected) in pairs {
        let body = info_json(status_str);
        let server = FixtureServer::start(serve_body(body));
        let obs = poller_for(&server)
            .poll_submission(&test_handle("1"), None)
            .unwrap_or_else(|e| panic!("poll failed for {status_str}: {e:?}"));
        match obs {
            PollObservation::Completed(r) => {
                assert_eq!(
                    r.verdict, *expected,
                    "verdict mismatch for status={status_str}"
                );
            }
            other => panic!("expected Completed for status={status_str}, got {other:?}"),
        }
    }
}

// ─── Test 5: unknown verdict becomes Other ────────────────────────────────────

#[test]
fn poll_unknown_verdict_becomes_other() {
    let body = include_str!("fixtures/librarychecker/submission-unknown-verdict.json").to_string();
    let server = FixtureServer::start(serve_body(body));
    let obs = poller_for(&server)
        .poll_submission(&test_handle("1238"), None)
        .expect("poll ok");
    match obs {
        PollObservation::Completed(r) => {
            assert_eq!(r.verdict, JudgeVerdict::Other("SomethingWeird".to_string()));
            assert!(r.testcases.is_empty());
        }
        other => panic!("expected Completed, got {other:?}"),
    }
}

// ─── Test 6: rejected with per-case metrics ───────────────────────────────────

#[test]
fn poll_rejected_maps_wrong_answer_with_metrics() {
    let body = include_str!("fixtures/librarychecker/submission-rejected.json").to_string();
    let server = FixtureServer::start(serve_body(body));
    let obs = poller_for(&server)
        .poll_submission(&test_handle("1237"), None)
        .expect("poll ok");
    let result = match obs {
        PollObservation::Completed(r) => r,
        other => panic!("expected Completed, got {other:?}"),
    };
    assert_eq!(result.verdict, JudgeVerdict::WrongAnswer);
    assert_eq!(result.testcases.len(), 2);
    let t0 = &result.testcases[0];
    assert_eq!(t0.verdict, JudgeVerdict::Accepted);
    assert_eq!(t0.time_ms, Some(10000)); // 10.0 s
    assert_eq!(t0.memory_kib, Some(4)); // 4096 bytes
    let t1 = &result.testcases[1];
    assert_eq!(t1.verdict, JudgeVerdict::WrongAnswer);
    assert_eq!(t1.time_ms, Some(15000)); // 15.0 s
    assert_eq!(t1.memory_kib, Some(8)); // 8192 bytes
}

// ─── Test 7: null case_results → empty vec ────────────────────────────────────

#[test]
fn poll_null_case_results_yields_empty_vec() {
    // Body has no `case_results` key (same as pending fixture).
    let body = info_json("AC");
    let server = FixtureServer::start(serve_body(body));
    let obs = poller_for(&server)
        .poll_submission(&test_handle("1"), None)
        .expect("poll ok");
    match obs {
        PollObservation::Completed(r) => assert!(r.testcases.is_empty()),
        other => panic!("expected Completed, got {other:?}"),
    }
}

// ─── Test 8: negative metrics → None ─────────────────────────────────────────

#[test]
fn poll_negative_metrics_become_none() {
    let cases = r#"[{"case":"t","status":"AC","time":-1.0,"memory":-1}]"#;
    let body = info_json_with_cases("AC", cases);
    let server = FixtureServer::start(serve_body(body));
    let obs = poller_for(&server)
        .poll_submission(&test_handle("1"), None)
        .expect("poll ok");
    let tc = match obs {
        PollObservation::Completed(r) => r.testcases.into_iter().next().expect("one testcase"),
        other => panic!("expected Completed, got {other:?}"),
    };
    assert!(
        tc.time_ms.is_none(),
        "expected None for negative time, got {:?}",
        tc.time_ms
    );
    assert!(
        tc.memory_kib.is_none(),
        "expected None for negative memory, got {:?}",
        tc.memory_kib
    );
}

// ─── Test 9: 429 with Retry-After ────────────────────────────────────────────

#[test]
fn poll_retry_after_included_in_rate_limited_summary() {
    let server = FixtureServer::start(|req| {
        let _ = req.respond(
            Response::empty(429u16).with_header(Header::from_bytes(b"Retry-After", b"42").unwrap()),
        );
    });
    let err = poller_for(&server)
        .poll_submission(&test_handle("1"), None)
        .expect_err("should fail");
    match err {
        PollSubmissionError::Infrastructure { kind, summary } => {
            assert_eq!(kind, InfrastructureErrorKind::RateLimited);
            assert!(
                summary.contains("42"),
                "summary should include retry-after secs: {summary}"
            );
        }
        other => panic!("expected Infrastructure, got {other:?}"),
    }
}

// ─── Test 10: 5xx → ServiceUnavailable ───────────────────────────────────────

#[test]
fn poll_5xx_becomes_service_unavailable() {
    let server = FixtureServer::start(|req| {
        let _ = req.respond(Response::empty(503u16));
    });
    let err = poller_for(&server)
        .poll_submission(&test_handle("1"), None)
        .expect_err("should fail");
    match err {
        PollSubmissionError::Infrastructure { kind, summary } => {
            assert_eq!(kind, InfrastructureErrorKind::ServiceUnavailable);
            assert!(
                summary.contains("503"),
                "summary should contain status code: {summary}"
            );
        }
        other => panic!("expected Infrastructure, got {other:?}"),
    }
}

// ─── Test 11: 404 → HandleNotFound ───────────────────────────────────────────

#[test]
fn poll_404_maps_to_handle_not_found() {
    let server = FixtureServer::start(|req| {
        let _ = req.respond(Response::empty(404u16));
    });
    let err = poller_for(&server)
        .poll_submission(&test_handle("9999"), None)
        .expect_err("should fail");
    match err {
        PollSubmissionError::HandleNotFound { summary } => {
            assert!(
                summary.contains("9999"),
                "summary should include submission id: {summary}"
            );
        }
        other => panic!("expected HandleNotFound, got {other:?}"),
    }
}

// ─── Test 12: malformed JSON → SchemaError ────────────────────────────────────

#[test]
fn poll_malformed_json_is_schema_error() {
    let server = FixtureServer::start(|req| {
        let _ = req.respond(Response::from_data(b"not json{".to_vec()));
    });
    let err = poller_for(&server)
        .poll_submission(&test_handle("1"), None)
        .expect_err("should fail");
    match err {
        PollSubmissionError::Infrastructure { kind, .. } => {
            assert_eq!(kind, InfrastructureErrorKind::SchemaError);
        }
        other => panic!("expected Infrastructure(SchemaError), got {other:?}"),
    }
}

// ─── Test 13: closed port → Network ──────────────────────────────────────────

#[test]
fn poll_transport_error_is_network() {
    // Bind a port, get the address, then drop the listener so nothing is
    // accepting connections.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);

    let poller =
        LibraryCheckerPoller::with_base_url(format!("http://{addr}")).expect("poller constructs");
    let err = poller
        .poll_submission(&test_handle("1"), None)
        .expect_err("should fail with transport error");
    match err {
        PollSubmissionError::Infrastructure { kind, .. } => {
            assert_eq!(kind, InfrastructureErrorKind::Network);
        }
        other => panic!("expected Infrastructure(Network), got {other:?}"),
    }
}

// ─── Test 14: time_ms rounds up ───────────────────────────────────────────────

#[test]
fn poll_time_ms_rounds_up() {
    // 0.0234 s → 23.4 ms → ceil → 24 ms
    let cases = r#"[{"case":"t","status":"AC","time":0.0234,"memory":0}]"#;
    let body = info_json_with_cases("AC", cases);
    let server = FixtureServer::start(serve_body(body));
    let obs = poller_for(&server)
        .poll_submission(&test_handle("1"), None)
        .expect("poll ok");
    let tc = match obs {
        PollObservation::Completed(r) => r.testcases.into_iter().next().expect("one testcase"),
        other => panic!("expected Completed, got {other:?}"),
    };
    assert_eq!(
        tc.time_ms,
        Some(24),
        "expected 24 ms (ceil of 23.4), got {:?}",
        tc.time_ms
    );
}

// ─── Test 15: memory_kib rounds up ───────────────────────────────────────────

#[test]
fn poll_memory_kib_rounds_up() {
    // 1025 bytes → ceil(1025 / 1024) = 2 KiB
    let cases = r#"[{"case":"t","status":"AC","time":0.0,"memory":1025}]"#;
    let body = info_json_with_cases("AC", cases);
    let server = FixtureServer::start(serve_body(body));
    let obs = poller_for(&server)
        .poll_submission(&test_handle("1"), None)
        .expect("poll ok");
    let tc = match obs {
        PollObservation::Completed(r) => r.testcases.into_iter().next().expect("one testcase"),
        other => panic!("expected Completed, got {other:?}"),
    };
    assert_eq!(
        tc.memory_kib,
        Some(2),
        "expected 2 KiB (ceil of 1025/1024), got {:?}",
        tc.memory_kib
    );
}

// ─── Test 16: registry contains LibraryChecker ────────────────────────────────

#[test]
fn poll_registry_contains_librarychecker() {
    let registry = build_poller_registry().expect("registry constructs");
    assert!(registry.contains(&OJKind::LibraryChecker));
}

// ─── Test 17: no sensitive field leakage ──────────────────────────────────────

#[test]
fn poll_summary_never_leaks_source_or_stderr() {
    let body = r#"{
        "overview": {
            "id": 9999,
            "problem_name": "aplusb",
            "lang": "rust",
            "is_latest": true,
            "status": "AC",
            "time": 1.0,
            "memory": 1024
        },
        "source": "SECRET_CODE_XYZ",
        "can_rejudge": false,
        "case_results": [
            {
                "case": "example_00",
                "status": "AC",
                "time": 1.0,
                "memory": 1024,
                "stderr": "SECRET_STDERR_XYZ",
                "checker_out": "SECRET_CHECKER_XYZ"
            }
        ]
    }"#
    .to_string();

    let server = FixtureServer::start(serve_body(body));
    let obs = poller_for(&server)
        .poll_submission(&test_handle("1"), None)
        .expect("poll ok");

    let debug_str = format!("{obs:?}");
    assert!(
        !debug_str.contains("SECRET_CODE_XYZ"),
        "source leaked into debug output: {debug_str}"
    );
    assert!(
        !debug_str.contains("SECRET_STDERR_XYZ"),
        "stderr leaked into debug output: {debug_str}"
    );
    assert!(
        !debug_str.contains("SECRET_CHECKER_XYZ"),
        "checker_out leaked into debug output: {debug_str}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Recovery tests (LibraryCheckerRecovery / Task 3)
// ═══════════════════════════════════════════════════════════════════════════

// ─── Recovery helpers ─────────────────────────────────────────────────────

fn firebase_session() -> Session {
    Session::Firebase {
        online_judge: OJKind::LibraryChecker,
        id_token: "test-id-token".to_string(),
        refresh_token: "test-refresh-token".to_string(),
    }
}

fn recovery_for(server: &FixtureServer) -> LibraryCheckerRecovery {
    LibraryCheckerRecovery::with_base_url(format!("http://{}", server.addr))
        .expect("recovery constructs")
}

fn current_user_json(name: &str) -> String {
    format!(
        r#"{{"user":{{"name":"{}","library_url":"","is_developer":false}}}}"#,
        name
    )
}

fn overview_json(id: i32, problem: &str, lang: &str, user: &str, time: &str) -> String {
    format!(
        r#"{{"id":{},"problem_name":"{}","lang":"{}","is_latest":true,"status":"AC","time":0.0,"memory":0,"user_name":"{}","submission_time":"{}"}}"#,
        id, problem, lang, user, time
    )
}

fn detail_json(id: i32, problem: &str, lang: &str, user: &str, source: &str, time: &str) -> String {
    let source_json = serde_json::to_string(source).expect("source serializes");
    format!(
        r#"{{"overview":{{"id":{},"problem_name":"{}","lang":"{}","is_latest":true,"status":"AC","time":0.0,"memory":0,"user_name":"{}","submission_time":"{}"}},  "source":{},"can_rejudge":false}}"#,
        id, problem, lang, user, time, source_json
    )
}

fn list_json(overviews: &[String]) -> String {
    format!(
        r#"{{"submissions":[{}],"count":{}}}"#,
        overviews.join(","),
        overviews.len()
    )
}

/// Builds a `RecoveryRequest` by hashing `source` the same way the
/// production code does (via `SubmissionRequest::from_request`).
fn recovery_request_for(problem_id: &str, lang_id: &str, source: &str) -> RecoveryRequest {
    let sub_req = SubmissionRequest {
        online_judge: OJKind::LibraryChecker,
        contest_id: format!("librarychecker-{problem_id}"),
        problem_id: problem_id.to_string(),
        lang_id: lang_id.to_string(),
        source: source.to_string(),
    };
    RecoveryRequest::from_request(&sub_req)
}

// ─── Recovery test 1: single hash match → Recovered ──────────────────────

#[test]
fn recover_returns_recovered_on_single_source_hash_match() {
    let source_match = "fn main_match() {}";
    let source_other = "fn main_other() {}";
    let time = "2024-01-15T10:00:00Z";

    let rec_req = recovery_request_for("aplusb", "rust", source_match);

    let alice_json = current_user_json("alice");
    let list = list_json(&[
        overview_json(1234, "aplusb", "rust", "alice", time),
        overview_json(1235, "aplusb", "rust", "alice", time),
    ]);
    let detail_1234 = detail_json(1234, "aplusb", "rust", "alice", source_match, time);
    let detail_1235 = detail_json(1235, "aplusb", "rust", "alice", source_other, time);

    let server = FixtureServer::start(move |req| {
        let url = req.url().to_string();
        let body = if url == "/auth/current_user" {
            alice_json.clone()
        } else if url.starts_with("/submissions/1234") {
            detail_1234.clone()
        } else if url.starts_with("/submissions/1235") {
            detail_1235.clone()
        } else {
            list.clone()
        };
        let _ = req.respond(Response::from_data(body.into_bytes()));
    });

    let outcome = recovery_for(&server)
        .recover_submission(&rec_req, Some(&firebase_session()))
        .expect("recovery ok");
    match outcome {
        RecoveryOutcome::Recovered { handle } => {
            assert_eq!(handle.submission_id, "1234");
            assert_eq!(handle.online_judge, OJKind::LibraryChecker);
            assert!(
                handle.submission_url.contains("1234"),
                "url should contain id: {}",
                handle.submission_url
            );
        }
        other => panic!("expected Recovered, got {other:?}"),
    }
}

// ─── Recovery test 2: empty list → AcceptanceUnknown ─────────────────────

#[test]
fn recover_returns_acceptance_unknown_on_zero_candidates() {
    let rec_req = recovery_request_for("aplusb", "rust", "fn main_zero() {}");
    let alice_json = current_user_json("alice");
    let empty_list = list_json(&[]);

    let server = FixtureServer::start(move |req| {
        let url = req.url().to_string();
        let body = if url == "/auth/current_user" {
            alice_json.clone()
        } else {
            empty_list.clone()
        };
        let _ = req.respond(Response::from_data(body.into_bytes()));
    });

    let outcome = recovery_for(&server)
        .recover_submission(&rec_req, Some(&firebase_session()))
        .expect("recovery ok");
    assert!(
        matches!(outcome, RecoveryOutcome::AcceptanceUnknown),
        "expected AcceptanceUnknown, got {outcome:?}"
    );
}

// ─── Recovery test 3: multiple matches → AcceptanceUnknown ───────────────

#[test]
fn recover_returns_acceptance_unknown_on_multiple_matches() {
    let source = "fn main_multi() {}";
    let time = "2024-01-15T10:00:00Z";
    let rec_req = recovery_request_for("aplusb", "rust", source);

    let alice_json = current_user_json("alice");
    let list = list_json(&[
        overview_json(1234, "aplusb", "rust", "alice", time),
        overview_json(1235, "aplusb", "rust", "alice", time),
    ]);
    // Both details return the same source → two matches.
    let detail = detail_json(1234, "aplusb", "rust", "alice", source, time);
    let detail2 = detail_json(1235, "aplusb", "rust", "alice", source, time);

    let server = FixtureServer::start(move |req| {
        let url = req.url().to_string();
        let body = if url == "/auth/current_user" {
            alice_json.clone()
        } else if url.starts_with("/submissions/1234") {
            detail.clone()
        } else if url.starts_with("/submissions/1235") {
            detail2.clone()
        } else {
            list.clone()
        };
        let _ = req.respond(Response::from_data(body.into_bytes()));
    });

    let outcome = recovery_for(&server)
        .recover_submission(&rec_req, Some(&firebase_session()))
        .expect("recovery ok");
    assert!(
        matches!(outcome, RecoveryOutcome::AcceptanceUnknown),
        "expected AcceptanceUnknown for multiple matches, got {outcome:?}"
    );
}

// ─── Recovery test 4: wrong problem → AcceptanceUnknown ──────────────────

#[test]
fn recover_ignores_wrong_problem() {
    let source = "fn main_wrong_prob() {}";
    let time = "2024-01-15T10:00:00Z";
    let rec_req = recovery_request_for("aplusb", "rust", source);

    let alice_json = current_user_json("alice");
    // Overview has problem_name = "different_problem", not "aplusb".
    let list = list_json(&[overview_json(
        1234,
        "different_problem",
        "rust",
        "alice",
        time,
    )]);

    let server = FixtureServer::start(move |req| {
        let url = req.url().to_string();
        let body = if url == "/auth/current_user" {
            alice_json.clone()
        } else {
            list.clone()
        };
        let _ = req.respond(Response::from_data(body.into_bytes()));
    });

    let outcome = recovery_for(&server)
        .recover_submission(&rec_req, Some(&firebase_session()))
        .expect("recovery ok");
    assert!(
        matches!(outcome, RecoveryOutcome::AcceptanceUnknown),
        "expected AcceptanceUnknown for wrong problem, got {outcome:?}"
    );
}

// ─── Recovery test 5: wrong language → AcceptanceUnknown ─────────────────

#[test]
fn recover_ignores_wrong_language() {
    let source = "fn main_wrong_lang() {}";
    let time = "2024-01-15T10:00:00Z";
    let rec_req = recovery_request_for("aplusb", "rust", source);

    let alice_json = current_user_json("alice");
    // Overview has lang = "cpp", not "rust".
    let list = list_json(&[overview_json(1234, "aplusb", "cpp", "alice", time)]);

    let server = FixtureServer::start(move |req| {
        let url = req.url().to_string();
        let body = if url == "/auth/current_user" {
            alice_json.clone()
        } else {
            list.clone()
        };
        let _ = req.respond(Response::from_data(body.into_bytes()));
    });

    let outcome = recovery_for(&server)
        .recover_submission(&rec_req, Some(&firebase_session()))
        .expect("recovery ok");
    assert!(
        matches!(outcome, RecoveryOutcome::AcceptanceUnknown),
        "expected AcceptanceUnknown for wrong language, got {outcome:?}"
    );
}

// ─── Recovery test 6: wrong user in list row → AcceptanceUnknown ─────────

#[test]
fn recover_ignores_wrong_user() {
    let source = "fn main_wrong_user() {}";
    let time = "2024-01-15T10:00:00Z";
    let rec_req = recovery_request_for("aplusb", "rust", source);

    // current_user is "bob", but overview's user_name is "alice".
    let bob_json = current_user_json("bob");
    let list = list_json(&[overview_json(1234, "aplusb", "rust", "alice", time)]);

    let server = FixtureServer::start(move |req| {
        let url = req.url().to_string();
        let body = if url == "/auth/current_user" {
            bob_json.clone()
        } else {
            list.clone()
        };
        let _ = req.respond(Response::from_data(body.into_bytes()));
    });

    let outcome = recovery_for(&server)
        .recover_submission(&rec_req, Some(&firebase_session()))
        .expect("recovery ok");
    assert!(
        matches!(outcome, RecoveryOutcome::AcceptanceUnknown),
        "expected AcceptanceUnknown for wrong user, got {outcome:?}"
    );
}

// ─── Recovery test 7: submission time outside attempt window → AcceptanceUnknown

#[test]
fn recover_ignores_submissions_outside_attempt_window() {
    let source = "fn main_old() {}";
    let rec_req_base = recovery_request_for("aplusb", "rust", source);

    let lower_bound = Utc::now();
    // Submission is 1 hour before lower_bound — well outside the 60s grace window.
    let old_time = lower_bound - ChronoDuration::hours(1);
    let old_time_str = old_time.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let rec_req = RecoveryRequest {
        submitted_at_lower_bound: Some(lower_bound),
        ..rec_req_base
    };

    let alice_json = current_user_json("alice");
    let list = list_json(&[overview_json(
        1234,
        "aplusb",
        "rust",
        "alice",
        &old_time_str,
    )]);

    let server = FixtureServer::start(move |req| {
        let url = req.url().to_string();
        let body = if url == "/auth/current_user" {
            alice_json.clone()
        } else {
            list.clone()
        };
        let _ = req.respond(Response::from_data(body.into_bytes()));
    });

    let outcome = recovery_for(&server)
        .recover_submission(&rec_req, Some(&firebase_session()))
        .expect("recovery ok");
    assert!(
        matches!(outcome, RecoveryOutcome::AcceptanceUnknown),
        "expected AcceptanceUnknown for outside-window submission, got {outcome:?}"
    );
}

// ─── Recovery test 8: pagination stops when entries older than window ─────

#[test]
fn recover_paginates_but_stops_when_older_than_window() {
    let lower_bound = Utc::now();
    // Recent time is just 1 second in the future (well within window).
    let recent_time_str = (lower_bound + ChronoDuration::seconds(1))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    // Old time is 120 s before lower_bound, beyond the 60 s grace.
    let old_time_str = (lower_bound - ChronoDuration::seconds(120))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    // Page 0 (skip=0): 100 recent entries with wrong problem — no detail fetch.
    let mut page0_entries = Vec::new();
    for i in 0..100i32 {
        page0_entries.push(overview_json(
            i + 2000,
            "wrong_problem",
            "rust",
            "alice",
            &recent_time_str,
        ));
    }
    let page0_json = list_json(&page0_entries);

    // Page 1 (skip=100): 1 old entry → stops pagination.
    let page1_json = list_json(&[overview_json(
        9999,
        "aplusb",
        "rust",
        "alice",
        &old_time_str,
    )]);

    let list_request_count = Arc::new(AtomicUsize::new(0));
    let list_count_clone = Arc::clone(&list_request_count);
    let alice_json = current_user_json("alice");

    let server = FixtureServer::start(move |req| {
        let url = req.url().to_string();
        let body = if url == "/auth/current_user" {
            alice_json.clone()
        } else if url.starts_with("/submissions/") {
            // No detail requests expected.
            "{}".to_string()
        } else {
            let page = list_count_clone.fetch_add(1, Ordering::SeqCst);
            if page == 0 {
                page0_json.clone()
            } else {
                page1_json.clone()
            }
        };
        let _ = req.respond(Response::from_data(body.into_bytes()));
    });

    let rec_req = RecoveryRequest {
        online_judge: OJKind::LibraryChecker,
        contest_id: "librarychecker-aplusb".to_string(),
        problem_id: "aplusb".to_string(),
        lang_id: "rust".to_string(),
        source_hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            .to_string(),
        submitted_at_lower_bound: Some(lower_bound),
    };

    let outcome = recovery_for(&server)
        .recover_submission(&rec_req, Some(&firebase_session()))
        .expect("recovery ok");

    assert_eq!(
        list_request_count.load(Ordering::SeqCst),
        2,
        "should make exactly 2 list requests (page 0 and page 1), not 3"
    );
    assert!(
        matches!(outcome, RecoveryOutcome::AcceptanceUnknown),
        "expected AcceptanceUnknown, got {outcome:?}"
    );
}

// ─── Recovery test 9: list 500 → AcceptanceUnknown ───────────────────────

#[test]
fn recover_returns_acceptance_unknown_on_list_failure() {
    let rec_req = recovery_request_for("aplusb", "rust", "fn main_list_fail() {}");
    let alice_json = current_user_json("alice");

    let server = FixtureServer::start(move |req| {
        let url = req.url().to_string();
        if url == "/auth/current_user" {
            let _ = req.respond(Response::from_data(alice_json.as_bytes().to_vec()));
        } else {
            let _ = req.respond(Response::empty(500u16));
        }
    });

    let outcome = recovery_for(&server)
        .recover_submission(&rec_req, Some(&firebase_session()))
        .expect("recovery ok");
    assert!(
        matches!(outcome, RecoveryOutcome::AcceptanceUnknown),
        "expected AcceptanceUnknown on list 500, got {outcome:?}"
    );
}

// ─── Recovery test 10: detail 500 → AcceptanceUnknown ────────────────────

#[test]
fn recover_returns_acceptance_unknown_on_detail_failure() {
    let source = "fn main_detail_fail() {}";
    let time = "2024-01-15T10:00:00Z";
    let rec_req = recovery_request_for("aplusb", "rust", source);

    let alice_json = current_user_json("alice");
    let list = list_json(&[overview_json(1234, "aplusb", "rust", "alice", time)]);

    let server = FixtureServer::start(move |req| {
        let url = req.url().to_string();
        if url == "/auth/current_user" {
            let _ = req.respond(Response::from_data(alice_json.as_bytes().to_vec()));
        } else if url.starts_with("/submissions/") {
            // Detail request fails.
            let _ = req.respond(Response::empty(500u16));
        } else {
            let _ = req.respond(Response::from_data(list.as_bytes().to_vec()));
        }
    });

    let outcome = recovery_for(&server)
        .recover_submission(&rec_req, Some(&firebase_session()))
        .expect("recovery ok");
    assert!(
        matches!(outcome, RecoveryOutcome::AcceptanceUnknown),
        "expected AcceptanceUnknown on detail 500, got {outcome:?}"
    );
}

// ─── Recovery test 11: duplicate ids in list → fetch detail once ──────────

#[test]
fn recover_dedupes_by_submission_id() {
    let source = "fn main_dedup() {}";
    let time = "2024-01-15T10:00:00Z";
    let rec_req = recovery_request_for("aplusb", "rust", source);

    let alice_json = current_user_json("alice");
    // Same overview twice in the list.
    let ov = overview_json(42, "aplusb", "rust", "alice", time);
    let list = list_json(&[ov.clone(), ov]);
    let detail = detail_json(42, "aplusb", "rust", "alice", source, time);

    let detail_count = Arc::new(AtomicUsize::new(0));
    let detail_count_clone = Arc::clone(&detail_count);

    let server = FixtureServer::start(move |req| {
        let url = req.url().to_string();
        let body = if url == "/auth/current_user" {
            alice_json.clone()
        } else if url.starts_with("/submissions/") {
            detail_count_clone.fetch_add(1, Ordering::SeqCst);
            detail.clone()
        } else {
            list.clone()
        };
        let _ = req.respond(Response::from_data(body.into_bytes()));
    });

    let outcome = recovery_for(&server)
        .recover_submission(&rec_req, Some(&firebase_session()))
        .expect("recovery ok");

    assert_eq!(
        detail_count.load(Ordering::SeqCst),
        1,
        "should fetch detail exactly once for duplicate ids"
    );
    match outcome {
        RecoveryOutcome::Recovered { handle } => {
            assert_eq!(handle.submission_id, "42");
        }
        other => panic!("expected Recovered after dedup, got {other:?}"),
    }
}

// ─── Recovery test 12: session = None → CredentialsMissing ───────────────

#[test]
fn recover_returns_credentials_missing_when_session_is_none() {
    // No server needed — the error fires before any HTTP is attempted.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);

    let recovery =
        LibraryCheckerRecovery::with_base_url(format!("http://{addr}")).expect("constructs");
    let rec_req = recovery_request_for("aplusb", "rust", "fn main() {}");

    let err = recovery
        .recover_submission(&rec_req, None)
        .expect_err("should return Err for missing session");
    match err {
        RecoverSubmissionError::Infrastructure { kind, .. } => {
            assert_eq!(kind, InfrastructureErrorKind::CredentialsMissing);
        }
    }
}

// ─── Recovery test 13: error summary is sanitized ─────────────────────────

#[test]
fn recover_sanitizes_summaries() {
    // The CredentialsMissing path (session=None) is the Err path. Verify its
    // summary is sanitized — no bearer tokens, cookies, or refresh_token strings.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);

    let recovery =
        LibraryCheckerRecovery::with_base_url(format!("http://{addr}")).expect("constructs");
    let rec_req = recovery_request_for("aplusb", "rust", "fn main() {}");

    let err = recovery
        .recover_submission(&rec_req, None)
        .expect_err("should error");
    match err {
        RecoverSubmissionError::Infrastructure { summary, .. } => {
            let lower = summary.to_lowercase();
            assert!(
                !lower.contains("bearer "),
                "summary must not contain bearer: {summary}"
            );
            assert!(
                !lower.contains("cookie="),
                "summary must not contain cookie=: {summary}"
            );
            assert!(
                !lower.contains("refresh_token"),
                "summary must not contain refresh_token: {summary}"
            );
        }
    }
}

// ─── Recovery test 14: source must not appear in debug output ─────────────

#[test]
fn recover_never_leaks_source_in_debug() {
    let secret_source = "fn main_SECRET_XYZ_DO_NOT_LEAK() {}";
    let time = "2024-01-15T10:00:00Z";
    let rec_req = recovery_request_for("aplusb", "rust", secret_source);

    let alice_json = current_user_json("alice");
    let list = list_json(&[overview_json(42, "aplusb", "rust", "alice", time)]);
    let detail = detail_json(42, "aplusb", "rust", "alice", secret_source, time);

    let server = FixtureServer::start(move |req| {
        let url = req.url().to_string();
        let body = if url == "/auth/current_user" {
            alice_json.clone()
        } else if url.starts_with("/submissions/") {
            detail.clone()
        } else {
            list.clone()
        };
        let _ = req.respond(Response::from_data(body.into_bytes()));
    });

    let outcome = recovery_for(&server)
        .recover_submission(&rec_req, Some(&firebase_session()))
        .expect("recovery ok");

    let debug_str = format!("{outcome:?}");
    assert!(
        !debug_str.contains("SECRET_XYZ_DO_NOT_LEAK"),
        "source leaked into debug output: {debug_str}"
    );
    assert!(
        matches!(outcome, RecoveryOutcome::Recovered { .. }),
        "expected Recovered for source-hash match"
    );
}

// ─── Recovery test 15: registry contains LibraryChecker ──────────────────

#[test]
fn recovery_registry_contains_librarychecker() {
    let registry = build_recovery_registry().expect("registry constructs");
    assert!(registry.contains(&OJKind::LibraryChecker));
}
