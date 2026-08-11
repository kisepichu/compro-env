//! Pinned Lean/Lake toolchain selection for the Lean adapter (spec §§6.8,
//! 6.9; plan 048 Task 1).
//!
//! `select_lean_toolchain` maps a build `TargetPlatform` to the one official
//! Lean 4.30.0 release archive that supports it. `all_lean_toolchain_specs`
//! returns every supported archive so `dependencies.toml` can list all three,
//! letting the per-target archive gate in `ArchiveDependency` pick the right
//! one at prepare time.
//!
//! After extraction, `validate_lean_layout` verifies the expected binaries
//! and shared Lean library directory are present, then runs `lean --version`
//! and `lake --version` through the caller-supplied readers, extracts the
//! Lean release string from each, and rejects any value that is not exactly
//! `4.30.0`. `default_lean_version_reader` and `default_lake_version_reader`
//! spawn the binary under a cleared environment with a minimal PATH so the
//! host system's Elan or ambient PATH can never impersonate the pinned
//! toolchain.
//!
//! `validate_lake_manifest` reads the checked-in `lake-manifest.json` and
//! confirms it declares the offline-buildable package shape the analyzer
//! expects; a missing or malformed file fails hard so an accidental
//! delete-and-commit cannot flip the adapter over to network fetches.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use domain::adapter_build::{ContentDigest, TargetPlatform};
use domain::adapter_prepare::{ArchiveFormat, PreparedSet};
use serde::Deserialize;
use thiserror::Error;

/// The single Lean release these adapter builds are pinned to.
pub const LEAN_EXPECTED_VERSION: &str = "4.30.0";

const LEAN_RELEASE_TAG: &str = "v4.30.0";
const LEAN_BASE_URL: &str = "https://github.com/leanprover/lean4/releases/download";

// SHA-256 digests of the three official Lean 4.30.0 release archives, taken
// from the plan. Keeping them here (and not in the manifest) means the plan's
// checksums are load-bearing in the type system: adding a new archive to the
// dependency manifest without also declaring it here yields a compile error.
const LEAN_LINUX_X64_ARCHIVE: &str = "lean-4.30.0-linux.tar.zst";
const LEAN_LINUX_X64_SHA256: &str =
    "4dad74141c2c119ca1aa626656be83b8e14238afba97271fd7bf1eb3f081b319";

const LEAN_LINUX_ARM64_ARCHIVE: &str = "lean-4.30.0-linux_aarch64.tar.zst";
const LEAN_LINUX_ARM64_SHA256: &str =
    "c99c6f0edd446956d4758c59d4383e8e6411ff6cc71a01f9caabe5eba454121d";

const LEAN_MACOS_ARM64_ARCHIVE: &str = "lean-4.30.0-darwin_aarch64.tar.zst";
const LEAN_MACOS_ARM64_SHA256: &str =
    "072dca4a38fbc0d3cedb96fea886cc243b424f2bd16247596200b9a9ab93f0f5";

/// Archive-name slugs used both here and in `dependencies.toml`.
pub const LEAN_TOOLCHAIN_LINUX_X64_NAME: &str = "lean-linux-x64";
pub const LEAN_TOOLCHAIN_LINUX_ARM64_NAME: &str = "lean-linux-arm64";
pub const LEAN_TOOLCHAIN_MACOS_ARM64_NAME: &str = "lean-macos-arm64";

/// One official Lean release archive, ready to be turned into an
/// `ArchiveDependency` for the dependency manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeanToolchainSpec {
    pub archive_name: String,
    pub url: String,
    pub sha256: ContentDigest,
    pub format: ArchiveFormat,
    pub target_os: String,
    pub target_arch: String,
    pub expected_version: &'static str,
}

/// Paths resolved inside an extracted Lean archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeanToolchainPaths {
    pub root: PathBuf,
    pub lean: PathBuf,
    pub lake: PathBuf,
    pub lib_dir: PathBuf,
    /// Normalized Lean release string parsed from `lean --version`.
    pub lean_version: String,
    /// Normalized Lean release string parsed from `lake --version`.
    pub lake_version: String,
}

#[derive(Debug, Error)]
pub enum LeanToolchainError {
    #[error(
        "no pinned Lean archive is available for target {os}/{arch}; \
         supported targets are linux/x86_64, linux/aarch64, macos/aarch64"
    )]
    UnsupportedTarget { os: String, arch: String },
    #[error("expected Lean binary is missing: {path}")]
    MissingBinary { path: String },
    #[error("expected Lean library directory is missing: {path}")]
    MissingLib { path: String },
    #[error("Lean version mismatch for {tool}: expected {expected}, got {actual:?}")]
    VersionMismatch {
        tool: &'static str,
        expected: String,
        actual: String,
    },
    #[error("could not extract Lean release from {tool} output: {raw:?}")]
    VersionUnparseable { tool: &'static str, raw: String },
    #[error("failed to read {tool} version: {source}")]
    VersionRead {
        tool: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("prepared Lean install for {archive_name:?} is not on disk at {path}")]
    PreparedInstallMissing { archive_name: String, path: String },
    #[error("prepared Lean install for {archive_name:?} does not contain bin/lean under {path}")]
    PreparedInstallLayout { archive_name: String, path: String },
    #[error("failed to read prepared Lean install directory {path}: {source}")]
    PreparedInstallIo {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("lake-manifest.json is missing at {path}")]
    LakeManifestMissing { path: String },
    #[error("failed to read lake-manifest.json at {path}: {source}")]
    LakeManifestIo {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("lake-manifest.json at {path} is not valid JSON: {source}")]
    LakeManifestParse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "lake-manifest.json at {path} is missing required field {field:?}; \
         a fresh checkout without a committed manifest cannot build offline"
    )]
    LakeManifestField { path: String, field: &'static str },
}

/// Return the toolchain spec for the current build target, or fail before any
/// download is attempted. Callers must never fall back to a host-installed
/// Lean.
pub fn select_lean_toolchain(
    platform: &TargetPlatform,
) -> Result<LeanToolchainSpec, LeanToolchainError> {
    all_lean_toolchain_specs()
        .into_iter()
        .find(|spec| spec.target_os == platform.os && spec.target_arch == platform.arch)
        .ok_or_else(|| LeanToolchainError::UnsupportedTarget {
            os: platform.os.clone(),
            arch: platform.arch.clone(),
        })
}

/// All supported Lean archive specs. Used to build `dependencies.toml` entries
/// so the manifest lists every triple and the per-target gate picks the right
/// one at prepare time.
pub fn all_lean_toolchain_specs() -> Vec<LeanToolchainSpec> {
    vec![
        spec_from_parts(
            LEAN_TOOLCHAIN_LINUX_X64_NAME,
            LEAN_LINUX_X64_ARCHIVE,
            LEAN_LINUX_X64_SHA256,
            "linux",
            "x86_64",
        ),
        spec_from_parts(
            LEAN_TOOLCHAIN_LINUX_ARM64_NAME,
            LEAN_LINUX_ARM64_ARCHIVE,
            LEAN_LINUX_ARM64_SHA256,
            "linux",
            "aarch64",
        ),
        spec_from_parts(
            LEAN_TOOLCHAIN_MACOS_ARM64_NAME,
            LEAN_MACOS_ARM64_ARCHIVE,
            LEAN_MACOS_ARM64_SHA256,
            "macos",
            "aarch64",
        ),
    ]
}

fn spec_from_parts(
    archive_name: &str,
    file_name: &str,
    sha256_hex: &str,
    target_os: &str,
    target_arch: &str,
) -> LeanToolchainSpec {
    LeanToolchainSpec {
        archive_name: archive_name.to_string(),
        url: format!("{LEAN_BASE_URL}/{LEAN_RELEASE_TAG}/{file_name}"),
        sha256: ContentDigest::from_hex(sha256_hex)
            .expect("pinned Lean SHA-256 constants are valid lowercase hex"),
        format: ArchiveFormat::TarZst,
        target_os: target_os.to_string(),
        target_arch: target_arch.to_string(),
        expected_version: LEAN_EXPECTED_VERSION,
    }
}

/// Verify the expected Lean release layout under `archive_root`. `lean_reader`
/// and `lake_reader` run `lean --version` and `lake --version` (or the
/// caller's stand-in) and must return the normalized Lean release string
/// (e.g. `"4.30.0"`) extracted from that output. Both a wrong version and a
/// missing file cause a hard failure — this pipeline never falls back to a
/// host toolchain.
pub fn validate_lean_layout<L, K>(
    archive_root: &Path,
    lean_reader: L,
    lake_reader: K,
) -> Result<LeanToolchainPaths, LeanToolchainError>
where
    L: FnOnce(&Path) -> Result<String, io::Error>,
    K: FnOnce(&Path) -> Result<String, io::Error>,
{
    let lean = archive_root.join("bin/lean");
    let lake = archive_root.join("bin/lake");
    let lib_dir = archive_root.join("lib/lean");

    if !lean.is_file() {
        return Err(LeanToolchainError::MissingBinary {
            path: lean.display().to_string(),
        });
    }
    if !lake.is_file() {
        return Err(LeanToolchainError::MissingBinary {
            path: lake.display().to_string(),
        });
    }
    if !lib_dir.is_dir() {
        return Err(LeanToolchainError::MissingLib {
            path: lib_dir.display().to_string(),
        });
    }

    let lean_version = lean_reader(&lean)
        .map(|s| s.trim().to_string())
        .map_err(|source| LeanToolchainError::VersionRead {
            tool: "lean",
            source,
        })?;
    if lean_version != LEAN_EXPECTED_VERSION {
        return Err(LeanToolchainError::VersionMismatch {
            tool: "lean",
            expected: LEAN_EXPECTED_VERSION.to_string(),
            actual: lean_version,
        });
    }

    let lake_version = lake_reader(&lake)
        .map(|s| s.trim().to_string())
        .map_err(|source| LeanToolchainError::VersionRead {
            tool: "lake",
            source,
        })?;
    if lake_version != LEAN_EXPECTED_VERSION {
        return Err(LeanToolchainError::VersionMismatch {
            tool: "lake",
            expected: LEAN_EXPECTED_VERSION.to_string(),
            actual: lake_version,
        });
    }

    Ok(LeanToolchainPaths {
        root: archive_root.to_path_buf(),
        lean,
        lake,
        lib_dir,
        lean_version,
        lake_version,
    })
}

/// Extract the Lean release ID (e.g. `4.30.0`) from raw `--version` output.
///
/// The scanner walks the raw output token-by-token (split on any non
/// digit-or-dot character, which also strips prerelease suffixes like
/// `-rc1`) and returns either the *first* or the *last* MAJOR.MINOR.PATCH
/// hit depending on `mode`:
///
/// * `Mode::First` for `lean --version`, which only mentions Lean's version.
/// * `Mode::Last` for `lake --version`, whose output begins with Lake's own
///   `5.x.x` version and ends with `Lean version 4.30.0`.
///
/// This keeps the parser independent of Lake's own version bump between
/// Lean releases.
fn extract_lean_release_at(
    tool: &'static str,
    raw: &str,
    mode: ReleaseScanMode,
) -> Result<String, LeanToolchainError> {
    let mut found: Option<String> = None;
    for token in raw.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        if !token.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let dots = token.chars().filter(|c| *c == '.').count();
        if dots < 2 {
            continue;
        }
        match mode {
            ReleaseScanMode::First => return Ok(token.to_string()),
            ReleaseScanMode::Last => found = Some(token.to_string()),
        }
    }
    found.ok_or(LeanToolchainError::VersionUnparseable {
        tool,
        raw: raw.to_string(),
    })
}

#[derive(Debug, Clone, Copy)]
enum ReleaseScanMode {
    First,
    Last,
}

/// Run `<binary> --version` under a cleared environment and a minimal PATH so
/// the host system cannot smuggle in a different Lean/Lake. Returns the raw
/// stdout.
fn read_version_stdout(binary: &Path, tool: &'static str) -> Result<String, io::Error> {
    let output = Command::new(binary)
        .arg("--version")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{tool} --version exited with status {}",
            output.status
        )));
    }
    String::from_utf8(output.stdout).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Spawn `lean --version` under a sanitized environment and return the parsed
/// Lean release (e.g. `4.30.0`). Errors surface both the raw parse failures
/// and the underlying I/O failures through `io::Error` so callers keep the
/// single `FnOnce(&Path) -> Result<String, io::Error>` shape.
pub fn default_lean_version_reader(lean: &Path) -> Result<String, io::Error> {
    let raw = read_version_stdout(lean, "lean")?;
    extract_lean_release_at("lean", &raw, ReleaseScanMode::First)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e}")))
}

/// Same shape as `default_lean_version_reader` but for Lake, whose own
/// version leads the output and whose Lean release trails it.
pub fn default_lake_version_reader(lake: &Path) -> Result<String, io::Error> {
    let raw = read_version_stdout(lake, "lake")?;
    extract_lean_release_at("lake", &raw, ReleaseScanMode::Last)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e}")))
}

/// Resolve the Lean install directory (the folder that contains `bin/`,
/// `lib/`, …) under a prepared set for the given target platform.
///
/// The dependency-manifest pipeline unpacks the Lean tarball into
/// `<prepared_root>/archives/<archive_name>/`. Official release archives ship
/// a top-level directory (e.g. `lean-4.30.0-linux/`), so we search one level
/// deeper for the directory that owns `bin/lean`. If a future archive
/// unpacks flat we still find `bin/lean` at the archive root and return that.
///
/// This is the helper `lean_build_plan` uses to derive `CE_LEAN_ROOT` for the
/// build driver; it also produces a `LeanToolchainPaths` via
/// `validate_lean_layout` so callers can audit the pinned identity.
pub fn locate_prepared_lean_root(
    prepared_set: &PreparedSet,
    platform: &TargetPlatform,
) -> Result<PathBuf, LeanToolchainError> {
    let spec = select_lean_toolchain(platform)?;
    let archives_dir = prepared_set.root.join("archives").join(&spec.archive_name);
    if !archives_dir.is_dir() {
        return Err(LeanToolchainError::PreparedInstallMissing {
            archive_name: spec.archive_name.clone(),
            path: archives_dir.display().to_string(),
        });
    }
    if archives_dir.join("bin/lean").is_file() {
        return Ok(archives_dir);
    }
    let entries =
        fs::read_dir(&archives_dir).map_err(|source| LeanToolchainError::PreparedInstallIo {
            path: archives_dir.display().to_string(),
            source,
        })?;
    for entry in entries {
        let entry = entry.map_err(|source| LeanToolchainError::PreparedInstallIo {
            path: archives_dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() && path.join("bin/lean").is_file() {
            return Ok(path);
        }
    }
    Err(LeanToolchainError::PreparedInstallLayout {
        archive_name: spec.archive_name,
        path: archives_dir.display().to_string(),
    })
}

// ─── lake-manifest.json validation ──────────────────────────────────────────

/// Minimal DTO for the fields we require in the checked-in
/// `lake-manifest.json`. Additional Lake fields are ignored so a Lake bump
/// that only adds keys still parses.
#[derive(Debug, Deserialize)]
struct LakeManifestFile {
    version: Option<serde_json::Value>,
    #[serde(rename = "packagesDir")]
    packages_dir: Option<String>,
    #[serde(default)]
    packages: Option<Vec<serde_json::Value>>,
    name: Option<String>,
}

/// Validate the shape of a checked-in `lake-manifest.json`. Missing files and
/// missing required fields both fail hard so a fresh checkout without a
/// committed manifest cannot flip the adapter over to a network fetch.
///
/// The set of required fields is intentionally minimal — the deep schema
/// belongs to Lake itself and moves with each Lean release. What matters
/// here is that the manifest is present, parses as JSON, declares its shape,
/// and enumerates its packages array (empty is fine when the adapter has no
/// external dependencies).
pub fn validate_lake_manifest(path: &Path) -> Result<(), LeanToolchainError> {
    let raw = fs::read_to_string(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            LeanToolchainError::LakeManifestMissing {
                path: path.display().to_string(),
            }
        } else {
            LeanToolchainError::LakeManifestIo {
                path: path.display().to_string(),
                source,
            }
        }
    })?;
    let file: LakeManifestFile =
        serde_json::from_str(&raw).map_err(|source| LeanToolchainError::LakeManifestParse {
            path: path.display().to_string(),
            source,
        })?;
    if file.version.is_none() {
        return Err(LeanToolchainError::LakeManifestField {
            path: path.display().to_string(),
            field: "version",
        });
    }
    if file.packages_dir.is_none() {
        return Err(LeanToolchainError::LakeManifestField {
            path: path.display().to_string(),
            field: "packagesDir",
        });
    }
    if file.packages.is_none() {
        return Err(LeanToolchainError::LakeManifestField {
            path: path.display().to_string(),
            field: "packages",
        });
    }
    if file.name.is_none() {
        return Err(LeanToolchainError::LakeManifestField {
            path: path.display().to_string(),
            field: "name",
        });
    }
    Ok(())
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use domain::adapter_prepare::{DependencyId, PreparedManifest};

    fn platform(os: &str, arch: &str) -> TargetPlatform {
        TargetPlatform {
            os: os.into(),
            arch: arch.into(),
        }
    }

    #[test]
    fn all_specs_use_tar_zst_and_pinned_release() {
        for spec in all_lean_toolchain_specs() {
            assert_eq!(spec.format, ArchiveFormat::TarZst);
            assert!(spec.url.contains(LEAN_RELEASE_TAG), "url = {}", spec.url);
            assert_eq!(spec.expected_version, LEAN_EXPECTED_VERSION);
            assert_eq!(spec.sha256.as_str().len(), 64);
        }
    }

    #[test]
    fn extract_lean_release_from_lean_output() {
        let raw = "Lean (version 4.30.0, x86_64-unknown-linux-gnu, commit abc, Release)\n";
        assert_eq!(
            extract_lean_release_at("lean", raw, ReleaseScanMode::First).unwrap(),
            "4.30.0"
        );
    }

    #[test]
    fn extract_lean_release_from_lake_output_uses_last() {
        // Lake output leads with its own version and trails with Lean's.
        let raw = "Lake version 5.0.0-abc (Lean version 4.30.0)\n";
        assert_eq!(
            extract_lean_release_at("lake", raw, ReleaseScanMode::Last).unwrap(),
            "4.30.0"
        );
        // First-hit would grab Lake's own version, which is a different pin
        // and would silently drift as Lake progresses.
        assert_eq!(
            extract_lean_release_at("lake", raw, ReleaseScanMode::First).unwrap(),
            "5.0.0"
        );
    }

    #[test]
    fn extract_lean_release_strips_prerelease_suffix() {
        let raw = "Lean (version 4.30.0-rc1, x86_64-unknown-linux-gnu, commit abc, Release)\n";
        assert_eq!(
            extract_lean_release_at("lean", raw, ReleaseScanMode::First).unwrap(),
            "4.30.0"
        );
    }

    #[test]
    fn extract_lean_release_rejects_output_without_release() {
        let err = extract_lean_release_at("lean", "no version here\n", ReleaseScanMode::First)
            .unwrap_err();
        assert!(matches!(err, LeanToolchainError::VersionUnparseable { .. }));
    }

    fn prepared_set_at(root: PathBuf, platform: TargetPlatform) -> PreparedSet {
        let digest = ContentDigest::from_sha256_bytes([0u8; 32]);
        PreparedSet {
            id: DependencyId::new(digest.clone()),
            root: root.clone(),
            manifest: PreparedManifest {
                id: DependencyId::new(digest),
                target_platform: platform,
                artifacts: vec![],
            },
        }
    }

    #[test]
    fn locate_prepared_lean_root_finds_nested_top_level_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plat = platform("linux", "x86_64");
        let top = tmp
            .path()
            .join("archives")
            .join(LEAN_TOOLCHAIN_LINUX_X64_NAME)
            .join("lean-4.30.0-linux");
        std::fs::create_dir_all(top.join("bin")).unwrap();
        std::fs::write(top.join("bin/lean"), b"stub").unwrap();
        let set = prepared_set_at(tmp.path().to_path_buf(), plat.clone());
        let root = locate_prepared_lean_root(&set, &plat).unwrap();
        assert_eq!(root, top);
    }

    #[test]
    fn locate_prepared_lean_root_missing_archive_fails() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plat = platform("linux", "x86_64");
        let set = prepared_set_at(tmp.path().to_path_buf(), plat.clone());
        let err = locate_prepared_lean_root(&set, &plat).unwrap_err();
        assert!(
            matches!(err, LeanToolchainError::PreparedInstallMissing { .. }),
            "expected PreparedInstallMissing, got {err:?}"
        );
    }
}
