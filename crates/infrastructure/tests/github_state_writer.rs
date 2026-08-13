//! Integration tests for the constrained GitHub verification-state writer.
//!
//! All HTTP fixtures are served by a `tiny_http::Server` bound to
//! `127.0.0.1:0`; no test contacts the real GitHub API. Each test drives
//! the writer through a scripted response sequence and asserts both the
//! per-request payloads and the aggregate call count.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{DateTime, FixedOffset};
use domain::library::{LanguageId, SolutionId};
use domain::verification::{
    AttemptId, ContentHash, LanguageBinding, PlanContext, StartingState, VerificationRecord,
    VerificationState, VerifyFingerprint,
};
use infrastructure::github::{
    BotPullRequestState, GitHubVerificationStateWriter, PersistError, PersistStateRequest,
    validate_result_path,
};
use secrecy::SecretString;
use tiny_http::{Response, Server};

// ─── Harness ────────────────────────────────────────────────────────────────

/// One scripted response the fake server should return for the next request.
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

    fn empty(status: u16) -> Self {
        Self {
            status,
            body: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    url: String,
    body: String,
    /// `Authorization` header if present.
    authorization: Option<String>,
}

/// Local tiny_http fixture that owns a scripted response queue.
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
                        let authorization = req
                            .headers()
                            .iter()
                            .find(|h| h.field.equiv("Authorization"))
                            .map(|h| h.value.as_str().to_string());
                        recorded_thread.lock().unwrap().push(RecordedRequest {
                            method: req.method().as_str().to_string(),
                            url: req.url().to_string(),
                            body,
                            authorization,
                        });
                        let i = index_thread.fetch_add(1, Ordering::SeqCst);
                        if i < script.len() {
                            let reply = script[i].clone();
                            let resp =
                                Response::from_string(reply.body).with_status_code(reply.status);
                            let _ = req.respond(resp);
                        } else {
                            // Unexpected extra request: signal by returning 500.
                            let resp =
                                Response::from_string("unexpected extra request".to_string())
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

// ─── Fixtures for VerificationRecord ────────────────────────────────────────

fn ts(offset_min: i64) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap()
        + chrono::Duration::minutes(offset_min)
}

fn language() -> LanguageBinding {
    LanguageBinding {
        language_id: LanguageId::parse("rust").unwrap(),
        oj_language_id: "rust".into(),
    }
}

fn plan_hash() -> ContentHash {
    ContentHash::parse("sha256:2222222222222222222222222222222222222222222222222222222222222222")
        .unwrap()
}

fn source_hash() -> ContentHash {
    ContentHash::parse("sha256:3333333333333333333333333333333333333333333333333333333333333333")
        .unwrap()
}

fn fingerprint() -> VerifyFingerprint {
    VerifyFingerprint::parse(
        "sha256:1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap()
}

fn starting_record(attempt: &str, replaces: Option<&str>) -> VerificationRecord {
    VerificationRecord {
        schema_version: 1,
        solution_id: SolutionId::parse("abc999/a/main").unwrap(),
        attempt_id: AttemptId::parse(attempt).unwrap(),
        replaces_attempt_id: replaces.map(|r| AttemptId::parse(r).unwrap()),
        fingerprint: fingerprint(),
        state: VerificationState::Starting(StartingState {
            plan_hash: plan_hash(),
            submitted_source_hash: source_hash(),
            language: language(),
            started_at: ts(0),
        }),
        plan_context: Some(PlanContext {
            language: language(),
            submitted_source_hash: source_hash(),
        }),
    }
}

fn base_sha() -> &'static str {
    "0123456789abcdef0123456789abcdef01234567"
}

fn commit_sha() -> &'static str {
    "aaaabbbbccccddddeeeeffff0000111122223333"
}

fn tree_sha() -> &'static str {
    "1111222233334444555566667777888899990000"
}

fn blob_sha() -> &'static str {
    "9999888877776666555544443333222211110000"
}

fn valid_request(record: VerificationRecord) -> PersistStateRequest {
    PersistStateRequest {
        repository: "owner/repo".into(),
        base_sha: base_sha().into(),
        branch: "automation/verify".into(),
        candidate: record,
    }
}

fn contents_response_for(record: &VerificationRecord) -> Reply {
    let json = serde_json::to_string(record).unwrap();
    let content = BASE64.encode(json.as_bytes());
    Reply::json(
        200,
        serde_json::json!({
            "content": content,
            "encoding": "base64",
            "name": "main.json",
            "path": "verification/results/abc999/a/main.json",
        }),
    )
}

fn writer(base_url: String) -> GitHubVerificationStateWriter {
    GitHubVerificationStateWriter::new(base_url, SecretString::from("test-token"))
}

// Common happy-path scripts ─────────────────────────────────────────────────

fn base_commit_tree_sha() -> &'static str {
    "cccccccccccccccccccccccccccccccccccccccc"
}

fn state_head_sha() -> &'static str {
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
}

fn get_ref_reply(sha: &str) -> Reply {
    Reply::json(
        200,
        serde_json::json!({
            "ref": "refs/heads/automation/verify",
            "object": { "sha": sha }
        }),
    )
}

fn happy_script() -> Vec<Reply> {
    vec![
        // 0. GET ref → state branch tip (anchor for the whole call).
        get_ref_reply(state_head_sha()),
        // 1. CAS GET → 404 (result absent)
        Reply::empty(404),
        // 2. POST blob
        Reply::json(201, serde_json::json!({ "sha": blob_sha() })),
        // 3. GET commit → resolve state head's tree
        Reply::json(
            200,
            serde_json::json!({
                "sha": state_head_sha(),
                "tree": { "sha": base_commit_tree_sha() }
            }),
        ),
        // 4. POST tree
        Reply::json(201, serde_json::json!({ "sha": tree_sha() })),
        // 5. POST commit
        Reply::json(201, serde_json::json!({ "sha": commit_sha() })),
        // 6. PATCH ref
        Reply::json(
            200,
            serde_json::json!({
                "ref": "refs/heads/automation/verify",
                "object": { "sha": commit_sha() }
            }),
        ),
    ]
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[test]
fn persist_rejects_wrong_branch() {
    // No fixture: any HTTP call would fail because we never point the writer
    // at a live server. This is the strongest way to assert "no HTTP call was
    // made" — if a request escaped, reqwest would report a connection error.
    let w = writer("http://127.0.0.1:1".into());
    let mut req = valid_request(starting_record("attempt-1", None));
    req.branch = "main".into();

    let err = w.persist(&req).unwrap_err();
    match err {
        PersistError::WrongBranch { branch } => assert_eq!(branch, "main"),
        other => panic!("expected WrongBranch, got {other:?}"),
    }
}

#[test]
fn persist_rejects_invalid_base_sha() {
    let w = writer("http://127.0.0.1:1".into());
    let mut req = valid_request(starting_record("attempt-1", None));
    req.base_sha = "not-hex".into();

    let err = w.persist(&req).unwrap_err();
    assert!(matches!(err, PersistError::InvalidBaseSha), "got {err:?}");
}

#[test]
fn persist_rejects_disallowed_path() {
    // `SolutionId` already forbids `..` and other escape segments, so we
    // exercise the defense-in-depth `validate_result_path` guard directly.
    let err = validate_result_path("verification/results/../evil.json").unwrap_err();
    assert!(matches!(err, PersistError::InvalidResultPath { .. }));

    let err = validate_result_path("workflows/verify.yml").unwrap_err();
    assert!(matches!(err, PersistError::InvalidResultPath { .. }));

    let err = validate_result_path("verification/results/foo.txt").unwrap_err();
    assert!(matches!(err, PersistError::InvalidResultPath { .. }));

    // Positive control: a canonical result path is accepted.
    validate_result_path("verification/results/abc999/a/main.json").unwrap();
}

#[test]
fn validate_result_path_rejects_bare_json_leaf() {
    // `verification/results/.json` slips past a naive prefix+suffix check but
    // is rejected by the classifier's `is_result_json_path`. Keep both guards
    // in agreement so a future path-construction bug cannot land here.
    let err = validate_result_path("verification/results/.json").unwrap_err();
    assert!(
        matches!(err, PersistError::InvalidResultPath { .. }),
        "got {err:?}"
    );
}

#[test]
fn persist_writes_blob_tree_commit_and_updates_ref_when_result_absent() {
    let fx = Fixture::start(happy_script());
    let w = writer(fx.base_url());

    let req = valid_request(starting_record("attempt-1", None));
    let out = w.persist(&req).expect("persist succeeds");

    assert_eq!(out.result_path, "verification/results/abc999/a/main.json");
    assert_eq!(out.blob_sha, blob_sha());
    assert_eq!(out.tree_sha, tree_sha());
    assert_eq!(out.commit_sha, commit_sha());

    let recorded = fx.recorded();
    assert_eq!(recorded.len(), 7, "expected 7 requests, got {recorded:#?}");

    // Every request must have carried the Bearer token.
    for r in &recorded {
        assert_eq!(
            r.authorization.as_deref(),
            Some("Bearer test-token"),
            "missing/incorrect Authorization on {} {}",
            r.method,
            r.url,
        );
    }

    // 0. GET ref → state branch tip.
    assert_eq!(recorded[0].method, "GET");
    assert_eq!(
        recorded[0].url,
        "/repos/owner/repo/git/refs/heads/automation/verify"
    );

    // 1. CAS GET at state_head.
    assert_eq!(recorded[1].method, "GET");
    assert!(
        recorded[1]
            .url
            .starts_with("/repos/owner/repo/contents/verification/results/abc999/a/main.json?ref="),
        "CAS url: {}",
        recorded[1].url
    );
    assert!(
        recorded[1]
            .url
            .ends_with(&format!("?ref={}", state_head_sha()))
    );

    // 2. POST blob.
    assert_eq!(recorded[2].method, "POST");
    assert_eq!(recorded[2].url, "/repos/owner/repo/git/blobs");
    let blob_body: serde_json::Value = serde_json::from_str(&recorded[2].body).unwrap();
    assert_eq!(blob_body["encoding"], "utf-8");
    let sent_content: &str = blob_body["content"].as_str().unwrap();
    // The blob content must round-trip to the same VerificationRecord.
    let round: VerificationRecord = serde_json::from_str(sent_content).unwrap();
    assert_eq!(round.attempt_id.as_str(), "attempt-1");

    // 3. GET commit → resolve state_head's tree.
    assert_eq!(recorded[3].method, "GET");
    assert_eq!(
        recorded[3].url,
        format!("/repos/owner/repo/git/commits/{}", state_head_sha())
    );

    // 4. POST tree — base_tree must be the resolved TREE sha, not the commit sha.
    assert_eq!(recorded[4].method, "POST");
    assert_eq!(recorded[4].url, "/repos/owner/repo/git/trees");
    let tree_body: serde_json::Value = serde_json::from_str(&recorded[4].body).unwrap();
    assert_eq!(tree_body["base_tree"], base_commit_tree_sha());
    let leaves = tree_body["tree"].as_array().unwrap();
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0]["path"], "verification/results/abc999/a/main.json");
    assert_eq!(leaves[0]["mode"], "100644");
    assert_eq!(leaves[0]["type"], "blob");
    assert_eq!(leaves[0]["sha"], blob_sha());

    // 5. POST commit with state_head as parent.
    assert_eq!(recorded[5].method, "POST");
    assert_eq!(recorded[5].url, "/repos/owner/repo/git/commits");
    let commit_body: serde_json::Value = serde_json::from_str(&recorded[5].body).unwrap();
    assert_eq!(commit_body["tree"], tree_sha());
    assert_eq!(commit_body["parents"][0], state_head_sha());
    assert!(
        commit_body["message"]
            .as_str()
            .unwrap()
            .contains("abc999/a/main")
    );

    // 6. PATCH ref.
    assert_eq!(recorded[6].method, "PATCH");
    assert_eq!(
        recorded[6].url,
        "/repos/owner/repo/git/refs/heads/automation/verify"
    );
    let patch_body: serde_json::Value = serde_json::from_str(&recorded[6].body).unwrap();
    assert_eq!(patch_body["sha"], commit_sha());
    assert_eq!(patch_body["force"], false);
}

#[test]
fn persist_fails_when_attempt_cas_mismatch() {
    // Server returns a record with a different attempt_id than the candidate
    // claims to replace. No further requests should be sent after the CAS
    // check (which is preceded by the mandatory state-branch head fetch).
    let remote = starting_record("attempt-remote", None);
    let script = vec![
        // 0. GET ref → state branch tip.
        get_ref_reply(state_head_sha()),
        // 1. CAS GET → existing record with different attempt_id.
        contents_response_for(&remote),
    ];
    let fx = Fixture::start(script);
    let w = writer(fx.base_url());

    let req = valid_request(starting_record("attempt-2", Some("attempt-someone-else")));
    let err = w.persist(&req).unwrap_err();
    match err {
        PersistError::AttemptCasMismatch { expected, actual } => {
            assert_eq!(expected.as_deref(), Some("attempt-someone-else"));
            assert_eq!(actual.as_deref(), Some("attempt-remote"));
        }
        other => panic!("expected AttemptCasMismatch, got {other:?}"),
    }

    let recorded = fx.recorded();
    assert_eq!(
        recorded.len(),
        2,
        "expected GET ref + GET contents, got {recorded:#?}"
    );
    assert_eq!(recorded[0].method, "GET");
    assert_eq!(
        recorded[0].url,
        "/repos/owner/repo/git/refs/heads/automation/verify"
    );
    assert_eq!(recorded[1].method, "GET");
    assert!(
        recorded[1]
            .url
            .ends_with(&format!("?ref={}", state_head_sha())),
        "CAS should target state branch tip, got {}",
        recorded[1].url,
    );
}

/// Helper: SHA of the branch head after a concurrent writer advanced it.
fn new_head_sha() -> &'static str {
    "5555555555555555555555555555555555555555"
}

fn new_head_tree_sha() -> &'static str {
    "6666666666666666666666666666666666666666"
}

fn rebuilt_tree_sha() -> &'static str {
    "7777777777777777777777777777777777777777"
}

fn rebuilt_commit_sha() -> &'static str {
    "8888888888888888888888888888888888888888"
}

#[test]
fn persist_retries_once_on_ref_update_conflict() {
    // First PATCH → 422. Writer refetches ref to learn the new HEAD, re-runs
    // the CAS check against that HEAD (still 404 = OK because we did not
    // expect a predecessor), resolves HEAD's tree, rebuilds the tree +
    // commit on that new parent, and re-PATCHes → 200.
    let script = vec![
        // 0. GET ref → state branch tip (used to anchor steps 1-5).
        get_ref_reply(state_head_sha()),
        // 1. CAS GET → 404 (result absent at state_head)
        Reply::empty(404),
        // 2. POST blob
        Reply::json(201, serde_json::json!({ "sha": blob_sha() })),
        // 3. GET commit → resolve state_head's tree
        Reply::json(
            200,
            serde_json::json!({
                "sha": state_head_sha(),
                "tree": { "sha": base_commit_tree_sha() }
            }),
        ),
        // 4. POST tree
        Reply::json(201, serde_json::json!({ "sha": tree_sha() })),
        // 5. POST commit
        Reply::json(201, serde_json::json!({ "sha": commit_sha() })),
        // 6. PATCH → 422 (non-fast-forward — someone else advanced the branch
        //    between step 0 and step 6).
        Reply::json(
            422,
            serde_json::json!({ "message": "Update is not a fast-forward" }),
        ),
        // 7. GET ref → new_head (the actual current tip after the race).
        Reply::json(
            200,
            serde_json::json!({
                "ref": "refs/heads/automation/verify",
                "object": { "sha": new_head_sha() }
            }),
        ),
        // 8. CAS re-check at new_head → 404 (still no record)
        Reply::empty(404),
        // 9. GET commit → resolve new_head's tree
        Reply::json(
            200,
            serde_json::json!({
                "sha": new_head_sha(),
                "tree": { "sha": new_head_tree_sha() }
            }),
        ),
        // 10. POST tree (rebuilt on new base_tree)
        Reply::json(201, serde_json::json!({ "sha": rebuilt_tree_sha() })),
        // 11. POST commit (parent = new_head)
        Reply::json(201, serde_json::json!({ "sha": rebuilt_commit_sha() })),
        // 12. PATCH → 200
        Reply::json(
            200,
            serde_json::json!({
                "ref": "refs/heads/automation/verify",
                "object": { "sha": rebuilt_commit_sha() }
            }),
        ),
    ];
    let fx = Fixture::start(script);
    let w = writer(fx.base_url());

    let req = valid_request(starting_record("attempt-1", None));
    let out = w.persist(&req).expect("persist succeeds after retry");

    // The returned SHAs reflect the rebuilt tree + commit, not the discarded
    // originals.
    assert_eq!(out.blob_sha, blob_sha());
    assert_eq!(out.tree_sha, rebuilt_tree_sha());
    assert_eq!(out.commit_sha, rebuilt_commit_sha());

    let recorded = fx.recorded();
    assert_eq!(
        recorded.len(),
        13,
        "expected 13 requests (step 0 anchor + 6 initial + 6 retry), got {recorded:#?}"
    );

    // 0. GET ref → state branch tip.
    assert_eq!(recorded[0].method, "GET");
    assert_eq!(
        recorded[0].url,
        "/repos/owner/repo/git/refs/heads/automation/verify"
    );

    // 1. CAS at state_head.
    assert_eq!(recorded[1].method, "GET");
    assert!(
        recorded[1]
            .url
            .starts_with("/repos/owner/repo/contents/verification/results/abc999/a/main.json?ref=")
    );
    assert!(
        recorded[1]
            .url
            .ends_with(&format!("?ref={}", state_head_sha()))
    );

    // 2. POST blob.
    assert_eq!(recorded[2].method, "POST");
    assert_eq!(recorded[2].url, "/repos/owner/repo/git/blobs");

    // 3. GET commit at state_head to resolve base tree.
    assert_eq!(recorded[3].method, "GET");
    assert_eq!(
        recorded[3].url,
        format!("/repos/owner/repo/git/commits/{}", state_head_sha())
    );

    // 4. POST tree.
    assert_eq!(recorded[4].method, "POST");
    assert_eq!(recorded[4].url, "/repos/owner/repo/git/trees");
    let tree1_body: serde_json::Value = serde_json::from_str(&recorded[4].body).unwrap();
    assert_eq!(tree1_body["base_tree"], base_commit_tree_sha());

    // 5. POST commit with state_head as parent.
    assert_eq!(recorded[5].method, "POST");
    assert_eq!(recorded[5].url, "/repos/owner/repo/git/commits");
    let commit1_body: serde_json::Value = serde_json::from_str(&recorded[5].body).unwrap();
    assert_eq!(commit1_body["tree"], tree_sha());
    assert_eq!(commit1_body["parents"][0], state_head_sha());

    // 6. PATCH → 422.
    assert_eq!(recorded[6].method, "PATCH");
    assert_eq!(
        recorded[6].url,
        "/repos/owner/repo/git/refs/heads/automation/verify"
    );
    let patch1_body: serde_json::Value = serde_json::from_str(&recorded[6].body).unwrap();
    assert_eq!(patch1_body["sha"], commit_sha());

    // Rebuild sequence.
    // 7. GET ref → new_head.
    assert_eq!(recorded[7].method, "GET");
    assert_eq!(
        recorded[7].url,
        "/repos/owner/repo/git/refs/heads/automation/verify"
    );

    // 8. CAS re-check at new_head.
    assert_eq!(recorded[8].method, "GET");
    assert!(
        recorded[8]
            .url
            .starts_with("/repos/owner/repo/contents/verification/results/abc999/a/main.json?ref="),
        "CAS refetch url: {}",
        recorded[8].url,
    );
    assert!(
        recorded[8]
            .url
            .ends_with(&format!("?ref={}", new_head_sha())),
        "CAS refetch should be at new_head, got: {}",
        recorded[8].url,
    );

    // 9. GET commit for new_head.
    assert_eq!(recorded[9].method, "GET");
    assert_eq!(
        recorded[9].url,
        format!("/repos/owner/repo/git/commits/{}", new_head_sha())
    );

    // 10. POST tree with new base_tree.
    assert_eq!(recorded[10].method, "POST");
    assert_eq!(recorded[10].url, "/repos/owner/repo/git/trees");
    let tree2_body: serde_json::Value = serde_json::from_str(&recorded[10].body).unwrap();
    assert_eq!(tree2_body["base_tree"], new_head_tree_sha());
    let leaves = tree2_body["tree"].as_array().unwrap();
    assert_eq!(leaves.len(), 1);
    // Same blob is reused — no new blob was created.
    assert_eq!(leaves[0]["sha"], blob_sha());

    // 11. POST commit with new_head as parent.
    assert_eq!(recorded[11].method, "POST");
    assert_eq!(recorded[11].url, "/repos/owner/repo/git/commits");
    let commit2_body: serde_json::Value = serde_json::from_str(&recorded[11].body).unwrap();
    assert_eq!(commit2_body["tree"], rebuilt_tree_sha());
    assert_eq!(commit2_body["parents"][0], new_head_sha());

    // 12. PATCH with rebuilt commit sha.
    assert_eq!(recorded[12].method, "PATCH");
    assert_eq!(
        recorded[12].url,
        "/repos/owner/repo/git/refs/heads/automation/verify"
    );
    let patch2_body: serde_json::Value = serde_json::from_str(&recorded[12].body).unwrap();
    assert_eq!(patch2_body["sha"], rebuilt_commit_sha());
    assert_eq!(patch2_body["force"], false);

    // No blob was recreated during the rebuild — only one blob POST total.
    let blob_posts = recorded
        .iter()
        .filter(|r| r.method == "POST" && r.url == "/repos/owner/repo/git/blobs")
        .count();
    assert_eq!(blob_posts, 1, "blob should not be recreated on rebuild");
}

#[test]
fn persist_conflict_becomes_cas_mismatch_when_new_head_holds_different_attempt() {
    // Setup: our candidate replaces "attempt-original". At base_sha the CAS
    // finds "attempt-original" (matches) and we proceed. The PATCH then 422s
    // because a concurrent writer advanced the branch. When we refetch the
    // head and re-run CAS against it, the record there has a *different*
    // attempt id from what we planned to replace — a genuine divergence, not
    // a race we can rebuild through — so the writer must surface it as
    // AttemptCasMismatch (not RefUpdateConflict) and MUST NOT PATCH again.
    let base_record = starting_record("attempt-original", None);
    let new_head_record = starting_record("attempt-conflicting", None);
    let script = vec![
        // 0. GET ref → state branch tip.
        get_ref_reply(state_head_sha()),
        // 1. CAS GET at state_head → matches expected predecessor.
        contents_response_for(&base_record),
        // 2. POST blob
        Reply::json(201, serde_json::json!({ "sha": blob_sha() })),
        // 3. GET commit → resolve state_head's tree
        Reply::json(
            200,
            serde_json::json!({
                "sha": state_head_sha(),
                "tree": { "sha": base_commit_tree_sha() }
            }),
        ),
        // 4. POST tree
        Reply::json(201, serde_json::json!({ "sha": tree_sha() })),
        // 5. POST commit
        Reply::json(201, serde_json::json!({ "sha": commit_sha() })),
        // 6. PATCH → 422
        Reply::json(422, serde_json::json!({ "message": "non-ff" })),
        // 7. GET ref → new_head
        Reply::json(
            200,
            serde_json::json!({
                "ref": "refs/heads/automation/verify",
                "object": { "sha": new_head_sha() }
            }),
        ),
        // 8. CAS re-check at new_head → record has a *different* attempt id
        //    from what our candidate expects to replace.
        contents_response_for(&new_head_record),
    ];
    let fx = Fixture::start(script);
    let w = writer(fx.base_url());

    let req = valid_request(starting_record("attempt-2", Some("attempt-original")));
    let err = w.persist(&req).unwrap_err();
    match err {
        PersistError::AttemptCasMismatch { expected, actual } => {
            assert_eq!(expected.as_deref(), Some("attempt-original"));
            assert_eq!(actual.as_deref(), Some("attempt-conflicting"));
        }
        other => panic!("expected AttemptCasMismatch, got {other:?}"),
    }

    let recorded = fx.recorded();
    // Exactly 9: initial GET ref + 6 through the first 422, then GET ref +
    // CAS re-check. No second PATCH, no tree/commit rebuild.
    assert_eq!(recorded.len(), 9, "expected 9 requests, got {recorded:#?}");
    let patch_count = recorded
        .iter()
        .filter(|r| {
            r.method == "PATCH" && r.url == "/repos/owner/repo/git/refs/heads/automation/verify"
        })
        .count();
    assert_eq!(patch_count, 1, "no rebuild PATCH should occur");
    // The refetch CAS must have queried at new_head (not state_head).
    assert!(
        recorded[8]
            .url
            .ends_with(&format!("?ref={}", new_head_sha())),
        "CAS refetch url: {}",
        recorded[8].url,
    );
}

#[test]
fn persist_fails_after_second_ref_conflict() {
    // First PATCH → 422. Full rebuild path runs (get_ref, CAS OK, resolve
    // tree, new tree, new commit), then the second PATCH also 422s.
    // Total 12 requests, terminal error is RefUpdateConflict — no further
    // retry.
    let script = vec![
        // 0. GET ref → state_head anchor.
        get_ref_reply(state_head_sha()),
        // 1. CAS GET → 404
        Reply::empty(404),
        // 2. POST blob
        Reply::json(201, serde_json::json!({ "sha": blob_sha() })),
        // 3. GET commit → base tree at state_head
        Reply::json(
            200,
            serde_json::json!({
                "sha": state_head_sha(),
                "tree": { "sha": base_commit_tree_sha() }
            }),
        ),
        // 4. POST tree
        Reply::json(201, serde_json::json!({ "sha": tree_sha() })),
        // 5. POST commit
        Reply::json(201, serde_json::json!({ "sha": commit_sha() })),
        // 6. PATCH → 422
        Reply::json(422, serde_json::json!({ "message": "non-ff" })),
        // 7. GET ref → new_head
        Reply::json(
            200,
            serde_json::json!({
                "ref": "refs/heads/automation/verify",
                "object": { "sha": new_head_sha() }
            }),
        ),
        // 8. CAS re-check at new_head → 404 (OK)
        Reply::empty(404),
        // 9. GET commit → new_head's tree
        Reply::json(
            200,
            serde_json::json!({
                "sha": new_head_sha(),
                "tree": { "sha": new_head_tree_sha() }
            }),
        ),
        // 10. POST tree
        Reply::json(201, serde_json::json!({ "sha": rebuilt_tree_sha() })),
        // 11. POST commit
        Reply::json(201, serde_json::json!({ "sha": rebuilt_commit_sha() })),
        // 12. PATCH → 422 again
        Reply::json(422, serde_json::json!({ "message": "non-ff" })),
    ];
    let fx = Fixture::start(script);
    let w = writer(fx.base_url());

    let req = valid_request(starting_record("attempt-1", None));
    let err = w.persist(&req).unwrap_err();
    assert!(
        matches!(err, PersistError::RefUpdateConflict),
        "got {err:?}"
    );

    let recorded = fx.recorded();
    assert_eq!(
        recorded.len(),
        13,
        "expected 13 requests (anchor + 6 initial + 6 retry), got {recorded:#?}"
    );
    // Exactly two PATCH attempts — no third retry.
    let patch_count = recorded
        .iter()
        .filter(|r| {
            r.method == "PATCH" && r.url == "/repos/owner/repo/git/refs/heads/automation/verify"
        })
        .count();
    assert_eq!(patch_count, 2, "expected exactly 2 PATCH attempts");
}

#[test]
fn set_pull_request_state_marks_draft() {
    let script = vec![Reply::json(
        200,
        serde_json::json!({ "number": 42, "draft": true }),
    )];
    let fx = Fixture::start(script);
    let w = writer(fx.base_url());
    w.bind_repository("owner/repo").expect("bind repo");

    w.set_pull_request_state(BotPullRequestState::Draft {
        pull_request_number: 42,
    })
    .expect("mark draft");

    let recorded = fx.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].method, "PATCH");
    assert_eq!(recorded[0].url, "/repos/owner/repo/pulls/42");
    let body: serde_json::Value = serde_json::from_str(&recorded[0].body).unwrap();
    assert_eq!(body["draft"], true);
}

#[test]
fn set_pull_request_state_ready_resolves_node_id_then_enables_auto_merge() {
    // GraphQL requires the PR's opaque base64-shaped node id, not the numeric
    // number. The writer must fetch `.node_id` from the REST endpoint first,
    // then PATCH `draft: false`, then POST the GraphQL mutation with the
    // resolved node id as the `pullRequestId` variable.
    let script = vec![
        // 1. GET /pulls/{n} → { node_id: "PR_kwDOTEST" }
        Reply::json(
            200,
            serde_json::json!({
                "number": 7,
                "node_id": "PR_kwDOTEST",
                "draft": true
            }),
        ),
        // 2. PATCH /pulls/{n} { draft: false }
        Reply::json(200, serde_json::json!({ "number": 7, "draft": false })),
        // 3. POST /graphql (enablePullRequestAutoMerge)
        Reply::json(
            200,
            serde_json::json!({
                "data": {
                    "enablePullRequestAutoMerge": { "clientMutationId": "ok" }
                }
            }),
        ),
    ];
    let fx = Fixture::start(script);
    let w = writer(fx.base_url());
    w.bind_repository("owner/repo").expect("bind repo");

    w.set_pull_request_state(BotPullRequestState::Ready {
        pull_request_number: 7,
        auto_merge: true,
    })
    .expect("mark ready + auto-merge");

    let recorded = fx.recorded();
    assert_eq!(recorded.len(), 3, "expected 3 requests, got {recorded:#?}");

    // First: GET pulls/7 to resolve the node id.
    assert_eq!(recorded[0].method, "GET");
    assert_eq!(recorded[0].url, "/repos/owner/repo/pulls/7");

    // Second: PATCH pulls/7 with { draft: false }
    assert_eq!(recorded[1].method, "PATCH");
    assert_eq!(recorded[1].url, "/repos/owner/repo/pulls/7");
    let body1: serde_json::Value = serde_json::from_str(&recorded[1].body).unwrap();
    assert_eq!(body1["draft"], false);

    // Third: POST /graphql with enablePullRequestAutoMerge, node id in vars.
    assert_eq!(recorded[2].method, "POST");
    assert_eq!(recorded[2].url, "/graphql");
    let body2: serde_json::Value = serde_json::from_str(&recorded[2].body).unwrap();
    let query = body2["query"].as_str().unwrap();
    assert!(
        query.contains("enablePullRequestAutoMerge"),
        "graphql body missing mutation: {query}"
    );
    // The mutation body must reference the resolved base64-shaped node id,
    // NOT the numeric PR number.
    let sent_id = body2["variables"]["pullRequestId"].as_str().unwrap();
    assert_eq!(
        sent_id, "PR_kwDOTEST",
        "graphql variable pullRequestId must be the resolved node_id (was: {sent_id})"
    );
    assert!(
        !sent_id.chars().all(|c| c.is_ascii_digit()),
        "pullRequestId variable must not be the raw numeric PR number (was: {sent_id})"
    );
}

#[test]
fn set_pull_request_state_ready_errors_when_graphql_returns_errors() {
    // GraphQL returns HTTP 200 even when the mutation itself failed — the
    // real signal lives in the response body's `errors` array. The writer
    // must inspect the body, surface a `GraphqlError`, and MUST NOT include
    // the raw upstream error text (which may echo repo/token metadata).
    let script = vec![
        // 1. GET /pulls/{n} → node_id
        Reply::json(
            200,
            serde_json::json!({
                "number": 7,
                "node_id": "PR_kwDOTEST",
                "draft": true
            }),
        ),
        // 2. PATCH /pulls/{n}
        Reply::json(200, serde_json::json!({ "number": 7, "draft": false })),
        // 3. POST /graphql — HTTP 200 but errors present in body
        Reply::json(
            200,
            serde_json::json!({
                "data": null,
                "errors": [{ "message": "internal repo secret_token_abc" }]
            }),
        ),
    ];
    let fx = Fixture::start(script);
    let w = writer(fx.base_url());
    w.bind_repository("owner/repo").expect("bind repo");

    let err = w
        .set_pull_request_state(BotPullRequestState::Ready {
            pull_request_number: 7,
            auto_merge: true,
        })
        .expect_err("GraphQL errors must surface as an Err, not a silent Ok(())");

    // Must be the dedicated GraphqlError variant, not a generic upstream 200.
    let display = format!("{err}");
    let debug = format!("{err:?}");
    match &err {
        PersistError::GraphqlError { op, count } => {
            assert_eq!(*count, 1);
            assert!(
                op.contains("enablePullRequestAutoMerge"),
                "expected op to reference the mutation, got {op:?}"
            );
        }
        other => panic!("expected GraphqlError, got {other:?}"),
    }

    // The mock GraphQL body contained a fake token — the Display and Debug
    // renderings of the error must NOT include it.
    assert!(
        !display.contains("secret_token_abc"),
        "Display leaked GraphQL body: {display}"
    );
    assert!(
        !debug.contains("secret_token_abc"),
        "Debug leaked GraphQL body: {debug}"
    );

    // Sanity check: the full 3-request sequence was actually issued (the
    // writer did not short-circuit before contacting graphql).
    let recorded = fx.recorded();
    assert_eq!(recorded.len(), 3, "expected 3 requests, got {recorded:#?}");
    assert_eq!(recorded[2].url, "/graphql");
}

#[test]
fn sanitized_errors_hide_response_body() {
    // Force the CAS GET to fail with a body containing a fake token; the
    // returned error must never surface the body. The step-0 GET ref reply
    // succeeds so the 500 lands on step 1 (the CAS GET) as intended — this
    // test targets the sanitisation on the contents-API path.
    let bad_body = "the internal error mentioned secret_token_abc in the trace".to_string();
    let script = vec![
        get_ref_reply(state_head_sha()),
        Reply {
            status: 500,
            body: bad_body.clone(),
        },
    ];
    let fx = Fixture::start(script);
    let w = writer(fx.base_url());

    let req = valid_request(starting_record("attempt-1", None));
    let err = w.persist(&req).unwrap_err();
    let display = format!("{err}");
    let debug = format!("{err:?}");
    assert!(
        !display.contains("secret_token_abc"),
        "Display leaked body: {display}"
    );
    assert!(
        !debug.contains("secret_token_abc"),
        "Debug leaked body: {debug}"
    );
    // The status code is fine to surface.
    assert!(display.contains("500"), "Display: {display}");
    match err {
        PersistError::UpstreamStatus { status, op } => {
            assert_eq!(status, 500);
            // Assert the failure targeted the CAS GET, not step 0.
            assert_eq!(op, "GET contents (cas)", "unexpected op: {op}");
        }
        other => panic!("expected UpstreamStatus, got {other:?}"),
    }
}

#[test]
fn token_is_never_logged_via_debug() {
    let w = GitHubVerificationStateWriter::new("http://127.0.0.1:1", SecretString::from("hunter2"));
    let debug = format!("{w:?}");
    assert!(!debug.contains("hunter2"), "Debug leaked token: {debug}");
    // The redaction marker from `secrecy` should appear.
    assert!(
        debug.contains("REDACTED"),
        "Debug missing redaction marker: {debug}"
    );
}

#[test]
fn find_or_open_bot_pr_returns_existing_when_list_non_empty() {
    let script = vec![Reply::json(
        200,
        serde_json::json!([{ "number": 42, "state": "open" }]),
    )];
    let fx = Fixture::start(script);
    let w = writer(fx.base_url());

    let n = w
        .find_or_open_bot_pr(
            "owner",
            "repo",
            "automation/verify",
            "main",
            "Automation: verification results",
            "body",
        )
        .expect("find existing PR");
    assert_eq!(n, 42);

    let recorded = fx.recorded();
    assert_eq!(
        recorded.len(),
        1,
        "expected only the list GET, got {recorded:#?}"
    );
    assert_eq!(recorded[0].method, "GET");
    assert_eq!(
        recorded[0].url,
        "/repos/owner/repo/pulls?head=owner:automation/verify&state=open&base=main"
    );
}

#[test]
fn find_or_open_bot_pr_opens_new_when_list_empty() {
    let script = vec![
        Reply::json(200, serde_json::json!([])),
        Reply::json(201, serde_json::json!({ "number": 99 })),
    ];
    let fx = Fixture::start(script);
    let w = writer(fx.base_url());

    let n = w
        .find_or_open_bot_pr(
            "owner",
            "repo",
            "automation/verify",
            "main",
            "Automation: verification results",
            "body",
        )
        .expect("open new PR");
    assert_eq!(n, 99);

    let recorded = fx.recorded();
    assert_eq!(recorded.len(), 2, "expected 2 requests, got {recorded:#?}");
    assert_eq!(recorded[0].method, "GET");
    assert_eq!(recorded[1].method, "POST");
    assert_eq!(recorded[1].url, "/repos/owner/repo/pulls");
    let body: serde_json::Value = serde_json::from_str(&recorded[1].body).unwrap();
    assert_eq!(body["draft"], true);
    assert_eq!(body["head"], "automation/verify");
    assert_eq!(body["base"], "main");
    assert_eq!(body["title"], "Automation: verification results");
}

#[test]
fn find_or_open_bot_pr_maps_non_2xx_to_upstream_status() {
    let script = vec![Reply::json(
        502,
        serde_json::json!({ "message": "bad gateway" }),
    )];
    let fx = Fixture::start(script);
    let w = writer(fx.base_url());

    let err = w
        .find_or_open_bot_pr(
            "owner",
            "repo",
            "automation/verify",
            "main",
            "title",
            "body",
        )
        .unwrap_err();
    match err {
        PersistError::UpstreamStatus { status, op } => {
            assert_eq!(status, 502);
            assert_eq!(op, "GET pulls?head (find bot pr)");
        }
        other => panic!("expected UpstreamStatus, got {other:?}"),
    }
}

#[test]
fn find_or_open_bot_pr_missing_number_maps_to_malformed_response() {
    // POST /pulls returns a body without `number`. The writer must surface
    // MalformedResponse rather than pretending everything succeeded.
    let script = vec![
        Reply::json(200, serde_json::json!([])),
        Reply::json(201, serde_json::json!({ "id": 1 })),
    ];
    let fx = Fixture::start(script);
    let w = writer(fx.base_url());

    let err = w
        .find_or_open_bot_pr(
            "owner",
            "repo",
            "automation/verify",
            "main",
            "title",
            "body",
        )
        .unwrap_err();
    match err {
        PersistError::MalformedResponse { op, field } => {
            assert_eq!(op, "POST pulls (open bot pr)");
            assert_eq!(field, "number");
        }
        other => panic!("expected MalformedResponse, got {other:?}"),
    }
}
