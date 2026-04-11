use anyhow::Result;
use domain::entity::{Contest, OJKind, Sample};
use usecases::repository::contest_repository::ContestRepository;

/// Manages solutions/ relative to the project root.
pub struct ContestRepositoryImpl {
    /// Project root path.
    root: std::path::PathBuf,
}

impl ContestRepositoryImpl {
    pub fn new(root: std::path::PathBuf) -> Self {
        Self { root }
    }
}

impl ContestRepositoryImpl {
    fn contest_dir(&self, contest_id: &str) -> std::path::PathBuf {
        self.root.join("solutions").join(contest_id)
    }

    fn ce_toml_path(&self, contest_id: &str) -> std::path::PathBuf {
        self.contest_dir(contest_id).join(".ce.toml")
    }
}

impl ContestRepository for ContestRepositoryImpl {
    fn exists(&self, contest_id: &str) -> Result<bool> {
        Ok(self.ce_toml_path(contest_id).exists())
    }

    fn exists_unstarted(&self, contest_id: &str) -> Result<bool> {
        Ok(self.contest_dir(contest_id).is_dir() && !self.ce_toml_path(contest_id).exists())
    }

    fn create_unstarted(&self, contest_id: &str) -> Result<()> {
        std::fs::create_dir_all(self.contest_dir(contest_id))?;
        Ok(())
    }

    fn create(&self, contest: &Contest) -> Result<()> {
        let contest_dir = self.contest_dir(&contest.id);
        std::fs::create_dir_all(&contest_dir)?;

        let toml_path = contest_dir.join(".ce.toml");
        if !toml_path.exists() {
            let mut toml = format!(
                "online_judge = \"{}\"\ncontest_id = \"{}\"\n",
                contest.online_judge.as_str(),
                contest.id
            );
            for problem in &contest.problems {
                toml.push_str(&format!(
                    "\n[[problems]]\nid = \"{}\"\ncode = \"{}\"\ntitle = \"{}\"\n",
                    problem.id, problem.code, problem.title
                ));
            }
            std::fs::write(&toml_path, toml)?;
        }

        for problem in &contest.problems {
            let tc_dir = contest_dir.join("testcases").join(&problem.code);
            std::fs::create_dir_all(&tc_dir)?;
            for (i, sample) in problem.samples.iter().enumerate() {
                let n = i + 1;
                let in_path = tc_dir.join(format!("{}.in", n));
                let out_path = tc_dir.join(format!("{}.out", n));
                if !in_path.exists() {
                    std::fs::write(&in_path, &sample.input)?;
                }
                if !out_path.exists() {
                    std::fs::write(&out_path, &sample.output)?;
                }
            }
        }

        Ok(())
    }

    fn get_oj_kind(&self, _contest_id: &str) -> Result<OJKind> {
        todo!()
    }

    fn get_samples(&self, _contest_id: &str, _problem_code: &str) -> Result<Vec<Sample>> {
        todo!()
    }

    fn list_problem_codes(&self, _contest_id: &str) -> Result<Vec<String>> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::entity::{Contest, OJKind, Problem, Sample};
    use std::fs;

    fn make_repo(root: &std::path::Path) -> ContestRepositoryImpl {
        ContestRepositoryImpl::new(root.to_path_buf())
    }

    #[test]
    fn exists_returns_false_when_ce_toml_not_present() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());
        let contest_dir = dir.path().join("solutions").join("abc334");
        fs::create_dir_all(&contest_dir).unwrap();
        // .ce.toml is NOT created
        let result = repo.exists("abc334").unwrap();
        assert!(!result);
    }

    #[test]
    fn exists_returns_true_when_ce_toml_present() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());
        let contest_dir = dir.path().join("solutions").join("abc334");
        fs::create_dir_all(&contest_dir).unwrap();
        fs::write(contest_dir.join(".ce.toml"), "online_judge = \"atcoder\"\n").unwrap();
        let result = repo.exists("abc334").unwrap();
        assert!(result);
    }

    #[test]
    fn exists_unstarted_returns_true_when_dir_exists_without_toml() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());
        let contest_dir = dir.path().join("solutions").join("abc334");
        fs::create_dir_all(&contest_dir).unwrap();
        // .ce.toml is NOT created
        let result = repo.exists_unstarted("abc334").unwrap();
        assert!(result);
    }

    #[test]
    fn exists_unstarted_returns_false_when_ce_toml_present() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());
        let contest_dir = dir.path().join("solutions").join("abc334");
        fs::create_dir_all(&contest_dir).unwrap();
        fs::write(contest_dir.join(".ce.toml"), "online_judge = \"atcoder\"\n").unwrap();
        let result = repo.exists_unstarted("abc334").unwrap();
        assert!(!result);
    }

    #[test]
    fn create_unstarted_creates_directory() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());
        repo.create_unstarted("abc334").unwrap();
        let contest_dir = dir.path().join("solutions").join("abc334");
        assert!(contest_dir.is_dir());
        assert!(!contest_dir.join(".ce.toml").exists());
    }

    fn make_contest() -> Contest {
        Contest {
            id: "abc334".to_string(),
            online_judge: OJKind::AtCoder,
            problems: vec![Problem {
                id: "abc334_a".to_string(),
                code: "a".to_string(),
                title: "Spoiler".to_string(),
                samples: vec![Sample {
                    input: "1\n".to_string(),
                    output: "2\n".to_string(),
                }],
            }],
        }
    }

    #[test]
    fn create_writes_ce_toml_and_testcases() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());
        let contest = make_contest();
        repo.create(&contest).unwrap();

        let contest_dir = dir.path().join("solutions").join("abc334");
        assert!(contest_dir.join(".ce.toml").exists());

        let tc_dir = contest_dir.join("testcases").join("a");
        assert!(tc_dir.join("1.in").exists());
        assert!(tc_dir.join("1.out").exists());
    }

    #[test]
    fn create_is_idempotent_for_ce_toml() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());
        let contest = make_contest();
        repo.create(&contest).unwrap();

        let toml_path = dir.path().join("solutions").join("abc334").join(".ce.toml");
        let content_before = fs::read_to_string(&toml_path).unwrap();
        let mtime_before = fs::metadata(&toml_path).unwrap().modified().unwrap();

        // Overwrite the toml with different content to detect if create() rewrites it
        fs::write(&toml_path, "# manually edited\n").unwrap();

        repo.create(&contest).unwrap();

        let content_after = fs::read_to_string(&toml_path).unwrap();
        // The file should NOT have been restored to the generated content (idempotent = no overwrite)
        assert_eq!(content_after, "# manually edited\n");
        let _ = (content_before, mtime_before); // used above
    }
}
