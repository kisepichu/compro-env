//! Tests for the strict dependency-manifest parser and prepared-set validator
//! (plan 041 Task 1). No network I/O — every fixture is constructed on disk.

use std::fs;
use std::path::{Path, PathBuf};

use domain::adapter_build::{ContentDigest, TargetPlatform};
use domain::adapter_prepare::{
    ArchiveFormat, DependencyId, ExpectedPreparedSet, PreparedArtifact, PreparedArtifactKind,
    PreparedManifest,
};
use infrastructure::library_adapter::prepared::{
    DEPENDENCY_MANIFEST_PATH, PrepareError, expected_dependency_id, load_dependency_manifest,
    validate_prepared_set, write_prepared_manifest_json,
};
use tempfile::TempDir;

fn write_file(root: &Path, rel: &str, contents: &str) -> PathBuf {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, contents).unwrap();
    path
}

fn write_manifest(root: &Path, contents: &str) {
    write_file(root, DEPENDENCY_MANIFEST_PATH, contents);
}

fn linux() -> TargetPlatform {
    TargetPlatform {
        os: "linux".into(),
        arch: "x86_64".into(),
    }
}

fn darwin() -> TargetPlatform {
    TargetPlatform {
        os: "darwin".into(),
        arch: "aarch64".into(),
    }
}

const VALID_SHA: &str = "3c8a3a1b7a1d6d9a5c74b8a9d0e2f1b7c6a8d9e0f1a2b3c4d5e6f708192a3b4c";
const VALID_SHA_2: &str = "aa8a3a1b7a1d6d9a5c74b8a9d0e2f1b7c6a8d9e0f1a2b3c4d5e6f708192a3b4c";
const VALID_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

// ─── load_dependency_manifest ────────────────────────────────────────────────

#[test]
fn load_dependency_manifest_parses_all_sections() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &format!(
            r#"
[[archives]]
name = "example"
url = "https://example.com/foo-1.0.tar.gz"
sha256 = "{VALID_SHA}"
format = "tar.gz"

[[git]]
name = "repo"
url = "https://github.com/example/repo.git"
commit = "{VALID_COMMIT}"
archive_sha256 = "{VALID_SHA_2}"

[[locals]]
name = "protocol"
path = "tools/library-analyzers/protocol"

[[toolchains]]
name = "rust"
version = "1.92.0"
components = ["rustfmt", "clippy"]
"#,
        ),
    );
    let manifest = load_dependency_manifest(dir.path()).unwrap();
    assert_eq!(manifest.archives.len(), 1);
    assert_eq!(manifest.archives[0].name, "example");
    assert_eq!(manifest.archives[0].format, ArchiveFormat::TarGz);
    assert_eq!(manifest.git.len(), 1);
    assert_eq!(manifest.git[0].commit, VALID_COMMIT);
    assert_eq!(manifest.locals.len(), 1);
    assert_eq!(manifest.locals[0].path, "tools/library-analyzers/protocol");
    assert_eq!(manifest.toolchains.len(), 1);
    assert_eq!(manifest.toolchains[0].components, vec!["rustfmt", "clippy"]);
}

#[test]
fn load_dependency_manifest_defaults_to_empty_sections() {
    let dir = TempDir::new().unwrap();
    write_manifest(dir.path(), "");
    let manifest = load_dependency_manifest(dir.path()).unwrap();
    assert!(manifest.archives.is_empty());
    assert!(manifest.git.is_empty());
    assert!(manifest.locals.is_empty());
    assert!(manifest.toolchains.is_empty());
}

#[test]
fn load_dependency_manifest_errors_when_missing() {
    let dir = TempDir::new().unwrap();
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(matches!(err, PrepareError::Io { .. }));
}

#[test]
fn load_dependency_manifest_rejects_unknown_toml_key() {
    let dir = TempDir::new().unwrap();
    write_manifest(dir.path(), "unknown_top_level = 1\n");
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(matches!(err, PrepareError::Toml { .. }));
}

#[test]
fn load_dependency_manifest_rejects_http_scheme() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &format!(
            r#"
[[archives]]
name = "a"
url = "http://example.com/x.tar.gz"
sha256 = "{VALID_SHA}"
format = "tar.gz"
"#,
        ),
    );
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(matches!(err, PrepareError::InvalidUrl { .. }), "{err:?}");
}

#[test]
fn load_dependency_manifest_rejects_ssh_url() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &format!(
            r#"
[[git]]
name = "r"
url = "ssh://git@github.com/example/repo.git"
commit = "{VALID_COMMIT}"
archive_sha256 = "{VALID_SHA}"
"#,
        ),
    );
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(matches!(err, PrepareError::InvalidUrl { .. }), "{err:?}");
}

#[test]
fn load_dependency_manifest_rejects_scp_style_url() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &format!(
            r#"
[[git]]
name = "r"
url = "git@github.com:example/repo.git"
commit = "{VALID_COMMIT}"
archive_sha256 = "{VALID_SHA}"
"#,
        ),
    );
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(matches!(err, PrepareError::InvalidUrl { .. }), "{err:?}");
}

#[test]
fn load_dependency_manifest_rejects_url_userinfo() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &format!(
            r#"
[[archives]]
name = "a"
url = "https://user:pass@example.com/x.tar.gz"
sha256 = "{VALID_SHA}"
format = "tar.gz"
"#,
        ),
    );
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(matches!(err, PrepareError::InvalidUrl { .. }), "{err:?}");
}

#[test]
fn load_dependency_manifest_rejects_partial_git_sha() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &format!(
            r#"
[[git]]
name = "r"
url = "https://github.com/example/repo.git"
commit = "0123456"
archive_sha256 = "{VALID_SHA}"
"#,
        ),
    );
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(
        matches!(err, PrepareError::InvalidGitCommit { .. }),
        "{err:?}"
    );
}

#[test]
fn load_dependency_manifest_rejects_uppercase_git_sha() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &format!(
            r#"
[[git]]
name = "r"
url = "https://github.com/example/repo.git"
commit = "ABCDEF0123456789ABCDEF0123456789ABCDEF01"
archive_sha256 = "{VALID_SHA}"
"#,
        ),
    );
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(
        matches!(err, PrepareError::InvalidGitCommit { .. }),
        "{err:?}"
    );
}

#[test]
fn load_dependency_manifest_rejects_invalid_sha_syntax() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        r#"
[[archives]]
name = "a"
url = "https://example.com/x.tar.gz"
sha256 = "notahash"
format = "tar.gz"
"#,
    );
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(matches!(err, PrepareError::InvalidDigest { .. }), "{err:?}");
}

#[test]
fn load_dependency_manifest_rejects_unknown_archive_format() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &format!(
            r#"
[[archives]]
name = "a"
url = "https://example.com/x.rar"
sha256 = "{VALID_SHA}"
format = "rar"
"#,
        ),
    );
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(
        matches!(err, PrepareError::InvalidArchiveFormat { .. }),
        "{err:?}"
    );
}

#[test]
fn load_dependency_manifest_accepts_tar_xz_format() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &format!(
            r#"
[[archives]]
name = "a"
url = "https://example.com/x.tar.xz"
sha256 = "{VALID_SHA}"
format = "tar.xz"
"#,
        ),
    );
    let manifest = load_dependency_manifest(dir.path()).unwrap();
    assert_eq!(manifest.archives[0].format, ArchiveFormat::TarXz);
}

#[test]
fn load_dependency_manifest_accepts_matched_target_gate() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &format!(
            r#"
[[archives]]
name = "gated"
url = "https://example.com/x.tar.xz"
sha256 = "{VALID_SHA}"
format = "tar.xz"
target_os = "linux"
target_arch = "x86_64"
"#,
        ),
    );
    let manifest = load_dependency_manifest(dir.path()).unwrap();
    assert_eq!(manifest.archives[0].target_os.as_deref(), Some("linux"));
    assert_eq!(manifest.archives[0].target_arch.as_deref(), Some("x86_64"));
}

#[test]
fn load_dependency_manifest_rejects_incomplete_target_gate() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &format!(
            r#"
[[archives]]
name = "only-os"
url = "https://example.com/x.tar.xz"
sha256 = "{VALID_SHA}"
format = "tar.xz"
target_os = "linux"
"#,
        ),
    );
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(
        matches!(err, PrepareError::IncompleteArchiveTarget { .. }),
        "{err:?}"
    );
}

#[test]
fn load_dependency_manifest_rejects_local_absolute_path() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        r#"
[[locals]]
name = "abs"
path = "/etc/passwd"
"#,
    );
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(matches!(err, PrepareError::NotRelative { .. }), "{err:?}");
}

#[test]
fn load_dependency_manifest_rejects_local_parent_traversal() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        r#"
[[locals]]
name = "up"
path = "../secret"
"#,
    );
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(matches!(err, PrepareError::InvalidPath { .. }), "{err:?}");
}

#[test]
fn load_dependency_manifest_rejects_duplicate_names() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &format!(
            r#"
[[archives]]
name = "same"
url = "https://example.com/a.tar.gz"
sha256 = "{VALID_SHA}"
format = "tar.gz"

[[git]]
name = "same"
url = "https://example.com/repo"
commit = "{VALID_COMMIT}"
archive_sha256 = "{VALID_SHA_2}"
"#,
        ),
    );
    let err = load_dependency_manifest(dir.path()).unwrap_err();
    assert!(matches!(err, PrepareError::Duplicate { .. }), "{err:?}");
}

// ─── expected_dependency_id ──────────────────────────────────────────────────

#[test]
fn expected_dependency_id_is_stable_across_declaration_order() {
    let a = TempDir::new().unwrap();
    write_manifest(
        a.path(),
        &format!(
            r#"
[[archives]]
name = "one"
url = "https://example.com/one.tar.gz"
sha256 = "{VALID_SHA}"
format = "tar.gz"

[[archives]]
name = "two"
url = "https://example.com/two.tar.gz"
sha256 = "{VALID_SHA_2}"
format = "tar.gz"
"#,
        ),
    );
    let manifest_a = load_dependency_manifest(a.path()).unwrap();
    let id_a = expected_dependency_id(a.path(), &manifest_a, &linux()).unwrap();

    let b = TempDir::new().unwrap();
    write_manifest(
        b.path(),
        &format!(
            r#"
[[archives]]
name = "two"
url = "https://example.com/two.tar.gz"
sha256 = "{VALID_SHA_2}"
format = "tar.gz"

[[archives]]
name = "one"
url = "https://example.com/one.tar.gz"
sha256 = "{VALID_SHA}"
format = "tar.gz"
"#,
        ),
    );
    let manifest_b = load_dependency_manifest(b.path()).unwrap();
    let id_b = expected_dependency_id(b.path(), &manifest_b, &linux()).unwrap();

    assert_eq!(id_a, id_b);
}

#[test]
fn expected_dependency_id_changes_when_platform_changes() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &format!(
            r#"
[[archives]]
name = "one"
url = "https://example.com/one.tar.gz"
sha256 = "{VALID_SHA}"
format = "tar.gz"
"#,
        ),
    );
    let manifest = load_dependency_manifest(dir.path()).unwrap();
    let linux_id = expected_dependency_id(dir.path(), &manifest, &linux()).unwrap();
    let darwin_id = expected_dependency_id(dir.path(), &manifest, &darwin()).unwrap();
    assert_ne!(linux_id, darwin_id);
}

#[test]
fn expected_dependency_id_includes_local_content() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        r#"
[[locals]]
name = "lib"
path = "lib"
"#,
    );
    write_file(dir.path(), "lib/a.txt", "hello\n");
    let manifest = load_dependency_manifest(dir.path()).unwrap();
    let before = expected_dependency_id(dir.path(), &manifest, &linux()).unwrap();

    write_file(dir.path(), "lib/a.txt", "world\n");
    let after = expected_dependency_id(dir.path(), &manifest, &linux()).unwrap();
    assert_ne!(before, after);
}

#[test]
fn expected_dependency_id_errors_when_local_path_missing() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        r#"
[[locals]]
name = "lib"
path = "nowhere"
"#,
    );
    let manifest = load_dependency_manifest(dir.path()).unwrap();
    let err = expected_dependency_id(dir.path(), &manifest, &linux()).unwrap_err();
    assert!(matches!(err, PrepareError::Missing { .. }), "{err:?}");
}

// ─── validate_prepared_set ───────────────────────────────────────────────────

fn write_prepared(dir: &Path, manifest: &PreparedManifest) {
    fs::create_dir_all(dir).unwrap();
    let json = write_prepared_manifest_json(manifest);
    fs::write(dir.join("manifest.json"), json).unwrap();
}

fn manifest_with(id: &DependencyId, platform: &TargetPlatform) -> PreparedManifest {
    PreparedManifest {
        id: id.clone(),
        target_platform: platform.clone(),
        artifacts: vec![],
    }
}

fn manifest_with_missing_artifact(
    id: &DependencyId,
    platform: &TargetPlatform,
) -> PreparedManifest {
    PreparedManifest {
        id: id.clone(),
        target_platform: platform.clone(),
        artifacts: vec![PreparedArtifact {
            name: "sample".into(),
            kind: PreparedArtifactKind::Archive,
            relative_path: "cargo-home/sample".into(),
            sha256: ContentDigest::from_hex(VALID_SHA).unwrap(),
            install_relative_path: None,
        }],
    }
}

#[test]
fn validate_prepared_set_accepts_matching_manifest() {
    let dir = TempDir::new().unwrap();
    let id = DependencyId::new(ContentDigest::from_sha256_bytes([1; 32]));
    let expected = ExpectedPreparedSet {
        id: id.clone(),
        target_platform: linux(),
    };
    write_prepared(dir.path(), &manifest_with(&id, &linux()));
    let set = validate_prepared_set(dir.path(), &expected).unwrap();
    assert_eq!(set.id, id);
    assert_eq!(set.root, dir.path());
}

#[test]
fn validate_prepared_set_rejects_missing_manifest() {
    let dir = TempDir::new().unwrap();
    let expected = ExpectedPreparedSet {
        id: DependencyId::new(ContentDigest::from_sha256_bytes([1; 32])),
        target_platform: linux(),
    };
    let err = validate_prepared_set(dir.path(), &expected).unwrap_err();
    assert!(matches!(err, PrepareError::PreparedManifestMissing { .. }));
}

#[test]
fn validate_prepared_set_rejects_directory_that_is_a_file() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("child");
    fs::write(&file, "x").unwrap();
    let expected = ExpectedPreparedSet {
        id: DependencyId::new(ContentDigest::from_sha256_bytes([1; 32])),
        target_platform: linux(),
    };
    let err = validate_prepared_set(&file, &expected).unwrap_err();
    assert!(matches!(err, PrepareError::NotADirectory { .. }), "{err:?}");
}

#[test]
fn validate_prepared_set_rejects_platform_mismatch() {
    let dir = TempDir::new().unwrap();
    let id = DependencyId::new(ContentDigest::from_sha256_bytes([1; 32]));
    write_prepared(dir.path(), &manifest_with(&id, &darwin()));
    let expected = ExpectedPreparedSet {
        id: id.clone(),
        target_platform: linux(),
    };
    let err = validate_prepared_set(dir.path(), &expected).unwrap_err();
    assert!(
        matches!(err, PrepareError::PreparedManifestMismatch { .. }),
        "{err:?}"
    );
}

#[test]
fn validate_prepared_set_rejects_id_mismatch() {
    let dir = TempDir::new().unwrap();
    let stored = DependencyId::new(ContentDigest::from_sha256_bytes([2; 32]));
    write_prepared(dir.path(), &manifest_with(&stored, &linux()));
    let expected = ExpectedPreparedSet {
        id: DependencyId::new(ContentDigest::from_sha256_bytes([1; 32])),
        target_platform: linux(),
    };
    let err = validate_prepared_set(dir.path(), &expected).unwrap_err();
    assert!(
        matches!(err, PrepareError::PreparedManifestMismatch { .. }),
        "{err:?}"
    );
}

#[test]
fn validate_prepared_set_rejects_incomplete_artifact() {
    let dir = TempDir::new().unwrap();
    let id = DependencyId::new(ContentDigest::from_sha256_bytes([1; 32]));
    let manifest = manifest_with_missing_artifact(&id, &linux());
    write_prepared(dir.path(), &manifest);
    // artifact file not written on disk -> incomplete
    let expected = ExpectedPreparedSet {
        id: id.clone(),
        target_platform: linux(),
    };
    let err = validate_prepared_set(dir.path(), &expected).unwrap_err();
    assert!(
        matches!(err, PrepareError::ArtifactMissing { .. }),
        "{err:?}"
    );
}

#[test]
fn validate_prepared_set_accepts_matching_artifact_content() {
    use sha2::Digest;
    let dir = TempDir::new().unwrap();
    let payload = b"hello prepared";
    let mut hasher = sha256_bytes();
    hasher.update(payload);
    let hex = to_hex(hasher.finalize().into());
    let id = DependencyId::new(ContentDigest::from_sha256_bytes([1; 32]));
    let manifest = PreparedManifest {
        id: id.clone(),
        target_platform: linux(),
        artifacts: vec![PreparedArtifact {
            name: "sample".into(),
            kind: PreparedArtifactKind::Archive,
            relative_path: "cargo-home/sample.bin".into(),
            sha256: ContentDigest::from_hex(hex).unwrap(),
            install_relative_path: None,
        }],
    };
    write_prepared(dir.path(), &manifest);
    write_file(dir.path(), "cargo-home/sample.bin", "hello prepared");
    let expected = ExpectedPreparedSet {
        id: id.clone(),
        target_platform: linux(),
    };
    let set = validate_prepared_set(dir.path(), &expected).unwrap();
    assert_eq!(set.manifest.artifacts[0].name, "sample");
}

#[cfg(unix)]
#[test]
fn validate_prepared_set_rejects_untracked_symlink() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    let id = DependencyId::new(ContentDigest::from_sha256_bytes([1; 32]));
    write_prepared(dir.path(), &manifest_with(&id, &linux()));
    fs::write(dir.path().join("target.txt"), b"real").unwrap();
    symlink(dir.path().join("target.txt"), dir.path().join("evil-link")).unwrap();
    let expected = ExpectedPreparedSet {
        id,
        target_platform: linux(),
    };
    let err = validate_prepared_set(dir.path(), &expected).unwrap_err();
    assert!(
        matches!(err, PrepareError::UntrackedPreparedEntry { .. }),
        "{err:?}"
    );
}

#[test]
fn validate_prepared_set_rejects_untracked_file() {
    let dir = TempDir::new().unwrap();
    let id = DependencyId::new(ContentDigest::from_sha256_bytes([1; 32]));
    write_prepared(dir.path(), &manifest_with(&id, &linux()));
    fs::write(dir.path().join("stray.txt"), b"not listed").unwrap();
    let expected = ExpectedPreparedSet {
        id,
        target_platform: linux(),
    };
    let err = validate_prepared_set(dir.path(), &expected).unwrap_err();
    assert!(
        matches!(err, PrepareError::UntrackedPreparedEntry { .. }),
        "{err:?}"
    );
}

#[test]
fn validate_prepared_set_rejects_hash_mismatch() {
    let dir = TempDir::new().unwrap();
    let id = DependencyId::new(ContentDigest::from_sha256_bytes([1; 32]));
    let manifest = PreparedManifest {
        id: id.clone(),
        target_platform: linux(),
        artifacts: vec![PreparedArtifact {
            name: "sample".into(),
            kind: PreparedArtifactKind::Archive,
            relative_path: "cargo-home/sample.bin".into(),
            sha256: ContentDigest::from_hex(VALID_SHA).unwrap(),
            install_relative_path: None,
        }],
    };
    write_prepared(dir.path(), &manifest);
    write_file(dir.path(), "cargo-home/sample.bin", "different content");
    let expected = ExpectedPreparedSet {
        id: id.clone(),
        target_platform: linux(),
    };
    let err = validate_prepared_set(dir.path(), &expected).unwrap_err();
    assert!(
        matches!(err, PrepareError::ArtifactHashMismatch { .. }),
        "{err:?}"
    );
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn sha256_bytes() -> sha2::Sha256 {
    use sha2::Digest;
    sha2::Sha256::new()
}

fn to_hex(bytes: [u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}
