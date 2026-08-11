//! Fixture-based integration tests for `LibraryCheckerPoller`.
//!
//! Each test spins a local `tiny_http` server so no real network is needed.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use domain::entity::OJKind;
use infrastructure::online_judge_impl::librarychecker::submission::LibraryCheckerPoller;
use infrastructure::submission_impl::poller::build_poller_registry;
use tiny_http::{Header, Response, Server};
use usecases::submission::{
    InfrastructureErrorKind, JudgeVerdict, PollObservation, PollSubmissionError, SubmissionHandle,
    SubmissionPoller,
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
