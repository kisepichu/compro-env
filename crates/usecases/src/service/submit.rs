use anyhow::Result;
use domain::entity::{Solution, SubmitResult};

use super::Service;

impl Service {
    /// Submits the solution to the OJ.
    pub fn submit(&self, _solution: &Solution) -> Result<SubmitResult> {
        todo!()
    }
}
