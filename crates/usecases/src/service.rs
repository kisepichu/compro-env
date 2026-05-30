use anyhow::Result;
use domain::entity::OJKind;

use crate::{
    config::Config,
    online_judge::{OnlineJudge, OnlineJudgeRegistry},
    repository::{
        contest_repository::ContestRepository, session_repository::SessionRepository,
        solution_repository::SolutionRepository,
    },
};

pub mod init;
pub mod login;
pub mod logout;
pub mod new_solution;
pub mod submit;
pub mod test;
pub mod whoami;

pub struct Service {
    pub(crate) oj_registry: Box<dyn OnlineJudgeRegistry>,
    pub(crate) contest_repo: Box<dyn ContestRepository>,
    pub(crate) solution_repo: Box<dyn SolutionRepository>,
    pub(crate) session_repo: Box<dyn SessionRepository>,
    pub(crate) config: Box<dyn Config>,
}

impl Service {
    pub fn new(
        oj_registry: Box<dyn OnlineJudgeRegistry>,
        contest_repo: Box<dyn ContestRepository>,
        solution_repo: Box<dyn SolutionRepository>,
        session_repo: Box<dyn SessionRepository>,
        config: Box<dyn Config>,
    ) -> Self {
        Self {
            oj_registry,
            contest_repo,
            solution_repo,
            session_repo,
            config,
        }
    }

    /// Resolves the `OnlineJudge` implementation for `oj` via the registry.
    pub(crate) fn online_judge(&self, oj: &OJKind) -> Result<&dyn OnlineJudge> {
        self.oj_registry.get(oj)
    }
}
