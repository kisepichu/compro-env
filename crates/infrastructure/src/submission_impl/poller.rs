//! Production `PollerRegistry` factory.
//!
//! AtCoder is `InteractiveUntrackable` and has no poller; only LibraryChecker
//! registers here.

use anyhow::Result;
use domain::entity::OJKind;
use usecases::submission::PollerRegistry;

use crate::online_judge_impl::librarychecker::submission::LibraryCheckerPoller;

/// Builds a `PollerRegistry` with every supported OJ's poller registered.
pub fn build_poller_registry() -> Result<PollerRegistry> {
    let mut registry = PollerRegistry::new();
    registry.register(
        OJKind::LibraryChecker,
        Box::new(LibraryCheckerPoller::new()?),
    );
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_poller_registry_contains_librarychecker() {
        let registry = build_poller_registry().expect("registry constructs");
        assert!(registry.contains(&OJKind::LibraryChecker));
    }
}
