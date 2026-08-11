//! Deterministic build inputs and manifests for language adapters (spec §6.9).
//!
//! The core owns the shape of the input declaration, the digest of that
//! declaration, and the manifest fields consumers assert against before using a
//! `target/library-analyzers/builds/<build-id>/` set. Filesystem walking and
//! TOML parsing live in `infrastructure` — this module stays free of I/O so it
//! can be reused by verify, `ce site-data`, and CI drivers.

use thiserror::Error;

// ─── ContentDigest ───────────────────────────────────────────────────────────

/// Lowercase hex-encoded SHA-256 (spec §6.9 input digest).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentDigest(String);

impl ContentDigest {
    /// Construct from a raw hex string. Rejects anything that is not exactly
    /// 64 lowercase hex characters so the digest stays comparable byte-for-byte.
    pub fn from_hex(value: impl Into<String>) -> Result<Self, ContentDigestError> {
        let value = value.into();
        if value.len() != 64 {
            return Err(ContentDigestError::InvalidLength {
                length: value.len(),
            });
        }
        if !value
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        {
            return Err(ContentDigestError::NotLowercaseHex { value });
        }
        Ok(Self(value))
    }

    /// Wrap an already-computed 32-byte SHA-256 as a hex digest.
    pub fn from_sha256_bytes(bytes: [u8; 32]) -> Self {
        let mut hex = String::with_capacity(64);
        for byte in bytes {
            hex.push_str(&format!("{byte:02x}"));
        }
        Self(hex)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContentDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ContentDigestError {
    #[error("content digest must be 64 hex characters (got {length})")]
    InvalidLength { length: usize },
    #[error("content digest must be lowercase hex: {value:?}")]
    NotLowercaseHex { value: String },
}

// ─── TargetPlatform ──────────────────────────────────────────────────────────

/// Build target platform baked into the input digest so different platforms
/// resolve to different `build-id`s (spec §6.9).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetPlatform {
    pub os: String,
    pub arch: String,
}

// ─── BuildInputs ─────────────────────────────────────────────────────────────

/// Declared entries in `tools/library-analyzers/build-inputs.toml`.
///
/// The kind is preserved so the digest walker can distinguish a directory to
/// recurse into from a single additional file (spec §6.9). Order of iteration
/// is preserved for diagnostics, but the digest itself sorts collected files by
/// repository-relative path in UTF-8 byte order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildInputs {
    pub entries: Vec<BuildInputEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInputEntry {
    pub kind: BuildInputKind,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildInputKind {
    Directory,
    File,
}

impl std::fmt::Display for BuildInputKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildInputKind::Directory => f.write_str("directory"),
            BuildInputKind::File => f.write_str("file"),
        }
    }
}

// ─── Manifests ───────────────────────────────────────────────────────────────

/// Recorded identity of one built adapter executable (spec §6.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterExecutableRecord {
    /// Language ID the build set publishes this executable under.
    pub language: String,
    /// `bin/<name>` file name inside the build set.
    pub file_name: String,
    /// Hex-encoded SHA-256 of the executable bytes.
    pub sha256: ContentDigest,
    /// Adapter identity reported by the handshake response.
    pub adapter_name: String,
    pub adapter_version: String,
    /// Toolchains observed during the handshake.
    pub toolchains: Vec<ToolchainRecord>,
}

/// Toolchain identity as observed by the handshake (spec §6.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolchainRecord {
    pub name: String,
    pub version: String,
    pub target: Option<String>,
}

/// Full manifest stored under `target/library-analyzers/builds/<build-id>/manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildManifest {
    pub input_digest: ContentDigest,
    pub target_platform: TargetPlatform,
    pub build_profile: String,
    pub protocol_version: u32,
    pub git_commit_sha: String,
    pub executables: Vec<AdapterExecutableRecord>,
}

/// Subset the pipeline recomputes before trusting a stored `BuildManifest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedBuild {
    pub input_digest: ContentDigest,
    pub target_platform: TargetPlatform,
    pub build_profile: String,
    pub protocol_version: u32,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BuildManifestError {
    #[error("build input digest mismatch: expected {expected}, found {actual}")]
    InputDigestMismatch {
        expected: ContentDigest,
        actual: ContentDigest,
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
    #[error("build profile mismatch: expected {expected:?}, found {actual:?}")]
    BuildProfileMismatch { expected: String, actual: String },
    #[error("protocol version mismatch: expected {expected}, found {actual}")]
    ProtocolVersionMismatch { expected: u32, actual: u32 },
}

/// Reject any stored manifest that would let the pipeline reuse a stale build set.
pub fn validate_build_manifest(
    expected: &ExpectedBuild,
    actual: &BuildManifest,
) -> Result<(), BuildManifestError> {
    if expected.protocol_version != actual.protocol_version {
        return Err(BuildManifestError::ProtocolVersionMismatch {
            expected: expected.protocol_version,
            actual: actual.protocol_version,
        });
    }
    if expected.target_platform != actual.target_platform {
        return Err(BuildManifestError::TargetPlatformMismatch {
            expected_os: expected.target_platform.os.clone(),
            expected_arch: expected.target_platform.arch.clone(),
            actual_os: actual.target_platform.os.clone(),
            actual_arch: actual.target_platform.arch.clone(),
        });
    }
    if expected.build_profile != actual.build_profile {
        return Err(BuildManifestError::BuildProfileMismatch {
            expected: expected.build_profile.clone(),
            actual: actual.build_profile.clone(),
        });
    }
    if expected.input_digest != actual.input_digest {
        return Err(BuildManifestError::InputDigestMismatch {
            expected: expected.input_digest.clone(),
            actual: actual.input_digest.clone(),
        });
    }
    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

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

    fn manifest(digest_seed: u8) -> BuildManifest {
        BuildManifest {
            input_digest: digest(digest_seed),
            target_platform: platform(),
            build_profile: "release".into(),
            protocol_version: 1,
            git_commit_sha: "0000000000000000000000000000000000000000".into(),
            executables: vec![],
        }
    }

    fn expected(digest_seed: u8) -> ExpectedBuild {
        ExpectedBuild {
            input_digest: digest(digest_seed),
            target_platform: platform(),
            build_profile: "release".into(),
            protocol_version: 1,
        }
    }

    #[test]
    fn content_digest_rejects_wrong_length() {
        assert!(matches!(
            ContentDigest::from_hex("abc"),
            Err(ContentDigestError::InvalidLength { .. })
        ));
    }

    #[test]
    fn content_digest_rejects_uppercase_hex() {
        let uppercase = "A".repeat(64);
        assert!(matches!(
            ContentDigest::from_hex(uppercase),
            Err(ContentDigestError::NotLowercaseHex { .. })
        ));
    }

    #[test]
    fn content_digest_from_bytes_is_lowercase_hex_length_64() {
        let digest = ContentDigest::from_sha256_bytes([0xab; 32]);
        assert_eq!(digest.as_str().len(), 64);
        assert!(
            digest
                .as_str()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }

    #[test]
    fn validate_manifest_accepts_matching_fields() {
        assert!(validate_build_manifest(&expected(1), &manifest(1)).is_ok());
    }

    #[test]
    fn validate_manifest_rejects_digest_mismatch() {
        let err = validate_build_manifest(&expected(1), &manifest(2)).unwrap_err();
        assert!(matches!(
            err,
            BuildManifestError::InputDigestMismatch { .. }
        ));
    }

    #[test]
    fn validate_manifest_rejects_platform_mismatch() {
        let actual = BuildManifest {
            target_platform: TargetPlatform {
                os: "darwin".into(),
                arch: "aarch64".into(),
            },
            ..manifest(1)
        };
        let err = validate_build_manifest(&expected(1), &actual).unwrap_err();
        assert!(matches!(
            err,
            BuildManifestError::TargetPlatformMismatch { .. }
        ));
    }

    #[test]
    fn validate_manifest_rejects_profile_mismatch() {
        let actual = BuildManifest {
            build_profile: "debug".into(),
            ..manifest(1)
        };
        let err = validate_build_manifest(&expected(1), &actual).unwrap_err();
        assert!(matches!(
            err,
            BuildManifestError::BuildProfileMismatch { .. }
        ));
    }

    #[test]
    fn validate_manifest_rejects_protocol_mismatch() {
        let actual = BuildManifest {
            protocol_version: 2,
            ..manifest(1)
        };
        let err = validate_build_manifest(&expected(1), &actual).unwrap_err();
        assert!(matches!(
            err,
            BuildManifestError::ProtocolVersionMismatch { .. }
        ));
    }
}
