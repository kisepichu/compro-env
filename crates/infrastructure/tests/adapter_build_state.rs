//! Integration tests for adapter build-state validation (plan 042 Task 1).
//!
//! Every test constructs an in-memory analyzer root under a `TempDir`,
//! materializes a manifest, executable(s), and `bin` symlink by hand, then
//! calls `inspect_build_state` / `derive_build_id` and inspects the outcome.
//!
//! No network, no external processes.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

use domain::adapter_build::{
    AdapterExecutableRecord, BuildId, BuildManifest, ContentDigest, ExpectedBuild, TargetPlatform,
    ToolchainRecord, UnsignedBuildManifest, validate_unsigned_manifest,
};
use fs2::FileExt;
use infrastructure::library_adapter::build_state::{
    BUILD_BIN_SUBDIR, BUILD_IN_PROGRESS_MARKER, BUILD_LOCK_FILE, BUILD_MANIFEST_FILE,
    BUILDS_SUBDIR, BuildStateError, CURRENT_BIN_LINK, derive_build_id, inspect_build_state,
    write_build_manifest_json,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn digest(seed: u8) -> ContentDigest {
    ContentDigest::from_sha256_bytes([seed; 32])
}

fn linux() -> TargetPlatform {
    TargetPlatform {
        os: "linux".into(),
        arch: "x86_64".into(),
    }
}

fn sha256_of(bytes: &[u8]) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out: [u8; 32] = hasher.finalize().into();
    ContentDigest::from_sha256_bytes(out)
}

fn record(
    language: &str,
    file_name: &str,
    contents: &[u8],
    adapter_name: &str,
    adapter_version: &str,
) -> AdapterExecutableRecord {
    AdapterExecutableRecord {
        language: language.into(),
        file_name: file_name.into(),
        sha256: sha256_of(contents),
        adapter_name: adapter_name.into(),
        adapter_version: adapter_version.into(),
        toolchains: vec![ToolchainRecord {
            name: "rustc".into(),
            version: "1.92.0".into(),
            target: Some("x86_64-unknown-linux-gnu".into()),
        }],
    }
}

fn base_manifest(execs: Vec<AdapterExecutableRecord>) -> BuildManifest {
    BuildManifest {
        input_digest: digest(0xaa),
        target_platform: linux(),
        build_profile: "release".into(),
        protocol_version: 1,
        git_commit_sha: "0".repeat(40),
        executables: execs,
    }
}

fn expected_from(m: &BuildManifest) -> ExpectedBuild {
    ExpectedBuild {
        input_digest: m.input_digest.clone(),
        target_platform: m.target_platform.clone(),
        build_profile: m.build_profile.clone(),
        protocol_version: m.protocol_version,
    }
}

/// Materialize a valid build set on disk and return `(analyzer_root, build_id)`.
fn install_build_set(
    analyzer_root: &Path,
    manifest: &BuildManifest,
    executable_bytes: &[(&str, &[u8])],
) -> BuildId {
    fs::create_dir_all(analyzer_root.join(BUILDS_SUBDIR)).unwrap();
    let unsigned = UnsignedBuildManifest::from(manifest);
    validate_unsigned_manifest(&unsigned).unwrap();
    let id = derive_build_id(&unsigned).unwrap();
    let build_dir = analyzer_root.join(BUILDS_SUBDIR).join(id.as_str());
    let bin_dir = build_dir.join(BUILD_BIN_SUBDIR);
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(
        build_dir.join(BUILD_MANIFEST_FILE),
        write_build_manifest_json(manifest),
    )
    .unwrap();
    for (name, contents) in executable_bytes {
        let path = bin_dir.join(name);
        fs::write(&path, contents).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
    }
    let link = analyzer_root.join(CURRENT_BIN_LINK);
    if link.exists() || fs::symlink_metadata(&link).is_ok() {
        fs::remove_file(&link).unwrap();
    }
    let target: PathBuf = PathBuf::from(BUILDS_SUBDIR)
        .join(id.as_str())
        .join(BUILD_BIN_SUBDIR);
    unix_fs::symlink(&target, &link).unwrap();
    id
}

// ─── derive_build_id ────────────────────────────────────────────────────────

#[test]
fn derive_build_id_is_deterministic_across_runs() {
    let m = UnsignedBuildManifest {
        input_digest: digest(1),
        target_platform: linux(),
        build_profile: "release".into(),
        protocol_version: 1,
        executables: vec![
            record("rust", "rust-analyzer", b"one", "rust-adapter", "1.0"),
            record("cpp", "cpp-analyzer", b"two", "cpp-adapter", "1.0"),
        ],
    };
    let a = derive_build_id(&m).unwrap();
    let b = derive_build_id(&m).unwrap();
    assert_eq!(a, b);
}

#[test]
fn derive_build_id_independent_of_executable_order() {
    let m1 = UnsignedBuildManifest {
        input_digest: digest(1),
        target_platform: linux(),
        build_profile: "release".into(),
        protocol_version: 1,
        executables: vec![
            record("rust", "rust-analyzer", b"one", "rust-adapter", "1.0"),
            record("cpp", "cpp-analyzer", b"two", "cpp-adapter", "1.0"),
        ],
    };
    let m2 = UnsignedBuildManifest {
        executables: vec![
            record("cpp", "cpp-analyzer", b"two", "cpp-adapter", "1.0"),
            record("rust", "rust-analyzer", b"one", "rust-adapter", "1.0"),
        ],
        ..m1.clone()
    };
    assert_eq!(derive_build_id(&m1).unwrap(), derive_build_id(&m2).unwrap());
}

#[test]
fn derive_build_id_changes_when_input_digest_changes() {
    let m1 = UnsignedBuildManifest {
        input_digest: digest(1),
        target_platform: linux(),
        build_profile: "release".into(),
        protocol_version: 1,
        executables: vec![record(
            "rust",
            "rust-analyzer",
            b"one",
            "rust-adapter",
            "1.0",
        )],
    };
    let m2 = UnsignedBuildManifest {
        input_digest: digest(2),
        ..m1.clone()
    };
    assert_ne!(derive_build_id(&m1).unwrap(), derive_build_id(&m2).unwrap());
}

#[test]
fn derive_build_id_rejects_duplicate_adapter_identity() {
    let m = UnsignedBuildManifest {
        input_digest: digest(1),
        target_platform: linux(),
        build_profile: "release".into(),
        protocol_version: 1,
        executables: vec![
            record("rust", "rust-analyzer", b"one", "shared-adapter", "1.0"),
            record("cpp", "cpp-analyzer", b"two", "shared-adapter", "1.0"),
        ],
    };
    let err = derive_build_id(&m).unwrap_err();
    assert!(matches!(err, BuildStateError::Derivation { .. }), "{err:?}");
}

// ─── inspect_build_state ────────────────────────────────────────────────────

#[test]
fn inspect_returns_usable_build_set_when_everything_matches() {
    let td = TempDir::new().unwrap();
    let m = base_manifest(vec![record(
        "rust",
        "rust-analyzer",
        b"rust bytes",
        "rust-adapter",
        "1.0",
    )]);
    let expected = expected_from(&m);
    install_build_set(td.path(), &m, &[("rust-analyzer", b"rust bytes")]);
    let set = inspect_build_state(td.path(), &expected).expect("valid set");
    assert_eq!(set.manifest, m);
}

#[test]
fn inspect_reports_missing_executable() {
    let td = TempDir::new().unwrap();
    let m = base_manifest(vec![
        record("rust", "rust-analyzer", b"rust", "rust-adapter", "1.0"),
        record("cpp", "cpp-analyzer", b"cpp", "cpp-adapter", "1.0"),
    ]);
    let expected = expected_from(&m);
    // Install only one executable — the second one will be missing from bin/.
    let id = install_build_set(td.path(), &m, &[("rust-analyzer", b"rust")]);
    let err = inspect_build_state(td.path(), &expected).unwrap_err();
    match err {
        BuildStateError::ExecutableMissing { file_name, .. } => {
            assert_eq!(file_name, "cpp-analyzer");
        }
        other => panic!("expected ExecutableMissing, got {other:?}"),
    }
    // Sanity check that the derived id matches the layout we set up.
    let unsigned = UnsignedBuildManifest::from(&m);
    assert_eq!(id, derive_build_id(&unsigned).unwrap());
}

#[test]
fn inspect_reports_wrong_executable_hash() {
    let td = TempDir::new().unwrap();
    let m = base_manifest(vec![record(
        "rust",
        "rust-analyzer",
        b"expected",
        "rust-adapter",
        "1.0",
    )]);
    let expected = expected_from(&m);
    install_build_set(td.path(), &m, &[("rust-analyzer", b"expected")]);
    // Overwrite executable with different bytes to break the hash.
    let unsigned = UnsignedBuildManifest::from(&m);
    let id = derive_build_id(&unsigned).unwrap();
    let bin = td
        .path()
        .join(BUILDS_SUBDIR)
        .join(id.as_str())
        .join(BUILD_BIN_SUBDIR)
        .join("rust-analyzer");
    fs::write(&bin, b"tampered").unwrap();
    let err = inspect_build_state(td.path(), &expected).unwrap_err();
    assert!(
        matches!(err, BuildStateError::ExecutableHashMismatch { .. }),
        "{err:?}"
    );
}

#[test]
fn inspect_reports_stale_input_digest() {
    let td = TempDir::new().unwrap();
    let m = base_manifest(vec![record(
        "rust",
        "rust-analyzer",
        b"payload",
        "rust-adapter",
        "1.0",
    )]);
    install_build_set(td.path(), &m, &[("rust-analyzer", b"payload")]);
    let expected = ExpectedBuild {
        input_digest: digest(0xff),
        ..expected_from(&m)
    };
    let err = inspect_build_state(td.path(), &expected).unwrap_err();
    assert!(matches!(err, BuildStateError::Mismatch { .. }), "{err:?}");
}

#[test]
fn inspect_reports_build_running_when_marker_and_lock_held() {
    let td = TempDir::new().unwrap();
    let m = base_manifest(vec![record(
        "rust",
        "rust-analyzer",
        b"payload",
        "rust-adapter",
        "1.0",
    )]);
    let expected = expected_from(&m);
    install_build_set(td.path(), &m, &[("rust-analyzer", b"payload")]);
    // Drop marker.
    fs::write(td.path().join(BUILD_IN_PROGRESS_MARKER), b"").unwrap();
    // Hold the advisory lock so inspect cannot acquire it.
    let lock_path = td.path().join(BUILD_LOCK_FILE);
    let lock = fs::File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    lock.try_lock_exclusive().unwrap();
    let err = inspect_build_state(td.path(), &expected).unwrap_err();
    assert!(
        matches!(err, BuildStateError::BuildRunning { .. }),
        "{err:?}"
    );
    // Drop the lock so the TempDir teardown does not race.
    FileExt::unlock(&lock).unwrap();
}

#[test]
fn inspect_reports_previous_failure_when_marker_and_lock_free() {
    let td = TempDir::new().unwrap();
    let m = base_manifest(vec![record(
        "rust",
        "rust-analyzer",
        b"payload",
        "rust-adapter",
        "1.0",
    )]);
    let expected = expected_from(&m);
    install_build_set(td.path(), &m, &[("rust-analyzer", b"payload")]);
    fs::write(td.path().join(BUILD_IN_PROGRESS_MARKER), b"").unwrap();
    let err = inspect_build_state(td.path(), &expected).unwrap_err();
    assert!(
        matches!(err, BuildStateError::PreviousBuildFailed { .. }),
        "{err:?}"
    );
}

#[test]
fn inspect_reports_bad_symlink_target() {
    let td = TempDir::new().unwrap();
    let m = base_manifest(vec![record(
        "rust",
        "rust-analyzer",
        b"payload",
        "rust-adapter",
        "1.0",
    )]);
    let expected = expected_from(&m);
    install_build_set(td.path(), &m, &[("rust-analyzer", b"payload")]);
    let link = td.path().join(CURRENT_BIN_LINK);
    fs::remove_file(&link).unwrap();
    // Point the symlink somewhere absolute and outside the analyzer root.
    let outside = TempDir::new().unwrap();
    unix_fs::symlink(outside.path(), &link).unwrap();
    let err = inspect_build_state(td.path(), &expected).unwrap_err();
    assert!(
        matches!(err, BuildStateError::CurrentBinBadTarget { .. }),
        "{err:?}"
    );
}

#[test]
fn inspect_reports_bad_symlink_when_using_dot_dot() {
    let td = TempDir::new().unwrap();
    let m = base_manifest(vec![record(
        "rust",
        "rust-analyzer",
        b"payload",
        "rust-adapter",
        "1.0",
    )]);
    let expected = expected_from(&m);
    install_build_set(td.path(), &m, &[("rust-analyzer", b"payload")]);
    let link = td.path().join(CURRENT_BIN_LINK);
    fs::remove_file(&link).unwrap();
    // A relative path that escapes via `..`.
    let bad_target = PathBuf::from("..")
        .join(BUILDS_SUBDIR)
        .join("evil")
        .join(BUILD_BIN_SUBDIR);
    unix_fs::symlink(&bad_target, &link).unwrap();
    let err = inspect_build_state(td.path(), &expected).unwrap_err();
    assert!(
        matches!(err, BuildStateError::CurrentBinBadTarget { .. }),
        "{err:?}"
    );
}

#[test]
fn inspect_reports_current_bin_missing() {
    let td = TempDir::new().unwrap();
    // Create only the builds/ directory; no `bin` symlink.
    fs::create_dir_all(td.path().join(BUILDS_SUBDIR)).unwrap();
    let m = base_manifest(vec![record(
        "rust",
        "rust-analyzer",
        b"payload",
        "rust-adapter",
        "1.0",
    )]);
    let err = inspect_build_state(td.path(), &expected_from(&m)).unwrap_err();
    assert!(
        matches!(err, BuildStateError::CurrentBinMissing { .. }),
        "{err:?}"
    );
}

#[test]
fn inspect_reports_build_id_mismatch_when_directory_renamed() {
    let td = TempDir::new().unwrap();
    let m = base_manifest(vec![record(
        "rust",
        "rust-analyzer",
        b"payload",
        "rust-adapter",
        "1.0",
    )]);
    let expected = expected_from(&m);
    let id = install_build_set(td.path(), &m, &[("rust-analyzer", b"payload")]);
    // Rename the build directory so the symlink resolves to a different name.
    let bogus_id = "b".repeat(64);
    let from = td.path().join(BUILDS_SUBDIR).join(id.as_str());
    let to = td.path().join(BUILDS_SUBDIR).join(&bogus_id);
    fs::rename(&from, &to).unwrap();
    let link = td.path().join(CURRENT_BIN_LINK);
    fs::remove_file(&link).unwrap();
    let target = PathBuf::from(BUILDS_SUBDIR)
        .join(&bogus_id)
        .join(BUILD_BIN_SUBDIR);
    unix_fs::symlink(&target, &link).unwrap();
    let err = inspect_build_state(td.path(), &expected).unwrap_err();
    assert!(
        matches!(err, BuildStateError::BuildIdMismatch { .. }),
        "{err:?}"
    );
}
