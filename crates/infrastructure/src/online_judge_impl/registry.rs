use anyhow::{Result, bail};
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
            // LibraryChecker is detected (Phase C) but its OnlineJudge impl lands in
            // Phase D (TASK-036). Return a clean error instead of panicking so the
            // binary stays usable until then.
            OJKind::LibraryChecker => bail!("LibraryChecker is not yet implemented (TASK-036)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_librarychecker_errors_until_implemented() {
        let registry = OnlineJudgeRegistryImpl::new().expect("registry constructs");
        let result = registry.get(&OJKind::LibraryChecker);
        assert!(result.is_err());
    }
}
