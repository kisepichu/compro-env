//! Command runner abstraction used by `ce check`, `ce test`, and future verify flows.
//!
//! The trait describes a single blocking execution of an external process. Concrete
//! implementations own OS-level details such as process groups and signal handling
//! (see `infrastructure::command_runner_impl::UnixCommandRunner`), while callers work
//! only against this port.
//!
//! `run_streaming` inherits the parent's stdout/stderr so that build output, test
//! runners, and lint tools display live in the terminal or CI log without buffering.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;

/// Description of a single command to run.
///
/// `environment` REPLACES the entire environment of the child; callers must include
/// every variable they need (e.g. `PATH`). This keeps the executed environment
/// reproducible and avoids leaking secrets from the parent process.
#[derive(Debug, Clone)]
pub struct CommandRequest {
    /// Executable to run. The runner spawns the program directly with `argv` and
    /// never wraps it in a shell.
    pub program: OsString,
    /// Arguments passed to the program, in order.
    pub arguments: Vec<OsString>,
    /// Working directory for the child process.
    pub current_dir: PathBuf,
    /// Full environment for the child (replaces, not merges).
    pub environment: BTreeMap<OsString, OsString>,
    /// Maximum wall-clock duration before the runner terminates the child.
    pub timeout: Duration,
}

/// Result of a completed command execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    /// Exit code reported by the child, or `None` if the process was killed
    /// (either by the timeout logic or by an external signal).
    pub exit_code: Option<i32>,
    /// Whether the runner had to terminate the child because it exceeded `timeout`.
    pub timed_out: bool,
}

/// Runs external commands with live-streamed stdout/stderr and timeout enforcement.
///
/// Implementations are responsible for isolating each child in its own process
/// group so that timeout kills reach child processes spawned by the command
/// (compilers, test runners, etc.), not only the immediate program.
pub trait CommandRunner {
    /// Runs the command described by `request` synchronously, streaming its
    /// stdout/stderr to the parent's terminal or CI log while it runs.
    fn run_streaming(&self, request: &CommandRequest) -> Result<CommandOutcome>;
}
