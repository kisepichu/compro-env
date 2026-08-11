//! Production `RecoveryRegistry` factory.
//!
//! Only LibraryChecker registers here; AtCoder is `InteractiveUntrackable`
//! and has no recovery adapter.

use anyhow::Result;
use domain::entity::OJKind;
use usecases::submission::RecoveryRegistry;

use crate::online_judge_impl::librarychecker::submission::LibraryCheckerRecovery;

/// Builds a `RecoveryRegistry` with every supported OJ's recovery adapter registered.
pub fn build_recovery_registry() -> Result<RecoveryRegistry> {
    let mut registry = RecoveryRegistry::new();
    registry.register(
        OJKind::LibraryChecker,
        Box::new(LibraryCheckerRecovery::new()?),
    );
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_recovery_registry_contains_librarychecker() {
        let registry = build_recovery_registry().expect("registry constructs");
        assert!(registry.contains(&OJKind::LibraryChecker));
    }
}
