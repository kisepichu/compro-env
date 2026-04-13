use anyhow::Result;
use domain::entity::{Language, OJKind, Solution};

use super::Service;

#[derive(Debug)]
pub struct InitResult {
    pub contest_id: String,
    pub oj_kind: OJKind,
    pub created_solutions: Vec<Solution>,
    /// Total number of sample input/output files implied by the fetched samples
    /// (each Sample = 2 files: `.in` and `.out`).
    ///
    /// This is not necessarily the number of files newly written, because the
    /// repository skips files that already exist on disk.
    pub total_sample_files: usize,
    /// True when the contest was already initialized (`.ce.toml` exists); no
    /// files were modified during this run.
    pub already_initialized: bool,
}

impl Service {
    /// Initializes a contest: fetches problems, saves test cases, and creates solution directories.
    ///
    /// `on_progress` is called with human-readable status messages during the wait loop.
    /// Pass `|_| {}` to suppress output.
    pub fn init(
        &self,
        contest_id: &str,
        oj: OJKind,
        lang: &Language,
        on_progress: &dyn Fn(&str),
    ) -> Result<InitResult> {
        // Step 0: Skip if already initialized; read the stored OJ kind from disk
        // rather than trusting the CLI argument, which may differ from what was
        // written when the contest was first initialised.
        if self.contest_repo.exists(contest_id)? {
            let oj_kind = self.contest_repo.get_oj_kind(contest_id)?;
            return Ok(InitResult {
                contest_id: contest_id.to_string(),
                oj_kind,
                created_solutions: vec![],
                total_sample_files: 0,
                already_initialized: true,
            });
        }

        // Step 1: Get session (None is allowed for public contests)
        let session = self.session_repo.get(&oj)?;

        // Step 2: Check start time and wait if needed
        let meta = self.online_judge.get_contest_meta(contest_id)?;
        if let Some(start_time) = meta.start_time {
            if start_time > chrono::Utc::now() {
                self.contest_repo.create_unstarted(contest_id)?;
                // Poll deadline: give up 60 seconds after start if problems never appear
                let post_start_deadline = start_time + chrono::Duration::seconds(60);
                loop {
                    let now = chrono::Utc::now();
                    let remaining = start_time - now;
                    if remaining > chrono::Duration::minutes(1) {
                        on_progress(&format!(
                            "Contest starts at {}. Remaining: {}m{}s",
                            start_time.format("%H:%M:%S"),
                            remaining.num_minutes(),
                            remaining.num_seconds() % 60,
                        ));
                        std::thread::sleep(std::time::Duration::from_secs(60));
                    } else if remaining > chrono::Duration::seconds(10) {
                        on_progress(&format!("{}s remaining...", remaining.num_seconds()));
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    } else {
                        // Within 10 seconds of start or already started: poll for problems
                        match self.online_judge.get_problems_detail(
                            contest_id,
                            session.as_ref(),
                            &meta.problem_id_hints,
                        ) {
                            Ok(problems) if !problems.is_empty() => {
                                return build_result(
                                    contest_id,
                                    oj,
                                    lang,
                                    problems,
                                    &*self.contest_repo,
                                    &*self.solution_repo,
                                );
                            }
                            Ok(_) => {
                                // No problems yet; keep polling until deadline
                                if now > post_start_deadline {
                                    anyhow::bail!(
                                        "timed out waiting for problems after contest start"
                                    );
                                }
                                std::thread::sleep(std::time::Duration::from_secs(1));
                            }
                            Err(e) => {
                                // Fail fast on definitive auth errors; treat other
                                // failures (e.g. transient 404/503) as retryable
                                // until the post-start deadline.
                                if e.downcast_ref::<domain::error::CeError>()
                                    .map(|ce| {
                                        matches!(ce, domain::error::CeError::NotLoggedIn { .. })
                                    })
                                    .unwrap_or(false)
                                {
                                    return Err(e);
                                }
                                if now > post_start_deadline {
                                    return Err(e);
                                }
                                std::thread::sleep(std::time::Duration::from_secs(1));
                            }
                        }
                    }
                }
            }
        }

        // Step 3: Get problems (no waiting required)
        let problems = self.online_judge.get_problems_detail(
            contest_id,
            session.as_ref(),
            &meta.problem_id_hints,
        )?;

        if problems.is_empty() {
            anyhow::bail!(
                "no problems found for contest \"{contest_id}\". \
                 The contest may not have started yet, or login may be required (`ce login`)."
            );
        }

        build_result(
            contest_id,
            oj,
            lang,
            problems,
            &*self.contest_repo,
            &*self.solution_repo,
        )
    }
}

fn build_result(
    contest_id: &str,
    oj: OJKind,
    lang: &Language,
    problems: Vec<domain::entity::Problem>,
    contest_repo: &dyn crate::repository::contest_repository::ContestRepository,
    solution_repo: &dyn crate::repository::solution_repository::SolutionRepository,
) -> Result<InitResult> {
    let total_sample_files: usize = problems.iter().map(|p| p.samples.len() * 2).sum();
    let oj_kind = oj.clone();
    let contest = domain::entity::Contest {
        id: contest_id.to_string(),
        online_judge: oj,
        problems,
    };
    // Create solution directories first so that a template-expansion failure
    // does not leave .ce.toml on disk. If a solution fails, the contest has
    // not been marked as initialized; the next run will skip already-existing
    // solution dirs (idempotent) and retry any that are missing.
    let mut created_solutions = Vec::new();
    for problem in &contest.problems {
        let solution = Solution {
            contest_id: contest_id.to_string(),
            problem_code: problem.code.clone(),
            problem_title: problem.title.clone(),
            name: "main".to_string(),
            language: lang.clone(),
        };
        solution_repo.create(&solution)?;
        created_solutions.push(solution);
    }
    // Write .ce.toml and test-case files only after all solutions succeed.
    contest_repo.create(&contest)?;
    Ok(InitResult {
        contest_id: contest_id.to_string(),
        oj_kind,
        created_solutions,
        total_sample_files,
        already_initialized: false,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use anyhow::Result;
    use chrono::{DateTime, Utc};
    use domain::entity::{Contest, Language, OJKind, Problem, Sample, Session, SubmitResult};

    use crate::{
        config::Config,
        online_judge::{ContestMeta, OnlineJudge},
        repository::{
            contest_repository::ContestRepository, session_repository::SessionRepository,
            solution_repository::SolutionRepository,
        },
        service::Service,
    };

    // ── Stub helpers ──────────────────────────────────────────────────────────

    fn make_problem(code: &str) -> Problem {
        Problem {
            id: format!("abc001_{code}"),
            code: code.to_string(),
            title: format!("Problem {}", code.to_uppercase()),
            samples: vec![Sample {
                input: "1\n".to_string(),
                output: "1\n".to_string(),
            }],
        }
    }

    const NO_PROGRESS: &dyn Fn(&str) = &|_| {};

    struct StubOJ {
        problems: Vec<Problem>,
        start_time: Option<DateTime<Utc>>,
    }

    impl OnlineJudge for StubOJ {
        fn name(&self) -> &str {
            "stub"
        }

        fn whoami(&self, _session: &Session) -> Result<String> {
            Ok("stub_user".to_string())
        }

        fn get_contest_meta(&self, _contest_id: &str) -> Result<ContestMeta> {
            Ok(ContestMeta {
                start_time: self.start_time,
                problem_id_hints: vec![],
            })
        }

        fn get_problems_detail(
            &self,
            _contest_id: &str,
            _session: Option<&Session>,
            _problem_id_hints: &[(String, String)],
        ) -> Result<Vec<Problem>> {
            Ok(self.problems.clone())
        }

        fn submit(
            &self,
            _contest_id: &str,
            _problem_id: &str,
            _lang_id: &str,
            _source: &str,
            _session: &Session,
        ) -> Result<SubmitResult> {
            todo!()
        }
    }

    struct StubSessionRepo {
        session: Option<Session>,
    }

    impl SessionRepository for StubSessionRepo {
        fn get(&self, _oj: &OJKind) -> Result<Option<Session>> {
            Ok(self.session.clone())
        }

        fn save(&self, _session: &Session) -> Result<()> {
            Ok(())
        }

        fn delete(&self, _oj: &OJKind) -> Result<bool> {
            Ok(true)
        }
    }

    struct StubContestRepo {
        create_unstarted_called: Arc<AtomicBool>,
    }

    impl ContestRepository for StubContestRepo {
        fn exists(&self, _contest_id: &str) -> Result<bool> {
            Ok(false)
        }

        fn exists_unstarted(&self, _contest_id: &str) -> Result<bool> {
            Ok(false)
        }

        fn create_unstarted(&self, _contest_id: &str) -> Result<()> {
            self.create_unstarted_called.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn create(&self, _contest: &Contest) -> Result<()> {
            Ok(())
        }

        fn get_oj_kind(&self, _contest_id: &str) -> Result<OJKind> {
            Ok(OJKind::AtCoder)
        }

        fn get_samples(&self, _contest_id: &str, _problem_code: &str) -> Result<Vec<Sample>> {
            Ok(vec![])
        }

        fn list_problem_codes(&self, _contest_id: &str) -> Result<Vec<String>> {
            Ok(vec![])
        }
    }

    struct StubSolutionRepo {
        created: RefCell<Vec<domain::entity::Solution>>,
    }

    impl SolutionRepository for StubSolutionRepo {
        fn list(
            &self,
            _contest_id: &str,
            _problem_code: &str,
        ) -> Result<Vec<domain::entity::Solution>> {
            Ok(vec![])
        }

        fn exists(
            &self,
            _contest_id: &str,
            _problem_code: &str,
            _name: &str,
            _lang: &Language,
        ) -> Result<bool> {
            Ok(false)
        }

        fn create(&self, solution: &domain::entity::Solution) -> Result<()> {
            self.created.borrow_mut().push(solution.clone());
            Ok(())
        }

        fn get_source(&self, _solution: &domain::entity::Solution) -> Result<String> {
            Ok(String::new())
        }
    }

    struct StubConfig;

    impl Config for StubConfig {
        fn default_language(&self) -> Result<Language> {
            Ok(Language::new("rust"))
        }

        fn default_online_judge(&self) -> OJKind {
            OJKind::AtCoder
        }

        fn test_command(&self, _lang: &Language) -> String {
            String::new()
        }

        fn run_command(&self, _lang: &Language) -> String {
            String::new()
        }

        fn submit_file(&self, _lang: &Language) -> String {
            String::new()
        }

        fn submit_preprocess(&self, _lang: &Language) -> String {
            String::new()
        }

        fn lang_id(&self, _lang: &Language, _oj: &OJKind) -> Option<String> {
            None
        }
    }

    fn make_service(
        oj: StubOJ,
        session: Option<Session>,
        contest_repo: StubContestRepo,
        solution_repo: StubSolutionRepo,
    ) -> Service {
        Service::new(
            Box::new(oj),
            Box::new(contest_repo),
            Box::new(solution_repo),
            Box::new(StubSessionRepo { session }),
            Box::new(StubConfig),
        )
    }

    fn make_contest_repo() -> StubContestRepo {
        StubContestRepo {
            create_unstarted_called: Arc::new(AtomicBool::new(false)),
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    /// When a session exists and the OJ returns 2 problems, `init()` calls `contest_repo.create`
    /// and `solution_repo.create` for each problem, and `InitResult` has 2 solutions.
    #[test]
    fn init_creates_contest_and_solutions() {
        let session = Session {
            online_judge: OJKind::AtCoder,
            cookie: "cookie_value".to_string(),
        };

        let service = make_service(
            StubOJ {
                problems: vec![make_problem("a"), make_problem("b")],
                start_time: None,
            },
            Some(session),
            make_contest_repo(),
            StubSolutionRepo {
                created: RefCell::new(vec![]),
            },
        );

        let result = service
            .init("abc001", OJKind::AtCoder, &Language::new("rust"), NO_PROGRESS)
            .unwrap();

        assert_eq!(result.contest_id, "abc001");
        assert_eq!(result.created_solutions.len(), 2);
        assert_eq!(result.created_solutions[0].problem_code, "a");
        assert_eq!(result.created_solutions[1].problem_code, "b");
    }

    /// When session is None, init() proceeds and calls get_problems_detail with session=None,
    /// returning a successful InitResult.
    #[test]
    fn init_succeeds_without_session() {
        let service = make_service(
            StubOJ {
                problems: vec![make_problem("a")],
                start_time: None,
            },
            None, // no session
            make_contest_repo(),
            StubSolutionRepo {
                created: RefCell::new(vec![]),
            },
        );

        let result = service.init("abc001", OJKind::AtCoder, &Language::new("rust"), NO_PROGRESS);

        assert!(
            result.is_ok(),
            "expected Ok, got: {:?}",
            result.unwrap_err()
        );
    }

    /// When `get_contest_meta().start_time` is `None`, `init()` proceeds directly to problem fetch
    /// without calling `create_unstarted`.
    #[test]
    fn init_skips_waiting_when_start_time_is_none() {
        let called = Arc::new(AtomicBool::new(false));
        let contest_repo = StubContestRepo {
            create_unstarted_called: called.clone(),
        };

        let session = Session {
            online_judge: OJKind::AtCoder,
            cookie: "cookie_value".to_string(),
        };

        let service = Service::new(
            Box::new(StubOJ {
                problems: vec![make_problem("a")],
                start_time: None, // no start time known
            }),
            Box::new(contest_repo),
            Box::new(StubSolutionRepo {
                created: RefCell::new(vec![]),
            }),
            Box::new(StubSessionRepo {
                session: Some(session),
            }),
            Box::new(StubConfig),
        );

        let _ = service.init("abc001", OJKind::AtCoder, &Language::new("rust"), NO_PROGRESS);

        assert!(
            !called.load(Ordering::SeqCst),
            "create_unstarted should not be called when start_time is None"
        );
    }

    /// When `contest_repo.exists()` returns true, `init()` returns `already_initialized = true`
    /// without fetching problems or creating any files.
    #[test]
    fn init_returns_already_initialized_when_contest_exists() {
        struct AlreadyExistsRepo;

        impl ContestRepository for AlreadyExistsRepo {
            fn exists(&self, _contest_id: &str) -> Result<bool> {
                Ok(true)
            }

            fn exists_unstarted(&self, _contest_id: &str) -> Result<bool> {
                Ok(false)
            }

            fn create_unstarted(&self, _contest_id: &str) -> Result<()> {
                panic!("create_unstarted must not be called");
            }

            fn create(&self, _contest: &Contest) -> Result<()> {
                panic!("create must not be called");
            }

            fn get_oj_kind(&self, _contest_id: &str) -> Result<OJKind> {
                Ok(OJKind::AtCoder)
            }

            fn get_samples(
                &self,
                _contest_id: &str,
                _problem_code: &str,
            ) -> Result<Vec<domain::entity::Sample>> {
                Ok(vec![])
            }

            fn list_problem_codes(&self, _contest_id: &str) -> Result<Vec<String>> {
                Ok(vec![])
            }
        }

        let service = Service::new(
            Box::new(StubOJ {
                problems: vec![make_problem("a")],
                start_time: None,
            }),
            Box::new(AlreadyExistsRepo),
            Box::new(StubSolutionRepo {
                created: RefCell::new(vec![]),
            }),
            Box::new(StubSessionRepo { session: None }),
            Box::new(StubConfig),
        );

        let result = service
            .init("abc001", OJKind::AtCoder, &Language::new("rust"), NO_PROGRESS)
            .unwrap();

        assert!(result.already_initialized);
        assert!(result.created_solutions.is_empty());
        assert_eq!(result.total_sample_files, 0);
    }

    /// When `get_problems_detail` returns an empty list, `init()` returns an error.
    #[test]
    fn init_errors_on_empty_problem_list() {
        let service = make_service(
            StubOJ {
                problems: vec![], // empty
                start_time: None,
            },
            None,
            make_contest_repo(),
            StubSolutionRepo {
                created: RefCell::new(vec![]),
            },
        );

        let result = service.init("abc001", OJKind::AtCoder, &Language::new("rust"), NO_PROGRESS);

        assert!(result.is_err(), "expected Err for empty problem list");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("no problems found"),
            "expected 'no problems found' in error, got: {msg}"
        );
    }
}
