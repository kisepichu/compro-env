use anyhow::{Context, Result};
use domain::entity::{Contest, OJKind, Problem, Sample};
use serde::{Deserialize, Serialize};
use usecases::repository::contest_repository::ContestRepository;

#[derive(Serialize)]
struct CeToml<'a> {
    online_judge: &'a str,
    contest_id: &'a str,
    problems: Vec<CeTomlProblem<'a>>,
}

#[derive(Serialize)]
struct CeTomlProblem<'a> {
    id: &'a str,
    code: &'a str,
    title: &'a str,
}

/// Owned version for deserialization.
#[derive(Deserialize)]
struct CeTomlOwned {
    online_judge: String,
    #[serde(default)]
    problems: Vec<CeTomlProblemOwned>,
}

#[derive(Deserialize)]
struct CeTomlProblemOwned {
    id: String,
    code: String,
    title: String,
}

impl ContestRepositoryImpl {
    fn read_ce_toml(&self, contest_id: &str) -> Result<CeTomlOwned> {
        let path = self.ce_toml_path(contest_id);
        let contents =
            std::fs::read_to_string(&path).with_context(|| format!("failed to read {path:?}"))?;
        toml::from_str(&contents).with_context(|| format!("failed to parse {path:?}"))
    }
}

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
            let data = CeToml {
                online_judge: contest.online_judge.as_str(),
                contest_id: &contest.id,
                problems: contest
                    .problems
                    .iter()
                    .map(|p| CeTomlProblem {
                        id: &p.id,
                        code: &p.code,
                        title: &p.title,
                    })
                    .collect(),
            };
            let toml = toml::to_string(&data)
                .map_err(|e| anyhow::anyhow!("failed to serialize .ce.toml: {e}"))?;
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

    fn get_oj_kind(&self, contest_id: &str) -> Result<OJKind> {
        let data = self.read_ce_toml(contest_id)?;
        data.online_judge
            .parse::<OJKind>()
            .map_err(|e| anyhow::anyhow!(e))
    }

    fn get_samples(&self, contest_id: &str, problem_code: &str) -> Result<Vec<Sample>> {
        let tc_dir = self
            .contest_dir(contest_id)
            .join("testcases")
            .join(problem_code);
        if !tc_dir.is_dir() {
            return Ok(vec![]);
        }
        let mut samples = Vec::new();
        let mut n = 1usize;
        loop {
            let in_path = tc_dir.join(format!("{n}.in"));
            let out_path = tc_dir.join(format!("{n}.out"));
            if !in_path.exists() {
                break;
            }
            let input = std::fs::read_to_string(&in_path)
                .with_context(|| format!("failed to read {in_path:?}"))?;
            let output = std::fs::read_to_string(&out_path)
                .with_context(|| format!("failed to read {out_path:?}"))?;
            samples.push(Sample { input, output });
            n += 1;
        }
        Ok(samples)
    }

    fn list_problem_codes(&self, contest_id: &str) -> Result<Vec<String>> {
        let tc_dir = self.contest_dir(contest_id).join("testcases");
        if !tc_dir.is_dir() {
            return Ok(vec![]);
        }
        let mut codes: Vec<String> = std::fs::read_dir(&tc_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        codes.sort();
        Ok(codes)
    }

    fn testcases_dir(&self, contest_id: &str, problem_code: &str) -> std::path::PathBuf {
        self.root
            .join("solutions")
            .join(contest_id)
            .join("testcases")
            .join(problem_code)
    }

    fn get_problem(&self, contest_id: &str, problem_code: &str) -> Result<Problem> {
        let data = self.read_ce_toml(contest_id)?;
        data.problems
            .into_iter()
            .find(|p| p.code == problem_code)
            .map(|p| Problem {
                id: p.id,
                code: p.code,
                title: p.title,
                samples: vec![],
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "problem code {:?} not found in .ce.toml for contest {:?}",
                    problem_code,
                    contest_id
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::entity::{Contest, OJKind, Problem, Sample};
    use serial_test::serial;
    use std::fs;

    fn make_repo(root: &std::path::Path) -> ContestRepositoryImpl {
        ContestRepositoryImpl::new(root.to_path_buf())
    }

    #[test]
    #[serial]
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
    #[serial]
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
    #[serial]
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
    #[serial]
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
    #[serial]
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
    #[serial]
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
    #[serial]
    fn list_problem_codes_returns_sorted_codes_from_testcases_dir() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());
        let contest = make_contest();
        repo.create(&contest).unwrap();

        let codes = repo.list_problem_codes("abc334").unwrap();
        assert_eq!(codes, vec!["a"]);
    }

    #[test]
    #[serial]
    fn list_problem_codes_returns_empty_when_no_testcases_dir() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());

        let codes = repo.list_problem_codes("abc334").unwrap();
        assert!(codes.is_empty());
    }

    #[test]
    #[serial]
    fn get_oj_kind_reads_ce_toml() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());
        let contest = make_contest();
        repo.create(&contest).unwrap();

        let oj = repo.get_oj_kind("abc334").unwrap();
        assert_eq!(oj, OJKind::AtCoder);
    }

    #[test]
    #[serial]
    fn get_samples_reads_testcase_files() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());
        let contest = make_contest();
        repo.create(&contest).unwrap();

        let samples = repo.get_samples("abc334", "a").unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].input, "1\n");
        assert_eq!(samples[0].output, "2\n");
    }

    #[test]
    #[serial]
    fn get_samples_returns_empty_when_no_testcases() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());

        let samples = repo.get_samples("abc334", "a").unwrap();
        assert!(samples.is_empty());
    }

    #[test]
    #[serial]
    fn get_problem_returns_problem_matching_code() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());
        let contest = make_contest();
        repo.create(&contest).unwrap();

        let problem = repo.get_problem("abc334", "a").unwrap();
        assert_eq!(problem.id, "abc334_a");
        assert_eq!(problem.title, "Spoiler");
    }

    #[test]
    #[serial]
    fn get_problem_returns_error_when_code_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let repo = make_repo(dir.path());
        let contest = make_contest();
        repo.create(&contest).unwrap();

        let result = repo.get_problem("abc334", "z");
        assert!(result.is_err());
    }

    #[test]
    #[serial]
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
