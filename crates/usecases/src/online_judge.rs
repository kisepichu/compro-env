use anyhow::Result;
use domain::entity::{Problem, Session, SubmitResult};

pub trait OnlineJudge {
    fn name(&self) -> &str;

    /// Returns the username of the currently logged-in user.
    fn whoami(&self, session: &Session) -> Result<String>;

    /// Fetches all problems with their samples.
    /// Public contests do not require a session; private contests require Some(&session).
    fn get_problems_detail(
        &self,
        contest_id: &str,
        session: Option<&Session>,
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

    /// Waits until the contest starts (polling).
    fn wait_for_start(&self, contest_id: &str) -> Result<()>;
}
