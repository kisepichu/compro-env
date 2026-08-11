use anyhow::Result;
use domain::entity::OJKind;

use crate::{
    check::{CheckSelection, CheckSummary, run_checks},
    command_runner::CommandRunner,
    config::Config,
    online_judge::{OnlineJudge, OnlineJudgeRegistry},
    repository::{
        contest_repository::ContestRepository, session_repository::SessionRepository,
        solution_repository::SolutionRepository, verification_repository::VerificationRepository,
    },
    submission::{PollerRegistry, RecoveryRegistry, StarterRegistry},
};

pub mod init;
pub mod login;
pub mod logout;
pub mod new_solution;
pub mod submit;
pub mod test;
pub mod verify;
pub mod whoami;

/// Optional cluster of ports that `ce verify` and the hidden `internal
/// verify-*` subcommands need on top of the plain `Service` wiring.
///
/// Held as `Option<_>` on [`Service`] so login/whoami/new/... paths continue
/// to construct the service without paying for a submission pipeline they do
/// not use. When verify is invoked without these ports wired we bail out with
/// a clear message rather than panic.
pub struct VerificationServices {
    pub pollers: PollerRegistry,
    pub recovery: RecoveryRegistry,
    pub verifications: Box<dyn VerificationRepository>,
}

pub struct Service {
    pub(crate) oj_registry: Box<dyn OnlineJudgeRegistry>,
    pub(crate) starter_registry: StarterRegistry,
    pub(crate) contest_repo: Box<dyn ContestRepository>,
    pub(crate) solution_repo: Box<dyn SolutionRepository>,
    pub(crate) session_repo: Box<dyn SessionRepository>,
    pub(crate) config: Box<dyn Config>,
    pub(crate) command_runner: Box<dyn CommandRunner>,
    pub(crate) verification: Option<VerificationServices>,
}

impl Service {
    pub fn new(
        oj_registry: Box<dyn OnlineJudgeRegistry>,
        starter_registry: StarterRegistry,
        contest_repo: Box<dyn ContestRepository>,
        solution_repo: Box<dyn SolutionRepository>,
        session_repo: Box<dyn SessionRepository>,
        config: Box<dyn Config>,
        command_runner: Box<dyn CommandRunner>,
    ) -> Self {
        Self {
            oj_registry,
            starter_registry,
            contest_repo,
            solution_repo,
            session_repo,
            config,
            command_runner,
            verification: None,
        }
    }

    /// Constructor variant for callers that also need the verification
    /// pipeline (pollers, recovery, and a verification-record repository).
    #[allow(clippy::too_many_arguments)]
    pub fn with_verification(
        oj_registry: Box<dyn OnlineJudgeRegistry>,
        starter_registry: StarterRegistry,
        contest_repo: Box<dyn ContestRepository>,
        solution_repo: Box<dyn SolutionRepository>,
        session_repo: Box<dyn SessionRepository>,
        config: Box<dyn Config>,
        command_runner: Box<dyn CommandRunner>,
        verification: VerificationServices,
    ) -> Self {
        Self {
            oj_registry,
            starter_registry,
            contest_repo,
            solution_repo,
            session_repo,
            config,
            command_runner,
            verification: Some(verification),
        }
    }

    /// Runs project-local library checks (`ce check`). Delegates to
    /// [`crate::check::run_checks`] using the injected command runner.
    pub fn check(
        &self,
        config: &domain::library::LibraryProjectConfig,
        selection: &CheckSelection,
        repository_root: &std::path::Path,
    ) -> Result<CheckSummary> {
        run_checks(
            config,
            selection,
            self.command_runner.as_ref(),
            repository_root,
        )
    }

    /// Resolves the `OnlineJudge` implementation for `oj` via the registry.
    pub(crate) fn online_judge(&self, oj: &OJKind) -> Result<&dyn OnlineJudge> {
        self.oj_registry.get(oj)
    }

    /// Access the injected [`Config`] port.
    pub fn config(&self) -> &dyn Config {
        self.config.as_ref()
    }

    /// Access the injected [`CommandRunner`] port.
    pub fn command_runner(&self) -> &dyn CommandRunner {
        self.command_runner.as_ref()
    }

    /// Access the injected [`SessionRepository`] port.
    pub fn session_repository(&self) -> &dyn SessionRepository {
        self.session_repo.as_ref()
    }

    /// Access the registered [`StarterRegistry`].
    pub fn starter_registry(&self) -> &StarterRegistry {
        &self.starter_registry
    }

    /// Access the optional verification bundle. `None` when the `Service`
    /// was constructed via [`Service::new`] rather than
    /// [`Service::with_verification`].
    pub fn verification_services(&self) -> Option<&VerificationServices> {
        self.verification.as_ref()
    }
}
