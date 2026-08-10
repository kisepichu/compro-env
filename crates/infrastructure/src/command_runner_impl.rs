//! Unix implementation of [`CommandRunner`](usecases::command_runner::CommandRunner).
//!
//! Each child runs in its own process group (`setpgid(0, 0)` via
//! `Command::process_group(0)`), so a timeout kill can reach every descendant
//! spawned by the command — the shell, the compiler it launched, and the test
//! binary that compiler produced — not only the top-level program. The runner
//! never wraps commands in `sh -c`; the caller is responsible for supplying an
//! argv (see spec §7.1).
//!
//! Timeout semantics: on expiry send `SIGTERM` to the process group and wait
//! up to five seconds for it to exit. If the group is still alive, escalate to
//! `SIGKILL` and reap the child. The returned outcome sets `timed_out = true`
//! and `exit_code = None`.
//!
//! The module is gated behind `#[cfg(unix)]` in `lib.rs`, so on non-Unix
//! targets it is simply not compiled — spec §7.1's "Unix 以外では unsupported"
//! becomes a compile-time absence rather than a runtime error.

use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use nix::errno::Errno;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

use usecases::command_runner::{CommandOutcome, CommandRequest, CommandRunner};

/// Grace period between SIGTERM and SIGKILL. Matches spec §7.1.
const KILL_GRACE: Duration = Duration::from_secs(5);
/// Polling interval when waiting for the child. Small enough to return
/// promptly on timeout, large enough to keep CPU use negligible.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// [`CommandRunner`] implementation for Unix targets.
///
/// Stateless — construct with `UnixCommandRunner` and reuse freely.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnixCommandRunner;

impl CommandRunner for UnixCommandRunner {
    fn run_streaming(&self, request: &CommandRequest) -> Result<CommandOutcome> {
        let mut command = Command::new(&request.program);
        command
            .args(&request.arguments)
            .current_dir(&request.current_dir)
            // Replace, not merge: only variables from `request.environment` are
            // visible to the child.
            .env_clear()
            .envs(&request.environment)
            // Inherit the parent's stdio so output streams live to the terminal
            // or CI log while the command is running.
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            // Put the child in a fresh process group; timeout kills target the
            // whole group so descendants are terminated with it.
            .process_group(0);

        let mut child = command.spawn().with_context(|| {
            format!(
                "failed to spawn command: {:?} {:?}",
                request.program, request.arguments
            )
        })?;
        let child_pid = Pid::from_raw(child.id() as i32);

        let deadline = Instant::now() + request.timeout;

        // Wait for the child to exit, up to `timeout`.
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Ok(CommandOutcome {
                        exit_code: status.code(),
                        timed_out: false,
                    });
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(e) => {
                    return Err(e).context("failed to poll child process status");
                }
            }
        }

        // Timeout hit: signal the whole process group. Send SIGTERM first,
        // wait up to KILL_GRACE, then SIGKILL. Errors from `killpg` when the
        // group has already exited (ESRCH) are ignored.
        let _ = killpg(child_pid, Signal::SIGTERM);
        let grace_deadline = Instant::now() + KILL_GRACE;
        while Instant::now() < grace_deadline {
            match child.try_wait() {
                Ok(Some(_)) => {
                    return Ok(CommandOutcome {
                        exit_code: None,
                        timed_out: true,
                    });
                }
                Ok(None) => std::thread::sleep(POLL_INTERVAL),
                Err(e) => return Err(e).context("failed to poll child during SIGTERM grace"),
            }
        }

        // Still alive — force-kill the group and reap.
        match killpg(child_pid, Signal::SIGKILL) {
            Ok(()) | Err(Errno::ESRCH) => {}
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "failed to SIGKILL child process group (pid {child_pid}): {e}"
                ));
            }
        }
        // Reap the child so we don't leave a zombie. `wait` blocks, but the
        // group has just been SIGKILLed so it returns nearly immediately.
        let _ = child.wait();

        Ok(CommandOutcome {
            exit_code: None,
            timed_out: true,
        })
    }
}
