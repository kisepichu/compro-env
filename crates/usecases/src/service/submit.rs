use anyhow::{Context, Result};
use domain::entity::{Language, Solution, SubmitResult};

use super::Service;

impl Service {
    /// Builds the browser submit URL and returns it.
    /// The caller (shell layer) opens the URL in the default browser.
    pub fn submit(
        &self,
        contest_id: &str,
        problem_code: &str,
        solution_name: &str,
    ) -> Result<SubmitResult> {
        // 0. Run the solution's test command before preparing submission.
        let test_exit_code = self.test(contest_id, problem_code, solution_name)?;
        if test_exit_code != 0 {
            anyhow::bail!(
                "pre-submit tests failed with exit code {test_exit_code}; submission skipped"
            );
        }

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

        // 4. Get lang_id from config.
        let lang_id = self.config.lang_id(&language, &oj_kind).ok_or_else(|| {
            anyhow::anyhow!(
                "lang_id not configured for language `{}` on `{}` \
                (check config.toml; config parse errors also produce this)",
                language,
                oj_kind
            )
        })?;

        // 5. Guard against source files too large for a browser URL.
        // Compute an upper bound on the base64 fragment length:
        //   JSON payload = overhead (~30 + lang_id.len()) + source (×2 worst-case JSON escaping)
        //   base64 expansion = ceil(json_bytes / 3) × 4
        let json_upper = source.len() * 2 + lang_id.len() + 30;
        let fragment_upper = json_upper.div_ceil(3) * 4;
        const MAX_FRAGMENT_BYTES: usize = 32 * 1024;
        if fragment_upper > MAX_FRAGMENT_BYTES {
            anyhow::bail!(
                "source file is too large to submit via URL fragment \
                 (estimated fragment {} bytes, max {})",
                fragment_upper,
                MAX_FRAGMENT_BYTES,
            );
        }

        // 6. Build the browser submit URL.
        let url = self
            .online_judge
            .build_submit_url(contest_id, &problem.id, &lang_id, &source);

        Ok(SubmitResult {
            submission_url: url,
        })
    }
}

#[cfg(all(test, unix))]
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

    // ── Stub helpers ─────────────────────────────────────────────────────────

    struct StubOJ {
        submit_url: String,
        panic_on_build_submit_url: bool,
    }
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
            if self.panic_on_build_submit_url {
                panic!("build_submit_url must not be called");
            }
            self.submit_url.clone()
        }
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
        fn submit_preprocess(&self, _: &Language) -> String {
            String::new()
        }
        fn lang_id(&self, _: &Language, _: &OJKind) -> Option<String> {
            self.lang_id.clone()
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

    fn make_service(
        solution_dir: PathBuf,
        source: Option<String>,
        lang_id: Option<String>,
        submit_url: String,
        panic_on_build_submit_url: bool,
    ) -> Service {
        Service::new(
            Box::new(StubOJ {
                submit_url,
                panic_on_build_submit_url,
            }),
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
            }),
        )
    }

    // ── Tests ────────────────────────────────────────────────────────────────

    /// Happy path: SubmitResult.submission_url is the URL returned by StubOJ.
    #[test]
    fn submit_happy_path_returns_submission_url() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ce.toml"),
            "language = \"rust\"\ntest_command = \"exit 0\"\n",
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
        assert_eq!(result.submission_url, expected_url);
    }

    /// A non-zero pre-submit test exits before source reading or URL generation.
    #[test]
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
            "language = \"rust\"\ntest_command = \"exit 0\"\n",
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
            "language = \"rust\"\ntest_command = \"exit 0\"\n",
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
}
