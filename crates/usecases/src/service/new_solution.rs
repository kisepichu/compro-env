use anyhow::Result;
use domain::entity::Solution;

use super::Service;

impl Service {
    /// Creates a new solution directory.
    pub fn new_solution(&self, _solution: Solution) -> Result<()> {
        todo!()
    }
}
