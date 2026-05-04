use anyhow::Result;
use domain::entity::OJKind;

use super::Service;

impl Service {
    /// Deletes the session for the given OJ from the session repository.
    /// Returns Ok(true) if a session was deleted, Ok(false) if none was present.
    pub fn logout(&self, oj: &OJKind) -> Result<bool> {
        self.session_repo.delete(oj)
    }
}
