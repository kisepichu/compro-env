use thiserror::Error;

/// Domain errors that can be matched. The shell layer downcasts these to control display.
#[derive(Error, Debug)]
pub enum CeError {
    #[error("not logged in for {oj}")]
    NotLoggedIn { oj: String },

    #[error("contest not found: {contest_id}")]
    ContestNotFound { contest_id: String },

    #[error("problem not found: {problem_code} in {contest_id}")]
    ProblemNotFound {
        contest_id: String,
        problem_code: String,
    },

    #[error("solution not found: {solution_name}")]
    SolutionNotFound { solution_name: String },

    #[error("session not set for {oj}. run `ce login` first.")]
    SessionNotFound { oj: String },

    #[error("contest has not started yet: {contest_id}")]
    ContestNotStarted { contest_id: String },
}
