//! Validate on-disk adapter build state and derive deterministic build IDs
//! (spec §6.9, plan 042 Task 1).
//!
//! `inspect_build_state` treats `target/library-analyzers/` as the analyzer
//! root and answers a single question: is the currently published `bin/`
//! symlink safe to use with today's inputs? It refuses to reuse a build set
//! whenever the marker still says a build is in progress or has failed, the
//! stored manifest disagrees with what the caller expects, the symlink points
//! outside `builds/<build-id>/bin`, the recomputed build id does not match the
//! `<build-id>` directory name, an executable is missing, or any executable's
//! bytes do not hash to the value recorded in the manifest.
//!
//! `derive_build_id` produces the content-addressed name of a build set from
//! the manifest fields the pipeline can recompute. Duplicate language IDs,
//! duplicate `bin/<file_name>`s, and duplicate adapter identities are rejected
//! before hashing so no two byte-different manifests can share an id.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use domain::adapter_build::{
    AdapterExecutableRecord, BuildId, BuildManifest, BuildManifestError, ContentDigest,
    ExpectedBuild, TargetPlatform, ToolchainRecord, UnsignedBuildManifest, validate_build_manifest,
    validate_unsigned_manifest,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Directory that holds `builds/`, `prepared/`, `bin`, and lock files.
pub const ANALYZER_ROOT_SUBDIR: &str = "library-analyzers";
/// Subdirectory under the analyzer root that stores build sets.
pub const BUILDS_SUBDIR: &str = "builds";
/// Symlink under the analyzer root that points at the current `bin/` directory.
pub const CURRENT_BIN_LINK: &str = "bin";
/// Marker file whose presence blocks analysis until the next successful build.
pub const BUILD_IN_PROGRESS_MARKER: &str = "build-in-progress";
/// OS advisory lock used by the build driver.
pub const BUILD_LOCK_FILE: &str = "build.lock";
/// Manifest file name inside a build set.
pub const BUILD_MANIFEST_FILE: &str = "manifest.json";
/// Bin subdirectory inside a build set.
pub const BUILD_BIN_SUBDIR: &str = "bin";

/// Domain separator woven into every build-id hash so unrelated hashes cannot
/// collide with this framing.
const BUILD_ID_DOMAIN: &[u8] = b"compro-env/adapter-build-id/v1\n";

// ─── Errors ─────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum BuildStateError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse manifest at {path}: {source}")]
    ManifestParse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("build manifest is missing at {path}")]
    ManifestMissing { path: String },
    #[error("stored manifest disagrees with the expected build: {source}")]
    Mismatch {
        #[source]
        source: BuildManifestError,
    },
    #[error("stored manifest is malformed: {source}")]
    Malformed {
        #[source]
        source: BuildManifestError,
    },
    #[error("build id derivation failed: {source}")]
    Derivation {
        #[source]
        source: BuildManifestError,
    },
    #[error(
        "build id mismatch: manifest hashes to {expected}, but the directory \
         name is {actual}"
    )]
    BuildIdMismatch { expected: BuildId, actual: String },
    #[error("current bin symlink at {path} is missing")]
    CurrentBinMissing { path: String },
    #[error("current bin at {path} must be a symlink to builds/<build-id>/bin")]
    CurrentBinNotSymlink { path: String },
    #[error(
        "current bin symlink at {path} points to {target}, expected a \
         relative path to builds/<build-id>/bin"
    )]
    CurrentBinBadTarget { path: String, target: String },
    #[error("executable {file_name:?} is missing from the build set at {path}")]
    ExecutableMissing { path: String, file_name: String },
    #[error(
        "executable {file_name:?} at {path} hash mismatch: manifest recorded \
         {expected}, actual {actual}"
    )]
    ExecutableHashMismatch {
        path: String,
        file_name: String,
        expected: String,
        actual: String,
    },
    #[error("another build is already running (advisory lock on {path} is held)")]
    BuildRunning { path: String },
    #[error(
        "previous adapter build did not finish: marker present at {marker_path} \
         and no build is running; re-run the build to clear it"
    )]
    PreviousBuildFailed { marker_path: String },
    #[error("manifest lists an unknown adapter toolchain / executable field: {field:?}")]
    UnknownManifestField { field: String },
    #[error("manifest at {path} has an invalid content digest {value:?}: {reason}")]
    InvalidDigest {
        path: String,
        value: String,
        reason: String,
    },
}

/// Validated build set: the caller can invoke the recorded executables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsableBuildSet {
    pub root: PathBuf,
    pub build_id: BuildId,
    pub manifest: BuildManifest,
}

// ─── derive_build_id ────────────────────────────────────────────────────────

/// Derive a deterministic `BuildId` from an unsigned manifest.
///
/// The build ID depends only on inputs the pipeline can recompute: the input
/// digest, target platform, build profile, protocol version, and a
/// canonicalized list of executable identities. Executables are sorted by
/// language ID before hashing so map iteration order or per-adapter build
/// completion order cannot change the id.
pub fn derive_build_id(manifest: &UnsignedBuildManifest) -> Result<BuildId, BuildStateError> {
    validate_unsigned_manifest(manifest)
        .map_err(|source| BuildStateError::Derivation { source })?;

    let mut execs = manifest.executables.clone();
    execs.sort_by(|a, b| a.language.cmp(&b.language));

    let mut hasher = Sha256::new();
    hasher.update(BUILD_ID_DOMAIN);
    write_framed(&mut hasher, manifest.input_digest.as_str().as_bytes());
    write_framed(&mut hasher, manifest.target_platform.os.as_bytes());
    write_framed(&mut hasher, manifest.target_platform.arch.as_bytes());
    write_framed(&mut hasher, manifest.build_profile.as_bytes());
    write_framed(
        &mut hasher,
        manifest.protocol_version.to_be_bytes().as_ref(),
    );
    write_framed(&mut hasher, format!("{}", execs.len()).as_bytes());
    for e in &execs {
        write_framed(&mut hasher, e.language.as_bytes());
        write_framed(&mut hasher, e.file_name.as_bytes());
        write_framed(&mut hasher, e.sha256.as_str().as_bytes());
        write_framed(&mut hasher, e.adapter_name.as_bytes());
        write_framed(&mut hasher, e.adapter_version.as_bytes());
        write_framed(&mut hasher, format!("{}", e.toolchains.len()).as_bytes());
        let mut toolchains = e.toolchains.clone();
        toolchains.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.version.cmp(&b.version))
                .then_with(|| a.target.cmp(&b.target))
        });
        for t in &toolchains {
            write_framed(&mut hasher, t.name.as_bytes());
            write_framed(&mut hasher, t.version.as_bytes());
            match &t.target {
                Some(v) => write_framed(&mut hasher, v.as_bytes()),
                None => write_framed(&mut hasher, b""),
            }
        }
    }
    let out: [u8; 32] = hasher.finalize().into();
    Ok(BuildId::new(ContentDigest::from_sha256_bytes(out)))
}

fn write_framed(hasher: &mut Sha256, data: &[u8]) {
    let len = data.len() as u64;
    hasher.update(len.to_be_bytes());
    hasher.update(data);
}

// ─── inspect_build_state ────────────────────────────────────────────────────

/// Validate the current build set under `root` against `expected`.
///
/// `root` is the analyzer root (usually `<repo>/target/library-analyzers`). On
/// success the caller receives a validated `UsableBuildSet`. Any inconsistency
/// with the marker, lock, symlink, manifest, or executables is surfaced as a
/// distinct error variant so the pipeline can decide whether to prompt for a
/// rebuild or refuse to run analysis at all.
pub fn inspect_build_state(
    root: &Path,
    expected: &ExpectedBuild,
) -> Result<UsableBuildSet, BuildStateError> {
    // 1. Marker vs. lock: if the marker is present, distinguish an active
    //    build from a previous crash before touching anything else.
    let marker_path = root.join(BUILD_IN_PROGRESS_MARKER);
    let lock_path = root.join(BUILD_LOCK_FILE);
    match fs::symlink_metadata(&marker_path) {
        Ok(_) => {
            if let Some(_probe) = try_probe_build_lock(&lock_path)? {
                // Lock was acquired -> no other process holds it.
                return Err(BuildStateError::PreviousBuildFailed {
                    marker_path: display_path(&marker_path),
                });
            } else {
                return Err(BuildStateError::BuildRunning {
                    path: display_path(&lock_path),
                });
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(BuildStateError::Io {
                path: display_path(&marker_path),
                source,
            });
        }
    }

    // 2. `bin` symlink must resolve to `builds/<build-id>/bin` under `root`.
    let bin_link = root.join(CURRENT_BIN_LINK);
    let link_meta = match fs::symlink_metadata(&bin_link) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(BuildStateError::CurrentBinMissing {
                path: display_path(&bin_link),
            });
        }
        Err(source) => {
            return Err(BuildStateError::Io {
                path: display_path(&bin_link),
                source,
            });
        }
    };
    if !link_meta.file_type().is_symlink() {
        return Err(BuildStateError::CurrentBinNotSymlink {
            path: display_path(&bin_link),
        });
    }
    let raw_target = fs::read_link(&bin_link).map_err(|source| BuildStateError::Io {
        path: display_path(&bin_link),
        source,
    })?;
    let build_id_dir_name = parse_relative_bin_target(&raw_target).ok_or_else(|| {
        BuildStateError::CurrentBinBadTarget {
            path: display_path(&bin_link),
            target: raw_target.display().to_string(),
        }
    })?;
    let build_dir = root.join(BUILDS_SUBDIR).join(&build_id_dir_name);
    let bin_dir = build_dir.join(BUILD_BIN_SUBDIR);

    // Ensure the symlink actually points inside our tree, not a same-name path
    // elsewhere. Canonicalize both sides and require the target to live under
    // `root/BUILDS_SUBDIR`.
    let target_canonical = bin_link
        .canonicalize()
        .map_err(|source| BuildStateError::Io {
            path: display_path(&bin_link),
            source,
        })?;
    let bin_canonical = bin_dir
        .canonicalize()
        .map_err(|source| BuildStateError::Io {
            path: display_path(&bin_dir),
            source,
        })?;
    if target_canonical != bin_canonical {
        return Err(BuildStateError::CurrentBinBadTarget {
            path: display_path(&bin_link),
            target: raw_target.display().to_string(),
        });
    }

    // 3. Read and parse manifest.
    let manifest_path = build_dir.join(BUILD_MANIFEST_FILE);
    let bytes = match fs::read(&manifest_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(BuildStateError::ManifestMissing {
                path: display_path(&manifest_path),
            });
        }
        Err(source) => {
            return Err(BuildStateError::Io {
                path: display_path(&manifest_path),
                source,
            });
        }
    };
    let dto: BuildManifestDto =
        serde_json::from_slice(&bytes).map_err(|source| BuildStateError::ManifestParse {
            path: display_path(&manifest_path),
            source,
        })?;
    let manifest = dto.into_domain(&display_path(&manifest_path))?;

    // 4. Validate against expected build parameters.
    validate_build_manifest(expected, &manifest)
        .map_err(|source| BuildStateError::Mismatch { source })?;

    // 5. Recompute and cross-check the build id against the directory name.
    let unsigned = UnsignedBuildManifest::from(&manifest);
    let derived = derive_build_id(&unsigned)?;
    if derived.as_str() != build_id_dir_name {
        return Err(BuildStateError::BuildIdMismatch {
            expected: derived,
            actual: build_id_dir_name,
        });
    }

    // 6. Byte-verify every executable in `bin/`.
    for exec in &manifest.executables {
        validate_file_name(&exec.file_name)?;
        let exec_path = bin_dir.join(&exec.file_name);
        let meta = match fs::symlink_metadata(&exec_path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(BuildStateError::ExecutableMissing {
                    path: display_path(&exec_path),
                    file_name: exec.file_name.clone(),
                });
            }
            Err(source) => {
                return Err(BuildStateError::Io {
                    path: display_path(&exec_path),
                    source,
                });
            }
        };
        if meta.file_type().is_symlink() {
            // Symlink executables would let the driver publish arbitrary code.
            return Err(BuildStateError::CurrentBinBadTarget {
                path: display_path(&exec_path),
                target: "symlink executable is not permitted".into(),
            });
        }
        if !meta.file_type().is_file() {
            return Err(BuildStateError::ExecutableMissing {
                path: display_path(&exec_path),
                file_name: exec.file_name.clone(),
            });
        }
        let actual = stream_sha256(&exec_path).map_err(|source| BuildStateError::Io {
            path: display_path(&exec_path),
            source,
        })?;
        if actual != exec.sha256 {
            return Err(BuildStateError::ExecutableHashMismatch {
                path: display_path(&exec_path),
                file_name: exec.file_name.clone(),
                expected: exec.sha256.to_string(),
                actual: actual.to_string(),
            });
        }
    }

    Ok(UsableBuildSet {
        root: build_dir,
        build_id: derived,
        manifest,
    })
}

fn validate_file_name(name: &str) -> Result<(), BuildStateError> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        || name == "."
        || name == ".."
    {
        return Err(BuildStateError::UnknownManifestField {
            field: format!("executable file_name {name:?}"),
        });
    }
    Ok(())
}

/// Parse a `bin` symlink target of the form `builds/<build-id>/bin` (relative
/// to the analyzer root) and return `<build-id>`. Reject absolute paths, `..`,
/// or targets pointing outside `builds/`.
fn parse_relative_bin_target(raw: &Path) -> Option<String> {
    if raw.is_absolute() {
        return None;
    }
    let components: Vec<_> = raw.components().collect();
    if components.len() != 3 {
        return None;
    }
    let expect_builds = matches!(
        components[0],
        std::path::Component::Normal(v) if v.to_str() == Some(BUILDS_SUBDIR),
    );
    let expect_bin = matches!(
        components[2],
        std::path::Component::Normal(v) if v.to_str() == Some(BUILD_BIN_SUBDIR),
    );
    if !expect_builds || !expect_bin {
        return None;
    }
    match components[1] {
        std::path::Component::Normal(v) => {
            let name = v.to_str()?;
            if name == "." || name == ".." {
                None
            } else {
                Some(name.to_string())
            }
        }
        _ => None,
    }
}

/// Try to non-blockingly acquire the build lock. Returns `Some(guard)` if we
/// obtained it (meaning no other process holds it), or `None` if the lock is
/// held by someone else. Any other filesystem error is surfaced verbatim.
fn try_probe_build_lock(path: &Path) -> Result<Option<ProbeLock>, BuildStateError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| BuildStateError::Io {
            path: display_path(parent),
            source,
        })?;
    }
    let file = fs::File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|source| BuildStateError::Io {
            path: display_path(path),
            source,
        })?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(ProbeLock { file })),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        // fs2 returns a variety of errors on non-Unix; treat any non-block
        // failure as "some other process has it" so callers stay safe.
        Err(_) => Ok(None),
    }
}

struct ProbeLock {
    file: fs::File,
}

impl Drop for ProbeLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn stream_sha256(path: &Path) -> Result<ContentDigest, std::io::Error> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let out: [u8; 32] = hasher.finalize().into();
    Ok(ContentDigest::from_sha256_bytes(out))
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

// ─── JSON DTO ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BuildManifestDto {
    pub(crate) input_digest: String,
    pub(crate) target_platform: TargetPlatformDto,
    pub(crate) build_profile: String,
    pub(crate) protocol_version: u32,
    pub(crate) git_commit_sha: String,
    pub(crate) executables: Vec<AdapterExecutableDto>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetPlatformDto {
    pub(crate) os: String,
    pub(crate) arch: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdapterExecutableDto {
    pub(crate) language: String,
    pub(crate) file_name: String,
    pub(crate) sha256: String,
    pub(crate) adapter_name: String,
    pub(crate) adapter_version: String,
    #[serde(default)]
    pub(crate) toolchains: Vec<ToolchainDto>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolchainDto {
    pub(crate) name: String,
    pub(crate) version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) target: Option<String>,
}

impl BuildManifestDto {
    pub(crate) fn from_domain(m: &BuildManifest) -> Self {
        Self {
            input_digest: m.input_digest.to_string(),
            target_platform: TargetPlatformDto {
                os: m.target_platform.os.clone(),
                arch: m.target_platform.arch.clone(),
            },
            build_profile: m.build_profile.clone(),
            protocol_version: m.protocol_version,
            git_commit_sha: m.git_commit_sha.clone(),
            executables: m
                .executables
                .iter()
                .map(|e| AdapterExecutableDto {
                    language: e.language.clone(),
                    file_name: e.file_name.clone(),
                    sha256: e.sha256.to_string(),
                    adapter_name: e.adapter_name.clone(),
                    adapter_version: e.adapter_version.clone(),
                    toolchains: e
                        .toolchains
                        .iter()
                        .map(|t| ToolchainDto {
                            name: t.name.clone(),
                            version: t.version.clone(),
                            target: t.target.clone(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    fn into_domain(self, manifest_path: &str) -> Result<BuildManifest, BuildStateError> {
        let input_digest = parse_digest(&self.input_digest, manifest_path)?;
        let mut executables = Vec::with_capacity(self.executables.len());
        for e in self.executables {
            let sha256 = parse_digest(&e.sha256, manifest_path)?;
            executables.push(AdapterExecutableRecord {
                language: e.language,
                file_name: e.file_name,
                sha256,
                adapter_name: e.adapter_name,
                adapter_version: e.adapter_version,
                toolchains: e
                    .toolchains
                    .into_iter()
                    .map(|t| ToolchainRecord {
                        name: t.name,
                        version: t.version,
                        target: t.target,
                    })
                    .collect(),
            });
        }
        Ok(BuildManifest {
            input_digest,
            target_platform: TargetPlatform {
                os: self.target_platform.os,
                arch: self.target_platform.arch,
            },
            build_profile: self.build_profile,
            protocol_version: self.protocol_version,
            git_commit_sha: self.git_commit_sha,
            executables,
        })
    }
}

fn parse_digest(value: &str, manifest_path: &str) -> Result<ContentDigest, BuildStateError> {
    ContentDigest::from_hex(value).map_err(|source| BuildStateError::InvalidDigest {
        path: manifest_path.to_string(),
        value: value.to_string(),
        reason: source.to_string(),
    })
}

/// Write a manifest to canonical JSON (stable field order, pretty-printed).
pub fn write_build_manifest_json(manifest: &BuildManifest) -> String {
    let dto = BuildManifestDto::from_domain(manifest);
    serde_json::to_string_pretty(&dto).expect("build manifest is always serializable")
}
