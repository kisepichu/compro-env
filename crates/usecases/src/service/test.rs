use anyhow::Result;
use domain::entity::Solution;

use super::Service;

pub struct TestResult {
    pub total: usize,
    pub passed: usize,
    pub cases: Vec<TestCaseResult>,
}

pub struct TestCaseResult {
    pub name: String,
    pub passed: bool,
    pub expected: String,
    pub actual: String,
    pub elapsed_ms: u64,
}

impl Service {
    /// Runs all sample test cases and returns the results.
    pub fn test(&self, _solution: &Solution) -> Result<TestResult> {
        todo!()
    }
}
