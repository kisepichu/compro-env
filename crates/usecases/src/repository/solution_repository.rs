use anyhow::Result;
use domain::entity::{Language, Sample, Solution};

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
    /// Tera context includes: contest.id, problem.code, problem.title, solution.name, samples
    fn create(&self, solution: &Solution, samples: &[Sample]) -> Result<()>;

    /// Reads the source code for submission.
    fn get_source(&self, solution: &Solution) -> Result<String>;

    /// Returns the path to the solution directory (does not check existence).
    fn solution_dir(
        &self,
        contest_id: &str,
        problem_code: &str,
        solution_name: &str,
    ) -> std::path::PathBuf;
}
