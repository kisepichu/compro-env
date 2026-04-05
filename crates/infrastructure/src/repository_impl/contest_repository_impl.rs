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

impl ContestRepository for ContestRepositoryImpl {
    fn exists(&self, _contest_id: &str) -> Result<bool> {
        todo!()
    }

    fn exists_unstarted(&self, _contest_id: &str) -> Result<bool> {
        todo!()
    }

    fn create_unstarted(&self, _contest_id: &str) -> Result<()> {
        todo!()
    }

    fn create(&self, _contest: &Contest) -> Result<()> {
        todo!()
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
