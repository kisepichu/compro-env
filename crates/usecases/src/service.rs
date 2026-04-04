use crate::{
    config::Config,
    online_judge::OnlineJudge,
    repository::{
        contest_repository::ContestRepository, session_repository::SessionRepository,
        solution_repository::SolutionRepository,
    },
};

pub mod init;
pub mod login;
pub mod new_solution;
pub mod submit;
pub mod test;
pub mod whoami;

pub struct Service {
    pub online_judge: Box<dyn OnlineJudge>,
    pub contest_repo: Box<dyn ContestRepository>,
    pub solution_repo: Box<dyn SolutionRepository>,
    pub session_repo: Box<dyn SessionRepository>,
    pub config: Box<dyn Config>,
}

impl Service {
    pub fn new(
        online_judge: Box<dyn OnlineJudge>,
        contest_repo: Box<dyn ContestRepository>,
        solution_repo: Box<dyn SolutionRepository>,
        session_repo: Box<dyn SessionRepository>,
        config: Box<dyn Config>,
    ) -> Self {
        Self {
            online_judge,
            contest_repo,
            solution_repo,
            session_repo,
            config,
        }
    }
}
