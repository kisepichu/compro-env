//! Integration tests for `UnixCommandRunner` (spec §7.1).
//!
//! The runner inherits stdout/stderr, so tests cannot capture them directly.
//! Instead each test asks the child to write to files (paths passed through the
//! environment) and inspects those files. This still exercises the streaming
//! spawn path — the runner never buffers output, so the child prints and exits
//! normally.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use infrastructure::command_runner_impl::UnixCommandRunner;
use usecases::command_runner::{CommandRequest, CommandRunner};

/// Absolute path to `/bin/sh`. Every test uses this so the runner never needs
/// to consult `PATH` when spawning the program itself.
const SH: &str = "/bin/sh";

/// Reuse the parent's `PATH` for locating standard utilities (`sleep`, `echo`,
/// etc.) inside the shell script. Test machines vary (NixOS keeps them under
/// `/run/current-system/sw/bin`, Debian-based CI under `/usr/bin`) so relying
/// on any hard-coded value is fragile.
fn parent_path() -> OsString {
    std::env::var_os("PATH").expect("parent process must have PATH set for these tests")
}

fn base_request(script: &str, workdir: &std::path::Path) -> CommandRequest {
    CommandRequest {
        program: OsString::from(SH),
        arguments: vec![OsString::from("-c"), OsString::from(script)],
        current_dir: workdir.to_path_buf(),
        environment: BTreeMap::new(),
        timeout: Duration::from_secs(10),
    }
}

fn env_with_path(pairs: &[(&str, &str)]) -> BTreeMap<OsString, OsString> {
    let mut env: BTreeMap<OsString, OsString> = pairs
        .iter()
        .map(|(k, v)| (OsString::from(*k), OsString::from(*v)))
        .collect();
    env.insert(OsString::from("PATH"), parent_path());
    env
}

fn pid_alive(pid: i32) -> bool {
    // `kill -0` returns 0 if the process exists and permissions allow signalling,
    // ESRCH (No such process) once the process has been fully reaped.
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
        Ok(()) => true,
        Err(nix::errno::Errno::ESRCH) => false,
        Err(_) => true, // EPERM or similar → process exists, just not ours.
    }
}

#[test]
fn streams_stdout_and_stderr_by_running_child() {
    let dir = tempfile::tempdir().unwrap();
    let out_path: PathBuf = dir.path().join("out.txt");
    let err_path: PathBuf = dir.path().join("err.txt");

    // Force the child to write to files so tests can observe without capturing
    // the parent's inherited stdio.
    let script = r#"echo hi > "$CE_OUT"; echo err 1>&2; echo err > "$CE_ERR""#;
    let mut req = base_request(script, dir.path());
    req.environment = env_with_path(&[
        ("CE_OUT", out_path.to_str().unwrap()),
        ("CE_ERR", err_path.to_str().unwrap()),
    ]);

    let outcome = UnixCommandRunner.run_streaming(&req).unwrap();

    assert_eq!(outcome.exit_code, Some(0));
    assert!(!outcome.timed_out);
    assert_eq!(fs::read_to_string(&out_path).unwrap().trim(), "hi");
    assert_eq!(fs::read_to_string(&err_path).unwrap().trim(), "err");
}

#[test]
fn propagates_zero_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let mut req = base_request("exit 0", dir.path());
    req.environment = env_with_path(&[]);

    let outcome = UnixCommandRunner.run_streaming(&req).unwrap();
    assert_eq!(outcome.exit_code, Some(0));
    assert!(!outcome.timed_out);
}

#[test]
fn propagates_non_zero_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let mut req = base_request("exit 42", dir.path());
    req.environment = env_with_path(&[]);

    let outcome = UnixCommandRunner.run_streaming(&req).unwrap();
    assert_eq!(outcome.exit_code, Some(42));
    assert!(!outcome.timed_out);
}

#[test]
fn environment_replaces_parent_environment() {
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("env.txt");

    // Deliberately do NOT set HOME in the request. It IS set in the parent
    // (cargo test's environment), so if env_clear() were skipped, $HOME would
    // leak into the child. Also record $FOO to prove the value we DID set
    // arrives intact.
    let script = r#"printf 'FOO=%s|HOME=%s' "$FOO" "$HOME" > "$OUTFILE""#;
    let mut req = base_request(script, dir.path());
    req.environment = env_with_path(&[("FOO", "bar"), ("OUTFILE", out_path.to_str().unwrap())]);

    // Sanity: the parent must actually have HOME for this test to be meaningful.
    assert!(
        std::env::var_os("HOME").is_some(),
        "parent HOME must be set for this test to detect a leak"
    );

    let outcome = UnixCommandRunner.run_streaming(&req).unwrap();
    assert_eq!(outcome.exit_code, Some(0));

    let contents = fs::read_to_string(&out_path).unwrap();
    assert_eq!(
        contents, "FOO=bar|HOME=",
        "parent HOME should not leak into child; got: {contents:?}"
    );
}

#[test]
fn timeout_kills_child_and_returns_promptly() {
    let dir = tempfile::tempdir().unwrap();
    let mut req = base_request("sleep 5", dir.path());
    req.environment = env_with_path(&[]);
    req.timeout = Duration::from_millis(200);

    let start = Instant::now();
    let outcome = UnixCommandRunner.run_streaming(&req).unwrap();
    let elapsed = start.elapsed();

    assert!(outcome.timed_out, "expected timed_out=true");
    assert_eq!(outcome.exit_code, None);
    assert!(
        elapsed < Duration::from_secs(2),
        "runner should have returned promptly, took {elapsed:?}"
    );
}

#[test]
fn sigterm_ignored_then_sigkill_after_five_seconds() {
    let dir = tempfile::tempdir().unwrap();
    // Trap SIGTERM as a no-op, then sleep. SIGTERM will not stop the shell,
    // so the runner must escalate to SIGKILL after ~5s.
    let script = r#"trap '' TERM; sleep 30"#;
    let mut req = base_request(script, dir.path());
    req.environment = env_with_path(&[]);
    req.timeout = Duration::from_millis(200);

    let start = Instant::now();
    let outcome = UnixCommandRunner.run_streaming(&req).unwrap();
    let elapsed = start.elapsed();

    assert!(outcome.timed_out, "expected timed_out=true");
    assert_eq!(outcome.exit_code, None);
    // ~200 ms until SIGTERM, ~5 s grace, then SIGKILL. Allow generous slack.
    assert!(
        elapsed >= Duration::from_secs(4),
        "expected escalation to take ~5s, but returned in {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "escalation took too long: {elapsed:?}"
    );
}

#[test]
fn timeout_kills_entire_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let pid_path = dir.path().join("pid");

    // Background a long sleep, record its PID, then wait. If only the top-level
    // shell were killed and not the group, the grandchild `sleep` would keep
    // running. Killing the group must reap `sleep` too.
    let script = r#"sleep 30 & echo $! > "$PIDFILE"; wait"#;
    let mut req = base_request(script, dir.path());
    req.environment = env_with_path(&[("PIDFILE", pid_path.to_str().unwrap())]);
    req.timeout = Duration::from_millis(300);

    let outcome = UnixCommandRunner.run_streaming(&req).unwrap();
    assert!(outcome.timed_out);

    // Give the OS a brief moment to reap the descendant.
    std::thread::sleep(Duration::from_millis(200));

    let pid_str = fs::read_to_string(&pid_path).unwrap();
    let pid: i32 = pid_str.trim().parse().expect("failed to parse child pid");
    assert!(
        !pid_alive(pid),
        "child sleep (pid {pid}) should have been killed with its process group"
    );
}

#[test]
fn subsequent_commands_run_independently() {
    let dir = tempfile::tempdir().unwrap();
    let runner = UnixCommandRunner;

    let mut req_ok = base_request("exit 0", dir.path());
    req_ok.environment = env_with_path(&[]);
    let outcome_ok = runner.run_streaming(&req_ok).unwrap();
    assert_eq!(outcome_ok.exit_code, Some(0));

    let mut req_fail = base_request("exit 7", dir.path());
    req_fail.environment = env_with_path(&[]);
    let outcome_fail = runner.run_streaming(&req_fail).unwrap();
    assert_eq!(outcome_fail.exit_code, Some(7));

    // A third run after the failure keeps working — nothing sticks.
    let outcome_again = runner.run_streaming(&req_ok).unwrap();
    assert_eq!(outcome_again.exit_code, Some(0));
    assert!(!outcome_again.timed_out);
}
