//! Integration tests for `ce verify` at the controller layer (spec §7.2,
//! §8, §8.1, §10). Each test spins a temp repository, injects fake OJ ports,
//! and drives `Controller::verify` end-to-end.

#![cfg(unix)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, FixedOffset, Utc};

use domain::analysis::{
    AnalysisSnapshot, AnalysisState, DiscoveredLanguage, DiscoveryManifest, LibraryFile,
    NormalizedLanguageAnalysis, NormalizedLibraryAnalysis, NormalizedSolutionAnalysis,
    TargetAnalysisState,
};
use domain::entity::{OJKind, Session};
use domain::library::{
    AnalyzerConfig, DEFAULT_TIMEOUT_SECONDS, LanguageConfig, LanguageId, LibraryId,
    LibraryProjectConfig, OnlineJudgeLanguageMapping, SiteConfig, SolutionId,
};
use domain::solution::{PublishedSolution, VerifySpec};
use domain::verification::{
    AttemptId, ContentHash, LanguageBinding, PendingState, SubmissionHandle as DomainHandle,
    VerdictKind, VerificationRecord, VerificationState, VerifyFingerprint,
};

use infrastructure::command_runner_impl::UnixCommandRunner;
use infrastructure::repository_impl::verification_repository_impl::VerificationRepositoryImpl;
use infrastructure::repository_impl::{
    contest_repository_impl::ContestRepositoryImpl, session_repository_impl::SessionRepositoryImpl,
    solution_repository_impl::SolutionRepositoryImpl,
};
use interfaces::controller::Controller;
use interfaces::controller::input::VerifyInput;
use usecases::clock::Clock;
use usecases::config::Config;
use usecases::id_generator::SequenceIdGenerator;
use usecases::online_judge::{
    ContestMeta, CredentialKind, Credentials, OnlineJudge, OnlineJudgeRegistry,
};
use usecases::service::verify::{VerifyOutcome, VerifyStatus, VerifyStatusLine};
use usecases::service::{Service, VerificationServices};
use usecases::submission::{
    JudgeResult, JudgeVerdict, PollObservation, PollSubmissionError, PollerRegistry,
    RecoverSubmissionError, RecoveryMode, RecoveryOutcome, RecoveryRegistry, RecoveryRequest,
    ResultDetailLevel, StartSubmissionError, StarterRegistry, SubmissionAdapterDescriptor,
    SubmissionHandle as PortHandle, SubmissionMode, SubmissionPoller, SubmissionRecovery,
    SubmissionRequest, SubmissionStart, SubmissionStarter, UnavailableReason,
};
use usecases::submission_lifecycle::{NoRetryHint, PollingPolicy, Sleeper};

// ─── Fixture builders ─────────────────────────────────────────────────────

const LC_SOLUTION: &str = "librarychecker-aplusb/aplusb/main";
const LC_SOLUTION_B: &str = "librarychecker-aplusb/other/main";
const AT_SOLUTION: &str = "abc999/a/main";

fn lc_id() -> SolutionId {
    SolutionId::parse(LC_SOLUTION).unwrap()
}
fn lc_id_b() -> SolutionId {
    SolutionId::parse(LC_SOLUTION_B).unwrap()
}
fn at_id() -> SolutionId {
    SolutionId::parse(AT_SOLUTION).unwrap()
}

fn rust_lang() -> LanguageId {
    LanguageId::parse("rust").unwrap()
}

fn library_id() -> LibraryId {
    LibraryId::parse("libraries/rust/algebra/monoid.rs").unwrap()
}

fn fixed_time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-08-11T09:00:00+00:00").unwrap()
}

/// Assemble the full temp-repo tree the verify command expects. Returns the
/// tempdir so callers can hold it alive.
fn make_repo(
    with_lc: bool,
    with_atcoder: bool,
    with_lc_b: bool,
    lc_test_command: &str,
) -> (
    tempfile::TempDir,
    PathBuf,
    LibraryProjectConfig,
    DiscoveryManifest,
) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    std::fs::create_dir_all(root.join("templates/rust")).unwrap();
    std::fs::create_dir_all(root.join("libraries/rust/algebra")).unwrap();
    std::fs::write(
        root.join("libraries/rust/algebra/monoid.rs"),
        b"pub trait Monoid {}\n",
    )
    .unwrap();

    let mut solutions = vec![];
    if with_lc {
        let sol_root = root.join(format!("solutions/{LC_SOLUTION}"));
        std::fs::create_dir_all(sol_root.join("src")).unwrap();
        std::fs::write(
            sol_root.join("src/main.rs"),
            b"fn main(){println!(\"lc\");}",
        )
        .unwrap();
        solutions.push(PublishedSolution {
            id: lc_id(),
            language: rust_lang(),
            root: format!("solutions/{LC_SOLUTION}"),
            entry: "src/main.rs".into(),
            solved_at: fixed_time(),
            test_command: lc_test_command.into(),
            test_timeout_seconds: 30,
            verify: Some(VerifySpec {
                libraries: vec![library_id()],
                oj_language_id: "rust".into(),
            }),
        });
    }
    if with_lc_b {
        let sol_root = root.join(format!("solutions/{LC_SOLUTION_B}"));
        std::fs::create_dir_all(sol_root.join("src")).unwrap();
        std::fs::write(
            sol_root.join("src/main.rs"),
            b"fn main(){println!(\"lcb\");}",
        )
        .unwrap();
        solutions.push(PublishedSolution {
            id: lc_id_b(),
            language: rust_lang(),
            root: format!("solutions/{LC_SOLUTION_B}"),
            entry: "src/main.rs".into(),
            solved_at: fixed_time(),
            test_command: "true".into(),
            test_timeout_seconds: 30,
            verify: Some(VerifySpec {
                libraries: vec![library_id()],
                oj_language_id: "rust".into(),
            }),
        });
    }
    if with_atcoder {
        let sol_root = root.join(format!("solutions/{AT_SOLUTION}"));
        std::fs::create_dir_all(sol_root.join("src")).unwrap();
        std::fs::write(
            sol_root.join("src/main.rs"),
            b"fn main(){println!(\"ac\");}",
        )
        .unwrap();
        solutions.push(PublishedSolution {
            id: at_id(),
            language: rust_lang(),
            root: format!("solutions/{AT_SOLUTION}"),
            entry: "src/main.rs".into(),
            solved_at: fixed_time(),
            test_command: "true".into(),
            test_timeout_seconds: 30,
            verify: Some(VerifySpec {
                libraries: vec![library_id()],
                oj_language_id: "5054".into(),
            }),
        });
    }

    // Build minimal LibraryProjectConfig.
    let mut languages = BTreeMap::new();
    let mut oj_map = BTreeMap::new();
    oj_map.insert(
        "librarychecker".into(),
        OnlineJudgeLanguageMapping {
            language_id: "rust".into(),
        },
    );
    oj_map.insert(
        "atcoder".into(),
        OnlineJudgeLanguageMapping {
            language_id: "5054".into(),
        },
    );
    languages.insert(
        rust_lang(),
        LanguageConfig {
            id: rust_lang(),
            display_name: Some("Rust".into()),
            root: "libraries/rust".into(),
            include: vec!["**/*.rs".into()],
            exclude: vec![],
            // Real language check: `true` (a POSIX no-op that exits 0).
            check_command: Some("true".into()),
            check_timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            syntax_highlight: Some("rust".into()),
            analyzer: AnalyzerConfig {
                command: vec!["./adapter".into()],
                timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            },
            expected_toolchains: vec![],
            online_judges: oj_map,
            entry_file: "src/main.rs".into(),
        },
    );
    let config = LibraryProjectConfig {
        languages,
        site: Some(SiteConfig {
            title: "t".into(),
            description: "d".into(),
            language: "en".into(),
            repository_url: "https://example.test".into(),
        }),
    };

    let mut manifest_langs = BTreeMap::new();
    manifest_langs.insert(
        rust_lang(),
        DiscoveredLanguage {
            id: rust_lang(),
            root: "libraries/rust".into(),
            display_name: "Rust".into(),
            description_path: None,
            analyzer_command: vec!["./adapter".into()],
        },
    );
    let manifest = DiscoveryManifest {
        languages: manifest_langs,
        libraries: vec![LibraryFile {
            id: library_id(),
            language: rust_lang(),
            source_path: "libraries/rust/algebra/monoid.rs".into(),
            description_path: None,
            published: true,
            managed: true,
            title: Some("Monoid".into()),
        }],
        solutions,
        diagnostics: vec![],
    };
    (tmp, root, config, manifest)
}

fn make_snapshot(manifest: &DiscoveryManifest) -> AnalysisSnapshot {
    let lang_id = rust_lang();
    let mut libraries = BTreeMap::new();
    for lib in &manifest.libraries {
        libraries.insert(
            lib.id.clone(),
            NormalizedLibraryAnalysis {
                id: lib.id.clone(),
                state: TargetAnalysisState {
                    dependency_state: AnalysisState::Complete,
                    symbol_state: AnalysisState::Complete,
                },
                direct_dependencies: vec![],
                symbols: vec![],
                diagnostics: vec![],
            },
        );
    }
    let mut solutions = BTreeMap::new();
    for sol in &manifest.solutions {
        solutions.insert(
            sol.id.clone(),
            NormalizedSolutionAnalysis {
                solution_id: sol.id.clone(),
                dependency_state: AnalysisState::Complete,
                direct_dependencies: vec![],
                diagnostics: vec![],
            },
        );
    }
    let mut languages = BTreeMap::new();
    languages.insert(
        lang_id.clone(),
        NormalizedLanguageAnalysis {
            language: lang_id,
            adapter_name: "fake".into(),
            adapter_version: "0.0.1".into(),
            observed_toolchains: vec![],
            analyzer_command: vec!["./adapter".into()],
            libraries,
            solutions,
        },
    );
    AnalysisSnapshot {
        schema_version: 1,
        repository_revision: "rev".into(),
        created_at: fixed_time(),
        discovery_hash: "d".into(),
        source_hashes: BTreeMap::new(),
        languages,
        snapshot_hash: "h".into(),
    }
}

// ─── Fake ports ───────────────────────────────────────────────────────────

struct StubOJ;
impl OnlineJudge for StubOJ {
    fn name(&self) -> &str {
        "stub"
    }
    fn credential_kind(&self) -> CredentialKind {
        CredentialKind::Cookie
    }
    fn login(&self, _: &Credentials) -> Result<Session> {
        unimplemented!()
    }
    fn whoami(&self, _: &Session) -> Result<String> {
        Ok(String::new())
    }
    fn get_contest_meta(&self, _: &str) -> Result<ContestMeta> {
        unimplemented!()
    }
    fn get_problems_detail(
        &self,
        _: &str,
        _: Option<&Session>,
        _: &[(String, String)],
    ) -> Result<Vec<domain::entity::Problem>> {
        unimplemented!()
    }
}

struct StubOJRegistry;
impl OnlineJudgeRegistry for StubOJRegistry {
    fn get(&self, _oj: &OJKind) -> Result<&dyn OnlineJudge> {
        static OJ: StubOJ = StubOJ;
        Ok(&OJ)
    }
}

struct StubConfig;
impl Config for StubConfig {
    fn default_language(&self) -> Result<domain::entity::Language> {
        Ok(domain::entity::Language::new("rust"))
    }
    fn default_online_judge(&self) -> OJKind {
        OJKind::LibraryChecker
    }
    fn submit_file(&self, _: &domain::entity::Language) -> String {
        "src/main.rs".into()
    }
    fn submit_preprocess(&self) -> Option<String> {
        None
    }
    fn lang_id(&self, _: &domain::entity::Language, _: &OJKind) -> Option<String> {
        None
    }
}

// A `Sleeper` that records durations but does not actually sleep.
struct NoopSleeper {
    durations: Arc<Mutex<Vec<Duration>>>,
}
impl Sleeper for NoopSleeper {
    fn sleep(&self, dur: Duration) {
        self.durations.lock().unwrap().push(dur);
    }
}
impl NoopSleeper {
    fn new() -> Self {
        Self {
            durations: Arc::new(Mutex::new(vec![])),
        }
    }
}

// Fake starter: scripted outcomes + call counter. When the outcomes queue is
// exhausted, `default` is returned repeatedly — this keeps tests with an
// unknown number of solutions from having to precount.
struct FakeStarter {
    outcomes: Mutex<Vec<Result<SubmissionStart, StartSubmissionError>>>,
    default: Mutex<Option<Box<dyn Fn() -> Result<SubmissionStart, StartSubmissionError> + Send>>>,
    mode: SubmissionMode,
    calls: Arc<Mutex<u32>>,
}
impl FakeStarter {
    fn trackable_always() -> Self {
        Self {
            outcomes: Mutex::new(vec![]),
            default: Mutex::new(Some(Box::new(|| {
                Ok(SubmissionStart::Trackable {
                    handle: PortHandle {
                        online_judge: OJKind::LibraryChecker,
                        submission_id: "42".into(),
                        submission_url: "https://judge/42".into(),
                        locator: None,
                        submitted_at: Utc::now(),
                    },
                })
            }))),
            mode: SubmissionMode::UnattendedTrackable,
            calls: Arc::new(Mutex::new(0)),
        }
    }
    fn atcoder_unavailable() -> Self {
        Self {
            outcomes: Mutex::new(vec![]),
            default: Mutex::new(Some(Box::new(|| {
                Ok(SubmissionStart::Unavailable {
                    reason: UnavailableReason::InteractiveUntrackable,
                })
            }))),
            mode: SubmissionMode::InteractiveUntrackable,
            calls: Arc::new(Mutex::new(0)),
        }
    }
    fn calls_shared(&self) -> Arc<Mutex<u32>> {
        Arc::clone(&self.calls)
    }
}
impl SubmissionStarter for FakeStarter {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        SubmissionAdapterDescriptor {
            name: match self.mode {
                SubmissionMode::UnattendedTrackable => "fake-lc".into(),
                _ => "fake-at".into(),
            },
            version: "1".into(),
            submission_mode: self.mode.clone(),
            result_detail: ResultDetailLevel::TestcaseDetails,
            recovery_mode: RecoveryMode::BestEffort,
        }
    }
    fn start_submission(
        &self,
        _request: &SubmissionRequest,
        _session: Option<&Session>,
    ) -> Result<SubmissionStart, StartSubmissionError> {
        *self.calls.lock().unwrap() += 1;
        let mut q = self.outcomes.lock().unwrap();
        if !q.is_empty() {
            return q.remove(0);
        }
        let guard = self.default.lock().unwrap();
        match guard.as_ref() {
            Some(f) => f(),
            None => panic!("FakeStarter exhausted"),
        }
    }
}

struct FakePoller {
    outcomes: Mutex<Vec<Result<PollObservation, PollSubmissionError>>>,
    default: PollObservation,
}
impl FakePoller {
    fn always_queued() -> Self {
        Self {
            outcomes: Mutex::new(vec![]),
            default: PollObservation::Queued,
        }
    }
    fn always_completed(verdict: JudgeVerdict) -> Self {
        Self {
            outcomes: Mutex::new(vec![]),
            default: PollObservation::Completed(JudgeResult {
                verdict,
                testcases: vec![],
            }),
        }
    }
}
impl SubmissionPoller for FakePoller {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        SubmissionAdapterDescriptor {
            name: "fake-poller".into(),
            version: "1".into(),
            submission_mode: SubmissionMode::UnattendedTrackable,
            result_detail: ResultDetailLevel::TestcaseDetails,
            recovery_mode: RecoveryMode::BestEffort,
        }
    }
    fn poll_submission(
        &self,
        _handle: &PortHandle,
        _session: Option<&Session>,
    ) -> Result<PollObservation, PollSubmissionError> {
        let mut q = self.outcomes.lock().unwrap();
        if q.is_empty() {
            Ok(self.default.clone())
        } else {
            q.remove(0)
        }
    }
}

struct FakeRecovery {
    outcome: Mutex<RecoveryOutcome>,
}
impl FakeRecovery {
    fn always(outcome: RecoveryOutcome) -> Self {
        Self {
            outcome: Mutex::new(outcome),
        }
    }
}
impl SubmissionRecovery for FakeRecovery {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        SubmissionAdapterDescriptor {
            name: "fake-rec".into(),
            version: "1".into(),
            submission_mode: SubmissionMode::UnattendedTrackable,
            result_detail: ResultDetailLevel::TestcaseDetails,
            recovery_mode: RecoveryMode::BestEffort,
        }
    }
    fn recover_submission(
        &self,
        _request: &RecoveryRequest,
        _session: Option<&Session>,
    ) -> Result<RecoveryOutcome, RecoverSubmissionError> {
        Ok(self.outcome.lock().unwrap().clone())
    }
}

// ─── Test scaffolding ────────────────────────────────────────────────────

struct TestEnv {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    config: LibraryProjectConfig,
    manifest: DiscoveryManifest,
    snapshot: AnalysisSnapshot,
    controller: Controller,
    starter_calls: BTreeMap<String, Arc<Mutex<u32>>>,
    _sleeper_durations: Arc<Mutex<Vec<Duration>>>,
}

struct BuildEnv {
    with_lc: bool,
    with_atcoder: bool,
    with_lc_b: bool,
    lc_test_command: String,
    lc_starter: Option<FakeStarter>,
    lc_poller: Option<FakePoller>,
    lc_recovery: Option<FakeRecovery>,
    atcoder_starter: Option<FakeStarter>,
    atcoder_poller: Option<FakePoller>,
    atcoder_recovery: Option<FakeRecovery>,
}

impl BuildEnv {
    fn default_lc_only() -> Self {
        Self {
            with_lc: true,
            with_atcoder: false,
            with_lc_b: false,
            lc_test_command: "true".into(),
            lc_starter: None,
            lc_poller: None,
            lc_recovery: None,
            atcoder_starter: None,
            atcoder_poller: None,
            atcoder_recovery: None,
        }
    }
}

fn build_env(spec: BuildEnv) -> TestEnv {
    let (tmp, root, config, manifest) = make_repo(
        spec.with_lc,
        spec.with_atcoder,
        spec.with_lc_b,
        &spec.lc_test_command,
    );
    let snapshot = make_snapshot(&manifest);

    let mut starters = StarterRegistry::new();
    let mut pollers = PollerRegistry::new();
    let mut recovery = RecoveryRegistry::new();
    let mut starter_calls: BTreeMap<String, Arc<Mutex<u32>>> = BTreeMap::new();

    if spec.with_lc || spec.with_lc_b {
        let starter = spec
            .lc_starter
            .unwrap_or_else(FakeStarter::trackable_always);
        starter_calls.insert("lc".into(), starter.calls_shared());
        starters.register(OJKind::LibraryChecker, Box::new(starter));
        pollers.register(
            OJKind::LibraryChecker,
            Box::new(
                spec.lc_poller
                    .unwrap_or_else(|| FakePoller::always_completed(JudgeVerdict::Accepted)),
            ),
        );
        recovery.register(
            OJKind::LibraryChecker,
            Box::new(
                spec.lc_recovery
                    .unwrap_or_else(|| FakeRecovery::always(RecoveryOutcome::AcceptanceUnknown)),
            ),
        );
    }
    if spec.with_atcoder {
        let starter = spec
            .atcoder_starter
            .unwrap_or_else(FakeStarter::atcoder_unavailable);
        starter_calls.insert("at".into(), starter.calls_shared());
        starters.register(OJKind::AtCoder, Box::new(starter));
        if let Some(p) = spec.atcoder_poller {
            pollers.register(OJKind::AtCoder, Box::new(p));
        }
        if let Some(r) = spec.atcoder_recovery {
            recovery.register(OJKind::AtCoder, Box::new(r));
        }
    }

    let verification = VerificationServices {
        pollers,
        recovery,
        verifications: Box::new(VerificationRepositoryImpl::new(root.clone())),
    };
    let service = Service::with_verification(
        Box::new(StubOJRegistry),
        starters,
        Box::new(ContestRepositoryImpl::new(root.clone())),
        Box::new(SolutionRepositoryImpl::new(root.clone())),
        Box::new(SessionRepositoryImpl),
        Box::new(StubConfig),
        Box::new(UnixCommandRunner),
        verification,
    );
    let controller = Controller::new(service);
    let sleeper_durations = Arc::new(Mutex::new(vec![]));
    TestEnv {
        _tmp: tmp,
        root,
        config,
        manifest,
        snapshot,
        controller,
        starter_calls,
        _sleeper_durations: sleeper_durations,
    }
}

/// Invokes `Controller::verify` with the given selection and returns the
/// outcome. The sleeper never actually blocks so the tests stay fast even
/// when the fake poller returns Queued forever.
fn run_verify(env: &TestEnv, selection: Option<&str>) -> Result<VerifyOutcome> {
    let selection = selection.map(|s| s.to_string());
    let ids = SequenceIdGenerator::new("test");
    let sleeper = NoopSleeper::new();
    env.controller.verify(
        &SelectionInput { selection },
        &env.root,
        &env.config,
        &env.manifest,
        &env.snapshot,
        &TestClock::new(),
        &ids,
        &sleeper,
        &NoRetryHint,
        // Fast policy so BudgetExhausted arrives within a handful of iterations
        // (interval 1s, budget 3s).
        PollingPolicy {
            initial_interval: Duration::from_millis(1),
            max_interval: Duration::from_millis(1),
            max_error_backoff: Duration::from_millis(1),
            total_budget: Duration::from_millis(50),
        },
    )
}

struct SelectionInput {
    selection: Option<String>,
}
impl VerifyInput for SelectionInput {
    fn solution(&self) -> Option<String> {
        self.selection.clone()
    }
}

/// A clock that advances 5 ms per call so the polling budget elapses in
/// bounded iterations without depending on wall-clock time.
struct TestClock {
    now: RefCell<DateTime<FixedOffset>>,
}
impl TestClock {
    fn new() -> Self {
        Self {
            now: RefCell::new(fixed_time()),
        }
    }
}
impl Clock for TestClock {
    fn now(&self) -> DateTime<FixedOffset> {
        let cur = *self.now.borrow();
        let next = cur + chrono::Duration::milliseconds(5);
        *self.now.borrow_mut() = next;
        cur
    }
}
// TestClock uses RefCell (single-threaded); the harness is single-threaded.
unsafe impl Sync for TestClock {}

fn find_status<'a>(outcome: &'a VerifyOutcome, id: &SolutionId) -> Option<&'a VerifyStatusLine> {
    outcome.statuses.iter().find(|s| &s.solution_id == id)
}

// ─── Tests ────────────────────────────────────────────────────────────────

/// 1. `verify` walks the entire manifest by default. LC → Accepted, AtCoder →
/// Unavailable. Overall exit 1 due to Unavailable.
#[test]
fn verify_bulk_mixed_ojs_exits_1_due_to_atcoder_unavailable() {
    let env = build_env(BuildEnv {
        with_atcoder: true,
        ..BuildEnv::default_lc_only()
    });
    let out = run_verify(&env, None).unwrap();
    let lc = find_status(&out, &lc_id()).unwrap();
    assert!(
        matches!(lc.status, VerifyStatus::Verified),
        "lc status: {:?}",
        lc.status
    );
    let at = find_status(&out, &at_id()).unwrap();
    assert!(
        matches!(at.status, VerifyStatus::Unavailable { .. }),
        "at status: {:?}",
        at.status
    );
    assert_eq!(out.exit_code(), 1);
}

/// 2. Single-target selection ignores AtCoder → exit 0.
#[test]
fn verify_single_target_lc_only_exits_0() {
    let env = build_env(BuildEnv {
        with_atcoder: true,
        ..BuildEnv::default_lc_only()
    });
    let out = run_verify(&env, Some(LC_SOLUTION)).unwrap();
    let lc = find_status(&out, &lc_id()).unwrap();
    assert!(matches!(lc.status, VerifyStatus::Verified));
    // AtCoder solution should NOT appear.
    assert!(find_status(&out, &at_id()).is_none());
    assert_eq!(out.exit_code(), 0);
}

/// 3. Existing terminal Accepted record with a matching fingerprint skips the
/// starter entirely.
#[test]
fn verify_skips_when_stored_record_matches_fingerprint() {
    // We first do a real accepted run to seed the on-disk record + fingerprint.
    let env1 = build_env(BuildEnv::default_lc_only());
    run_verify(&env1, None).unwrap();

    // Second env reuses the same root but with a starter that panics if
    // called. Reading the same repo produces the same fingerprint, so the
    // second run must skip.
    let (tmp, root, config, manifest) = (env1._tmp, env1.root, env1.config, env1.manifest);
    let snapshot = env1.snapshot;
    let mut starters = StarterRegistry::new();
    let starter = FakeStarter {
        outcomes: Mutex::new(vec![]),
        default: Mutex::new(None),
        mode: SubmissionMode::UnattendedTrackable,
        calls: Arc::new(Mutex::new(0)),
    };
    let calls = starter.calls_shared();
    starters.register(OJKind::LibraryChecker, Box::new(starter));
    let mut pollers = PollerRegistry::new();
    pollers.register(
        OJKind::LibraryChecker,
        Box::new(FakePoller::always_completed(JudgeVerdict::Accepted)),
    );
    let mut recovery = RecoveryRegistry::new();
    recovery.register(
        OJKind::LibraryChecker,
        Box::new(FakeRecovery::always(RecoveryOutcome::AcceptanceUnknown)),
    );
    let service = Service::with_verification(
        Box::new(StubOJRegistry),
        starters,
        Box::new(ContestRepositoryImpl::new(root.clone())),
        Box::new(SolutionRepositoryImpl::new(root.clone())),
        Box::new(SessionRepositoryImpl),
        Box::new(StubConfig),
        Box::new(UnixCommandRunner),
        VerificationServices {
            pollers,
            recovery,
            verifications: Box::new(VerificationRepositoryImpl::new(root.clone())),
        },
    );
    let controller = Controller::new(service);
    let out = controller
        .verify(
            &SelectionInput { selection: None },
            &root,
            &config,
            &manifest,
            &snapshot,
            &TestClock::new(),
            &SequenceIdGenerator::new("skip"),
            &NoopSleeper::new(),
            &NoRetryHint,
            PollingPolicy::verify_defaults(),
        )
        .unwrap();
    drop(tmp);
    assert_eq!(*calls.lock().unwrap(), 0, "starter must NOT be called");
    assert_eq!(out.exit_code(), 0);
}

/// 4. Stable ordering: two LC solutions both verified → status lines in
/// ascending solution-id order.
#[test]
fn verify_status_lines_are_sorted_by_solution_id() {
    let env = build_env(BuildEnv {
        with_lc_b: true,
        ..BuildEnv::default_lc_only()
    });
    let out = run_verify(&env, None).unwrap();
    let ids: Vec<_> = out
        .statuses
        .iter()
        .map(|s| s.solution_id.as_str().to_string())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "statuses must be sorted by solution id");
}

/// 5. Resume-first: seed a Starting record for LC. Fake recovery says
/// Recovered → poller returns Completed. Starter's start-count stays 0.
#[test]
fn verify_resumes_starting_record_before_launching_new_work() {
    let env = build_env(BuildEnv::default_lc_only());
    // Seed a Starting record on disk BEFORE the run.
    let repo = VerificationRepositoryImpl::new(env.root.clone());
    use usecases::repository::verification_repository::VerificationRepository;
    let starting = seed_starting_record(&lc_id());
    repo.compare_and_swap(&lc_id(), None, &starting).unwrap();

    // New env for this test with a recovery that reports Recovered and a
    // starter with 0 outcomes (must never be called).
    let mut recovery = RecoveryRegistry::new();
    recovery.register(
        OJKind::LibraryChecker,
        Box::new(FakeRecovery::always(RecoveryOutcome::Recovered {
            handle: PortHandle {
                online_judge: OJKind::LibraryChecker,
                submission_id: "999".into(),
                submission_url: "https://judge/999".into(),
                locator: None,
                submitted_at: Utc::now(),
            },
        })),
    );
    let mut starters = StarterRegistry::new();
    let starter = FakeStarter {
        outcomes: Mutex::new(vec![]),
        default: Mutex::new(None),
        mode: SubmissionMode::UnattendedTrackable,
        calls: Arc::new(Mutex::new(0)),
    };
    let calls = starter.calls_shared();
    starters.register(OJKind::LibraryChecker, Box::new(starter));
    let mut pollers = PollerRegistry::new();
    pollers.register(
        OJKind::LibraryChecker,
        Box::new(FakePoller::always_completed(JudgeVerdict::Accepted)),
    );
    let service = Service::with_verification(
        Box::new(StubOJRegistry),
        starters,
        Box::new(ContestRepositoryImpl::new(env.root.clone())),
        Box::new(SolutionRepositoryImpl::new(env.root.clone())),
        Box::new(SessionRepositoryImpl),
        Box::new(StubConfig),
        Box::new(UnixCommandRunner),
        VerificationServices {
            pollers,
            recovery,
            verifications: Box::new(VerificationRepositoryImpl::new(env.root.clone())),
        },
    );
    let controller = Controller::new(service);
    let out = controller
        .verify(
            &SelectionInput { selection: None },
            &env.root,
            &env.config,
            &env.manifest,
            &env.snapshot,
            &TestClock::new(),
            &SequenceIdGenerator::new("resume"),
            &NoopSleeper::new(),
            &NoRetryHint,
            PollingPolicy {
                initial_interval: Duration::from_millis(1),
                max_interval: Duration::from_millis(1),
                max_error_backoff: Duration::from_millis(1),
                total_budget: Duration::from_millis(50),
            },
        )
        .unwrap();
    assert_eq!(*calls.lock().unwrap(), 0, "starter must not be called");
    // Verified after resume → poll to completion.
    let lc = find_status(&out, &lc_id()).unwrap();
    assert!(
        matches!(lc.status, VerifyStatus::Verified),
        "lc status: {:?}",
        lc.status
    );
    assert_eq!(out.exit_code(), 0);
}

fn seed_starting_record(solution: &SolutionId) -> VerificationRecord {
    use domain::verification::StartingState;
    VerificationRecord {
        schema_version: 1,
        solution_id: solution.clone(),
        attempt_id: AttemptId::parse("attempt-seed").unwrap(),
        replaces_attempt_id: None,
        fingerprint: VerifyFingerprint::parse(
            "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        )
        .unwrap(),
        state: VerificationState::Starting(StartingState {
            plan_hash: ContentHash::parse(
                "sha256:2222222222222222222222222222222222222222222222222222222222222222",
            )
            .unwrap(),
            submitted_source_hash: ContentHash::parse(
                "sha256:3333333333333333333333333333333333333333333333333333333333333333",
            )
            .unwrap(),
            language: LanguageBinding {
                language_id: rust_lang(),
                oj_language_id: "rust".into(),
            },
            started_at: fixed_time(),
        }),
        plan_context: None,
    }
}

/// 6. Test-command barrier: LC's `test_command = "false"` blocks starter.
#[test]
fn verify_blocks_start_when_solution_test_fails() {
    let env = build_env(BuildEnv {
        lc_test_command: "false".into(),
        ..BuildEnv::default_lc_only()
    });
    let out = run_verify(&env, None).unwrap();
    let calls = env.starter_calls["lc"].lock().unwrap();
    assert_eq!(*calls, 0);
    let lc = find_status(&out, &lc_id()).unwrap();
    assert!(
        matches!(lc.status, VerifyStatus::TestFailed { .. }),
        "{:?}",
        lc.status
    );
    assert_eq!(out.exit_code(), 1);
}

/// 7. AtCoder-only run: overall exit 1 with Unavailable.
#[test]
fn verify_atcoder_only_returns_unavailable_exit_1() {
    let env = build_env(BuildEnv {
        with_lc: false,
        with_atcoder: true,
        ..BuildEnv::default_lc_only()
    });
    let out = run_verify(&env, None).unwrap();
    let at = find_status(&out, &at_id()).unwrap();
    assert!(matches!(at.status, VerifyStatus::Unavailable { .. }));
    assert_eq!(out.exit_code(), 1);
}

/// 8. Rejected: fake poller returns WA. Record persists rejected; exit 1.
#[test]
fn verify_rejected_verdict_returns_exit_1() {
    let env = build_env(BuildEnv {
        lc_poller: Some(FakePoller::always_completed(JudgeVerdict::WrongAnswer)),
        ..BuildEnv::default_lc_only()
    });
    let out = run_verify(&env, None).unwrap();
    let lc = find_status(&out, &lc_id()).unwrap();
    assert!(
        matches!(lc.status, VerifyStatus::Rejected { .. }),
        "{:?}",
        lc.status
    );
    assert_eq!(out.exit_code(), 1);
    // Verify the record persisted as Completed(rejected).
    let repo = VerificationRepositoryImpl::new(env.root.clone());
    use usecases::repository::verification_repository::VerificationRepository;
    let stored = repo.load(&lc_id()).unwrap().unwrap();
    match stored.state {
        VerificationState::Completed(c) => {
            assert_eq!(c.verdict.kind, VerdictKind::WrongAnswer);
        }
        other => panic!("expected Completed, got {other:?}"),
    }
}

/// 9. Budget exhausted then resume: first run polls forever (BudgetExhausted).
/// Second run's poller returns Accepted → exit 0.
#[test]
fn verify_budget_exhausted_then_resume_to_accepted() {
    let env = build_env(BuildEnv {
        lc_poller: Some(FakePoller::always_queued()),
        ..BuildEnv::default_lc_only()
    });
    let out = run_verify(&env, None).unwrap();
    let lc = find_status(&out, &lc_id()).unwrap();
    assert!(
        matches!(lc.status, VerifyStatus::Pending { .. }),
        "{:?}",
        lc.status
    );
    assert_eq!(out.exit_code(), 1);

    // Second run — set up a new controller with a poller that accepts.
    let (root, config, manifest, snapshot) = (env.root, env.config, env.manifest, env.snapshot);
    let mut starters = StarterRegistry::new();
    let starter = FakeStarter {
        outcomes: Mutex::new(vec![]),
        default: Mutex::new(None),
        mode: SubmissionMode::UnattendedTrackable,
        calls: Arc::new(Mutex::new(0)),
    };
    starters.register(OJKind::LibraryChecker, Box::new(starter));
    let mut pollers = PollerRegistry::new();
    pollers.register(
        OJKind::LibraryChecker,
        Box::new(FakePoller::always_completed(JudgeVerdict::Accepted)),
    );
    let mut recovery = RecoveryRegistry::new();
    recovery.register(
        OJKind::LibraryChecker,
        Box::new(FakeRecovery::always(RecoveryOutcome::AcceptanceUnknown)),
    );
    let service = Service::with_verification(
        Box::new(StubOJRegistry),
        starters,
        Box::new(ContestRepositoryImpl::new(root.clone())),
        Box::new(SolutionRepositoryImpl::new(root.clone())),
        Box::new(SessionRepositoryImpl),
        Box::new(StubConfig),
        Box::new(UnixCommandRunner),
        VerificationServices {
            pollers,
            recovery,
            verifications: Box::new(VerificationRepositoryImpl::new(root.clone())),
        },
    );
    let controller = Controller::new(service);
    let out2 = controller
        .verify(
            &SelectionInput { selection: None },
            &root,
            &config,
            &manifest,
            &snapshot,
            &TestClock::new(),
            &SequenceIdGenerator::new("resume2"),
            &NoopSleeper::new(),
            &NoRetryHint,
            PollingPolicy {
                initial_interval: Duration::from_millis(1),
                max_interval: Duration::from_millis(1),
                max_error_backoff: Duration::from_millis(1),
                total_budget: Duration::from_millis(50),
            },
        )
        .unwrap();
    let lc = find_status(&out2, &lc_id()).unwrap();
    assert!(
        matches!(lc.status, VerifyStatus::Verified),
        "{:?}",
        lc.status
    );
    assert_eq!(out2.exit_code(), 0);
}

/// 10. One in-flight per OJ: seed a Queued record for LC solution A; verify
/// solution B (same OJ) → starter is NOT called for B; A resumes to Completed.
#[test]
fn verify_one_in_flight_per_oj_blocks_second_solution() {
    // Repo has both LC solutions but the second (LC_SOLUTION_B) is the target.
    let env = build_env(BuildEnv {
        with_lc_b: true,
        // Poller must accept so seeded A completes cleanly.
        lc_poller: Some(FakePoller::always_completed(JudgeVerdict::Accepted)),
        ..BuildEnv::default_lc_only()
    });
    // Seed a Queued record for LC A (main).
    let repo = VerificationRepositoryImpl::new(env.root.clone());
    use usecases::repository::verification_repository::VerificationRepository;
    let queued_record = seed_queued_record(&lc_id());
    repo.compare_and_swap(&lc_id(), None, &queued_record)
        .unwrap();

    let out = run_verify(&env, Some(LC_SOLUTION_B)).unwrap();
    // Starter should not be invoked for B (OJ blocked).
    let starter_calls = env.starter_calls["lc"].lock().unwrap();
    assert_eq!(*starter_calls, 0);
    // Solution B should either be OjBlocked or absent from targeting result.
    let b = find_status(&out, &lc_id_b()).unwrap();
    assert!(
        matches!(b.status, VerifyStatus::OjBlocked { .. }),
        "b status: {:?}",
        b.status
    );
    // Solution A should have advanced to Completed on disk.
    let stored = repo.load(&lc_id()).unwrap().unwrap();
    match stored.state {
        VerificationState::Completed(_) => {}
        other => panic!("expected Completed for A, got {other:?}"),
    }
}

fn seed_queued_record(solution: &SolutionId) -> VerificationRecord {
    VerificationRecord {
        schema_version: 1,
        solution_id: solution.clone(),
        attempt_id: AttemptId::parse("queued-seed").unwrap(),
        replaces_attempt_id: None,
        fingerprint: VerifyFingerprint::parse(
            "sha256:9999999999999999999999999999999999999999999999999999999999999999",
        )
        .unwrap(),
        state: VerificationState::Queued(PendingState {
            handle: DomainHandle {
                oj: "librarychecker".into(),
                submission_id: "77".into(),
                submission_url: "https://judge/77".into(),
                locator: None,
                submitted_at: fixed_time(),
            },
            observed_at: fixed_time(),
        }),
        plan_context: None,
    }
}

/// 11. Hidden internal verify-prepare writes a plan JSON that round-trips;
/// verify-start on that plan succeeds; verify-poll drives to terminal.
#[test]
fn internal_verify_prepare_start_and_poll_round_trip() {
    let env = build_env(BuildEnv::default_lc_only());
    use interfaces::controller::input::{
        InternalVerifyPollInput, InternalVerifyPrepareInput, InternalVerifyStartInput,
    };
    struct PrepIn(String, String, Option<String>);
    impl InternalVerifyPrepareInput for PrepIn {
        fn solution(&self) -> String {
            self.0.clone()
        }
        fn plan_out(&self) -> String {
            self.1.clone()
        }
        fn starting_out(&self) -> Option<String> {
            self.2.clone()
        }
    }
    struct StartIn(String);
    impl InternalVerifyStartInput for StartIn {
        fn plan_in(&self) -> String {
            self.0.clone()
        }
    }
    struct PollIn(String);
    impl InternalVerifyPollInput for PollIn {
        fn solution(&self) -> String {
            self.0.clone()
        }
    }
    let plan_path = env.root.join("plan.json");
    let starting_path = env.root.join("starting.json");
    let ids = SequenceIdGenerator::new("internal");
    let sleeper = NoopSleeper::new();
    let path = env
        .controller
        .internal_verify_prepare(
            &PrepIn(
                LC_SOLUTION.into(),
                plan_path.display().to_string(),
                Some(starting_path.display().to_string()),
            ),
            &env.root,
            &env.config,
            &env.manifest,
            &env.snapshot,
            &TestClock::new(),
            &ids,
            &sleeper,
            &NoRetryHint,
            PollingPolicy {
                initial_interval: Duration::from_millis(1),
                max_interval: Duration::from_millis(1),
                max_error_backoff: Duration::from_millis(1),
                total_budget: Duration::from_millis(50),
            },
        )
        .expect("prepare succeeds");
    assert!(std::fs::metadata(&path).is_ok(), "plan file should exist");

    // Round-trip parse.
    let bytes = std::fs::read(&path).unwrap();
    let plan =
        usecases::verification::plan::SubmissionPlan::from_canonical_json_bytes(&bytes).unwrap();
    assert_eq!(plan.body.solution_id.as_str(), LC_SOLUTION);

    // The `--starting-out` sidecar must round-trip to a `Starting`
    // VerificationRecord whose plan_hash matches the freshly written plan.
    let starting_bytes = std::fs::read(&starting_path).expect("starting file should exist");
    let starting: VerificationRecord = serde_json::from_slice(&starting_bytes)
        .expect("starting file parses as VerificationRecord");
    assert_eq!(starting.solution_id.as_str(), LC_SOLUTION);
    match &starting.state {
        VerificationState::Starting(s) => {
            assert_eq!(s.plan_hash, plan.plan_hash);
        }
        other => panic!("expected Starting state, got {other:?}"),
    }

    // Now start via internal-verify-start.
    let ids2 = SequenceIdGenerator::new("internal2");
    let sleeper2 = NoopSleeper::new();
    let event = env
        .controller
        .internal_verify_start(
            &StartIn(plan_path.display().to_string()),
            &env.root,
            &env.config,
            &env.manifest,
            &env.snapshot,
            &TestClock::new(),
            &ids2,
            &sleeper2,
            &NoRetryHint,
            PollingPolicy {
                initial_interval: Duration::from_millis(1),
                max_interval: Duration::from_millis(1),
                max_error_backoff: Duration::from_millis(1),
                total_budget: Duration::from_millis(50),
            },
        )
        .expect("start succeeds");
    use usecases::submission_lifecycle::StartEvent;
    assert!(matches!(event, StartEvent::Trackable { .. }), "{event:?}");

    // Poll drives to terminal (fake poller always Accepted).
    let ids3 = SequenceIdGenerator::new("internal3");
    let sleeper3 = NoopSleeper::new();
    let poll_event = env
        .controller
        .internal_verify_poll(
            &PollIn(LC_SOLUTION.into()),
            &env.root,
            &env.config,
            &env.manifest,
            &env.snapshot,
            &TestClock::new(),
            &ids3,
            &sleeper3,
            &NoRetryHint,
            PollingPolicy {
                initial_interval: Duration::from_millis(1),
                max_interval: Duration::from_millis(1),
                max_error_backoff: Duration::from_millis(1),
                total_budget: Duration::from_millis(50),
            },
        )
        .expect("poll succeeds");
    use usecases::submission_lifecycle::PollEvent;
    assert!(
        matches!(poll_event, PollEvent::Completed { .. }),
        "{poll_event:?}"
    );
}
