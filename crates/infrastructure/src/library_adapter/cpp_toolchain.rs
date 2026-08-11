//! Pinned Clang/LLVM toolchain selection for the C++ adapter (spec §§6.7,
//! 6.9; plan 045 Task 1).
//!
//! `select_cpp_toolchain` maps a build `TargetPlatform` to the one official
//! LLVM 22.1.0 release archive that supports it. `all_cpp_toolchain_specs`
//! returns every supported archive so `dependencies.toml` can list all three,
//! letting the per-target archive gate in `ArchiveDependency` pick the right
//! one at prepare time.
//!
//! After extraction, `validate_llvm_layout` verifies the expected binaries,
//! library directory, and Clang include tree are present, then runs
//! `llvm-config --version` and rejects any output that is not exactly
//! `22.1.0`. The command is spawned via `default_version_reader` under a
//! cleared environment with a minimal PATH so the host system's `LLVM_CONFIG`
//! or `PATH` can never impersonate the pinned build.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use domain::adapter_build::{ContentDigest, TargetPlatform};
use domain::adapter_prepare::{ArchiveFormat, PreparedSet};
use thiserror::Error;

/// The single Clang/LLVM release these C++ analyzer builds are pinned to.
pub const LLVM_EXPECTED_VERSION: &str = "22.1.0";

const LLVM_RELEASE_TAG: &str = "llvmorg-22.1.0";
const LLVM_BASE_URL: &str = "https://github.com/llvm/llvm-project/releases/download";

// SHA-256 digests of the three official LLVM 22.1.0 release archives, taken
// from the plan. Keeping them here (and not in the manifest) means the plan's
// checksums are load-bearing in the type system: adding a new archive to the
// dependency manifest without also declaring it here yields a compile error.
const LLVM_LINUX_X64_ARCHIVE: &str = "LLVM-22.1.0-Linux-X64.tar.xz";
const LLVM_LINUX_X64_SHA256: &str =
    "8d662e425e46c48b45f5f970770b5e37f323607c8c2cbc371593fc9c4ba1e7b3";

const LLVM_LINUX_ARM64_ARCHIVE: &str = "LLVM-22.1.0-Linux-ARM64.tar.xz";
const LLVM_LINUX_ARM64_SHA256: &str =
    "e3b4205fe45d5561dec9d46465873a79c26b25b028b310515b38c34f668c6aec";

const LLVM_MACOS_ARM64_ARCHIVE: &str = "LLVM-22.1.0-macOS-ARM64.tar.xz";
const LLVM_MACOS_ARM64_SHA256: &str =
    "cd5e615f4dab23d0239359cd343202c5f6ceeaf072c245a3c685d73afac09646";

/// Archive-name slugs used both here and in `dependencies.toml`.
pub const CPP_TOOLCHAIN_LINUX_X64_NAME: &str = "llvm-linux-x64";
pub const CPP_TOOLCHAIN_LINUX_ARM64_NAME: &str = "llvm-linux-arm64";
pub const CPP_TOOLCHAIN_MACOS_ARM64_NAME: &str = "llvm-macos-arm64";

/// One official LLVM release archive, ready to be turned into an
/// `ArchiveDependency` for the dependency manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CppToolchainSpec {
    pub archive_name: String,
    pub url: String,
    pub sha256: ContentDigest,
    pub format: ArchiveFormat,
    pub target_os: String,
    pub target_arch: String,
    pub expected_version: &'static str,
}

/// Paths resolved inside an extracted LLVM archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CppToolchainPaths {
    pub root: PathBuf,
    pub clang: PathBuf,
    pub llvm_config: PathBuf,
    pub lib_dir: PathBuf,
    pub include_dir: PathBuf,
    /// Normalized `llvm-config --version` output (whitespace trimmed).
    pub version: String,
}

#[derive(Debug, Error)]
pub enum CppToolchainError {
    #[error(
        "no pinned LLVM archive is available for target {os}/{arch}; \
         supported targets are linux/x86_64, linux/aarch64, macos/aarch64"
    )]
    UnsupportedTarget { os: String, arch: String },
    #[error("expected LLVM binary is missing: {path}")]
    MissingBinary { path: String },
    #[error("expected LLVM library directory is missing: {path}")]
    MissingLib { path: String },
    #[error("expected LLVM include directory is missing: {path}")]
    MissingInclude { path: String },
    #[error("LLVM version mismatch: expected {expected}, got {actual:?}")]
    VersionMismatch { expected: String, actual: String },
    #[error("failed to read LLVM version: {0}")]
    VersionRead(#[source] io::Error),
    #[error("prepared LLVM install for {archive_name:?} is not on disk at {path}")]
    PreparedInstallMissing { archive_name: String, path: String },
    #[error("prepared LLVM install for {archive_name:?} does not contain bin/clang under {path}")]
    PreparedInstallLayout { archive_name: String, path: String },
    #[error("failed to read prepared LLVM install directory {path}: {source}")]
    PreparedInstallIo {
        path: String,
        #[source]
        source: io::Error,
    },
}

/// Return the toolchain spec for the current build target, or fail before any
/// download is attempted. Callers must never fall back to a host-installed
/// Clang.
pub fn select_cpp_toolchain(
    platform: &TargetPlatform,
) -> Result<CppToolchainSpec, CppToolchainError> {
    all_cpp_toolchain_specs()
        .into_iter()
        .find(|spec| spec.target_os == platform.os && spec.target_arch == platform.arch)
        .ok_or_else(|| CppToolchainError::UnsupportedTarget {
            os: platform.os.clone(),
            arch: platform.arch.clone(),
        })
}

/// All supported LLVM archive specs. Used to build `dependencies.toml` entries
/// so the manifest lists every triple and the per-target gate picks the right
/// one at prepare time.
pub fn all_cpp_toolchain_specs() -> Vec<CppToolchainSpec> {
    vec![
        spec_from_parts(
            CPP_TOOLCHAIN_LINUX_X64_NAME,
            LLVM_LINUX_X64_ARCHIVE,
            LLVM_LINUX_X64_SHA256,
            "linux",
            "x86_64",
        ),
        spec_from_parts(
            CPP_TOOLCHAIN_LINUX_ARM64_NAME,
            LLVM_LINUX_ARM64_ARCHIVE,
            LLVM_LINUX_ARM64_SHA256,
            "linux",
            "aarch64",
        ),
        spec_from_parts(
            CPP_TOOLCHAIN_MACOS_ARM64_NAME,
            LLVM_MACOS_ARM64_ARCHIVE,
            LLVM_MACOS_ARM64_SHA256,
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
) -> CppToolchainSpec {
    CppToolchainSpec {
        archive_name: archive_name.to_string(),
        url: format!("{LLVM_BASE_URL}/{LLVM_RELEASE_TAG}/{file_name}"),
        sha256: ContentDigest::from_hex(sha256_hex)
            .expect("pinned LLVM SHA-256 constants are valid lowercase hex"),
        format: ArchiveFormat::TarXz,
        target_os: target_os.to_string(),
        target_arch: target_arch.to_string(),
        expected_version: LLVM_EXPECTED_VERSION,
    }
}

/// Verify the expected LLVM release layout under `archive_root`. `version_reader`
/// runs `llvm-config --version` (or the caller's stand-in) and must return the
/// trimmed textual version. Both a wrong version and a missing file cause a
/// hard failure — this pipeline never falls back to a host toolchain.
pub fn validate_llvm_layout<F>(
    archive_root: &Path,
    version_reader: F,
) -> Result<CppToolchainPaths, CppToolchainError>
where
    F: FnOnce(&Path) -> Result<String, io::Error>,
{
    let clang = archive_root.join("bin/clang");
    let llvm_config = archive_root.join("bin/llvm-config");
    let lib_dir = archive_root.join("lib");
    let include_dir = archive_root.join("include/clang");

    if !clang.is_file() {
        return Err(CppToolchainError::MissingBinary {
            path: clang.display().to_string(),
        });
    }
    if !llvm_config.is_file() {
        return Err(CppToolchainError::MissingBinary {
            path: llvm_config.display().to_string(),
        });
    }
    if !lib_dir.is_dir() {
        return Err(CppToolchainError::MissingLib {
            path: lib_dir.display().to_string(),
        });
    }
    if !include_dir.is_dir() {
        return Err(CppToolchainError::MissingInclude {
            path: include_dir.display().to_string(),
        });
    }

    let raw = version_reader(&llvm_config).map_err(CppToolchainError::VersionRead)?;
    let version = raw.trim().to_string();
    if version != LLVM_EXPECTED_VERSION {
        return Err(CppToolchainError::VersionMismatch {
            expected: LLVM_EXPECTED_VERSION.to_string(),
            actual: version,
        });
    }

    Ok(CppToolchainPaths {
        root: archive_root.to_path_buf(),
        clang,
        llvm_config,
        lib_dir,
        include_dir,
        version,
    })
}

/// Run `<llvm_config> --version` under a cleared environment and a minimal
/// PATH so the host system cannot smuggle in a different LLVM. Returns the
/// trimmed stdout.
pub fn default_version_reader(llvm_config: &Path) -> Result<String, io::Error> {
    let output = Command::new(llvm_config)
        .arg("--version")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "llvm-config --version exited with status {}",
            output.status
        )));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(text.trim().to_string())
}

/// Resolve the LLVM install directory (the folder that contains `bin/`,
/// `lib/`, `include/`, and CMake package files) under a prepared set for the
/// given target platform.
///
/// The dependency-manifest pipeline unpacks the LLVM tarball into
/// `<prepared_root>/archives/<archive_name>/`. Official release archives ship
/// a top-level directory (e.g. `LLVM-22.1.0-Linux-X64/`), so we search one
/// level deeper for the directory that owns `bin/clang`. If future archives
/// unpack flat we still find `bin/clang` at the archive root and return that.
///
/// This is the helper `cpp_build_plan` uses to derive `CE_LLVM_DIR` for the
/// build.sh driver; it also produces a `CppToolchainPaths` via
/// `validate_llvm_layout` so callers can audit the pinned identity.
pub fn locate_prepared_llvm_root(
    prepared_set: &PreparedSet,
    platform: &TargetPlatform,
) -> Result<PathBuf, CppToolchainError> {
    let spec = select_cpp_toolchain(platform)?;
    let archives_dir = prepared_set.root.join("archives").join(&spec.archive_name);
    if !archives_dir.is_dir() {
        return Err(CppToolchainError::PreparedInstallMissing {
            archive_name: spec.archive_name.clone(),
            path: archives_dir.display().to_string(),
        });
    }
    // Prefer the tarball's top-level directory (contains bin/, lib/, ...).
    // Fall back to `archives_dir` itself if a future flat archive lands here.
    if archives_dir.join("bin/clang").is_file() {
        return Ok(archives_dir);
    }
    let entries =
        fs::read_dir(&archives_dir).map_err(|source| CppToolchainError::PreparedInstallIo {
            path: archives_dir.display().to_string(),
            source,
        })?;
    for entry in entries {
        let entry = entry.map_err(|source| CppToolchainError::PreparedInstallIo {
            path: archives_dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() && path.join("bin/clang").is_file() {
            return Ok(path);
        }
    }
    Err(CppToolchainError::PreparedInstallLayout {
        archive_name: spec.archive_name,
        path: archives_dir.display().to_string(),
    })
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
    fn all_specs_use_tar_xz_and_pinned_release() {
        for spec in all_cpp_toolchain_specs() {
            assert_eq!(spec.format, ArchiveFormat::TarXz);
            assert!(spec.url.contains(LLVM_RELEASE_TAG), "url = {}", spec.url);
            assert_eq!(spec.expected_version, LLVM_EXPECTED_VERSION);
            assert_eq!(spec.sha256.as_str().len(), 64);
        }
    }

    #[test]
    fn select_returns_linux_x86_64_spec() {
        let spec = select_cpp_toolchain(&platform("linux", "x86_64")).unwrap();
        assert_eq!(spec.archive_name, CPP_TOOLCHAIN_LINUX_X64_NAME);
    }

    /// Helper: build a `PreparedSet` whose `root` points at a temporary
    /// directory. The manifest inside is a placeholder — `locate_prepared_llvm_root`
    /// only inspects the on-disk layout, not the manifest.
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
    fn locate_prepared_llvm_root_finds_nested_top_level_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plat = platform("linux", "x86_64");
        let top = tmp
            .path()
            .join("archives")
            .join(CPP_TOOLCHAIN_LINUX_X64_NAME)
            .join("LLVM-22.1.0-Linux-X64");
        std::fs::create_dir_all(top.join("bin")).unwrap();
        std::fs::write(top.join("bin/clang"), b"stub").unwrap();
        let set = prepared_set_at(tmp.path().to_path_buf(), plat.clone());
        let root = locate_prepared_llvm_root(&set, &plat).unwrap();
        assert_eq!(root, top);
    }

    #[test]
    fn locate_prepared_llvm_root_accepts_flat_layout() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plat = platform("linux", "x86_64");
        let flat = tmp
            .path()
            .join("archives")
            .join(CPP_TOOLCHAIN_LINUX_X64_NAME);
        std::fs::create_dir_all(flat.join("bin")).unwrap();
        std::fs::write(flat.join("bin/clang"), b"stub").unwrap();
        let set = prepared_set_at(tmp.path().to_path_buf(), plat.clone());
        let root = locate_prepared_llvm_root(&set, &plat).unwrap();
        assert_eq!(root, flat);
    }

    #[test]
    fn locate_prepared_llvm_root_missing_archive_fails() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plat = platform("linux", "x86_64");
        let set = prepared_set_at(tmp.path().to_path_buf(), plat.clone());
        let err = locate_prepared_llvm_root(&set, &plat).unwrap_err();
        assert!(
            matches!(err, CppToolchainError::PreparedInstallMissing { .. }),
            "expected PreparedInstallMissing, got {err:?}"
        );
    }

    #[test]
    fn locate_prepared_llvm_root_rejects_archive_without_clang() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plat = platform("linux", "x86_64");
        let archive_dir = tmp
            .path()
            .join("archives")
            .join(CPP_TOOLCHAIN_LINUX_X64_NAME);
        std::fs::create_dir_all(&archive_dir).unwrap();
        // Empty directory: no bin/clang, no children with bin/clang.
        let set = prepared_set_at(tmp.path().to_path_buf(), plat.clone());
        let err = locate_prepared_llvm_root(&set, &plat).unwrap_err();
        assert!(
            matches!(err, CppToolchainError::PreparedInstallLayout { .. }),
            "expected PreparedInstallLayout, got {err:?}"
        );
    }
}
