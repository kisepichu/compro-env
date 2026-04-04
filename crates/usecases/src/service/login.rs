use anyhow::Result;
use domain::entity::{OJKind, Session};

use super::Service;

impl Service {
    /// Saves the REVEL_SESSION cookie to the session repository.
    pub fn login(&self, oj: OJKind, cookie: String) -> Result<()> {
        anyhow::ensure!(!cookie.is_empty(), "cookie must not be empty");
        let session = Session {
            online_judge: oj,
            cookie,
        };
        self.session_repo.save(&session)
    }
}
