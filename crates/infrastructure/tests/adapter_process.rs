//! Tests for the strict adapter process runner (plan 040 Task 2).

#![cfg(unix)]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use infrastructure::library_adapter::process::{
    DEFAULT_STDERR_TAIL_BYTES, ProcessLibraryAdapterRunner,
};
use library_adapter_protocol::{AnalysisRequest, SCHEMA_VERSION};
use serial_test::serial;
use tempfile::TempDir;
use usecases::library_adapter::{AdapterRunError, LibraryAdapterRunner};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/adapter-process")
        .join(name)
}

fn write_script(dir: &TempDir, name: &str, contents: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

fn empty_request(language: &str) -> AnalysisRequest {
    AnalysisRequest {
        schema_version: SCHEMA_VERSION,
        repository_root: ".".into(),
        language: language.into(),
        libraries: vec![],
        solutions: vec![],
    }
}

fn runner(env: BTreeMap<String, String>) -> ProcessLibraryAdapterRunner {
    ProcessLibraryAdapterRunner::new(std::env::current_dir().unwrap(), env)
}

fn minimal_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    // Explicitly forward the parent PATH so `env`/`sh`/`bash` can still be
    // located on hosts (e.g. NixOS) that place them outside `/usr/bin`. Using
    // the parent value is still an explicit allowlist entry: `env_clear` is
    // called first and only the values inserted here reach the child.
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
    env.insert("PATH".into(), path);
    env
}

// ─── happy path ──────────────────────────────────────────────────────────────

#[test]
#[serial]
fn returns_parsed_response_on_success() {
    let r = runner(minimal_env());
    let response = r
        .analyze(
            &fixture("valid.sh"),
            &empty_request("rust"),
            Duration::from_secs(10),
        )
        .unwrap();
    assert_eq!(response.schema_version, SCHEMA_VERSION);
    assert_eq!(response.adapter.name, "test-fixture");
    assert!(response.libraries.is_empty());
    assert!(response.solutions.is_empty());
}

#[test]
#[serial]
fn stdin_is_closed_so_adapter_can_finish() {
    // valid.sh reads stdin to EOF via `cat >/dev/null`. If the runner did not
    // close stdin, cat would block forever and the runner would time out.
    let r = runner(minimal_env());
    let response = r
        .analyze(
            &fixture("valid.sh"),
            &empty_request("rust"),
            Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(response.schema_version, SCHEMA_VERSION);
}

#[test]
#[serial]
fn only_one_json_document_is_parsed() {
    // Adapter emits a valid document followed by trailing junk on the same
    // line — serde_json::from_str must reject it because it does not consume
    // trailing content.
    let dir = TempDir::new().unwrap();
    let script = write_script(
        &dir,
        "multi.sh",
        r#"#!/usr/bin/env bash
cat >/dev/null
printf '{"schema_version":1,"adapter":{"name":"a","version":"1","toolchains":[]},"libraries":[],"solutions":[]}{"trailing":true}\n'
"#,
    );
    let r = runner(minimal_env());
    let err = r
        .analyze(&script, &empty_request("rust"), Duration::from_secs(5))
        .unwrap_err();
    assert!(matches!(err, AdapterRunError::InvalidJson { .. }));
}

// ─── failure modes ───────────────────────────────────────────────────────────

#[test]
#[serial]
fn errors_on_invalid_json_output() {
    let r = runner(minimal_env());
    let err = r
        .analyze(
            &fixture("invalid-json.sh"),
            &empty_request("rust"),
            Duration::from_secs(5),
        )
        .unwrap_err();
    assert!(matches!(err, AdapterRunError::InvalidJson { .. }));
}

#[test]
#[serial]
fn errors_on_nonzero_exit() {
    let dir = TempDir::new().unwrap();
    let script = write_script(
        &dir,
        "nonzero.sh",
        r#"#!/usr/bin/env bash
cat >/dev/null
printf 'boom\n' >&2
exit 3
"#,
    );
    let r = runner(minimal_env());
    let err = r
        .analyze(&script, &empty_request("rust"), Duration::from_secs(5))
        .unwrap_err();
    match err {
        AdapterRunError::NonZeroExit { stderr_tail, .. } => {
            assert!(stderr_tail.contains("boom"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
#[serial]
fn nonzero_exit_takes_priority_over_valid_stdout() {
    // Even if the adapter prints a valid response, a nonzero exit must reject
    // the stdout entirely (spec §6.3).
    let dir = TempDir::new().unwrap();
    let script = write_script(
        &dir,
        "nonzero-with-stdout.sh",
        r#"#!/usr/bin/env bash
cat >/dev/null
cat <<'JSON'
{"schema_version":1,"adapter":{"name":"a","version":"1","toolchains":[]},"libraries":[],"solutions":[]}
JSON
exit 1
"#,
    );
    let r = runner(minimal_env());
    let err = r
        .analyze(&script, &empty_request("rust"), Duration::from_secs(5))
        .unwrap_err();
    assert!(matches!(err, AdapterRunError::NonZeroExit { .. }));
}

#[test]
#[serial]
fn times_out_when_adapter_does_not_return() {
    let r = runner(minimal_env());
    let err = r
        .analyze(
            &fixture("timeout.sh"),
            &empty_request("rust"),
            Duration::from_millis(200),
        )
        .unwrap_err();
    assert!(matches!(err, AdapterRunError::Timeout { .. }));
}

#[test]
#[serial]
fn enforces_stdout_limit() {
    // Runner is configured with a tiny stdout limit, and the adapter writes
    // more than that before finishing.
    let dir = TempDir::new().unwrap();
    let script = write_script(
        &dir,
        "flood.sh",
        r#"#!/usr/bin/env bash
cat >/dev/null
# Print ~10 KiB of output.
head -c 10240 /dev/urandom | base64
"#,
    );
    let r = ProcessLibraryAdapterRunner::new(std::env::current_dir().unwrap(), minimal_env())
        .with_stdout_limit_bytes(512);
    let err = r
        .analyze(&script, &empty_request("rust"), Duration::from_secs(5))
        .unwrap_err();
    assert!(matches!(err, AdapterRunError::StdoutLimit { .. }));
}

#[test]
#[serial]
fn stderr_tail_is_bounded() {
    // Adapter writes far more than the tail limit — we should still get a
    // NonZeroExit and the captured stderr should be at most the limit.
    let dir = TempDir::new().unwrap();
    let script = write_script(
        &dir,
        "stderr-flood.sh",
        r#"#!/usr/bin/env bash
cat >/dev/null
for _ in $(seq 1 200); do
  head -c 1024 /dev/urandom | base64 >&2
done
exit 1
"#,
    );
    let r = ProcessLibraryAdapterRunner::new(std::env::current_dir().unwrap(), minimal_env())
        .with_stderr_tail_bytes(512);
    let err = r
        .analyze(&script, &empty_request("rust"), Duration::from_secs(5))
        .unwrap_err();
    match err {
        AdapterRunError::NonZeroExit { stderr_tail, .. } => {
            assert!(
                stderr_tail.len() <= 512,
                "stderr tail was {} bytes",
                stderr_tail.len()
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
#[serial]
fn rejects_response_with_wrong_schema_version() {
    let dir = TempDir::new().unwrap();
    let script = write_script(
        &dir,
        "wrong-schema.sh",
        r#"#!/usr/bin/env bash
cat >/dev/null
cat <<'JSON'
{"schema_version":99,"adapter":{"name":"a","version":"1","toolchains":[]},"libraries":[],"solutions":[]}
JSON
"#,
    );
    let r = runner(minimal_env());
    let err = r
        .analyze(&script, &empty_request("rust"), Duration::from_secs(5))
        .unwrap_err();
    assert!(matches!(err, AdapterRunError::ProtocolVersion { .. }));
}

#[test]
#[serial]
fn rejects_non_utf8_stdout() {
    let dir = TempDir::new().unwrap();
    let script = write_script(
        &dir,
        "not-utf8.sh",
        r#"#!/usr/bin/env sh
cat >/dev/null
# 0xFF is not valid UTF-8. Use POSIX octal escapes for portability.
printf '\377\376\372'
"#,
    );
    let r = runner(minimal_env());
    let err = r
        .analyze(&script, &empty_request("rust"), Duration::from_secs(5))
        .unwrap_err();
    assert!(matches!(err, AdapterRunError::StdoutNotUtf8 { .. }));
}

// ─── environment isolation ───────────────────────────────────────────────────

#[test]
#[serial]
fn environment_is_isolated_from_parent() {
    // Spawn the runner with only allowlisted variables. Cargo always sets
    // CARGO_MANIFEST_DIR in the parent when running tests, so if `env_clear`
    // were skipped that variable would land in the child. We never mutate the
    // parent env here — Rust flags `std::env::set_var` as `unsafe` because it
    // races with other threads reading env, so tests must not touch it.
    let cargo_manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("Cargo sets CARGO_MANIFEST_DIR when running tests");

    let dir = TempDir::new().unwrap();
    let marker = dir.path().join("env.txt");
    let marker_str = marker.display().to_string();
    let script = write_script(
        &dir,
        "env-echo.sh",
        &format!(
            r#"#!/usr/bin/env sh
cat >/dev/null
env > "{marker_str}"
cat <<'JSON'
{{"schema_version":1,"adapter":{{"name":"a","version":"1","toolchains":[]}},"libraries":[],"solutions":[]}}
JSON
"#,
        ),
    );
    let mut env = minimal_env();
    env.insert("CE_ADAPTER_TAG".into(), "allowed".into());
    let r = ProcessLibraryAdapterRunner::new(std::env::current_dir().unwrap(), env);
    let response = r
        .analyze(&script, &empty_request("rust"), Duration::from_secs(5))
        .unwrap();
    assert_eq!(response.adapter.name, "a");

    let observed = std::fs::read_to_string(&marker).unwrap();
    // Positive: allowlisted values are visible.
    assert!(
        observed.contains("CE_ADAPTER_TAG=allowed"),
        "child env missing allowlisted CE_ADAPTER_TAG:\n{observed}"
    );
    // Negative: the parent's CARGO_MANIFEST_DIR must never leak into the child.
    assert!(
        !observed.contains(&format!("CARGO_MANIFEST_DIR={cargo_manifest_dir}")),
        "child env leaked CARGO_MANIFEST_DIR:\n{observed}"
    );
    // Belt-and-suspenders: no key we did not allowlist should appear.
    let child_keys: std::collections::BTreeSet<&str> = observed
        .lines()
        .filter_map(|line| line.split_once('=').map(|(k, _)| k))
        .collect();
    for parent_only in ["CARGO_MANIFEST_DIR", "RUSTUP_TOOLCHAIN"] {
        assert!(
            !child_keys.contains(parent_only),
            "child env leaked {parent_only}:\n{observed}"
        );
    }
}

// ─── argv correctness ────────────────────────────────────────────────────────

#[test]
#[serial]
fn passes_extra_args_to_executable() {
    let dir = TempDir::new().unwrap();
    let marker = dir.path().join("argv.txt");
    let marker_str = marker.display().to_string();
    let script = write_script(
        &dir,
        "argv-echo.sh",
        &format!(
            r#"#!/usr/bin/env sh
cat >/dev/null
printf '%s\n' "$@" > "{marker_str}"
cat <<'JSON'
{{"schema_version":1,"adapter":{{"name":"a","version":"1","toolchains":[]}},"libraries":[],"solutions":[]}}
JSON
"#,
        ),
    );
    let r = ProcessLibraryAdapterRunner::new(std::env::current_dir().unwrap(), minimal_env())
        .with_extra_args(vec!["--first".into(), "value with space".into()]);
    let _ = r
        .analyze(&script, &empty_request("rust"), Duration::from_secs(5))
        .unwrap();
    let observed = std::fs::read_to_string(&marker).unwrap();
    assert_eq!(observed, "--first\nvalue with space\n");
}

// ─── defaults ────────────────────────────────────────────────────────────────

#[test]
#[serial]
fn stderr_tail_default_is_finite() {
    assert!(DEFAULT_STDERR_TAIL_BYTES <= 64 * 1024);
}
