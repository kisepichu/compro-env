//! GitHub API clients used by the verify state-writer pipeline (spec §15.1,
//! §15.4).
//!
//! Only the constrained state writer lives here. The writer never spawns
//! `git`, never installs credentials, and never touches paths outside
//! `verification/results/**`; every mutating call is fenced by
//! guard clauses so the App token has no way to affect anything else in the
//! repository.

pub mod verification_state_writer;

pub use verification_state_writer::{
    BotPullRequestState, GitHubVerificationStateWriter, PersistError, PersistStateRequest,
    PersistedState, validate_result_path,
};
