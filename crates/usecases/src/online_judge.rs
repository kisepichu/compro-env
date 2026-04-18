use anyhow::Result;
use chrono::{DateTime, Utc};
use domain::entity::{Problem, Session, SubmitResult};

pub struct ContestMeta {
    pub start_time: Option<DateTime<Utc>>,
    pub problem_id_hints: Vec<(String, String)>, // (problem_code, problem_id)
}

pub trait OnlineJudge {
    fn name(&self) -> &str;

    /// Returns the username of the currently logged-in user.
    fn whoami(&self, session: &Session) -> Result<String>;

    /// Returns contest metadata including start time and problem id hints.
    fn get_contest_meta(&self, contest_id: &str) -> Result<ContestMeta>;

    /// Fetches all problems with their samples.
    /// Public contests do not require a session; private contests require Some(&session).
    fn get_problems_detail(
        &self,
        contest_id: &str,
        session: Option<&Session>,
        problem_id_hints: &[(String, String)],
    ) -> Result<Vec<Problem>>;

    /// Submits a solution to the OJ.
    fn submit(
        &self,
        contest_id: &str,
        problem_id: &str,
        lang_id: &str,
        source: &str,
        session: &Session,
    ) -> Result<SubmitResult>;
}
