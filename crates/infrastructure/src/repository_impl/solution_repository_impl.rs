use anyhow::Result;
use domain::entity::{Language, Solution};
use usecases::repository::solution_repository::SolutionRepository;

pub struct SolutionRepositoryImpl {
    root: std::path::PathBuf,
}

impl SolutionRepositoryImpl {
    pub fn new(root: std::path::PathBuf) -> Self {
        Self { root }
    }
}

impl SolutionRepository for SolutionRepositoryImpl {
    fn list(&self, _contest_id: &str, _problem_code: &str) -> Result<Vec<Solution>> {
        todo!()
    }

    fn exists(
        &self,
        _contest_id: &str,
        _problem_code: &str,
        _name: &str,
        _lang: &Language,
    ) -> Result<bool> {
        todo!()
    }

    fn create(&self, _solution: &Solution) -> Result<()> {
        todo!()
    }

    fn get_source(&self, _solution: &Solution) -> Result<String> {
        todo!()
    }
}
