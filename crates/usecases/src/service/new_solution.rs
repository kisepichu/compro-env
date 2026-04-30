use anyhow::Result;
use domain::entity::Solution;

use super::Service;

impl Service {
    /// Creates a new solution directory.
    ///
    /// Validates that the contest is initialized, the problem code exists, and the
    /// solution directory does not already exist before expanding templates.
    pub fn new_solution(&self, solution: Solution) -> Result<()> {
        if !self.contest_repo.exists(&solution.contest_id)? {
            anyhow::bail!(
                "contest '{}' is not initialized. Run `ce init {}` first.",
                solution.contest_id,
                solution.contest_id
            );
        }

        let codes = self.contest_repo.list_problem_codes(&solution.contest_id)?;
        if !codes.iter().any(|c| c == &solution.problem_code) {
            anyhow::bail!(
                "problem '{}' not found in contest '{}'. Available: {}",
                solution.problem_code,
                solution.contest_id,
                codes.join(", ")
            );
        }

        if self.solution_repo.exists(
            &solution.contest_id,
            &solution.problem_code,
            &solution.name,
        )? {
            anyhow::bail!(
                "solution already exists: solutions/{}/{}/{}",
                solution.contest_id,
                solution.problem_code,
                solution.name
            );
        }

        let samples = self
            .contest_repo
            .get_samples(&solution.contest_id, &solution.problem_code)?;

        let problem = self
            .contest_repo
            .get_problem(&solution.contest_id, &solution.problem_code)
            .ok();
        let input_format_raw = problem
            .as_ref()
            .and_then(|p| p.input_format_raw.as_deref())
            .unwrap_or("");

        self.solution_repo
            .create(&solution, &samples, input_format_raw)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        config::Config,
        online_judge::{ContestMeta, OnlineJudge},
        repository::{
            contest_repository::ContestRepository, session_repository::SessionRepository,
            solution_repository::SolutionRepository,
        },
        service::Service,
    };
    use anyhow::Result;
    use domain::entity::{Contest, Language, OJKind, Problem, Sample, Session, Solution};
    use std::path::PathBuf;

    // ── Minimal stubs ────────────────────────────────────────────────────────

    struct StubOJ;
    impl OnlineJudge for StubOJ {
        fn name(&self) -> &str {
            "stub"
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
        fn build_submit_url(&self, _: &str, _: &str, _: &str, _: &str) -> String {
            todo!()
        }
    }

    struct StubSession;
    impl SessionRepository for StubSession {
        fn get(&self, _: &OJKind) -> Result<Option<Session>> {
            Ok(None)
        }
        fn save(&self, _: &Session) -> Result<()> {
            Ok(())
        }
        fn delete(&self, _: &OJKind) -> Result<bool> {
            Ok(false)
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
        fn submit_file(&self, _: &Language) -> String {
            String::new()
        }
        fn submit_preprocess(&self, _: &Language) -> String {
            String::new()
        }
        fn lang_id(&self, _: &Language, _: &OJKind) -> Option<String> {
            None
        }
    }

    struct StubContestRepo {
        contest_exists: bool,
        problem_codes: Vec<String>,
    }
    impl ContestRepository for StubContestRepo {
        fn exists(&self, _: &str) -> Result<bool> {
            Ok(self.contest_exists)
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
            Ok(vec![Sample {
                input: "1\n".into(),
                output: "2\n".into(),
            }])
        }
        fn list_problem_codes(&self, _: &str) -> Result<Vec<String>> {
            Ok(self.problem_codes.clone())
        }
        fn testcases_dir(&self, _: &str, _: &str) -> PathBuf {
            PathBuf::from("/tmp/testcases")
        }
        fn get_problem(&self, _: &str, code: &str) -> Result<domain::entity::Problem> {
            Ok(domain::entity::Problem {
                id: format!("stub_{code}"),
                code: code.to_string(),
                title: code.to_string(),
                samples: vec![],
                input_format_raw: None,
                constraints_raw: None,
            })
        }
    }

    struct StubSolutionRepo {
        solution_exists: bool,
    }
    impl SolutionRepository for StubSolutionRepo {
        fn list(&self, _: &str, _: &str) -> Result<Vec<Solution>> {
            Ok(vec![])
        }
        fn exists(&self, _: &str, _: &str, _: &str) -> Result<bool> {
            Ok(self.solution_exists)
        }
        fn create(&self, _: &Solution, _: &[Sample], _: &str) -> Result<()> {
            Ok(())
        }
        fn get_source(&self, _: &Solution, _: &str) -> Result<String> {
            Ok(String::new())
        }
        fn solution_dir(&self, _: &str, _: &str, _: &str) -> PathBuf {
            PathBuf::from("/tmp/solution_dir")
        }
    }

    fn make_solution(contest_id: &str, problem_code: &str, name: &str) -> Solution {
        Solution {
            contest_id: contest_id.to_string(),
            problem_code: problem_code.to_string(),
            problem_title: problem_code.to_string(),
            name: name.to_string(),
            language: Language::new("rust"),
        }
    }

    fn make_service(
        contest_exists: bool,
        problem_codes: Vec<&str>,
        solution_exists: bool,
    ) -> Service {
        Service::new(
            Box::new(StubOJ),
            Box::new(StubContestRepo {
                contest_exists,
                problem_codes: problem_codes.iter().map(|s| s.to_string()).collect(),
            }),
            Box::new(StubSolutionRepo { solution_exists }),
            Box::new(StubSession),
            Box::new(StubConfig),
        )
    }

    /// Success: contest exists, problem_code is valid, solution does not already exist.
    #[test]
    fn new_solution_succeeds() {
        let service = make_service(true, vec!["a", "b", "c"], false);
        let solution = make_solution("abc001", "a", "sol2");
        assert!(service.new_solution(solution).is_ok());
    }

    /// Error: contest does not exist — message should mention `ce init`.
    #[test]
    fn new_solution_errors_when_contest_not_initialized() {
        let service = make_service(false, vec![], false);
        let solution = make_solution("abc001", "a", "main");
        let err = service.new_solution(solution).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("ce init"),
            "expected 'ce init' in error, got: {msg}"
        );
    }

    /// Error: problem_code not in list — message should include available codes.
    #[test]
    fn new_solution_errors_when_problem_code_not_found() {
        let service = make_service(true, vec!["a", "b"], false);
        let solution = make_solution("abc001", "z", "main");
        let err = service.new_solution(solution).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Available: a, b"),
            "expected available problem codes in error, got: {msg}"
        );
    }

    /// Error: solution already exists.
    #[test]
    fn new_solution_errors_when_solution_already_exists() {
        let service = make_service(true, vec!["a"], true);
        let solution = make_solution("abc001", "a", "main");
        let err = service.new_solution(solution).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("already exists"),
            "expected 'already exists' in error, got: {msg}"
        );
    }
}
