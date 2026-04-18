use anyhow::Result;
use domain::entity::{Sample, Solution};

pub trait SolutionRepository {
    /// Scans the filesystem and returns the list of solutions.
    fn list(&self, contest_id: &str, problem_code: &str) -> Result<Vec<Solution>>;

    fn exists(&self, contest_id: &str, problem_code: &str, name: &str) -> Result<bool>;

    /// Creates the solution directory and expands templates.
    /// Tera context includes: contest.id, problem.code, problem.title, solution.name, samples
    fn create(&self, solution: &Solution, samples: &[Sample]) -> Result<()>;

    /// Reads the source code for submission from the given file path relative to the solution dir.
    fn get_source(&self, solution: &Solution, file_path: &str) -> Result<String>;

    /// Returns the path to the solution directory (does not check existence).
    fn solution_dir(
        &self,
        contest_id: &str,
        problem_code: &str,
        solution_name: &str,
    ) -> std::path::PathBuf;
}
