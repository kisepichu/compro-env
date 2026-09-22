//! Strict adapter process runner (spec §6.9).
//!
//! Executes an adapter binary with piped stdin/stdout/stderr, a cleared
//! environment plus an explicit allowlist, and a hard timeout. Rejects any
//! response that is not a single UTF-8 JSON document matching the shared
//! protocol version. On Unix the child is placed in its own session so a
//! timeout can terminate the whole process group instead of just the launcher.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use library_adapter_protocol::{
    AnalysisRequest, AnalysisResponse, ProtocolVersionError, SCHEMA_VERSION, validate_version,
};
use usecases::library_adapter::{AdapterRunError, LibraryAdapterRunner};
use wait_timeout::ChildExt;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

/// Spec §6.3: MVP hard limit on adapter stdout.
pub const DEFAULT_STDOUT_LIMIT_BYTES: usize = 64 * 1024 * 1024;

/// Bounded stderr tail kept in memory for error messages. The full stderr is
/// still available on the child's terminal or CI log; this limit only affects
/// how much of it is embedded in `AdapterRunError`.
pub const DEFAULT_STDERR_TAIL_BYTES: usize = 8 * 1024;

/// Concrete `LibraryAdapterRunner` that shells out to a real executable.
///
/// The runner keeps no environment of its own; each `analyze` call receives
/// the sanitized env directly so per-language runtime env (e.g. Lean's
/// `CE_LEAN_ROOT`) can flow through the same runner instance.
pub struct ProcessLibraryAdapterRunner {
    working_directory: PathBuf,
    stdout_limit_bytes: usize,
    stderr_tail_bytes: usize,
    extra_args: Vec<String>,
}

impl ProcessLibraryAdapterRunner {
    pub fn new(working_directory: PathBuf) -> Self {
        Self {
            working_directory,
            stdout_limit_bytes: DEFAULT_STDOUT_LIMIT_BYTES,
            stderr_tail_bytes: DEFAULT_STDERR_TAIL_BYTES,
            extra_args: Vec::new(),
        }
    }

    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }

    pub fn with_stdout_limit_bytes(mut self, limit: usize) -> Self {
        self.stdout_limit_bytes = limit;
        self
    }

    pub fn with_stderr_tail_bytes(mut self, limit: usize) -> Self {
        self.stderr_tail_bytes = limit;
        self
    }
}

impl LibraryAdapterRunner for ProcessLibraryAdapterRunner {
    fn analyze(
        &self,
        executable: &Path,
        request: &AnalysisRequest,
        timeout: Duration,
        environment: &BTreeMap<String, String>,
    ) -> Result<AnalysisResponse, AdapterRunError> {
        let command_display = executable.display().to_string();

        let request_json = serde_json::to_vec(request)
            .map_err(|source| AdapterRunError::RequestSerialization { source })?;

        let mut cmd = Command::new(executable);
        cmd.args(&self.extra_args);
        cmd.current_dir(&self.working_directory);
        cmd.env_clear();
        for (k, v) in environment {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        #[cfg(unix)]
        {
            // Place the child in its own session so timeouts can terminate the
            // whole process group. `setsid` succeeds unconditionally in a fresh
            // fork because the child is never already a session leader.
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }

        let mut child = spawn_with_etxtbsy_retry(&mut cmd, &command_display)?;

        let pid = child.id();
        let mut stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        // Concurrent I/O prevents pipe deadlocks: we drain stdout/stderr while
        // pushing the request onto stdin.
        let write_join = thread::spawn(move || -> std::io::Result<()> {
            stdin.write_all(&request_json)?;
            drop(stdin);
            Ok(())
        });

        let stdout_limit = self.stdout_limit_bytes;
        let stdout_join = thread::spawn(move || read_stdout_bounded(stdout, stdout_limit));

        let stderr_tail = self.stderr_tail_bytes;
        let stderr_join = thread::spawn(move || read_stderr_tail(stderr, stderr_tail));

        let status_result = child.wait_timeout(timeout);
        match status_result {
            Ok(Some(status)) => {
                let _ = write_join.join();
                let stdout_result = stdout_join
                    .join()
                    .unwrap_or_else(|_| Ok(ReadStdout::completed(Vec::new())));
                let stderr_bytes = stderr_join.join().unwrap_or_default();
                let stderr_tail_string = String::from_utf8_lossy(&stderr_bytes).into_owned();

                let stdout = match stdout_result {
                    Ok(stdout) => stdout,
                    Err(source) => {
                        return Err(AdapterRunError::Io {
                            command: command_display,
                            source,
                        });
                    }
                };

                if !status.success() {
                    return Err(AdapterRunError::NonZeroExit {
                        command: command_display,
                        status: format_exit_status(&status),
                        stderr_tail: stderr_tail_string,
                    });
                }

                if stdout.exceeded {
                    return Err(AdapterRunError::StdoutLimit {
                        command: command_display,
                        limit_bytes: stdout_limit,
                        stderr_tail: stderr_tail_string,
                    });
                }

                let stdout_str = std::str::from_utf8(&stdout.bytes).map_err(|_| {
                    AdapterRunError::StdoutNotUtf8 {
                        command: command_display.clone(),
                        stderr_tail: stderr_tail_string.clone(),
                    }
                })?;

                let response: AnalysisResponse =
                    serde_json::from_str(stdout_str).map_err(|source| {
                        AdapterRunError::InvalidJson {
                            command: command_display.clone(),
                            source,
                            stderr_tail: stderr_tail_string.clone(),
                        }
                    })?;

                if response.schema_version != request.schema_version
                    || response.schema_version != SCHEMA_VERSION
                {
                    return Err(AdapterRunError::ProtocolVersion {
                        command: command_display,
                        source: ProtocolVersionError {
                            actual: response.schema_version,
                            expected: SCHEMA_VERSION,
                        },
                        stderr_tail: stderr_tail_string,
                    });
                }
                // Sanity check even when the previous comparison passed.
                validate_version(response.schema_version).map_err(|source| {
                    AdapterRunError::ProtocolVersion {
                        command: command_display,
                        source,
                        stderr_tail: stderr_tail_string,
                    }
                })?;
                Ok(response)
            }
            Ok(None) => {
                // Timeout: terminate the child (and, on Unix, its whole process
                // group) before waiting so descendants cannot outlive us.
                terminate_child(&mut child, pid);
                let _ = child.wait();
                let _ = write_join.join();
                let _ = stdout_join.join();
                let stderr_bytes = stderr_join.join().unwrap_or_default();
                Err(AdapterRunError::Timeout {
                    command: command_display,
                    timeout_ms: timeout.as_millis(),
                    stderr_tail: String::from_utf8_lossy(&stderr_bytes).into_owned(),
                })
            }
            Err(source) => {
                // wait_timeout itself failed; make sure the child does not linger.
                terminate_child(&mut child, pid);
                let _ = child.wait();
                Err(AdapterRunError::Io {
                    command: command_display,
                    source,
                })
            }
        }
    }
}

/// Result of the bounded stdout reader.
struct ReadStdout {
    bytes: Vec<u8>,
    exceeded: bool,
}

impl ReadStdout {
    fn completed(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            exceeded: false,
        }
    }
}

fn read_stdout_bounded(mut reader: impl Read, limit: usize) -> std::io::Result<ReadStdout> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut exceeded = false;
    let mut sink = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        if !exceeded {
            if buffer.len() + n > limit {
                let take = limit.saturating_sub(buffer.len());
                buffer.extend_from_slice(&chunk[..take]);
                exceeded = true;
            } else {
                buffer.extend_from_slice(&chunk[..n]);
            }
        }
        if exceeded {
            // Keep draining without storing so the child can finish writing.
            while reader.read(&mut sink)? != 0 {}
            break;
        }
    }
    Ok(ReadStdout {
        bytes: buffer,
        exceeded,
    })
}

fn read_stderr_tail(mut reader: impl Read, tail_limit: usize) -> Vec<u8> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buffer.extend_from_slice(&chunk[..n]);
                if buffer.len() > tail_limit.saturating_mul(2) {
                    let drop_from = buffer.len() - tail_limit;
                    buffer.drain(..drop_from);
                }
            }
            Err(_) => break,
        }
    }
    if buffer.len() > tail_limit {
        let start = buffer.len() - tail_limit;
        buffer.drain(..start);
    }
    buffer
}

fn format_exit_status(status: &std::process::ExitStatus) -> String {
    if let Some(code) = status.code() {
        format!("code {code}")
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(signal) = status.signal() {
                return format!("signal {signal}");
            }
        }
        "unknown".to_string()
    }
}

/// Spawn the child, retrying on Linux `ETXTBSY` (`ExecutableFileBusy`).
///
/// `pre_exec` forces `Command::spawn` down the fork+exec path (posix_spawn
/// cannot run arbitrary pre-exec code), which exposes the ETXTBSY race
/// described in rust-lang/rust#114554: a concurrent fork in another thread
/// can inherit a write fd on the executable file, so the exec sees the file
/// as still-open-for-writing and fails with EBUSY (errno 26). The window is
/// tiny; a few short retries clear it without letting a genuine bug hang.
fn spawn_with_etxtbsy_retry(
    cmd: &mut Command,
    command_display: &str,
) -> Result<std::process::Child, AdapterRunError> {
    let mut attempt: u32 = 0;
    loop {
        match cmd.spawn() {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 4 => {
                std::thread::sleep(Duration::from_millis(20 * u64::from(attempt + 1)));
                attempt += 1;
                continue;
            }
            Err(source) => {
                return Err(AdapterRunError::Spawn {
                    command: command_display.to_string(),
                    source,
                });
            }
        }
    }
}

/// Terminate the child (and, on Unix, its whole process group) so a timeout
/// cannot leave descendants running. `Child::kill` is a belt-and-suspenders
/// call that also fires on non-Unix targets where process groups do not exist.
fn terminate_child(child: &mut std::process::Child, pid: u32) {
    #[cfg(unix)]
    {
        let pgid: libc::pid_t = pid as libc::pid_t;
        unsafe {
            libc::killpg(pgid, libc::SIGTERM);
        }
        std::thread::sleep(Duration::from_millis(200));
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
    let _ = child.kill();
}
