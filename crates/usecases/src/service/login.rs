use anyhow::Result;
use domain::entity::OJKind;

use super::Service;
use crate::online_judge::Credentials;

impl Service {
    /// Authenticates with the OJ using the given credentials and saves the session.
    ///
    /// The OJ implementation decides how to turn credentials into a `Session`
    /// (AtCoder wraps a cookie; OJs with programmatic login obtain a token).
    pub fn login(&self, oj: OJKind, credentials: Credentials) -> Result<()> {
        let session = self.online_judge(&oj)?.login(&credentials)?;
        self.session_repo.save(&session)
    }
}
