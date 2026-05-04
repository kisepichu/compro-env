use anyhow::Result;
use domain::entity::{Contest, OJKind, Problem, Sample};
use std::path::PathBuf;

pub trait ContestRepository {
    /// Returns true if the contest directory exists.
    fn exists(&self, contest_id: &str) -> Result<bool>;

    /// Returns true if an unstarted placeholder for the contest exists.
    fn exists_unstarted(&self, contest_id: &str) -> Result<bool>;

    /// Creates a placeholder directory for a contest that has not started yet.
    fn create_unstarted(&self, contest_id: &str) -> Result<()>;

    /// Creates the contest directory, writes .ce.toml, and saves test cases.
    fn create(&self, contest: &Contest) -> Result<()>;

    /// Reads the OJ kind from .ce.toml.
    fn get_oj_kind(&self, contest_id: &str) -> Result<OJKind>;

    /// Reads sample I/O from testcases/{problem_code}/.
    fn get_samples(&self, contest_id: &str, problem_code: &str) -> Result<Vec<Sample>>;

    /// Returns the list of problem codes found under testcases/.
    fn list_problem_codes(&self, contest_id: &str) -> Result<Vec<String>>;

    /// Returns the absolute path to testcases/{problem_code}/ under the contest directory.
    fn testcases_dir(&self, contest_id: &str, problem_code: &str) -> PathBuf;

    /// Returns the Problem whose `code` matches `problem_code` from .ce.toml.
    /// Returns an error if the problem is not found.
    fn get_problem(&self, contest_id: &str, problem_code: &str) -> Result<Problem>;
}
