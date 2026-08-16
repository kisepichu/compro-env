//! Integration tests for `build_adapters` (plan 042 Task 2).
//!
//! Each test builds a minimal "prepared set" and a couple of fake adapter
//! shell scripts, feeds them through the real build driver plus the real
//! `ProcessLibraryAdapterRunner`, and asserts that the driver either publishes
//! a fully-populated build set atomically or leaves the marker in place.
//!
//! No network, no external processes beyond `sh` (via the sanitized PATH).

#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use domain::adapter_build::{ContentDigest, TargetPlatform};
use domain::adapter_prepare::{DependencyId, PreparedManifest, PreparedSet};
use fs2::FileExt;
use infrastructure::library_adapter::build::{
    BuildError, BuildLock, BuildRequest, LanguageBuildPlan, build_adapters,
};
use infrastructure::library_adapter::build_state::{
    BUILD_BIN_SUBDIR, BUILD_IN_PROGRESS_MARKER, BUILD_LOCK_FILE, BUILD_MANIFEST_FILE,
    BUILDS_SUBDIR, CURRENT_BIN_LINK,
};
use infrastructure::library_adapter::process::ProcessLibraryAdapterRunner;
use serial_test::serial;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

// ─── Helpers ────────────────────────────────────────────────────────────────

fn linux() -> TargetPlatform {
    TargetPlatform {
        os: "linux".into(),
        arch: "x86_64".into(),
    }
}

fn digest(seed: u8) -> ContentDigest {
    ContentDigest::from_sha256_bytes([seed; 32])
}

fn sha256_of(bytes: &[u8]) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out: [u8; 32] = hasher.finalize().into();
    ContentDigest::from_sha256_bytes(out)
}

/// Return a fake prepared set root under `td` that satisfies the on-disk
/// invariants `build_adapters` checks (existence of the root directory).
fn fake_prepared_set(td: &Path) -> PreparedSet {
    let root = td.join("prepared-fake");
    fs::create_dir_all(&root).unwrap();
    PreparedSet {
        id: DependencyId::new(digest(0xdd)),
        root: root.clone(),
        manifest: PreparedManifest {
            id: DependencyId::new(digest(0xdd)),
            target_platform: linux(),
            artifacts: vec![],
        },
    }
}

fn allowlist_path_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
    env.insert("PATH".into(), path);
    env
}

/// Write a fake adapter that returns the given identity when handed the
/// standard empty AnalysisRequest.
fn write_fake_adapter_script(
    dir: &Path,
    name: &str,
    adapter_name: &str,
    adapter_version: &str,
) -> PathBuf {
    let path = dir.join(name);
    let script = format!(
        r#"#!/usr/bin/env bash
set -eu
cat >/dev/null
cat <<JSON
{{
  "schema_version": 1,
  "adapter": {{
    "name": "{adapter_name}",
    "version": "{adapter_version}",
    "toolchains": [
      {{"name":"toolchain-a","version":"1.0.0"}}
    ]
  }},
  "libraries": [],
  "solutions": []
}}
JSON
"#
    );
    fs::write(&path, script).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

/// Write a fake adapter that responds with wrong identity.
fn write_wrong_identity_adapter(dir: &Path, name: &str) -> PathBuf {
    write_fake_adapter_script(dir, name, "wrong-name", "wrong-version")
}

/// Build a plan whose build command copies `source` into `bin/<file_name>`.
fn plan_that_copies(
    language: &str,
    file_name: &str,
    adapter_name: &str,
    adapter_version: &str,
    source: &Path,
) -> LanguageBuildPlan {
    LanguageBuildPlan {
        language: language.into(),
        file_name: file_name.into(),
        expected_adapter_name: adapter_name.into(),
        expected_adapter_version: adapter_version.into(),
        argv: vec![
            "sh".into(),
            "-c".into(),
            format!(
                "cp '{}' \"$CE_ADAPTER_STAGE_BIN/{}\" && chmod +x \"$CE_ADAPTER_STAGE_BIN/{}\"",
                source.display(),
                file_name,
                file_name,
            ),
        ],
        environment: allowlist_path_env(),
        working_directory: None,
        output_relative_path: format!("{BUILD_BIN_SUBDIR}/{file_name}"),
        handshake_environment: allowlist_path_env(),
    }
}

fn request_for(td: &Path, plans: Vec<LanguageBuildPlan>) -> BuildRequest {
    BuildRequest {
        repository_root: td.to_path_buf(),
        analyzer_root: td.join("analyzer"),
        target_platform: linux(),
        build_profile: "release".into(),
        protocol_version: 1,
        input_digest: digest(0x11),
        git_commit_sha: "0".repeat(40),
        prepared_set: fake_prepared_set(td),
        language_plans: plans,
        handshake_timeout: Duration::from_secs(5),
    }
}

fn make_runner() -> ProcessLibraryAdapterRunner {
    // `ProcessLibraryAdapterRunner` no longer owns an env: each `analyze`
    // (and therefore each `handshake_adapter`) call takes the sanitized
    // env directly, sourced from `LanguageBuildPlan::handshake_environment`.
    ProcessLibraryAdapterRunner::new(std::env::current_dir().unwrap())
}

// ─── Successful atomic switch ───────────────────────────────────────────────

#[test]
#[serial]
fn publishes_full_build_set_atomically() {
    let td = TempDir::new().unwrap();
    let rust_src = write_fake_adapter_script(td.path(), "rust-fake.sh", "rust-adapter", "1.0.0");
    let cpp_src = write_fake_adapter_script(td.path(), "cpp-fake.sh", "cpp-adapter", "1.0.0");
    let plans = vec![
        plan_that_copies("rust", "rust-analyzer", "rust-adapter", "1.0.0", &rust_src),
        plan_that_copies("cpp", "cpp-analyzer", "cpp-adapter", "1.0.0", &cpp_src),
    ];
    let request = request_for(td.path(), plans);
    let runner = make_runner();
    let set = build_adapters(&request, &runner).expect("build succeeds");

    // Build set on disk.
    let build_dir = request
        .analyzer_root
        .join(BUILDS_SUBDIR)
        .join(set.build_id.as_str());
    assert!(build_dir.join(BUILD_MANIFEST_FILE).is_file());
    assert!(
        build_dir
            .join(BUILD_BIN_SUBDIR)
            .join("rust-analyzer")
            .is_file()
    );
    assert!(
        build_dir
            .join(BUILD_BIN_SUBDIR)
            .join("cpp-analyzer")
            .is_file()
    );
    // bin symlink -> builds/<id>/bin.
    let link = request.analyzer_root.join(CURRENT_BIN_LINK);
    let target = fs::read_link(&link).unwrap();
    assert_eq!(
        target,
        PathBuf::from(BUILDS_SUBDIR)
            .join(set.build_id.as_str())
            .join(BUILD_BIN_SUBDIR)
    );
    // Marker cleared on success.
    assert!(
        !request
            .analyzer_root
            .join(BUILD_IN_PROGRESS_MARKER)
            .exists()
    );
    // No staging directory left behind.
    let leftover: Vec<_> = fs::read_dir(request.analyzer_root.join(BUILDS_SUBDIR))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("staging-"))
        .collect();
    assert!(leftover.is_empty(), "staging leftover: {leftover:?}");
    // Executables recorded with real sha256.
    assert_eq!(set.manifest.executables.len(), 2);
    for exec in &set.manifest.executables {
        let bytes = fs::read(build_dir.join(BUILD_BIN_SUBDIR).join(&exec.file_name)).unwrap();
        assert_eq!(exec.sha256, sha256_of(&bytes));
        assert_eq!(exec.toolchains.len(), 1);
    }
}

// ─── Stable language order ──────────────────────────────────────────────────

#[test]
#[serial]
fn build_id_independent_of_plan_input_order() {
    let td1 = TempDir::new().unwrap();
    let td2 = TempDir::new().unwrap();
    let mk_plans = |root: &Path| -> Vec<LanguageBuildPlan> {
        let rust_src = write_fake_adapter_script(root, "rust-fake.sh", "rust-adapter", "1.0.0");
        let cpp_src = write_fake_adapter_script(root, "cpp-fake.sh", "cpp-adapter", "1.0.0");
        vec![
            plan_that_copies("rust", "rust-analyzer", "rust-adapter", "1.0.0", &rust_src),
            plan_that_copies("cpp", "cpp-analyzer", "cpp-adapter", "1.0.0", &cpp_src),
        ]
    };
    let mut plans_a = mk_plans(td1.path());
    let mut plans_b = mk_plans(td2.path());
    plans_b.reverse();

    let runner = make_runner();
    let set_a = build_adapters(&request_for(td1.path(), plans_a.split_off(0)), &runner).unwrap();
    let set_b = build_adapters(&request_for(td2.path(), plans_b.split_off(0)), &runner).unwrap();
    assert_eq!(set_a.build_id, set_b.build_id);
}

// ─── Missing prepared set ───────────────────────────────────────────────────

#[test]
#[serial]
fn errors_when_prepared_set_missing() {
    let td = TempDir::new().unwrap();
    let rust_src = write_fake_adapter_script(td.path(), "rust-fake.sh", "rust-adapter", "1.0.0");
    let plans = vec![plan_that_copies(
        "rust",
        "rust-analyzer",
        "rust-adapter",
        "1.0.0",
        &rust_src,
    )];
    let mut request = request_for(td.path(), plans);
    // Point prepared_set at a non-existent directory.
    request.prepared_set.root = td.path().join("does-not-exist");
    let runner = make_runner();
    let err = build_adapters(&request, &runner).unwrap_err();
    assert!(
        matches!(err, BuildError::PreparedSetMissing { .. }),
        "{err:?}"
    );
    // Marker was never created and no builds directory exists.
    assert!(
        !request
            .analyzer_root
            .join(BUILD_IN_PROGRESS_MARKER)
            .exists()
    );
}

// ─── Sanitized environment ──────────────────────────────────────────────────

#[test]
#[serial]
fn build_command_environment_is_sanitized() {
    let td = TempDir::new().unwrap();
    let rust_src = write_fake_adapter_script(td.path(), "rust-fake.sh", "rust-adapter", "1.0.0");
    let env_out = td.path().join("env-snapshot.txt");
    let mut plan = plan_that_copies("rust", "rust-analyzer", "rust-adapter", "1.0.0", &rust_src);
    // Replace the argv so we can capture the child env.
    plan.argv = vec![
        "sh".into(),
        "-c".into(),
        format!(
            "env > '{}' && cp '{}' \"$CE_ADAPTER_STAGE_BIN/rust-analyzer\" && chmod +x \
             \"$CE_ADAPTER_STAGE_BIN/rust-analyzer\"",
            env_out.display(),
            rust_src.display(),
        ),
    ];
    plan.environment
        .insert("CE_BUILD_TAG".into(), "explicit-value".into());
    let request = request_for(td.path(), vec![plan]);
    let runner = make_runner();
    let _ = build_adapters(&request, &runner).expect("build succeeds");
    let observed = fs::read_to_string(&env_out).unwrap();
    // Positive: our allowlisted var is present.
    assert!(
        observed.contains("CE_BUILD_TAG=explicit-value"),
        "expected CE_BUILD_TAG=explicit-value in env:\n{observed}"
    );
    // Negative: CARGO_MANIFEST_DIR (set by cargo test harness) must not leak.
    let parent_manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    assert!(
        !observed.contains(&format!("CARGO_MANIFEST_DIR={parent_manifest}")),
        "parent CARGO_MANIFEST_DIR leaked into build command env:\n{observed}"
    );
    // Positive: driver-injected paths must be exposed.
    assert!(
        observed.contains("CE_ADAPTER_STAGE_BIN="),
        "driver did not inject CE_ADAPTER_STAGE_BIN:\n{observed}"
    );
}

// ─── Nonzero build command ──────────────────────────────────────────────────

#[test]
#[serial]
fn nonzero_build_command_fails_and_keeps_marker() {
    let td = TempDir::new().unwrap();
    let plans = vec![LanguageBuildPlan {
        language: "rust".into(),
        file_name: "rust-analyzer".into(),
        expected_adapter_name: "rust-adapter".into(),
        expected_adapter_version: "1.0.0".into(),
        argv: vec!["sh".into(), "-c".into(), "echo failing >&2; exit 7".into()],
        environment: allowlist_path_env(),
        working_directory: None,
        output_relative_path: format!("{BUILD_BIN_SUBDIR}/rust-analyzer"),
        handshake_environment: allowlist_path_env(),
    }];
    let request = request_for(td.path(), plans);
    let runner = make_runner();
    let err = build_adapters(&request, &runner).unwrap_err();
    match err {
        BuildError::BuildCommandFailed { status, .. } => {
            assert!(status.contains("code 7"), "unexpected status: {status:?}");
        }
        other => panic!("expected BuildCommandFailed, got {other:?}"),
    }
    // Marker retained, old set (none in this case) unchanged.
    assert!(
        request
            .analyzer_root
            .join(BUILD_IN_PROGRESS_MARKER)
            .exists()
    );
    assert!(!request.analyzer_root.join(CURRENT_BIN_LINK).exists());
}

// ─── Handshake identity mismatch ────────────────────────────────────────────

#[test]
#[serial]
fn handshake_identity_mismatch_fails_and_keeps_marker() {
    let td = TempDir::new().unwrap();
    let rust_src = write_wrong_identity_adapter(td.path(), "rust-wrong.sh");
    let plans = vec![plan_that_copies(
        "rust",
        "rust-analyzer",
        "expected-name",
        "expected-version",
        &rust_src,
    )];
    let request = request_for(td.path(), plans);
    let runner = make_runner();
    let err = build_adapters(&request, &runner).unwrap_err();
    assert!(
        matches!(err, BuildError::HandshakeIdentityMismatch { .. }),
        "{err:?}"
    );
    assert!(
        request
            .analyzer_root
            .join(BUILD_IN_PROGRESS_MARKER)
            .exists()
    );
    assert!(!request.analyzer_root.join(CURRENT_BIN_LINK).exists());
}

// ─── Crash recovery: prior marker without lock ──────────────────────────────

#[test]
#[serial]
fn recovers_when_previous_run_left_marker_but_freed_lock() {
    let td = TempDir::new().unwrap();
    let rust_src = write_fake_adapter_script(td.path(), "rust-fake.sh", "rust-adapter", "1.0.0");
    let plans = vec![plan_that_copies(
        "rust",
        "rust-analyzer",
        "rust-adapter",
        "1.0.0",
        &rust_src,
    )];
    let request = request_for(td.path(), plans);
    fs::create_dir_all(&request.analyzer_root).unwrap();
    // Simulate a crash: marker present, lock free.
    fs::write(
        request.analyzer_root.join(BUILD_IN_PROGRESS_MARKER),
        b"stale",
    )
    .unwrap();
    let runner = make_runner();
    // A retry should acquire the lock and succeed, clearing the stale marker.
    let set = build_adapters(&request, &runner).expect("retry recovers");
    assert!(
        !request
            .analyzer_root
            .join(BUILD_IN_PROGRESS_MARKER)
            .exists()
    );
    let target = fs::read_link(request.analyzer_root.join(CURRENT_BIN_LINK)).unwrap();
    assert_eq!(
        target,
        PathBuf::from(BUILDS_SUBDIR)
            .join(set.build_id.as_str())
            .join(BUILD_BIN_SUBDIR)
    );
}

// ─── Concurrent lock ────────────────────────────────────────────────────────

#[test]
#[serial]
fn build_lock_is_exclusive() {
    let td = TempDir::new().unwrap();
    let analyzer_root = td.path().join("analyzer");
    fs::create_dir_all(&analyzer_root).unwrap();
    let lock_path = analyzer_root.join(BUILD_LOCK_FILE);
    let _held = BuildLock::acquire(&lock_path).unwrap();
    let err = BuildLock::acquire(&lock_path).unwrap_err();
    assert!(matches!(err, BuildError::LockContended { .. }), "{err:?}");
}

// ─── Missing prepared-set file after plan runs (spec §6.9: refuse fallbacks) ──

#[test]
#[serial]
fn missing_output_executable_fails_and_keeps_marker() {
    let td = TempDir::new().unwrap();
    let plans = vec![LanguageBuildPlan {
        language: "rust".into(),
        file_name: "rust-analyzer".into(),
        expected_adapter_name: "rust-adapter".into(),
        expected_adapter_version: "1.0.0".into(),
        // Successful command that produces no output file.
        argv: vec!["sh".into(), "-c".into(), "true".into()],
        environment: allowlist_path_env(),
        working_directory: None,
        output_relative_path: format!("{BUILD_BIN_SUBDIR}/rust-analyzer"),
        handshake_environment: allowlist_path_env(),
    }];
    let request = request_for(td.path(), plans);
    let runner = make_runner();
    let err = build_adapters(&request, &runner).unwrap_err();
    assert!(matches!(err, BuildError::OutputMissing { .. }), "{err:?}");
    assert!(
        request
            .analyzer_root
            .join(BUILD_IN_PROGRESS_MARKER)
            .exists()
    );
}

// ─── Duplicate language plan ────────────────────────────────────────────────

#[test]
#[serial]
fn duplicate_language_plan_is_rejected() {
    let td = TempDir::new().unwrap();
    let src = write_fake_adapter_script(td.path(), "rust-fake.sh", "rust-adapter", "1.0.0");
    let plans = vec![
        plan_that_copies("rust", "rust-analyzer", "rust-adapter", "1.0.0", &src),
        plan_that_copies("rust", "other", "rust-adapter", "1.0.0", &src),
    ];
    let request = request_for(td.path(), plans);
    let runner = make_runner();
    let err = build_adapters(&request, &runner).unwrap_err();
    assert!(
        matches!(err, BuildError::DuplicateLanguagePlan { .. }),
        "{err:?}"
    );
}

// ─── Force lock contention through the driver ────────────────────────────────

#[test]
#[serial]
fn build_refuses_when_lock_already_held() {
    let td = TempDir::new().unwrap();
    let analyzer_root = td.path().join("analyzer");
    fs::create_dir_all(&analyzer_root).unwrap();
    // Hold the lock manually before invoking the driver.
    let lock = fs::File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(analyzer_root.join(BUILD_LOCK_FILE))
        .unwrap();
    lock.try_lock_exclusive().unwrap();

    let rust_src = write_fake_adapter_script(td.path(), "rust-fake.sh", "rust-adapter", "1.0.0");
    let plans = vec![plan_that_copies(
        "rust",
        "rust-analyzer",
        "rust-adapter",
        "1.0.0",
        &rust_src,
    )];
    let request = request_for(td.path(), plans);
    let runner = make_runner();
    let err = build_adapters(&request, &runner).unwrap_err();
    assert!(matches!(err, BuildError::LockContended { .. }), "{err:?}");
    FileExt::unlock(&lock).unwrap();
}
