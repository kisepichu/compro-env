use anyhow::Result;
use domain::{entity::OJKind, error::CeError};

use super::Service;

impl Service {
    /// Returns the logged-in username. Returns CeError::SessionNotFound if not logged in.
    pub fn whoami(&self, oj: &OJKind) -> Result<String> {
        let session = self
            .session_repo
            .get(oj)?
            .ok_or_else(|| CeError::SessionNotFound { oj: oj.to_string() })?;
        self.online_judge.whoami(&session)
    }
}
