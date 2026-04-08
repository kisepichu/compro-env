use anyhow::Result;
use domain::entity::{OJKind, Session};

pub trait SessionRepository {
    /// Reads a session from ~/.config/ce/session.toml.
    fn get(&self, oj: &OJKind) -> Result<Option<Session>>;

    /// Saves a session to ~/.config/ce/session.toml.
    fn save(&self, session: &Session) -> Result<()>;

    /// Deletes the session for the given OJ.
    /// Returns Ok(true) if a session was deleted, Ok(false) if none was present.
    fn delete(&self, oj: &OJKind) -> Result<bool>;
}
