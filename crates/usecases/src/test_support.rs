//! Test-only helpers shared across `service::*` unit tests.
//!
//! Compiled only under `cfg(test)`. Nothing in production code depends on it.

use anyhow::Result;
use std::process::Command;

use crate::command_runner::{CommandOutcome, CommandRequest, CommandRunner};

/// Minimal shell-based [`CommandRunner`] used by the `service` unit tests.
///
/// The unit tests do not want to depend on the timeout-aware
/// `UnixCommandRunner` (which lives in `infrastructure`), but they still need
/// the behaviour of "spawn `sh -c <script>` and return its exit code" — this is
/// what `Service::test` used to do before the runner port was introduced.
///
/// This stub does not enforce timeouts because the migrated `Service::test`
/// tests only assert exit codes (0 vs 1) and error messages, never timeouts.
#[derive(Debug, Default, Clone, Copy)]
pub struct SpawningCommandRunner;

impl CommandRunner for SpawningCommandRunner {
    fn run_streaming(&self, request: &CommandRequest) -> Result<CommandOutcome> {
        let mut cmd = Command::new(&request.program);
        cmd.args(&request.arguments)
            .current_dir(&request.current_dir)
            .env_clear()
            .envs(&request.environment);
        let status = cmd.status()?;
        Ok(CommandOutcome {
            exit_code: status.code(),
            timed_out: false,
        })
    }
}
