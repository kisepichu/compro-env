//! Integration tests for the submission lifecycle orchestrator.
//!
//! Every test exercises the module through its three public entry points
//! (`resume_pending`, `start_plan`, `poll_handle`) using in-memory fakes for
//! every port. The tests cover the checklist from
//! `docs/superpowers/plans/2026-08-10-library-verify-command.md`:
//! resume-per-state, recovery outcomes, persistence order, polling cadence,
//! `Retry-After`, error backoff, budget exhaustion, terminal refusal, duplicate
//! start, one-in-flight-per-OJ.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, FixedOffset, TimeZone, Utc};

use domain::entity::{OJKind, Session};
use domain::library::{LanguageId, LibraryId, SolutionId};
use domain::online_judge::{RecoveryMode, ResultDetail, SubmissionCapabilities, SubmissionMode};
use domain::verification::{
    AcceptanceUnknownState, AttemptId, ContentHash, ErrorKind, FailureStage, InfrastructureFailure,
    LanguageBinding, PendingState, StartingState, SubmissionHandle as DomainHandle, SubmittedState,
    UnavailableReason as DomainUnavailableReason, UnavailableState, VerificationRecord,
    VerificationState, VerifyFingerprint,
};

// Bring test-only imports separately so unused-warnings guide us if we drift.

use usecases::clock::Clock;
use usecases::repository::session_repository::SessionRepository;
use usecases::repository::verification_repository::VerificationRepository;
use usecases::submission::{
    InfrastructureErrorKind, JudgeResult, JudgeVerdict, PollObservation, PollSubmissionError,
    PollerRegistry, RecoverSubmissionError, RecoveryOutcome, RecoveryRegistry, RecoveryRequest,
    ResultDetailLevel, StartSubmissionError, StarterRegistry, SubmissionAdapterDescriptor,
    SubmissionHandle as PortHandle, SubmissionMode as PortMode, SubmissionPoller,
    SubmissionRecovery, SubmissionRequest, SubmissionStart, SubmissionStarter,
};
use usecases::submission_lifecycle::{
    NoRetryHint, PollEvent, PollingPolicy, RetryAfterHint, Sleeper, StartEvent, SubmissionPorts,
    VerificationRepositories, VerifySelection, poll_handle, resume_pending, start_plan,
};
use usecases::verification::plan::{SubmissionPlan, SubmissionPlanBody};

// ─── Fixtures ───────────────────────────────────────────────────────────────

fn fixed_offset_time(minutes: i64) -> DateTime<FixedOffset> {
    let base: DateTime<FixedOffset> =
        DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").expect("static rfc3339 parses");
    base + chrono::Duration::minutes(minutes)
}

fn utc_time(minutes: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 10, 9, 0, 0).unwrap() + chrono::Duration::minutes(minutes)
}

fn fp() -> VerifyFingerprint {
    VerifyFingerprint::parse(
        "sha256:1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap()
}

fn hash(byte: u8) -> ContentHash {
    let hex = format!("{:02x}", byte).repeat(32);
    ContentHash::parse(&format!("sha256:{hex}")).unwrap()
}

fn lc_solution() -> SolutionId {
    SolutionId::parse("librarychecker-aplusb/aplusb/main").unwrap()
}

fn lc_solution_b() -> SolutionId {
    SolutionId::parse("librarychecker-aplusb/aplusb/alt").unwrap()
}

fn atcoder_solution() -> SolutionId {
    SolutionId::parse("abc999/a/main").unwrap()
}

fn attempt(name: &str) -> AttemptId {
    AttemptId::parse(name).unwrap()
}

fn binding() -> LanguageBinding {
    LanguageBinding {
        language_id: LanguageId::parse("rust").unwrap(),
        oj_language_id: "rust".into(),
    }
}

fn make_starting_record(solution: &SolutionId, attempt_id: &str) -> VerificationRecord {
    VerificationRecord {
        schema_version: 1,
        solution_id: solution.clone(),
        attempt_id: attempt(attempt_id),
        replaces_attempt_id: None,
        fingerprint: fp(),
        state: VerificationState::Starting(StartingState {
            plan_hash: hash(0xaa),
            submitted_source_hash: hash(0xbb),
            language: binding(),
            started_at: fixed_offset_time(0),
        }),
        plan_context: None,
    }
}

fn make_acceptance_unknown(solution: &SolutionId, attempt_id: &str) -> VerificationRecord {
    VerificationRecord {
        schema_version: 1,
        solution_id: solution.clone(),
        attempt_id: attempt(attempt_id),
        replaces_attempt_id: None,
        fingerprint: fp(),
        state: VerificationState::AcceptanceUnknown(AcceptanceUnknownState {
            plan_hash: hash(0xaa),
            submitted_source_hash: hash(0xbb),
            language: binding(),
            started_at: fixed_offset_time(0),
            observed_at: fixed_offset_time(1),
            summary: "network drop after POST".into(),
        }),
        plan_context: None,
    }
}

fn make_domain_handle(oj: &str, id: &str) -> DomainHandle {
    DomainHandle {
        oj: oj.into(),
        submission_id: id.into(),
        submission_url: format!("https://example.test/{id}"),
        locator: None,
        submitted_at: fixed_offset_time(0),
    }
}

fn make_submitted(
    solution: &SolutionId,
    attempt_id: &str,
    oj: &str,
    sub_id: &str,
) -> VerificationRecord {
    VerificationRecord {
        schema_version: 1,
        solution_id: solution.clone(),
        attempt_id: attempt(attempt_id),
        replaces_attempt_id: None,
        fingerprint: fp(),
        state: VerificationState::Submitted(SubmittedState {
            handle: make_domain_handle(oj, sub_id),
            submitted_at: fixed_offset_time(0),
        }),
        plan_context: None,
    }
}

fn make_queued(
    solution: &SolutionId,
    attempt_id: &str,
    oj: &str,
    sub_id: &str,
) -> VerificationRecord {
    VerificationRecord {
        schema_version: 1,
        solution_id: solution.clone(),
        attempt_id: attempt(attempt_id),
        replaces_attempt_id: None,
        fingerprint: fp(),
        state: VerificationState::Queued(PendingState {
            handle: make_domain_handle(oj, sub_id),
            observed_at: fixed_offset_time(1),
        }),
        plan_context: None,
    }
}

fn make_judging(
    solution: &SolutionId,
    attempt_id: &str,
    oj: &str,
    sub_id: &str,
) -> VerificationRecord {
    VerificationRecord {
        schema_version: 1,
        solution_id: solution.clone(),
        attempt_id: attempt(attempt_id),
        replaces_attempt_id: None,
        fingerprint: fp(),
        state: VerificationState::Judging(PendingState {
            handle: make_domain_handle(oj, sub_id),
            observed_at: fixed_offset_time(1),
        }),
        plan_context: None,
    }
}

fn make_infra_failure(
    solution: &SolutionId,
    attempt_id: &str,
    handle: Option<DomainHandle>,
) -> VerificationRecord {
    VerificationRecord {
        schema_version: 1,
        solution_id: solution.clone(),
        attempt_id: attempt(attempt_id),
        replaces_attempt_id: None,
        fingerprint: fp(),
        state: VerificationState::InfrastructureFailure(InfrastructureFailure {
            stage: FailureStage::Poll,
            error_kind: ErrorKind::Network,
            retryable: true,
            retry_count: 1,
            next_retry_at: None,
            updated_at: fixed_offset_time(2),
            summary: "poll network drop".into(),
            plan_hash: Some(hash(0xaa)),
            handle,
        }),
        plan_context: None,
    }
}

fn make_plan(
    solution: &SolutionId,
    attempt_id: &str,
    oj: &str,
    contest_id: &str,
    problem_code: &str,
) -> SubmissionPlan {
    let body = SubmissionPlanBody {
        schema_version: 1,
        solution_id: solution.clone(),
        attempt_id: attempt(attempt_id),
        replaces_attempt_id: None,
        oj: oj.into(),
        contest_id: contest_id.into(),
        problem_code: problem_code.into(),
        language: binding(),
        submitted_source_path: "solutions/main.rs".into(),
        submitted_source_bytes: b"fn main() {}".to_vec(),
        submitted_source_hash: hash(0xcc),
        fingerprint: fp(),
        verifies: vec![LibraryId::parse("libraries/rust/algebra/monoid.rs").unwrap()],
        started_at: fixed_offset_time(0),
    };
    SubmissionPlan {
        body,
        plan_hash: hash(0xdd),
    }
}

// ─── Fakes ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum CallLog {
    RepoWrite {
        solution: String,
        attempt: String,
        state: &'static str,
    },
    Start,
    Poll,
    Recover,
    Sleep(Duration),
}

#[derive(Default)]
struct RecordingLog {
    entries: Mutex<Vec<CallLog>>,
}

impl RecordingLog {
    fn push(&self, e: CallLog) {
        self.entries.lock().unwrap().push(e);
    }
    fn snapshot(&self) -> Vec<CallLog> {
        self.entries.lock().unwrap().clone()
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

struct FakeRepo {
    inner: Mutex<HashMap<SolutionId, VerificationRecord>>,
    log: Arc<RecordingLog>,
}

impl FakeRepo {
    fn new(log: Arc<RecordingLog>) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            log,
        }
    }
    fn seed(&self, rec: VerificationRecord) {
        self.inner
            .lock()
            .unwrap()
            .insert(rec.solution_id.clone(), rec);
    }
}

impl VerificationRepository for FakeRepo {
    fn load(&self, id: &SolutionId) -> Result<Option<VerificationRecord>> {
        Ok(self.inner.lock().unwrap().get(id).cloned())
    }
    fn load_all(
        &self,
        discovered: &BTreeSet<SolutionId>,
    ) -> Result<BTreeMap<SolutionId, VerificationRecord>> {
        let map = self.inner.lock().unwrap();
        Ok(map
            .iter()
            .filter(|(k, _)| discovered.contains(*k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }
    fn compare_and_swap(
        &self,
        id: &SolutionId,
        expected: Option<&AttemptId>,
        next: &VerificationRecord,
    ) -> Result<()> {
        let mut map = self.inner.lock().unwrap();
        let current = map.get(id).cloned();
        match (expected, &current) {
            (None, None) => {}
            (Some(a), Some(existing)) if existing.attempt_id == *a => {}
            _ => anyhow::bail!("compare_and_swap precondition failed for {}", id.as_str()),
        }
        self.log.push(CallLog::RepoWrite {
            solution: id.as_str().to_string(),
            attempt: next.attempt_id.to_string(),
            state: state_label(&next.state),
        });
        map.insert(id.clone(), next.clone());
        Ok(())
    }
    fn remove_if_attempt(&self, id: &SolutionId, expected: &AttemptId) -> Result<()> {
        let mut map = self.inner.lock().unwrap();
        if let Some(existing) = map.get(id)
            && existing.attempt_id == *expected
        {
            map.remove(id);
        }
        Ok(())
    }
}

struct FakeSessionRepo;
impl SessionRepository for FakeSessionRepo {
    fn get(&self, _oj: &OJKind) -> Result<Option<Session>> {
        Ok(None)
    }
    fn save(&self, _session: &Session) -> Result<()> {
        Ok(())
    }
    fn delete(&self, _oj: &OJKind) -> Result<bool> {
        Ok(false)
    }
}

struct FakeClock(RefCell<DateTime<FixedOffset>>);
impl FakeClock {
    fn new(start: DateTime<FixedOffset>) -> Self {
        Self(RefCell::new(start))
    }
    fn advance(&self, dur: Duration) {
        let cur = *self.0.borrow();
        *self.0.borrow_mut() =
            cur + chrono::Duration::from_std(dur).expect("std duration converts");
    }
}
impl Clock for FakeClock {
    fn now(&self) -> DateTime<FixedOffset> {
        *self.0.borrow()
    }
}
// FakeClock uses RefCell (not thread-safe) but the orchestrator is single-threaded.
// Test framework requires Sync via unsafe impl.
unsafe impl Sync for FakeClock {}

struct RecordingSleeper {
    log: Arc<RecordingLog>,
    clock: Arc<FakeClock>,
}
impl Sleeper for RecordingSleeper {
    fn sleep(&self, dur: Duration) {
        self.log.push(CallLog::Sleep(dur));
        self.clock.advance(dur);
    }
}

struct FakeStarter {
    descriptor: SubmissionAdapterDescriptor,
    log: Arc<RecordingLog>,
    outcomes: Mutex<Vec<Result<SubmissionStart, StartSubmissionError>>>,
}
impl FakeStarter {
    fn new(
        log: Arc<RecordingLog>,
        outcomes: Vec<Result<SubmissionStart, StartSubmissionError>>,
    ) -> Self {
        Self {
            descriptor: SubmissionAdapterDescriptor {
                name: "fake".into(),
                version: "1".into(),
                submission_mode: PortMode::UnattendedTrackable,
                result_detail: ResultDetailLevel::TestcaseDetails,
                recovery_mode: usecases::submission::RecoveryMode::BestEffort,
            },
            log,
            outcomes: Mutex::new(outcomes),
        }
    }
}
impl SubmissionStarter for FakeStarter {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        self.descriptor.clone()
    }
    fn start_submission(
        &self,
        _request: &SubmissionRequest,
        _session: Option<&Session>,
    ) -> Result<SubmissionStart, StartSubmissionError> {
        self.log.push(CallLog::Start);
        let mut q = self.outcomes.lock().unwrap();
        if q.is_empty() {
            panic!("FakeStarter has no more scripted outcomes");
        }
        q.remove(0)
    }
}

struct FakePoller {
    descriptor: SubmissionAdapterDescriptor,
    log: Arc<RecordingLog>,
    observations: Mutex<Vec<Result<PollObservation, PollSubmissionError>>>,
    retry_after: Arc<Mutex<Option<Duration>>>,
    retry_after_script: Mutex<Vec<Option<Duration>>>,
}
impl FakePoller {
    fn new(
        log: Arc<RecordingLog>,
        observations: Vec<Result<PollObservation, PollSubmissionError>>,
    ) -> Self {
        Self {
            descriptor: SubmissionAdapterDescriptor {
                name: "fake".into(),
                version: "1".into(),
                submission_mode: PortMode::UnattendedTrackable,
                result_detail: ResultDetailLevel::TestcaseDetails,
                recovery_mode: usecases::submission::RecoveryMode::BestEffort,
            },
            log,
            observations: Mutex::new(observations),
            retry_after: Arc::new(Mutex::new(None)),
            retry_after_script: Mutex::new(vec![]),
        }
    }
    fn with_retry_after_script(mut self, script: Vec<Option<Duration>>) -> Self {
        self.retry_after_script = Mutex::new(script);
        self
    }
    fn retry_after_shared(&self) -> Arc<Mutex<Option<Duration>>> {
        Arc::clone(&self.retry_after)
    }
}
impl SubmissionPoller for FakePoller {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        self.descriptor.clone()
    }
    fn poll_submission(
        &self,
        _handle: &PortHandle,
        _session: Option<&Session>,
    ) -> Result<PollObservation, PollSubmissionError> {
        self.log.push(CallLog::Poll);
        // Update retry-after for the "last" observation from the script.
        let mut script = self.retry_after_script.lock().unwrap();
        if !script.is_empty() {
            *self.retry_after.lock().unwrap() = script.remove(0);
        }
        let mut q = self.observations.lock().unwrap();
        if q.is_empty() {
            panic!("FakePoller has no more scripted observations");
        }
        q.remove(0)
    }
}

/// A `RetryAfterHint` view backed by a shared `Arc<Mutex<Option<Duration>>>`
/// that the `FakePoller` writes to as it observes.
struct SharedRetryHint {
    shared: Arc<Mutex<Option<Duration>>>,
}
impl RetryAfterHint for SharedRetryHint {
    fn last_retry_after(&self, _oj: &OJKind) -> Option<Duration> {
        *self.shared.lock().unwrap()
    }
}

struct FakeRecovery {
    descriptor: SubmissionAdapterDescriptor,
    log: Arc<RecordingLog>,
    outcome: Mutex<Result<RecoveryOutcome, RecoverSubmissionError>>,
}
impl FakeRecovery {
    fn new(
        log: Arc<RecordingLog>,
        outcome: Result<RecoveryOutcome, RecoverSubmissionError>,
        recovery_mode: usecases::submission::RecoveryMode,
    ) -> Self {
        Self {
            descriptor: SubmissionAdapterDescriptor {
                name: "fake".into(),
                version: "1".into(),
                submission_mode: PortMode::UnattendedTrackable,
                result_detail: ResultDetailLevel::TestcaseDetails,
                recovery_mode,
            },
            log,
            outcome: Mutex::new(outcome),
        }
    }
}
impl SubmissionRecovery for FakeRecovery {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        self.descriptor.clone()
    }
    fn recover_submission(
        &self,
        _request: &RecoveryRequest,
        _session: Option<&Session>,
    ) -> Result<RecoveryOutcome, RecoverSubmissionError> {
        self.log.push(CallLog::Recover);
        // Clone via take-and-restore-a-copy is awkward; instead do a manual
        // clone via matching.
        let g = self.outcome.lock().unwrap();
        clone_recovery_outcome(&g)
    }
}

fn clone_recovery_outcome(
    r: &Result<RecoveryOutcome, RecoverSubmissionError>,
) -> Result<RecoveryOutcome, RecoverSubmissionError> {
    match r {
        Ok(o) => Ok(o.clone()),
        Err(e) => Err(e.clone()),
    }
}

// ─── Helper: assemble ports + repos ──────────────────────────────────────

struct Env {
    starters: StarterRegistry,
    pollers: PollerRegistry,
    recovery: RecoveryRegistry,
    sessions: FakeSessionRepo,
    clock: Arc<FakeClock>,
    sleeper: RecordingSleeper,
    retry_hint: Box<dyn RetryAfterHint>,
    policy: PollingPolicy,
}

impl Env {
    fn ports(&self) -> SubmissionPorts<'_> {
        SubmissionPorts {
            starters: &self.starters,
            pollers: &self.pollers,
            recovery: &self.recovery,
            sessions: &self.sessions,
            clock: self.clock.as_ref(),
            sleeper: &self.sleeper,
            retry_hint: self.retry_hint.as_ref(),
            policy: self.policy.clone(),
        }
    }
}

fn make_env(
    log: Arc<RecordingLog>,
    clock: Arc<FakeClock>,
    starter_outcomes_lc: Vec<Result<SubmissionStart, StartSubmissionError>>,
    poll_observations_lc: Vec<Result<PollObservation, PollSubmissionError>>,
    recovery_lc: Option<Result<RecoveryOutcome, RecoverSubmissionError>>,
    recovery_mode: usecases::submission::RecoveryMode,
    policy: PollingPolicy,
) -> Env {
    let mut starters = StarterRegistry::new();
    let starter = FakeStarter::new(Arc::clone(&log), starter_outcomes_lc);
    starters.register(OJKind::LibraryChecker, Box::new(starter));

    let mut pollers = PollerRegistry::new();
    let poller = FakePoller::new(Arc::clone(&log), poll_observations_lc);
    pollers.register(OJKind::LibraryChecker, Box::new(poller));

    let mut recovery = RecoveryRegistry::new();
    if let Some(r) = recovery_lc {
        recovery.register(
            OJKind::LibraryChecker,
            Box::new(FakeRecovery::new(Arc::clone(&log), r, recovery_mode)),
        );
    }
    let sleeper = RecordingSleeper {
        log: Arc::clone(&log),
        clock: Arc::clone(&clock),
    };
    Env {
        starters,
        pollers,
        recovery,
        sessions: FakeSessionRepo,
        clock,
        sleeper,
        retry_hint: Box::new(NoRetryHint),
        policy,
    }
}

fn repos<'a>(repo: &'a FakeRepo, solutions: &[SolutionId]) -> VerificationRepositories<'a> {
    VerificationRepositories {
        records: repo,
        known_solutions: solutions.iter().cloned().collect(),
    }
}

// ─── Tests: start_plan ─────────────────────────────────────────────────────

#[test]
fn start_plan_persists_starting_before_calling_starter() {
    // Spec §8.2 boundary: the Starting write MUST be visible before any OJ
    // contact so a crash before start_submission can be resumed.
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    let plan = make_plan(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "librarychecker-aplusb",
        "aplusb",
    );
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![Ok(SubmissionStart::Trackable {
            handle: PortHandle {
                online_judge: OJKind::LibraryChecker,
                submission_id: "42".into(),
                submission_url: "https://judge/42".into(),
                locator: None,
                submitted_at: utc_time(0),
            },
        })],
        vec![],
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let event = start_plan(&repos_bundle, &env.ports(), &plan).unwrap();
    let entries = log.snapshot();

    // The FIRST log entry MUST be the Starting write, and the second the
    // starter call. The third is the Submitted write.
    match &entries[0] {
        CallLog::RepoWrite { state, .. } => assert_eq!(*state, "Starting"),
        e => panic!("expected Starting write first, got {e:?}"),
    }
    assert_eq!(entries[1], CallLog::Start);
    match &entries[2] {
        CallLog::RepoWrite { state, .. } => assert_eq!(*state, "Submitted"),
        e => panic!("expected Submitted write second, got {e:?}"),
    }

    match event {
        StartEvent::Trackable { record } => {
            assert!(matches!(record.state, VerificationState::Submitted(_)));
        }
        e => panic!("unexpected start event {e:?}"),
    }
}

#[test]
fn start_plan_maps_acceptance_unknown_to_persisted_state() {
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    let plan = make_plan(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "librarychecker-aplusb",
        "aplusb",
    );
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![Err(StartSubmissionError::AcceptanceUnknown {
            summary: "network drop".into(),
        })],
        vec![],
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let event = start_plan(&repos_bundle, &env.ports(), &plan).unwrap();
    match event {
        StartEvent::AcceptanceUnknown { record } => {
            assert!(matches!(
                record.state,
                VerificationState::AcceptanceUnknown(_)
            ));
        }
        e => panic!("unexpected event {e:?}"),
    }
}

#[test]
fn start_plan_refuses_duplicate_start_on_same_attempt() {
    // Spec §8.2: a Starting record for the same attempt must not re-invoke
    // the starter — the same attempt ID must not be started twice.
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    let plan = make_plan(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "librarychecker-aplusb",
        "aplusb",
    );
    // Seed a Starting record for the same attempt.
    repo.seed(make_starting_record(&lc_solution(), "attempt-1"));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![],
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let err = start_plan(&repos_bundle, &env.ports(), &plan).unwrap_err();
    assert!(
        err.to_string().contains("start_plan called twice"),
        "message: {err}"
    );
}

#[test]
fn start_plan_refuses_when_record_is_past_starting() {
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    let plan = make_plan(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "librarychecker-aplusb",
        "aplusb",
    );
    // Seed a Submitted record for the same attempt.
    repo.seed(make_submitted(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "42",
    ));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![],
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let err = start_plan(&repos_bundle, &env.ports(), &plan).unwrap_err();
    assert!(err.to_string().contains("past Starting"), "message: {err}");
}

#[test]
fn start_plan_blocks_when_same_oj_has_another_in_flight_solution() {
    // Spec §8.3: one in-flight submission per OJ. If solution X is Queued for
    // LC, a fresh plan for solution Y on LC must be rejected.
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_queued(
        &lc_solution_b(),
        "other-attempt",
        "librarychecker",
        "77",
    ));
    let plan = make_plan(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "librarychecker-aplusb",
        "aplusb",
    );
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![],
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution(), lc_solution_b()]);
    let err = start_plan(&repos_bundle, &env.ports(), &plan).unwrap_err();
    assert!(err.to_string().contains("in-flight"), "message: {err}");
    // No write, no starter call.
    let entries = log.snapshot();
    assert!(entries.iter().all(|e| !matches!(e, CallLog::Start)));
}

#[test]
fn start_plan_on_a_different_oj_is_not_blocked() {
    // Same as above but the in-flight is on AtCoder — LC start must proceed.
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    // Seed AtCoder-owned in-flight (contest_id starts with abc → AtCoder).
    repo.seed(make_queued(
        &atcoder_solution(),
        "other-attempt",
        "atcoder",
        "77",
    ));
    let plan = make_plan(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "librarychecker-aplusb",
        "aplusb",
    );
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![Ok(SubmissionStart::Trackable {
            handle: PortHandle {
                online_judge: OJKind::LibraryChecker,
                submission_id: "42".into(),
                submission_url: "https://judge/42".into(),
                locator: None,
                submitted_at: utc_time(0),
            },
        })],
        vec![],
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution(), atcoder_solution()]);
    let event = start_plan(&repos_bundle, &env.ports(), &plan).expect("start_plan succeeds");
    assert!(matches!(event, StartEvent::Trackable { .. }));
}

// ─── Tests: poll_handle ────────────────────────────────────────────────────

#[test]
fn poll_handle_persists_completed_and_refuses_further_polls() {
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    // Seed Submitted so we can poll from Submitted → Completed.
    repo.seed(make_submitted(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "42",
    ));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![Ok(PollObservation::Completed(JudgeResult {
            verdict: JudgeVerdict::Accepted,
            testcases: vec![],
        }))],
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let record = repo.load(&lc_solution()).unwrap().unwrap();
    let event = poll_handle(&repos_bundle, &env.ports(), &record).unwrap();
    match event {
        PollEvent::Completed { record } => {
            assert!(matches!(record.state, VerificationState::Completed(_)));
        }
        e => panic!("unexpected event {e:?}"),
    }
    // Re-polling a terminal record must error.
    let terminal = repo.load(&lc_solution()).unwrap().unwrap();
    let err = poll_handle(&repos_bundle, &env.ports(), &terminal).unwrap_err();
    assert!(
        err.to_string().contains("terminal record"),
        "message: {err}"
    );
}

/// Regression: `CompletedState` must quote the plan's real
/// `submitted_source_hash` (and `language`) via `PlanContext`, not the zero
/// fallback (spec §11 "十分な証跡"). Without this the record would silently
/// persist an all-zero content hash whenever we complete from
/// Submitted/Queued/Judging (i.e. the normal happy path).
#[test]
fn poll_handle_completed_cites_plan_context_hash_and_language() {
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));

    // Seed a Submitted record whose only source of language / source-hash is
    // the top-level `plan_context` — matching what `start_plan` writes after
    // Starting → Submitted.
    let plan_hash = hash(0xbb);
    let plan_lang = binding();
    let mut seeded = make_submitted(&lc_solution(), "attempt-1", "librarychecker", "42");
    seeded.plan_context = Some(domain::verification::PlanContext {
        language: plan_lang.clone(),
        submitted_source_hash: plan_hash.clone(),
    });
    repo.seed(seeded);

    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![Ok(PollObservation::Completed(JudgeResult {
            verdict: JudgeVerdict::Accepted,
            testcases: vec![],
        }))],
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let record = repo.load(&lc_solution()).unwrap().unwrap();
    let ev = poll_handle(&repos_bundle, &env.ports(), &record).unwrap();
    match ev {
        PollEvent::Completed { record } => match record.state {
            VerificationState::Completed(c) => {
                assert_eq!(
                    c.submitted_source_hash, plan_hash,
                    "CompletedState.submitted_source_hash must come from PlanContext"
                );
                assert_eq!(
                    c.language, plan_lang,
                    "CompletedState.language must come from PlanContext"
                );
            }
            other => panic!("expected Completed, got {other:?}"),
        },
        other => panic!("expected PollEvent::Completed, got {other:?}"),
    }
}

#[test]
fn poll_handle_refuses_to_poll_a_terminal_unavailable_record() {
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    let mut rec = make_submitted(&lc_solution(), "attempt-1", "librarychecker", "42");
    rec.state = VerificationState::Unavailable(UnavailableState {
        reason: DomainUnavailableReason::InteractiveUntrackable,
        capabilities: SubmissionCapabilities {
            submission_mode: SubmissionMode::InteractiveUntrackable,
            result_detail: ResultDetail::OverallOnly,
            recovery_mode: RecoveryMode::None,
        },
        observed_at: fixed_offset_time(1),
        summary: "interactive only".into(),
    });
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![],
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let err = poll_handle(&repos_bundle, &env.ports(), &rec).unwrap_err();
    assert!(err.to_string().contains("terminal record"));
}

#[test]
fn poll_handle_cadence_starts_at_two_and_backs_off_to_fifteen() {
    // Spec §8.3: initial 2s, back off to max 15s across repeated pending.
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_submitted(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "42",
    ));
    // A run of pending states, then Completed at the end.
    let mut obs = vec![Ok(PollObservation::Queued); 6];
    obs.push(Ok(PollObservation::Completed(JudgeResult {
        verdict: JudgeVerdict::Accepted,
        testcases: vec![],
    })));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        obs,
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let record = repo.load(&lc_solution()).unwrap().unwrap();
    poll_handle(&repos_bundle, &env.ports(), &record).unwrap();
    let sleeps: Vec<Duration> = log
        .snapshot()
        .into_iter()
        .filter_map(|e| match e {
            CallLog::Sleep(d) => Some(d),
            _ => None,
        })
        .collect();
    // Expect exponential doubling capped at 15s: 2, 4, 8, 15, 15, 15
    assert_eq!(sleeps[0], Duration::from_secs(2));
    assert_eq!(sleeps[1], Duration::from_secs(4));
    assert_eq!(sleeps[2], Duration::from_secs(8));
    assert!(sleeps.iter().all(|d| *d <= Duration::from_secs(15)));
    // At least one 15s sleep once the cap is reached.
    assert!(sleeps.contains(&Duration::from_secs(15)));
}

#[test]
fn poll_handle_respects_retry_after_when_larger_than_backoff() {
    // Spec §8.3: honour Retry-After when longer than the computed backoff.
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_submitted(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "42",
    ));

    let mut starters = StarterRegistry::new();
    starters.register(
        OJKind::LibraryChecker,
        Box::new(FakeStarter::new(Arc::clone(&log), vec![])),
    );

    let poller = FakePoller::new(
        Arc::clone(&log),
        vec![
            Ok(PollObservation::Queued),
            Ok(PollObservation::Completed(JudgeResult {
                verdict: JudgeVerdict::Accepted,
                testcases: vec![],
            })),
        ],
    )
    .with_retry_after_script(vec![Some(Duration::from_secs(30)), None]);
    // Grab the shared retry-after cell BEFORE moving the poller into the box.
    let retry_shared = poller.retry_after_shared();

    let mut pollers = PollerRegistry::new();
    pollers.register(OJKind::LibraryChecker, Box::new(poller));

    let recovery = RecoveryRegistry::new();
    let sleeper = RecordingSleeper {
        log: Arc::clone(&log),
        clock: Arc::clone(&clock),
    };
    let hint = SharedRetryHint {
        shared: retry_shared,
    };

    let ports = SubmissionPorts {
        starters: &starters,
        pollers: &pollers,
        recovery: &recovery,
        sessions: &FakeSessionRepo,
        clock: clock.as_ref(),
        sleeper: &sleeper,
        retry_hint: &hint,
        policy: PollingPolicy::verify_defaults(),
    };
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let record = repo.load(&lc_solution()).unwrap().unwrap();
    poll_handle(&repos_bundle, &ports, &record).unwrap();
    let sleeps: Vec<Duration> = log
        .snapshot()
        .into_iter()
        .filter_map(|e| match e {
            CallLog::Sleep(d) => Some(d),
            _ => None,
        })
        .collect();
    // First sleep should be 30s (Retry-After) instead of 2s.
    assert_eq!(sleeps[0], Duration::from_secs(30));
}

#[test]
fn poll_handle_infrastructure_error_backoff_caps_at_thirty_seconds() {
    // Spec §8.3: exponential backoff on transient infra errors capped at 30s.
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_submitted(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "42",
    ));
    // A run of retryable infra errors, then Completed.
    let mut obs: Vec<Result<PollObservation, PollSubmissionError>> = vec![];
    for _ in 0..8 {
        obs.push(Err(PollSubmissionError::Infrastructure {
            kind: InfrastructureErrorKind::Network,
            summary: "flaky".into(),
        }));
    }
    obs.push(Ok(PollObservation::Completed(JudgeResult {
        verdict: JudgeVerdict::Accepted,
        testcases: vec![],
    })));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        obs,
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let record = repo.load(&lc_solution()).unwrap().unwrap();
    poll_handle(&repos_bundle, &env.ports(), &record).unwrap();
    let sleeps: Vec<Duration> = log
        .snapshot()
        .into_iter()
        .filter_map(|e| match e {
            CallLog::Sleep(d) => Some(d),
            _ => None,
        })
        .collect();
    // None of the infra-error sleeps exceed the 30-second cap.
    assert!(
        sleeps.iter().all(|d| *d <= Duration::from_secs(30)),
        "sleeps: {sleeps:?}"
    );
    // The last sleep before recovery should be at the cap.
    assert!(sleeps.contains(&Duration::from_secs(30)));
}

/// Regression: a non-retryable poll-time infra failure MUST persist the
/// current handle, so `resume_pending` can drive the record forward on the
/// next tick (spec §8.3). Without the handle, resume would classify the
/// failure as operator-only and never re-poll.
#[test]
fn poll_handle_non_retryable_infra_error_preserves_handle_for_resume() {
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_submitted(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "42",
    ));
    let obs: Vec<Result<PollObservation, PollSubmissionError>> =
        vec![Err(PollSubmissionError::Infrastructure {
            kind: InfrastructureErrorKind::AuthenticationRejected,
            summary: "credentials rejected".into(),
        })];
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        obs,
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let record = repo.load(&lc_solution()).unwrap().unwrap();
    let ev = poll_handle(&repos_bundle, &env.ports(), &record).unwrap();
    match ev {
        PollEvent::InfrastructureError { record } => match record.state {
            VerificationState::InfrastructureFailure(f) => {
                assert!(
                    f.handle.is_some(),
                    "InfrastructureFailure must carry the handle so resume can re-poll"
                );
                assert_eq!(
                    f.handle.as_ref().map(|h| h.submission_id.as_str()),
                    Some("42"),
                );
            }
            other => panic!("expected InfrastructureFailure, got {other:?}"),
        },
        other => panic!("expected PollEvent::InfrastructureError, got {other:?}"),
    }
}

#[test]
fn poll_handle_exhausts_15_minute_budget() {
    // Spec §8.3: hard 15-minute wall-clock budget. When exceeded, the record
    // must persist at the last observed pending state and BudgetExhausted is
    // returned.
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_submitted(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "42",
    ));
    // Never-ending pending stream.
    let obs: Vec<Result<PollObservation, PollSubmissionError>> =
        (0..500).map(|_| Ok(PollObservation::Queued)).collect();
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        obs,
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let record = repo.load(&lc_solution()).unwrap().unwrap();
    let event = poll_handle(&repos_bundle, &env.ports(), &record).unwrap();
    match event {
        PollEvent::BudgetExhausted { record } => {
            assert!(matches!(
                record.state,
                VerificationState::Queued(_) | VerificationState::Judging(_)
            ));
        }
        e => panic!("unexpected event {e:?}"),
    }
}

// ─── Tests: resume_pending ─────────────────────────────────────────────────

#[test]
fn resume_pending_drives_starting_via_recovered_handle() {
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_starting_record(&lc_solution(), "attempt-1"));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![Ok(PollObservation::Completed(JudgeResult {
            verdict: JudgeVerdict::Accepted,
            testcases: vec![],
        }))],
        Some(Ok(RecoveryOutcome::Recovered {
            handle: PortHandle {
                online_judge: OJKind::LibraryChecker,
                submission_id: "42".into(),
                submission_url: "https://judge/42".into(),
                locator: None,
                submitted_at: utc_time(0),
            },
        })),
        usecases::submission::RecoveryMode::Exact,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let summary = resume_pending(
        &repos_bundle,
        &env.ports(),
        &VerifySelection {
            solutions: vec![lc_solution()],
        },
    )
    .unwrap();
    assert!(summary.terminal.contains(&lc_solution()));
    assert!(summary.in_flight_ojs.is_empty());
}

#[test]
fn resume_pending_starting_confirmed_not_accepted_marks_replan() {
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_starting_record(&lc_solution(), "attempt-1"));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![],
        Some(Ok(RecoveryOutcome::ConfirmedNotAccepted)),
        usecases::submission::RecoveryMode::Exact,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let summary = resume_pending(
        &repos_bundle,
        &env.ports(),
        &VerifySelection {
            solutions: vec![lc_solution()],
        },
    )
    .unwrap();
    assert_eq!(summary.replan_candidates, vec![lc_solution()]);
    // Record is NOT removed by this module (spec: caller decides).
    assert!(repo.load(&lc_solution()).unwrap().is_some());
}

#[test]
fn resume_pending_starting_ambiguous_recovery_moves_to_acceptance_unknown() {
    // best_effort recovery returning AcceptanceUnknown means we cannot prove
    // non-acceptance; module persists AcceptanceLost transition and records
    // an operator action.
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_starting_record(&lc_solution(), "attempt-1"));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![],
        Some(Ok(RecoveryOutcome::AcceptanceUnknown)),
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let summary = resume_pending(
        &repos_bundle,
        &env.ports(),
        &VerifySelection {
            solutions: vec![lc_solution()],
        },
    )
    .unwrap();
    assert_eq!(summary.operator_actions.len(), 1);
    let final_rec = repo.load(&lc_solution()).unwrap().unwrap();
    assert!(matches!(
        final_rec.state,
        VerificationState::AcceptanceUnknown(_)
    ));
    assert!(summary.in_flight_ojs.contains(&OJKind::LibraryChecker));
}

#[test]
fn resume_pending_acceptance_unknown_stays_when_recovery_ambiguous() {
    // For an existing AU record, ambiguous recovery leaves the state as-is
    // (spec: no re-transition to itself) and records an operator action.
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_acceptance_unknown(&lc_solution(), "attempt-1"));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![],
        Some(Ok(RecoveryOutcome::AcceptanceUnknown)),
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let summary = resume_pending(
        &repos_bundle,
        &env.ports(),
        &VerifySelection {
            solutions: vec![lc_solution()],
        },
    )
    .unwrap();
    assert!(!summary.operator_actions.is_empty());
    // State unchanged.
    let rec = repo.load(&lc_solution()).unwrap().unwrap();
    assert!(matches!(rec.state, VerificationState::AcceptanceUnknown(_)));
}

#[test]
fn resume_pending_recovery_infra_error_records_operator_action() {
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_starting_record(&lc_solution(), "attempt-1"));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![],
        Some(Err(RecoverSubmissionError::Infrastructure {
            kind: InfrastructureErrorKind::CredentialsMissing,
            summary: "no session".into(),
        })),
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let summary = resume_pending(
        &repos_bundle,
        &env.ports(),
        &VerifySelection {
            solutions: vec![lc_solution()],
        },
    )
    .unwrap();
    assert!(!summary.operator_actions.is_empty());
    let rec = repo.load(&lc_solution()).unwrap().unwrap();
    assert!(matches!(
        rec.state,
        VerificationState::InfrastructureFailure(_)
    ));
}

#[test]
fn resume_pending_drives_submitted_forward_to_completed() {
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_submitted(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "42",
    ));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![Ok(PollObservation::Completed(JudgeResult {
            verdict: JudgeVerdict::Accepted,
            testcases: vec![],
        }))],
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let summary = resume_pending(
        &repos_bundle,
        &env.ports(),
        &VerifySelection {
            solutions: vec![lc_solution()],
        },
    )
    .unwrap();
    assert!(summary.terminal.contains(&lc_solution()));
}

#[test]
fn resume_pending_drives_queued_forward_to_completed() {
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_queued(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "42",
    ));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![Ok(PollObservation::Completed(JudgeResult {
            verdict: JudgeVerdict::Accepted,
            testcases: vec![],
        }))],
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let summary = resume_pending(
        &repos_bundle,
        &env.ports(),
        &VerifySelection {
            solutions: vec![lc_solution()],
        },
    )
    .unwrap();
    assert!(summary.terminal.contains(&lc_solution()));
}

#[test]
fn resume_pending_drives_judging_forward_to_completed() {
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_judging(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "42",
    ));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![Ok(PollObservation::Completed(JudgeResult {
            verdict: JudgeVerdict::Accepted,
            testcases: vec![],
        }))],
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let summary = resume_pending(
        &repos_bundle,
        &env.ports(),
        &VerifySelection {
            solutions: vec![lc_solution()],
        },
    )
    .unwrap();
    assert!(summary.terminal.contains(&lc_solution()));
}

#[test]
fn resume_pending_drives_infrastructure_failure_with_handle_forward() {
    // Spec §8.3: handle acquired InfrastructureFailure resumes via poll.
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_infra_failure(
        &lc_solution(),
        "attempt-1",
        Some(make_domain_handle("librarychecker", "42")),
    ));
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        vec![Ok(PollObservation::Completed(JudgeResult {
            verdict: JudgeVerdict::Accepted,
            testcases: vec![],
        }))],
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let summary = resume_pending(
        &repos_bundle,
        &env.ports(),
        &VerifySelection {
            solutions: vec![lc_solution()],
        },
    )
    .unwrap();
    assert!(summary.terminal.contains(&lc_solution()));
}

#[test]
fn resume_pending_records_in_flight_when_budget_exhausted() {
    // Spec §8.3: budget-exhausted resume must leave the OJ in the
    // in-flight set so no new starts happen.
    let log = Arc::new(RecordingLog::default());
    let clock = Arc::new(FakeClock::new(fixed_offset_time(0)));
    let repo = FakeRepo::new(Arc::clone(&log));
    repo.seed(make_submitted(
        &lc_solution(),
        "attempt-1",
        "librarychecker",
        "42",
    ));
    let obs: Vec<Result<PollObservation, PollSubmissionError>> =
        (0..500).map(|_| Ok(PollObservation::Queued)).collect();
    let env = make_env(
        Arc::clone(&log),
        Arc::clone(&clock),
        vec![],
        obs,
        None,
        usecases::submission::RecoveryMode::BestEffort,
        PollingPolicy::verify_defaults(),
    );
    let repos_bundle = repos(&repo, &[lc_solution()]);
    let summary = resume_pending(
        &repos_bundle,
        &env.ports(),
        &VerifySelection {
            solutions: vec![lc_solution()],
        },
    )
    .unwrap();
    assert!(summary.in_flight_ojs.contains(&OJKind::LibraryChecker));
}
