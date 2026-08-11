//! Production `StarterRegistry` factory that wires every supported OJ's
//! `SubmissionStarter`.

use anyhow::Result;
use domain::entity::OJKind;
use usecases::submission::StarterRegistry;

use crate::submission_impl::{atcoder::AtCoderStarter, librarychecker::LibraryCheckerStarter};

/// Builds a `StarterRegistry` with every supported OJ's starter registered.
pub fn build_starter_registry() -> Result<StarterRegistry> {
    let mut registry = StarterRegistry::new();
    registry.register(OJKind::AtCoder, Box::new(AtCoderStarter::new()?));
    registry.register(
        OJKind::LibraryChecker,
        Box::new(LibraryCheckerStarter::new()?),
    );
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_resolves_both_ojs() {
        let registry = build_starter_registry().expect("registry constructs");
        assert!(registry.contains(&OJKind::AtCoder));
        assert!(registry.contains(&OJKind::LibraryChecker));
    }
}
