//! Library platform orchestration service (spec §12, §14).
//!
//! Groups the ports that produce the immutable inputs for `project_site_data`
//! into one struct so higher layers can inject alternate implementations for
//! testing (fake analyzers, in-memory verification stores, deterministic git
//! histories). The service itself is a thin coordinator: each port stays
//! independently testable and no discovery, analysis, verification, or git
//! logic lives here.
//!
//! Wire-up of individual driver runs is intentionally left to the controller /
//! infrastructure layer (Task 3). The projection function is exposed as
//! [`crate::site_data::project_site_data`] so callers that already have a
//! snapshot in hand can bypass the service.

use std::marker::PhantomData;

/// Placeholder trait boundaries — concrete traits arrive with the site-data
/// controller in Task 3. The service keeps the four ports as generics so the
/// callers can inject mocks without touching the projection type.
pub struct LibraryPlatformService<D, A, V, G> {
    pub discovery: D,
    pub analyzer: A,
    pub verification: V,
    pub git_history: G,
    _marker: PhantomData<()>,
}

impl<D, A, V, G> LibraryPlatformService<D, A, V, G> {
    pub fn new(discovery: D, analyzer: A, verification: V, git_history: G) -> Self {
        Self {
            discovery,
            analyzer,
            verification,
            git_history,
            _marker: PhantomData,
        }
    }
}
