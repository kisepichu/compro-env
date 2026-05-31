use anyhow::Result;
use domain::entity::OJKind;
use usecases::online_judge::{OnlineJudge, OnlineJudgeRegistry};

use crate::online_judge_impl::atcoder::AtCoder;
use crate::online_judge_impl::librarychecker::LibraryChecker;

/// Production registry mapping each `OJKind` to its `OnlineJudge` implementation.
pub struct OnlineJudgeRegistryImpl {
    atcoder: AtCoder,
    librarychecker: LibraryChecker,
}

impl OnlineJudgeRegistryImpl {
    pub fn new() -> Result<Self> {
        Ok(Self {
            atcoder: AtCoder::new()?,
            librarychecker: LibraryChecker::new()?,
        })
    }
}

impl OnlineJudgeRegistry for OnlineJudgeRegistryImpl {
    fn get(&self, oj: &OJKind) -> Result<&dyn OnlineJudge> {
        match oj {
            OJKind::AtCoder => Ok(&self.atcoder),
            OJKind::LibraryChecker => Ok(&self.librarychecker),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_resolves_both_ojs() {
        let registry = OnlineJudgeRegistryImpl::new().expect("registry constructs");
        assert_eq!(
            registry
                .get(&OJKind::AtCoder)
                .expect("atcoder resolves")
                .name(),
            "atcoder"
        );
        assert_eq!(
            registry
                .get(&OJKind::LibraryChecker)
                .expect("librarychecker resolves")
                .name(),
            "librarychecker"
        );
    }
}
