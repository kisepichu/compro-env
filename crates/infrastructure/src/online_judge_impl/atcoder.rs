use anyhow::Result;
use domain::entity::{Problem, Session, SubmitResult};
use usecases::online_judge::OnlineJudge;

pub struct AtCoder;

impl OnlineJudge for AtCoder {
    fn name(&self) -> &str {
        "atcoder"
    }

    fn whoami(&self, _session: &Session) -> Result<String> {
        todo!()
    }

    fn get_problems_detail(
        &self,
        _contest_id: &str,
        _session: Option<&Session>,
    ) -> Result<Vec<Problem>> {
        todo!()
    }

    fn submit(
        &self,
        _contest_id: &str,
        _problem_id: &str,
        _lang_id: &str,
        _source: &str,
        _session: &Session,
    ) -> Result<SubmitResult> {
        todo!()
    }

    fn wait_for_start(&self, _contest_id: &str) -> Result<()> {
        todo!()
    }
}
