use anyhow::Result;
use domain::entity::{Language, Solution};

pub trait SolutionRepository {
    /// Scans the filesystem and returns the list of solutions.
    fn list(&self, contest_id: &str, problem_code: &str) -> Result<Vec<Solution>>;

    fn exists(
        &self,
        contest_id: &str,
        problem_code: &str,
        name: &str,
        lang: &Language,
    ) -> Result<bool>;

    /// Creates the solution directory, expands templates, and updates Cargo.toml members.
    fn create(&self, solution: &Solution) -> Result<()>;

    /// Reads the source code for submission.
    fn get_source(&self, solution: &Solution) -> Result<String>;
}
