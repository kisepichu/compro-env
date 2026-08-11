//! Pinned dependency preparation types for language adapters (spec §6.9).
//!
//! The core owns the shape of the dependency manifest, the derivation of the
//! content-addressed `dependency-id`, and the shape of the prepared-set
//! manifest that `infrastructure` writes and re-validates. Filesystem, HTTP,
//! and archive I/O stay in `infrastructure` so the pure data model can be
//! shared with `ce site-data`, verify, and CI drivers.

use std::path::PathBuf;

use thiserror::Error;

use crate::adapter_build::{ContentDigest, TargetPlatform};

// ─── DependencyManifest ─────────────────────────────────────────────────────

/// Parsed contents of `tools/library-analyzers/dependencies.toml`.
///
/// All fields are optional in the file but always present here so the digest
/// derivation walks a stable, canonical shape.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DependencyManifest {
    pub archives: Vec<ArchiveDependency>,
    pub git: Vec<GitDependency>,
    pub locals: Vec<LocalDependency>,
    pub toolchains: Vec<ToolchainPin>,
}

/// Public HTTPS archive pinned by SHA-256.
///
/// A single manifest may list several per-target archives (for example, one
/// LLVM tarball per supported triple). `target_os` and `target_arch` filter
/// which archive `prepare` downloads on the current host; both must be `Some`
/// or both `None`. `None` means the archive is used on every target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveDependency {
    pub name: String,
    /// Public HTTPS URL. `infrastructure` rejects HTTP, SSH, SCP-style URLs,
    /// and URL userinfo before this struct is constructed.
    pub url: String,
    pub sha256: ContentDigest,
    pub format: ArchiveFormat,
    /// Target OS gate. `Some("linux")` means the archive is only downloaded on
    /// Linux hosts. Must be set together with `target_arch` or both left
    /// `None`.
    pub target_os: Option<String>,
    /// Target CPU architecture gate. See `target_os`.
    pub target_arch: Option<String>,
}

/// Archive formats accepted by the safe extractor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchiveFormat {
    TarGz,
    TarXz,
    Zip,
}

impl ArchiveFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            ArchiveFormat::TarGz => "tar.gz",
            ArchiveFormat::TarXz => "tar.xz",
            ArchiveFormat::Zip => "zip",
        }
    }
}

impl std::fmt::Display for ArchiveFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Git dependency pinned by full commit SHA and archive checksum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDependency {
    pub name: String,
    pub url: String,
    /// Full 40-character lowercase hex commit hash.
    pub commit: String,
    /// SHA-256 of the archived commit tarball fetched over HTTPS.
    pub archive_sha256: ContentDigest,
}

/// Repository-relative directory or file whose content is folded into the
/// dependency id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDependency {
    pub name: String,
    pub path: String,
}

/// Toolchain identity pin (`name`, `version`, optional components).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolchainPin {
    pub name: String,
    pub version: String,
    pub components: Vec<String>,
}

// ─── DependencyId ───────────────────────────────────────────────────────────

/// Content-addressed identifier for a prepared dependency set.
///
/// Derived from the normalized manifest plus target platform. The same
/// manifest on the same platform always yields the same id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DependencyId(ContentDigest);

impl DependencyId {
    pub fn new(digest: ContentDigest) -> Self {
        Self(digest)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn digest(&self) -> &ContentDigest {
        &self.0
    }
}

impl std::fmt::Display for DependencyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

// ─── Prepared manifest ──────────────────────────────────────────────────────

/// Manifest written under `target/library-analyzers/prepared/<id>/manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedManifest {
    pub id: DependencyId,
    pub target_platform: TargetPlatform,
    pub artifacts: Vec<PreparedArtifact>,
}

/// Recorded identity of one prepared artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedArtifact {
    pub name: String,
    pub kind: PreparedArtifactKind,
    /// Location relative to the prepared set root (`cargo-home/…`, etc.).
    /// This is the byte-hashed source: a downloaded tarball, commit archive,
    /// or copied local file.
    pub relative_path: String,
    /// SHA-256 of the file at `relative_path`.
    pub sha256: ContentDigest,
    /// Optional installation subtree relative to the prepared set root.
    /// Set when the artifact is unpacked (archives). Its contents are trusted
    /// because they are derived deterministically from `relative_path`, whose
    /// hash is verified byte-for-byte.
    pub install_relative_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparedArtifactKind {
    Archive,
    Git,
    Local,
    Toolchain,
}

impl std::fmt::Display for PreparedArtifactKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PreparedArtifactKind::Archive => "archive",
            PreparedArtifactKind::Git => "git",
            PreparedArtifactKind::Local => "local",
            PreparedArtifactKind::Toolchain => "toolchain",
        })
    }
}

/// Caller-supplied expectations checked against the on-disk prepared set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedPreparedSet {
    pub id: DependencyId,
    pub target_platform: TargetPlatform,
}

/// Validated prepared set: the on-disk contents matched the expectations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSet {
    pub id: DependencyId,
    pub root: PathBuf,
    pub manifest: PreparedManifest,
}

// ─── Errors ─────────────────────────────────────────────────────────────────

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PreparedManifestError {
    #[error("prepared dependency id mismatch: expected {expected}, found {actual}")]
    DependencyIdMismatch {
        expected: DependencyId,
        actual: DependencyId,
    },
    #[error(
        "target platform mismatch: expected {expected_os}/{expected_arch}, \
         found {actual_os}/{actual_arch}"
    )]
    TargetPlatformMismatch {
        expected_os: String,
        expected_arch: String,
        actual_os: String,
        actual_arch: String,
    },
}

/// Reject any stored prepared manifest that does not match the caller's
/// expectations. Byte-level artifact verification is performed by
/// `infrastructure` against the on-disk files.
pub fn validate_prepared_manifest(
    expected: &ExpectedPreparedSet,
    actual: &PreparedManifest,
) -> Result<(), PreparedManifestError> {
    if expected.target_platform != actual.target_platform {
        return Err(PreparedManifestError::TargetPlatformMismatch {
            expected_os: expected.target_platform.os.clone(),
            expected_arch: expected.target_platform.arch.clone(),
            actual_os: actual.target_platform.os.clone(),
            actual_arch: actual.target_platform.arch.clone(),
        });
    }
    if expected.id != actual.id {
        return Err(PreparedManifestError::DependencyIdMismatch {
            expected: expected.id.clone(),
            actual: actual.id.clone(),
        });
    }
    Ok(())
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(seed: u8) -> ContentDigest {
        ContentDigest::from_sha256_bytes([seed; 32])
    }

    fn platform() -> TargetPlatform {
        TargetPlatform {
            os: "linux".into(),
            arch: "x86_64".into(),
        }
    }

    fn manifest(id_seed: u8) -> PreparedManifest {
        PreparedManifest {
            id: DependencyId::new(digest(id_seed)),
            target_platform: platform(),
            artifacts: vec![],
        }
    }

    #[test]
    fn prepared_artifact_defaults_install_path_to_none() {
        let a = PreparedArtifact {
            name: "x".into(),
            kind: PreparedArtifactKind::Local,
            relative_path: "cargo-home/x".into(),
            sha256: digest(0),
            install_relative_path: None,
        };
        assert!(a.install_relative_path.is_none());
    }

    fn expected(id_seed: u8) -> ExpectedPreparedSet {
        ExpectedPreparedSet {
            id: DependencyId::new(digest(id_seed)),
            target_platform: platform(),
        }
    }

    #[test]
    fn validate_prepared_manifest_accepts_matching_fields() {
        assert!(validate_prepared_manifest(&expected(1), &manifest(1)).is_ok());
    }

    #[test]
    fn validate_prepared_manifest_rejects_id_mismatch() {
        let err = validate_prepared_manifest(&expected(1), &manifest(2)).unwrap_err();
        assert!(matches!(
            err,
            PreparedManifestError::DependencyIdMismatch { .. }
        ));
    }

    #[test]
    fn validate_prepared_manifest_rejects_platform_mismatch() {
        let actual = PreparedManifest {
            target_platform: TargetPlatform {
                os: "darwin".into(),
                arch: "aarch64".into(),
            },
            ..manifest(1)
        };
        let err = validate_prepared_manifest(&expected(1), &actual).unwrap_err();
        assert!(matches!(
            err,
            PreparedManifestError::TargetPlatformMismatch { .. }
        ));
    }

    #[test]
    fn dependency_id_display_matches_digest() {
        let id = DependencyId::new(digest(0xab));
        assert_eq!(id.to_string(), id.digest().as_str());
    }

    #[test]
    fn archive_format_display_round_trips() {
        assert_eq!(ArchiveFormat::TarGz.to_string(), "tar.gz");
        assert_eq!(ArchiveFormat::TarXz.to_string(), "tar.xz");
        assert_eq!(ArchiveFormat::Zip.to_string(), "zip");
    }

    #[test]
    fn archive_dependency_can_carry_target_gate() {
        let dep = ArchiveDependency {
            name: "llvm-linux-x64".into(),
            url: "https://example.com/x.tar.xz".into(),
            sha256: digest(0),
            format: ArchiveFormat::TarXz,
            target_os: Some("linux".into()),
            target_arch: Some("x86_64".into()),
        };
        assert_eq!(dep.target_os.as_deref(), Some("linux"));
        assert_eq!(dep.target_arch.as_deref(), Some("x86_64"));
    }
}
