//! Integration test for the `ce internal verify-pr-set-state` shell helper.
//!
//! Drives the shell-level [`pr_set_state_with_io`] against a scripted
//! `tiny_http` server so no real GitHub API is contacted.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, FixedOffset};
use domain::library::{LanguageId, SolutionId};
use domain::online_judge::{RecoveryMode, ResultDetail, SubmissionCapabilities, SubmissionMode};
use domain::verification::{
    AttemptId, CompletedState, ContentHash, LanguageBinding, PlanContext, StartingState,
    SubmissionHandle, SubmissionSummary, Verdict, VerdictKind, VerificationRecord,
    VerificationState, VerifyFingerprint,
};
use infrastructure::shell::pr_set_state_with_io;
use tiny_http::{Response, Server};

#[derive(Clone)]
struct Reply {
    status: u16,
    body: String,
}

impl Reply {
    fn json(status: u16, body: serde_json::Value) -> Self {
        Self {
            status,
            body: body.to_string(),
        }
    }
}

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    url: String,
    body: String,
}

struct Fixture {
    addr: SocketAddr,
    recorded: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn start(script: Vec<Reply>) -> Self {
        let server = Server::http("127.0.0.1:0").expect("bind fixture server");
        let addr = server.server_addr().to_ip().expect("fixture ipv4 addr");
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let index = Arc::new(AtomicUsize::new(0));

        let recorded_thread = Arc::clone(&recorded);
        let stop_thread = Arc::clone(&stop);
        let index_thread = Arc::clone(&index);

        let handle = thread::spawn(move || {
            while !stop_thread.load(Ordering::SeqCst) {
                match server.recv_timeout(Duration::from_millis(100)) {
                    Ok(Some(mut req)) => {
                        let mut body = String::new();
                        let _ = req.as_reader().read_to_string(&mut body);
                        recorded_thread.lock().unwrap().push(RecordedRequest {
                            method: req.method().as_str().to_string(),
                            url: req.url().to_string(),
                            body,
                        });
                        let i = index_thread.fetch_add(1, Ordering::SeqCst);
                        if i < script.len() {
                            let reply = script[i].clone();
                            let resp =
                                Response::from_string(reply.body).with_status_code(reply.status);
                            let _ = req.respond(resp);
                        } else {
                            let resp = Response::from_string("unexpected".to_string())
                                .with_status_code(500);
                            let _ = req.respond(resp);
                        }
                    }
                    Ok(None) => continue,
                    Err(_) => break,
                }
            }
        });

        Self {
            addr,
            recorded,
            stop,
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn recorded(&self) -> Vec<RecordedRequest> {
        self.recorded.lock().unwrap().clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn ts() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap()
}

fn language() -> LanguageBinding {
    LanguageBinding {
        language_id: LanguageId::parse("rust").unwrap(),
        oj_language_id: "rust".into(),
    }
}

fn hash() -> ContentHash {
    ContentHash::parse("sha256:2222222222222222222222222222222222222222222222222222222222222222")
        .unwrap()
}

fn fingerprint() -> VerifyFingerprint {
    VerifyFingerprint::parse(
        "sha256:1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap()
}

fn starting_record() -> VerificationRecord {
    VerificationRecord {
        schema_version: 1,
        solution_id: SolutionId::parse("abc999/a/main").unwrap(),
        attempt_id: AttemptId::parse("attempt-1").unwrap(),
        replaces_attempt_id: None,
        fingerprint: fingerprint(),
        state: VerificationState::Starting(StartingState {
            plan_hash: hash(),
            submitted_source_hash: hash(),
            language: language(),
            started_at: ts(),
        }),
        plan_context: Some(PlanContext {
            language: language(),
            submitted_source_hash: hash(),
        }),
    }
}

fn completed_accepted_record() -> VerificationRecord {
    VerificationRecord {
        schema_version: 1,
        solution_id: SolutionId::parse("abc999/a/main").unwrap(),
        attempt_id: AttemptId::parse("attempt-1").unwrap(),
        replaces_attempt_id: None,
        fingerprint: fingerprint(),
        state: VerificationState::Completed(CompletedState {
            verdict: Verdict {
                kind: VerdictKind::Accepted,
                raw: "AC".into(),
            },
            verified_libraries: Vec::new(),
            language: language(),
            verified_at: ts(),
            capabilities: SubmissionCapabilities {
                submission_mode: SubmissionMode::UnattendedTrackable,
                result_detail: ResultDetail::TestcaseDetails,
                recovery_mode: RecoveryMode::BestEffort,
            },
            submitted_source_hash: hash(),
            input_hashes: Default::default(),
            summary: SubmissionSummary {
                max_execution_time_ms: None,
                max_memory_bytes: None,
            },
            test_cases: None,
            handle: SubmissionHandle {
                oj: "librarychecker".into(),
                submission_id: "sub".into(),
                submission_url: "https://example.test/sub".into(),
                locator: None,
                submitted_at: ts(),
            },
            extra: Default::default(),
        }),
        plan_context: Some(PlanContext {
            language: language(),
            submitted_source_hash: hash(),
        }),
    }
}

fn write_record(record: &VerificationRecord) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().expect("temp file");
    use std::io::Write as _;
    let bytes = serde_json::to_vec(record).expect("serialize record");
    f.write_all(&bytes).expect("write record");
    f
}

/// `Starting` state → Draft branch: only the list-then-PATCH sequence runs
/// (find existing PR + PATCH draft:false=false).
#[test]
fn starting_record_keeps_pr_draft_and_reuses_existing_pr() {
    // 1. GET /pulls?head=owner:automation/verify&state=open&base=main → [{number: 5}]
    // 2. PATCH /pulls/5 { draft: true }
    let script = vec![
        Reply::json(200, serde_json::json!([{ "number": 5, "state": "open" }])),
        Reply::json(200, serde_json::json!({ "number": 5, "draft": true })),
    ];
    let fx = Fixture::start(script);
    let record = write_record(&starting_record());
    let n = pr_set_state_with_io(
        &fx.base_url(),
        record.path().to_str().unwrap(),
        "owner/repo",
        "automation/verify",
        "main",
        "test-token",
    )
    .expect("succeeds");
    assert_eq!(n, 5);

    let recorded = fx.recorded();
    assert_eq!(recorded.len(), 2, "expected 2 requests, got {recorded:#?}");
    assert_eq!(recorded[0].method, "GET");
    assert_eq!(
        recorded[0].url,
        "/repos/owner/repo/pulls?head=owner:automation/verify&state=open&base=main"
    );
    assert_eq!(recorded[1].method, "PATCH");
    assert_eq!(recorded[1].url, "/repos/owner/repo/pulls/5");
    let body: serde_json::Value = serde_json::from_str(&recorded[1].body).unwrap();
    // draft = !ready; for the Draft branch, ready=false so draft=true.
    assert_eq!(body["draft"], true);
}

/// Completed(Accepted) → ReadyAutoMerge branch: list → PATCH ready + auto_merge
/// via node id + GraphQL.
#[test]
fn completed_accepted_flips_pr_to_ready_and_auto_merge() {
    // 1. GET /pulls?... → []  (opens a new PR)
    // 2. POST /pulls → { number: 9 }
    // 3. GET /pulls/9 → { node_id: "PR_kwDOTEST" } (resolve node id)
    // 4. PATCH /pulls/9 { draft: false }
    // 5. POST /graphql → { data: {...} }
    let script = vec![
        Reply::json(200, serde_json::json!([])),
        Reply::json(201, serde_json::json!({ "number": 9 })),
        Reply::json(
            200,
            serde_json::json!({ "number": 9, "node_id": "PR_kwDOTEST" }),
        ),
        Reply::json(200, serde_json::json!({ "number": 9, "draft": false })),
        Reply::json(
            200,
            serde_json::json!({
                "data": { "enablePullRequestAutoMerge": { "clientMutationId": "ok" } }
            }),
        ),
    ];
    let fx = Fixture::start(script);
    let record = write_record(&completed_accepted_record());
    let n = pr_set_state_with_io(
        &fx.base_url(),
        record.path().to_str().unwrap(),
        "owner/repo",
        "automation/verify",
        "main",
        "test-token",
    )
    .expect("succeeds");
    assert_eq!(n, 9);

    let recorded = fx.recorded();
    assert_eq!(recorded.len(), 5, "expected 5 requests, got {recorded:#?}");
    assert_eq!(recorded[0].method, "GET");
    assert!(recorded[0].url.starts_with("/repos/owner/repo/pulls?head="));
    assert_eq!(recorded[1].method, "POST");
    assert_eq!(recorded[1].url, "/repos/owner/repo/pulls");
    assert_eq!(recorded[2].method, "GET");
    assert_eq!(recorded[2].url, "/repos/owner/repo/pulls/9");
    assert_eq!(recorded[3].method, "PATCH");
    let patch_body: serde_json::Value = serde_json::from_str(&recorded[3].body).unwrap();
    assert_eq!(patch_body["draft"], false);
    assert_eq!(recorded[4].method, "POST");
    assert_eq!(recorded[4].url, "/graphql");
}

#[test]
fn invalid_repository_slug_fails_early() {
    let fx = Fixture::start(vec![]);
    let record = write_record(&starting_record());
    let err = pr_set_state_with_io(
        &fx.base_url(),
        record.path().to_str().unwrap(),
        "not-a-slug",
        "automation/verify",
        "main",
        "test-token",
    )
    .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("owner/repo"),
        "expected error mentioning owner/repo, got: {msg}"
    );
    // No HTTP call was made.
    assert!(
        fx.recorded().is_empty(),
        "no request should have been sent, got {:#?}",
        fx.recorded()
    );
}
