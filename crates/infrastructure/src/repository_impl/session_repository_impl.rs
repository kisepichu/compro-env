use anyhow::Result;
use domain::entity::{OJKind, Session};
use usecases::repository::session_repository::SessionRepository;

pub struct SessionRepositoryImpl;

impl SessionRepository for SessionRepositoryImpl {
    fn get(&self, _oj: &OJKind) -> Result<Option<Session>> {
        todo!()
    }

    fn save(&self, _session: &Session) -> Result<()> {
        todo!()
    }
}
