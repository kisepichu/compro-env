use anyhow::Result;
use domain::entity::OJKind;
use usecases::online_judge::{OnlineJudge, OnlineJudgeRegistry};

use crate::online_judge_impl::atcoder::AtCoder;

/// Production registry mapping each `OJKind` to its `OnlineJudge` implementation.
pub struct OnlineJudgeRegistryImpl {
    atcoder: AtCoder,
}

impl OnlineJudgeRegistryImpl {
    pub fn new() -> Result<Self> {
        Ok(Self {
            atcoder: AtCoder::new()?,
        })
    }
}

impl OnlineJudgeRegistry for OnlineJudgeRegistryImpl {
    fn get(&self, oj: &OJKind) -> Result<&dyn OnlineJudge> {
        match oj {
            OJKind::AtCoder => Ok(&self.atcoder),
        }
    }
}
