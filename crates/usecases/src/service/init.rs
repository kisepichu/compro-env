use anyhow::Result;
use domain::entity::{Language, OJKind, Solution};

use super::Service;

pub struct InitResult {
    pub contest_id: String,
    pub created_solutions: Vec<Solution>,
}

impl Service {
    /// Initializes a contest: fetches problems, saves test cases, and creates solution directories.
    pub fn init(&self, _contest_id: &str, _oj: OJKind, _lang: &Language) -> Result<InitResult> {
        todo!()
    }
}
