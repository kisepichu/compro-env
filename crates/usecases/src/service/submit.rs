use anyhow::{Context, Result};
use domain::entity::{Language, OJKind, Solution};

use super::Service;
use crate::submission::{StartSubmissionError, SubmissionRequest, SubmissionStart};

/// Everything `submit` needs after source preparation (read file + preprocess hook).
struct PreparedSubmission {
    oj_kind: OJKind,
    problem_id: String,
    lang_id: String,
    /// The exact source that will be sent to the OJ (post-preprocess).
    source: String,
}

impl Service {
    /// Submits a solution via the OJ recorded in `.ce.toml`.
    ///
    /// Returns a [`SubmissionStart`]. The shell layer maps `UserActionRequired`
    /// (AtCoder) to a browser open, `Trackable` (LibraryChecker) to a printed
    /// submission URL, and `Unavailable` to a hard error.
    pub fn submit(
        &self,
        contest_id: &str,
        problem_code: &str,
        solution_name: &str,
    ) -> Result<SubmissionStart> {
        // 0. Run the solution's test command before preparing submission.
        //
        // `Service::test` currently executes `test_command` via `sh -c`, so
        // enforce the pre-submit gate only on platforms where that contract is
        // supported. Non-Unix submit still builds the browser URL as before.
        #[cfg(unix)]
        {
            let test_exit_code = self.test(contest_id, problem_code, solution_name)?;
            if test_exit_code != 0 {
                anyhow::bail!(
                    "pre-submit tests failed with exit code {test_exit_code}; submission skipped"
                );
            }
        }

        let prepared = self.prepare_submission(contest_id, problem_code, solution_name)?;

        // Delegate to the OJ's SubmissionStarter (spec §8). AtCoder returns
        // `UserActionRequired`; LibraryChecker returns `Trackable`. Some OJs
        // (LibraryChecker) require a session; pass it when available.
        let starter = self.starter_registry.get(&prepared.oj_kind)?;
        let session = self.session_repo.get(&prepared.oj_kind)?;
        let request = SubmissionRequest {
            online_judge: prepared.oj_kind.clone(),
            contest_id: contest_id.to_string(),
            problem_id: prepared.problem_id,
            lang_id: prepared.lang_id,
            source: prepared.source,
        };
        // Normalize `Err(Unavailable)` back into `Ok(SubmissionStart::Unavailable)`
        // so the shell's "unavailable" branch is reached uniformly. The starter's
        // internal `Err(Unavailable)` and its returned `Ok(SubmissionStart::Unavailable)`
        // both mean "this OJ can't serve this request" — the shell only needs to
        // handle it in one place.
        match starter.start_submission(&request, session.as_ref()) {
            Ok(start) => Ok(start),
            Err(StartSubmissionError::Unavailable { reason }) => {
                Ok(SubmissionStart::Unavailable { reason })
            }
            Err(e) => Err(anyhow::anyhow!(e)),
        }
    }

    /// Prepares the submission source and returns it WITHOUT contacting the OJ
    /// (no pre-submit test, no network). This is exactly the source `submit` would
    /// send, so `ce submit --dry-run` can inspect formatting/library expansion safely.
    pub fn submit_dry_run(
        &self,
        contest_id: &str,
        problem_code: &str,
        solution_name: &str,
    ) -> Result<String> {
        Ok(self
            .prepare_submission(contest_id, problem_code, solution_name)?
            .source)
    }

    /// Reads the solution source, resolves the OJ/problem/lang_id, and runs the
    /// preprocess hook. Shared by `submit` and `submit_dry_run`; performs no network
    /// I/O and no pre-submit test.
    fn prepare_submission(
        &self,
        contest_id: &str,
        problem_code: &str,
        solution_name: &str,
    ) -> Result<PreparedSubmission> {
        // 1. Locate solution directory and read ce.toml for language.
        let solution_dir = self
            .solution_repo
            .solution_dir(contest_id, problem_code, solution_name);
        if !solution_dir.is_dir() {
            anyhow::bail!("solution directory not found: {solution_dir:?}");
        }

        let ce_toml_path = solution_dir.join("ce.toml");
        if !ce_toml_path.is_file() {
            anyhow::bail!("ce.toml not found: {ce_toml_path:?}");
        }
        let ce_toml_contents = std::fs::read_to_string(&ce_toml_path)
            .with_context(|| format!("failed to read ce.toml: {ce_toml_path:?}"))?;
        let ce_table: toml::Table = toml::from_str(&ce_toml_contents)
            .with_context(|| format!("failed to parse {ce_toml_path:?}"))?;
        let lang_str = ce_table
            .get("language")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("`language` key not found in {ce_toml_path:?}"))?;
        let normalized = lang_str.trim().to_lowercase();
        let language = normalized.parse::<Language>().map_err(|e| {
            anyhow::anyhow!("invalid `language` value `{lang_str}` in {ce_toml_path:?}: {e}")
        })?;

        // 2. Get OJKind and problem_id from .ce.toml.
        let oj_kind = self.contest_repo.get_oj_kind(contest_id)?;
        let problem = self.contest_repo.get_problem(contest_id, problem_code)?;

        // 3. Read source file.
        let file_path = self.config.submit_file(&language);
        let solution = Solution {
            contest_id: contest_id.to_string(),
            problem_code: problem_code.to_string(),
            problem_title: String::new(),
            name: solution_name.to_string(),
            language: language.clone(),
        };
        let source = self.solution_repo.get_source(&solution, &file_path)?;

        // 4. Resolve lang_id: prefer the user's config.toml mapping; otherwise fall back
        // to the OJ's default (LibraryChecker derives it from the language name).
        let oj = self.online_judge(&oj_kind)?;
        let lang_id = self
            .config
            .lang_id(&language, &oj_kind)
            .or_else(|| oj.default_lang_id(&language))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "lang_id not configured for language `{}` on `{}` \
                    (check config.toml; config parse errors also produce this)",
                    language,
                    oj_kind
                )
            })?;

        // 5. Run the preprocess hook if configured. The hook receives the original
        // source on stdin and prints the submission source to stdout; a non-zero exit
        // aborts submission. Language/OJ branching lives in the user's script (passed
        // via env), so there is a single global hook rather than a per-language one.
        // Unix-only (uses `sh -c`, like `ce test`); other platforms skip it.
        #[cfg(unix)]
        let source = match self.config.submit_preprocess() {
            Some(command) if !command.trim().is_empty() => run_preprocess_hook(
                &command,
                &source,
                &PreprocessContext {
                    language: language.as_str(),
                    oj: oj_kind.as_str(),
                    contest_id,
                    problem_code,
                    problem_id: &problem.id,
                    solution_name,
                    solution_dir: &solution_dir,
                    source_file: &solution_dir.join(&file_path),
                    lang_id: &lang_id,
                    project_root: self.config.project_root(),
                },
            )?,
            _ => source,
        };

        Ok(PreparedSubmission {
            oj_kind,
            problem_id: problem.id,
            lang_id,
            source,
        })
    }
}

/// Context passed to the preprocess hook as environment variables.
#[cfg(unix)]
struct PreprocessContext<'a> {
    language: &'a str,
    oj: &'a str,
    contest_id: &'a str,
    problem_code: &'a str,
    problem_id: &'a str,
    solution_name: &'a str,
    solution_dir: &'a std::path::Path,
    source_file: &'a std::path::Path,
    lang_id: &'a str,
    project_root: &'a std::path::Path,
}

/// Runs the user's preprocess `command` via `sh -c`, feeding `source` on stdin and
/// returning its stdout as the submission source. The hook's stderr is streamed to the
/// terminal. A non-zero exit is reported as an error so submission is aborted.
#[cfg(unix)]
fn run_preprocess_hook(command: &str, source: &str, ctx: &PreprocessContext) -> Result<String> {
    use std::io::{Read as _, Write as _};
    use std::process::{Command, Stdio};

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(ctx.solution_dir)
        .env("CE_LANGUAGE", ctx.language)
        .env("CE_OJ", ctx.oj)
        .env("CE_CONTEST_ID", ctx.contest_id)
        .env("CE_PROBLEM_CODE", ctx.problem_code)
        .env("CE_PROBLEM_ID", ctx.problem_id)
        .env("CE_SOLUTION_NAME", ctx.solution_name)
        .env("CE_SOLUTION_DIR", ctx.solution_dir)
        .env("CE_SOURCE_FILE", ctx.source_file)
        .env("CE_LANG_ID", ctx.lang_id)
        .env("CE_PROJECT_ROOT", ctx.project_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| "failed to launch preprocess hook via sh")?;

    // Write the source on a separate thread while we drain stdout here. Both the
    // source and the hook's output can exceed the OS pipe buffer (expanded sources
    // are large), so writing all of stdin before reading stdout would deadlock.
    let mut stdin = child
        .stdin
        .take()
        .expect("stdin was requested via Stdio::piped");
    let source_bytes = source.as_bytes().to_vec();
    let writer = std::thread::spawn(move || {
        // A hook may legitimately not read all of stdin (e.g. cargo-equip reads the
        // crate, not stdin); the resulting BrokenPipe is expected, so ignore write
        // errors. The hook's exit status is the real signal.
        let _ = stdin.write_all(&source_bytes);
        // `stdin` drops here, closing the pipe so a reading hook sees EOF.
    });

    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .expect("stdout was requested via Stdio::piped")
        .read_to_end(&mut stdout)
        .with_context(|| "failed to read preprocess hook stdout")?;

    let status = child
        .wait()
        .with_context(|| "failed to wait for preprocess hook")?;
    let _ = writer.join();

    if !status.success() {
        anyhow::bail!(
            "preprocess hook failed with exit code {}; submission skipped",
            status.code().unwrap_or(1)
        );
    }
    String::from_utf8(stdout).with_context(|| "preprocess hook produced non-UTF-8 output")
}

#[cfg(test)]
mod tests {
    use crate::{
        config::Config,
        online_judge::{
            ContestMeta, CredentialKind, Credentials, OnlineJudge, OnlineJudgeRegistry,
            SingleOnlineJudge,
        },
        repository::{
            contest_repository::ContestRepository, session_repository::SessionRepository,
            solution_repository::SolutionRepository,
        },
        service::Service,
        submission::{
            RecoveryMode, ResultDetailLevel, StartSubmissionError, StarterRegistry,
            SubmissionAdapterDescriptor, SubmissionHandle, SubmissionMode, SubmissionRequest,
            SubmissionStart, SubmissionStarter,
        },
        test_support::SpawningCommandRunner,
    };
    use anyhow::Result;
    use domain::entity::{Contest, Language, OJKind, Problem, Sample, Session, Solution};
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    // ── Stub helpers ─────────────────────────────────────────────────────────

    struct StubOJ;
    impl OnlineJudge for StubOJ {
        fn name(&self) -> &str {
            "stub"
        }
        fn credential_kind(&self) -> CredentialKind {
            CredentialKind::Cookie
        }
        fn login(&self, _: &Credentials) -> Result<Session> {
            todo!()
        }
        fn whoami(&self, _: &Session) -> Result<String> {
            Ok(String::new())
        }
        fn get_contest_meta(&self, _: &str) -> Result<ContestMeta> {
            todo!()
        }
        fn get_problems_detail(
            &self,
            _: &str,
            _: Option<&Session>,
            _: &[(String, String)],
        ) -> Result<Vec<Problem>> {
            todo!()
        }
    }

    /// Starter stub mirroring the old `StubOJ::submit`: returns `UserActionRequired`
    /// with `submit_url`, or panics when `panic_on_submit` is set (used to verify that
    /// the pre-submit gate short-circuits before the starter is reached).
    struct StubStarter {
        submit_url: String,
        panic_on_submit: bool,
    }
    impl SubmissionStarter for StubStarter {
        fn descriptor(&self) -> SubmissionAdapterDescriptor {
            stub_descriptor()
        }
        fn start_submission(
            &self,
            _request: &SubmissionRequest,
            _session: Option<&Session>,
        ) -> Result<SubmissionStart, StartSubmissionError> {
            if self.panic_on_submit {
                panic!("start_submission must not be called");
            }
            Ok(SubmissionStart::UserActionRequired {
                url: self.submit_url.clone(),
            })
        }
    }

    fn stub_descriptor() -> SubmissionAdapterDescriptor {
        SubmissionAdapterDescriptor {
            name: "stub".to_string(),
            version: "1".to_string(),
            submission_mode: SubmissionMode::UnattendedTrackable,
            result_detail: ResultDetailLevel::TestcaseDetails,
            recovery_mode: RecoveryMode::BestEffort,
        }
    }

    /// Builds a StarterRegistry containing a single starter registered for AtCoder
    /// (matching `StubContestRepo::get_oj_kind`).
    fn registry_with(starter: Box<dyn SubmissionStarter>) -> StarterRegistry {
        let mut registry = StarterRegistry::new();
        registry.register(OJKind::AtCoder, starter);
        registry
    }

    struct StubSession {
        session: Option<Session>,
    }
    impl SessionRepository for StubSession {
        fn get(&self, _: &OJKind) -> Result<Option<Session>> {
            Ok(self.session.clone())
        }
        fn save(&self, _: &Session) -> Result<()> {
            Ok(())
        }
        fn delete(&self, _: &OJKind) -> Result<bool> {
            Ok(false)
        }
    }

    struct StubConfig {
        lang_id: Option<String>,
        submit_file: String,
        submit_preprocess: Option<String>,
    }
    impl Config for StubConfig {
        fn default_language(&self) -> Result<Language> {
            Ok(Language::new("rust"))
        }
        fn default_online_judge(&self) -> OJKind {
            OJKind::AtCoder
        }
        fn submit_file(&self, _: &Language) -> String {
            self.submit_file.clone()
        }
        fn submit_preprocess(&self) -> Option<String> {
            self.submit_preprocess.clone()
        }
        fn lang_id(&self, _: &Language, _: &OJKind) -> Option<String> {
            self.lang_id.clone()
        }
        fn project_root(&self) -> &std::path::Path {
            std::path::Path::new("/tmp/stub-project-root")
        }
    }

    struct StubContestRepo {
        problem: Problem,
    }
    impl ContestRepository for StubContestRepo {
        fn exists(&self, _: &str) -> Result<bool> {
            Ok(true)
        }
        fn exists_unstarted(&self, _: &str) -> Result<bool> {
            Ok(false)
        }
        fn create_unstarted(&self, _: &str) -> Result<()> {
            Ok(())
        }
        fn create(&self, _: &Contest) -> Result<()> {
            Ok(())
        }
        fn get_oj_kind(&self, _: &str) -> Result<OJKind> {
            Ok(OJKind::AtCoder)
        }
        fn get_samples(&self, _: &str, _: &str) -> Result<Vec<Sample>> {
            Ok(vec![])
        }
        fn list_problem_codes(&self, _: &str) -> Result<Vec<String>> {
            Ok(vec![])
        }
        fn testcases_dir(&self, _: &str, _: &str) -> PathBuf {
            PathBuf::from("/tmp/testcases")
        }
        fn get_problem(&self, _: &str, _: &str) -> Result<Problem> {
            Ok(self.problem.clone())
        }
    }

    struct StubSolutionRepo {
        solution_dir: PathBuf,
        /// If Some, get_source returns Ok with this content; if None, returns Err.
        source: Option<String>,
    }
    impl SolutionRepository for StubSolutionRepo {
        fn list(&self, _: &str, _: &str) -> Result<Vec<Solution>> {
            Ok(vec![])
        }
        fn exists(&self, _: &str, _: &str, _: &str) -> Result<bool> {
            Ok(false)
        }
        fn create(&self, _: &Solution, _: &[Sample], _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn get_source(&self, _: &Solution, _: &str) -> Result<String> {
            match &self.source {
                Some(s) => Ok(s.clone()),
                None => Err(anyhow::anyhow!("source file not found")),
            }
        }
        fn solution_dir(&self, _: &str, _: &str, _: &str) -> PathBuf {
            self.solution_dir.clone()
        }
    }

    fn default_problem() -> Problem {
        Problem {
            id: "abc001_a".to_string(),
            code: "a".to_string(),
            title: "Problem A".to_string(),
            samples: vec![],
            input_format_raw: None,
            constraints_raw: None,
        }
    }

    /// OJ stub that exposes a configurable `default_lang_id`, so tests can assert how
    /// submit resolves it.
    struct LangCapturingOJ {
        default_lang_id: Option<String>,
    }
    impl OnlineJudge for LangCapturingOJ {
        fn name(&self) -> &str {
            "stub"
        }
        fn credential_kind(&self) -> CredentialKind {
            CredentialKind::Cookie
        }
        fn default_lang_id(&self, _: &Language) -> Option<String> {
            self.default_lang_id.clone()
        }
        fn login(&self, _: &Credentials) -> Result<Session> {
            todo!()
        }
        fn whoami(&self, _: &Session) -> Result<String> {
            Ok(String::new())
        }
        fn get_contest_meta(&self, _: &str) -> Result<ContestMeta> {
            todo!()
        }
        fn get_problems_detail(
            &self,
            _: &str,
            _: Option<&Session>,
            _: &[(String, String)],
        ) -> Result<Vec<Problem>> {
            todo!()
        }
    }

    /// Starter that records the `lang_id` in the incoming `SubmissionRequest` and
    /// returns a `Trackable` outcome.
    struct LangCapturingStarter {
        received_lang_id: Rc<RefCell<Option<String>>>,
    }
    impl SubmissionStarter for LangCapturingStarter {
        fn descriptor(&self) -> SubmissionAdapterDescriptor {
            stub_descriptor()
        }
        fn start_submission(
            &self,
            request: &SubmissionRequest,
            _session: Option<&Session>,
        ) -> Result<SubmissionStart, StartSubmissionError> {
            *self.received_lang_id.borrow_mut() = Some(request.lang_id.clone());
            Ok(SubmissionStart::Trackable {
                handle: SubmissionHandle {
                    online_judge: OJKind::AtCoder,
                    submission_id: "1".to_string(),
                    submission_url: "https://example.test/submission/1".to_string(),
                    locator: None,
                    submitted_at: chrono::Utc::now(),
                },
            })
        }
    }

    /// Builds a Service whose OJ is a `LangCapturingOJ`. Returns the service, the
    /// captured-lang_id handle, and the TempDir (kept alive for the test).
    fn make_capturing_service(
        config_lang_id: Option<String>,
        default_lang_id: Option<String>,
    ) -> (Service, Rc<RefCell<Option<String>>>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ce.toml"),
            ce_toml_with_language_and_passing_test(),
        )
        .unwrap();
        let received = Rc::new(RefCell::new(None));
        let starter = Box::new(LangCapturingStarter {
            received_lang_id: Rc::clone(&received),
        });
        let service = Service::new(
            Box::new(SingleOnlineJudge::new(Box::new(LangCapturingOJ {
                default_lang_id,
            }))),
            registry_with(starter),
            Box::new(StubContestRepo {
                problem: default_problem(),
            }),
            Box::new(StubSolutionRepo {
                solution_dir: dir.path().to_path_buf(),
                source: Some("fn main() {}".to_string()),
            }),
            Box::new(StubSession { session: None }),
            Box::new(StubConfig {
                lang_id: config_lang_id,
                submit_file: "src/main.rs".to_string(),
                submit_preprocess: None,
            }),
            Box::new(SpawningCommandRunner),
        );
        (service, received, dir)
    }

    /// When config.toml has no lang_id, submit falls back to the OJ's default_lang_id.
    #[test]
    fn submit_falls_back_to_default_lang_id_when_config_missing() {
        let (service, received, _dir) = make_capturing_service(None, Some("rust".to_string()));
        service.submit("abc001", "a", "main").unwrap();
        assert_eq!(
            received.borrow().as_deref(),
            Some("rust"),
            "expected submit to use the OJ default_lang_id when config has none"
        );
    }

    /// A configured lang_id takes priority over the OJ's default_lang_id.
    #[test]
    fn submit_prefers_config_lang_id_over_default() {
        let (service, received, _dir) =
            make_capturing_service(Some("9999".to_string()), Some("rust".to_string()));
        service.submit("abc001", "a", "main").unwrap();
        assert_eq!(
            received.borrow().as_deref(),
            Some("9999"),
            "expected configured lang_id to take priority over the default"
        );
    }

    fn make_service(
        solution_dir: PathBuf,
        source: Option<String>,
        lang_id: Option<String>,
        submit_url: String,
        panic_on_submit: bool,
    ) -> Service {
        Service::new(
            Box::new(SingleOnlineJudge::new(Box::new(StubOJ))),
            registry_with(Box::new(StubStarter {
                submit_url,
                panic_on_submit,
            })),
            Box::new(StubContestRepo {
                problem: default_problem(),
            }),
            Box::new(StubSolutionRepo {
                solution_dir,
                source,
            }),
            Box::new(StubSession { session: None }),
            Box::new(StubConfig {
                lang_id,
                submit_file: "src/main.rs".to_string(),
                submit_preprocess: None,
            }),
            Box::new(SpawningCommandRunner),
        )
    }

    // ── Tests ────────────────────────────────────────────────────────────────

    fn ce_toml_with_language_and_passing_test() -> &'static str {
        if cfg!(unix) {
            "language = \"rust\"\ntest_command = \"exit 0\"\n"
        } else {
            "language = \"rust\"\n"
        }
    }

    /// A registry that records which OJKind was requested, returning a fixed StubOJ.
    /// `requested` is shared via Rc so the test can inspect it after the Service runs.
    struct RecordingRegistry {
        oj: StubOJ,
        requested: Rc<RefCell<Vec<OJKind>>>,
    }
    impl OnlineJudgeRegistry for RecordingRegistry {
        fn get(&self, oj: &OJKind) -> Result<&dyn OnlineJudge> {
            self.requested.borrow_mut().push(oj.clone());
            Ok(&self.oj)
        }
    }

    /// submit resolves the OnlineJudge using the OJKind recorded in .ce.toml
    /// (ContestRepository::get_oj_kind), not a fixed implementation.
    ///
    /// The submit code path calls `self.online_judge(&oj_kind)?` to resolve the OJ's
    /// `default_lang_id` fallback, so the RecordingRegistry still observes the OJKind
    /// resolved from `.ce.toml`.
    #[test]
    fn submit_resolves_online_judge_from_contest_oj_kind() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ce.toml"),
            ce_toml_with_language_and_passing_test(),
        )
        .unwrap();
        let requested = Rc::new(RefCell::new(vec![]));
        let starter = Box::new(StubStarter {
            submit_url: "https://atcoder.jp/contests/abc001/submit#ce=XXX".to_string(),
            panic_on_submit: false,
        });
        let service = Service::new(
            Box::new(RecordingRegistry {
                oj: StubOJ,
                requested: Rc::clone(&requested),
            }),
            registry_with(starter),
            // StubContestRepo::get_oj_kind returns OJKind::AtCoder.
            Box::new(StubContestRepo {
                problem: default_problem(),
            }),
            Box::new(StubSolutionRepo {
                solution_dir: dir.path().to_path_buf(),
                source: Some("fn main() {}".to_string()),
            }),
            Box::new(StubSession { session: None }),
            Box::new(StubConfig {
                lang_id: Some("6088".to_string()),
                submit_file: "src/main.rs".to_string(),
                submit_preprocess: None,
            }),
            Box::new(SpawningCommandRunner),
        );
        service.submit("abc001", "a", "main").unwrap();
        // The OJ resolved for submission is the one stored in .ce.toml.
        let requested = requested.borrow();
        assert!(
            requested.contains(&OJKind::AtCoder),
            "expected submit to resolve the OJ from .ce.toml (AtCoder), got: {requested:?}"
        );
    }

    /// Happy path: submit returns the `UserActionRequired` outcome with the URL from
    /// the starter.
    #[test]
    fn submit_happy_path_returns_open_browser_outcome() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ce.toml"),
            ce_toml_with_language_and_passing_test(),
        )
        .unwrap();
        let expected_url =
            "https://atcoder.jp/contests/abc001/submit?taskScreenName=abc001_a#ce=XXX".to_string();
        let service = make_service(
            dir.path().to_path_buf(),
            Some("fn main() {}".to_string()),
            Some("6088".to_string()),
            expected_url.clone(),
            false,
        );
        let result = service.submit("abc001", "a", "main").unwrap();
        match result {
            SubmissionStart::UserActionRequired { url } => assert_eq!(url, expected_url),
            other => panic!("expected UserActionRequired, got {other:?}"),
        }
    }

    /// Starter that reports `Err(StartSubmissionError::Unavailable)`. Service::submit
    /// normalizes this to `Ok(SubmissionStart::Unavailable)` so the shell layer has a
    /// single "unavailable" branch to handle.
    struct UnavailableStarter;
    impl SubmissionStarter for UnavailableStarter {
        fn descriptor(&self) -> SubmissionAdapterDescriptor {
            stub_descriptor()
        }
        fn start_submission(
            &self,
            _request: &SubmissionRequest,
            _session: Option<&Session>,
        ) -> Result<SubmissionStart, StartSubmissionError> {
            Err(StartSubmissionError::Unavailable {
                reason: crate::submission::UnavailableReason::InteractiveUntrackable,
            })
        }
    }

    /// Regression: `Err(Unavailable)` from a starter is turned into
    /// `Ok(SubmissionStart::Unavailable)` so callers do not need to duplicate
    /// the "OJ cannot serve this request" handling in two places.
    #[test]
    fn submit_normalizes_err_unavailable_into_ok_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ce.toml"),
            ce_toml_with_language_and_passing_test(),
        )
        .unwrap();
        let service = Service::new(
            Box::new(SingleOnlineJudge::new(Box::new(StubOJ))),
            registry_with(Box::new(UnavailableStarter)),
            Box::new(StubContestRepo {
                problem: default_problem(),
            }),
            Box::new(StubSolutionRepo {
                solution_dir: dir.path().to_path_buf(),
                source: Some("fn main() {}".to_string()),
            }),
            Box::new(StubSession { session: None }),
            Box::new(StubConfig {
                lang_id: Some("6088".to_string()),
                submit_file: "src/main.rs".to_string(),
                submit_preprocess: None,
            }),
            Box::new(SpawningCommandRunner),
        );
        match service.submit("abc001", "a", "main") {
            Ok(SubmissionStart::Unavailable { reason }) => {
                assert!(matches!(
                    reason,
                    crate::submission::UnavailableReason::InteractiveUntrackable
                ));
            }
            other => panic!("expected Ok(Unavailable), got {other:?}"),
        }
    }

    /// A non-zero pre-submit test exits before source reading or URL generation.
    #[test]
    #[cfg(unix)]
    fn submit_skips_when_pre_submit_test_fails() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ce.toml"),
            "language = \"rust\"\ntest_command = \"exit 7\"\n",
        )
        .unwrap();
        let service = make_service(
            dir.path().to_path_buf(),
            Some("fn main() {}".to_string()),
            Some("6088".to_string()),
            "https://example.com".to_string(),
            true,
        );
        let err = service.submit("abc001", "a", "main").unwrap_err();
        assert!(
            err.to_string().contains("submission skipped"),
            "unexpected error: {err}"
        );
    }

    /// ce.toml missing in solution dir => error message contains "ce.toml".
    #[test]
    fn submit_errors_when_ce_toml_missing() {
        let dir = tempfile::tempdir().unwrap();
        let service = make_service(
            dir.path().to_path_buf(),
            Some("fn main() {}".to_string()),
            Some("6088".to_string()),
            "https://example.com".to_string(),
            false,
        );
        let err = service.submit("abc001", "a", "main").unwrap_err();
        assert!(
            err.to_string().contains("ce.toml"),
            "unexpected error: {err}"
        );
    }

    /// ce.toml has no `language` key => error message contains "language".
    #[test]
    fn submit_errors_when_language_key_missing_in_ce_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ce.toml"), "test_command = \"exit 0\"\n").unwrap();
        let service = make_service(
            dir.path().to_path_buf(),
            Some("fn main() {}".to_string()),
            Some("6088".to_string()),
            "https://example.com".to_string(),
            false,
        );
        let err = service.submit("abc001", "a", "main").unwrap_err();
        assert!(
            err.to_string().contains("language"),
            "unexpected error: {err}"
        );
    }

    /// get_source returns an error => error is propagated.
    #[test]
    fn submit_errors_when_source_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ce.toml"),
            ce_toml_with_language_and_passing_test(),
        )
        .unwrap();
        let service = make_service(
            dir.path().to_path_buf(),
            None, // get_source returns Err
            Some("6088".to_string()),
            "https://example.com".to_string(),
            false,
        );
        let err = service.submit("abc001", "a", "main").unwrap_err();
        assert!(
            !err.to_string().is_empty(),
            "expected a non-empty error when source file is missing, got: {err}"
        );
    }

    /// config.lang_id returns None => error contains "lang_id".
    #[test]
    fn submit_errors_when_lang_id_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ce.toml"),
            ce_toml_with_language_and_passing_test(),
        )
        .unwrap();
        let service = make_service(
            dir.path().to_path_buf(),
            Some("fn main() {}".to_string()),
            None, // lang_id returns None
            "https://example.com".to_string(),
            false,
        );
        let err = service.submit("abc001", "a", "main").unwrap_err();
        assert!(
            err.to_string().contains("lang_id"),
            "unexpected error: {err}"
        );
    }

    // ── preprocess hook tests (Unix-only: the hook runs via `sh -c`) ───────────

    /// Starter that records the `source` in the incoming `SubmissionRequest`, so
    /// preprocess tests can assert what was actually submitted.
    #[cfg(unix)]
    struct SourceCapturingStarter {
        received_source: Rc<RefCell<Option<String>>>,
    }
    #[cfg(unix)]
    impl SubmissionStarter for SourceCapturingStarter {
        fn descriptor(&self) -> SubmissionAdapterDescriptor {
            stub_descriptor()
        }
        fn start_submission(
            &self,
            request: &SubmissionRequest,
            _session: Option<&Session>,
        ) -> Result<SubmissionStart, StartSubmissionError> {
            *self.received_source.borrow_mut() = Some(request.source.clone());
            Ok(SubmissionStart::Trackable {
                handle: SubmissionHandle {
                    online_judge: OJKind::AtCoder,
                    submission_id: "1".to_string(),
                    submission_url: "https://example.test/submission/1".to_string(),
                    locator: None,
                    submitted_at: chrono::Utc::now(),
                },
            })
        }
    }

    /// Builds a Service whose starter records the submitted source and whose config
    /// carries the given preprocess command and source. Returns the service, the
    /// captured-source handle, and the TempDir (kept alive for the test).
    #[cfg(unix)]
    fn make_preprocess_service(
        preprocess: Option<String>,
        source: &str,
    ) -> (Service, Rc<RefCell<Option<String>>>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ce.toml"),
            ce_toml_with_language_and_passing_test(),
        )
        .unwrap();
        let received = Rc::new(RefCell::new(None));
        let starter = Box::new(SourceCapturingStarter {
            received_source: Rc::clone(&received),
        });
        let service = Service::new(
            Box::new(SingleOnlineJudge::new(Box::new(StubOJ))),
            registry_with(starter),
            Box::new(StubContestRepo {
                problem: default_problem(),
            }),
            Box::new(StubSolutionRepo {
                solution_dir: dir.path().to_path_buf(),
                source: Some(source.to_string()),
            }),
            Box::new(StubSession { session: None }),
            Box::new(StubConfig {
                lang_id: Some("6088".to_string()),
                submit_file: "src/main.rs".to_string(),
                submit_preprocess: preprocess,
            }),
            Box::new(SpawningCommandRunner),
        );
        (service, received, dir)
    }

    /// A configured preprocess hook transforms the source: its stdout is submitted.
    #[test]
    #[cfg(unix)]
    fn submit_runs_preprocess_hook_and_submits_its_stdout() {
        let (service, received, _dir) =
            make_preprocess_service(Some("printf '%s' TRANSFORMED".to_string()), "ORIGINAL");
        service.submit("abc001", "a", "main").unwrap();
        assert_eq!(
            received.borrow().as_deref(),
            Some("TRANSFORMED"),
            "expected the hook's stdout to be submitted"
        );
    }

    /// With no preprocess hook configured, the original source is submitted unchanged.
    #[test]
    #[cfg(unix)]
    fn submit_without_preprocess_submits_original_source() {
        let (service, received, _dir) = make_preprocess_service(None, "ORIGINAL");
        service.submit("abc001", "a", "main").unwrap();
        assert_eq!(
            received.borrow().as_deref(),
            Some("ORIGINAL"),
            "expected the original source to be submitted when no hook is set"
        );
    }

    /// A hook that exits non-zero aborts submission; the OJ's submit is never reached.
    #[test]
    #[cfg(unix)]
    fn submit_aborts_when_preprocess_hook_fails() {
        let (service, received, _dir) =
            make_preprocess_service(Some("exit 3".to_string()), "ORIGINAL");
        let err = service.submit("abc001", "a", "main").unwrap_err();
        assert!(
            err.to_string().contains("preprocess hook failed"),
            "unexpected error: {err}"
        );
        assert!(
            received.borrow().is_none(),
            "submit must not be called when the preprocess hook fails"
        );
    }

    /// The hook receives the documented context env vars. The script verifies each and
    /// passes stdin through with `cat`; any mismatch makes it exit non-zero (→ submit
    /// would error), so an Ok result with the unchanged source confirms the env.
    #[test]
    #[cfg(unix)]
    fn submit_passes_context_env_to_preprocess_hook() {
        let script = "test \"$CE_LANGUAGE\" = rust \
             && test \"$CE_OJ\" = atcoder \
             && test \"$CE_LANG_ID\" = 6088 \
             && test \"$CE_CONTEST_ID\" = abc001 \
             && test \"$CE_PROBLEM_CODE\" = a \
             && test \"$CE_PROBLEM_ID\" = abc001_a \
             && test \"$CE_SOLUTION_NAME\" = main \
             && test -n \"$CE_SOLUTION_DIR\" \
             && test -n \"$CE_SOURCE_FILE\" \
             && cat";
        let (service, received, _dir) =
            make_preprocess_service(Some(script.to_string()), "ORIGINAL");
        service
            .submit("abc001", "a", "main")
            .expect("expected submit to succeed when env vars match");
        assert_eq!(
            received.borrow().as_deref(),
            Some("ORIGINAL"),
            "expected stdin to pass through once env vars matched"
        );
    }

    /// dry-run returns the preprocessed source and never contacts the OJ.
    #[test]
    #[cfg(unix)]
    fn submit_dry_run_returns_preprocessed_source_without_submitting() {
        let (service, received, _dir) =
            make_preprocess_service(Some("printf '%s' TRANSFORMED".to_string()), "ORIGINAL");
        let out = service.submit_dry_run("abc001", "a", "main").unwrap();
        assert_eq!(
            out, "TRANSFORMED",
            "dry-run should return the preprocessed source"
        );
        assert!(
            received.borrow().is_none(),
            "dry-run must not call the OJ's submit"
        );
    }

    /// Regression: a hook that both consumes a large stdin and emits a large stdout
    /// (here `cat` with ~1 MB) must not deadlock on the pipe buffers.
    #[test]
    #[cfg(unix)]
    fn submit_preprocess_handles_large_io_without_deadlock() {
        let big = "x".repeat(1_000_000);
        let (service, _received, _dir) = make_preprocess_service(Some("cat".to_string()), &big);
        let out = service.submit_dry_run("abc001", "a", "main").unwrap();
        assert_eq!(
            out.len(),
            big.len(),
            "cat should echo the full large source"
        );
    }

    /// Regression: a hook that ignores a large stdin (like cargo-equip, which reads the
    /// crate not stdin) must not hang or error on the unread input.
    #[test]
    #[cfg(unix)]
    fn submit_preprocess_ok_when_hook_ignores_large_stdin() {
        let big = "y".repeat(1_000_000);
        let (service, _received, _dir) =
            make_preprocess_service(Some("printf '%s' DONE".to_string()), &big);
        let out = service.submit_dry_run("abc001", "a", "main").unwrap();
        assert_eq!(
            out, "DONE",
            "hook output should be used even if stdin is unread"
        );
    }
}
