//! Orchestrates downloading, verifying, and atomically publishing prepared
//! dependency sets (spec §6.9, plan 041 Task 2).
//!
//! `prepare_dependencies` walks a parsed `DependencyManifest`, downloads each
//! archive and Git tarball into a per-run staging directory, verifies every
//! SHA-256 checksum, extracts archives with the safe extractor, writes a
//! `manifest.json`, fsyncs it, and atomically renames the staging directory to
//! `<prepared_root>/<dependency-id>/`. Concurrency is protected by an OS
//! advisory lock on `<prepared_root>/prepare.lock` that fails fast.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use domain::adapter_build::{ContentDigest, TargetPlatform};
use domain::adapter_prepare::{
    ArchiveDependency, DependencyId, DependencyManifest, ExpectedPreparedSet, GitDependency,
    PreparedArtifact, PreparedArtifactKind, PreparedManifest, PreparedSet,
};
use fs2::FileExt;
use thiserror::Error;

use super::archive::{ArchiveError, extract_archive};
use super::download::{DownloadError, DownloadPolicy, download_artifact};
use super::prepared::{
    DEPENDENCY_MANIFEST_PATH, PREPARED_MANIFEST_FILE, PrepareError, expected_dependency_id,
    validate_prepared_set, write_prepared_manifest_json,
};

pub const PREPARE_LOCK_FILE: &str = "prepare.lock";
pub const PREPARED_SUBDIR: &str = "prepared";
pub const STAGING_PREFIX: &str = "staging-";

/// Immutable request to `prepare_dependencies`.
#[derive(Debug, Clone)]
pub struct PrepareRequest {
    pub repository_root: PathBuf,
    /// Root that holds `prepared/`, `prepare.lock`, and staging directories.
    /// Usually `<repo>/target/library-analyzers`.
    pub prepared_root: PathBuf,
    pub target_platform: TargetPlatform,
    pub download_policy: DownloadPolicy,
}

#[derive(Debug, Error)]
pub enum PrepareRunError {
    #[error(transparent)]
    Prepare(#[from] PrepareError),
    #[error(transparent)]
    Download(#[from] DownloadError),
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    #[error("failed to acquire prepare lock at {path}: another prepare is running")]
    LockContended { path: String },
    #[error("failed to create {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to publish staging directory to {target}: {source}")]
    Publish {
        target: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to remove staging directory at {path}: {source}")]
    Cleanup {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Prepare dependencies declared in the manifest and publish them into
/// `<prepared_root>/prepared/<dependency-id>/`. Returns the validated set.
///
/// Failure leaves no cache hit: partial staging directories are removed and
/// the final prepared directory is only published after every artifact has
/// been checksummed.
pub fn prepare_dependencies(
    request: &PrepareRequest,
    manifest: &DependencyManifest,
) -> Result<PreparedSet, PrepareRunError> {
    let prepared_dir = request.prepared_root.join(PREPARED_SUBDIR);
    fs::create_dir_all(&prepared_dir).map_err(|source| PrepareRunError::CreateDir {
        path: prepared_dir.display().to_string(),
        source,
    })?;
    fs::create_dir_all(&request.prepared_root).map_err(|source| PrepareRunError::CreateDir {
        path: request.prepared_root.display().to_string(),
        source,
    })?;

    let _lock = PrepareLock::acquire(&request.prepared_root)?;

    let id = expected_dependency_id(&request.repository_root, manifest, &request.target_platform)?;

    let final_dir = prepared_dir.join(id.as_str());
    if final_dir.exists() {
        // Already prepared — re-validate byte-for-byte and return.
        let expected = ExpectedPreparedSet {
            id: id.clone(),
            target_platform: request.target_platform.clone(),
        };
        let set = validate_prepared_set(&final_dir, &expected)?;
        return Ok(set);
    }

    // Fresh staging directory for this run.
    let staging = prepared_dir.join(format!("{STAGING_PREFIX}{}", id.as_str()));
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|source| PrepareRunError::Cleanup {
            path: staging.display().to_string(),
            source,
        })?;
    }
    fs::create_dir_all(&staging).map_err(|source| PrepareRunError::CreateDir {
        path: staging.display().to_string(),
        source,
    })?;

    let mut artifacts: Vec<PreparedArtifact> = Vec::new();
    let outcome = (|| -> Result<(), PrepareRunError> {
        for archive in &manifest.archives {
            artifacts.push(prepare_archive(
                &staging,
                archive,
                &request.download_policy,
            )?);
        }
        for git in &manifest.git {
            artifacts.push(prepare_git(&staging, git, &request.download_policy)?);
        }
        // Locals contribute to the dependency id, not to disk artifacts.
        // Toolchains are declarative; downloads are added in later plans.
        Ok(())
    })();
    if let Err(err) = outcome {
        let _ = fs::remove_dir_all(&staging);
        return Err(err);
    }

    let manifest_out = PreparedManifest {
        id: id.clone(),
        target_platform: request.target_platform.clone(),
        artifacts,
    };
    let manifest_json = write_prepared_manifest_json(&manifest_out);
    let manifest_path = staging.join(PREPARED_MANIFEST_FILE);
    write_and_fsync(&manifest_path, manifest_json.as_bytes())?;
    fsync_directory(&staging)?;

    fs::rename(&staging, &final_dir).map_err(|source| PrepareRunError::Publish {
        target: final_dir.display().to_string(),
        source,
    })?;
    fsync_directory(&prepared_dir)?;

    let expected = ExpectedPreparedSet {
        id: id.clone(),
        target_platform: request.target_platform.clone(),
    };
    let set = validate_prepared_set(&final_dir, &expected)?;
    Ok(set)
}

fn prepare_archive(
    staging: &Path,
    archive: &ArchiveDependency,
    policy: &DownloadPolicy,
) -> Result<PreparedArtifact, PrepareRunError> {
    let downloads = staging.join("downloads");
    fs::create_dir_all(&downloads).map_err(|source| PrepareRunError::CreateDir {
        path: downloads.display().to_string(),
        source,
    })?;
    let archive_path = downloads.join(format!("{}.{}", archive.name, archive.format.as_str()));
    let downloaded = download_artifact(&archive.url, &archive_path, &archive.sha256, policy)?;
    let extract_root = staging.join("archives").join(&archive.name);
    extract_archive(&downloaded.path, archive.format, &extract_root)?;
    Ok(PreparedArtifact {
        name: archive.name.clone(),
        kind: PreparedArtifactKind::Archive,
        relative_path: format!("downloads/{}.{}", archive.name, archive.format.as_str()),
        sha256: archive.sha256.clone(),
    })
}

fn prepare_git(
    staging: &Path,
    git: &GitDependency,
    policy: &DownloadPolicy,
) -> Result<PreparedArtifact, PrepareRunError> {
    let downloads = staging.join("downloads");
    fs::create_dir_all(&downloads).map_err(|source| PrepareRunError::CreateDir {
        path: downloads.display().to_string(),
        source,
    })?;
    let archive_path = downloads.join(format!("{}-{}.archive", git.name, git.commit));
    let _downloaded = download_artifact(&git.url, &archive_path, &git.archive_sha256, policy)?;
    Ok(PreparedArtifact {
        name: git.name.clone(),
        kind: PreparedArtifactKind::Git,
        relative_path: format!("downloads/{}-{}.archive", git.name, git.commit),
        sha256: git.archive_sha256.clone(),
    })
}

fn write_and_fsync(path: &Path, bytes: &[u8]) -> Result<(), PrepareRunError> {
    let mut file = File::create(path).map_err(|source| PrepareRunError::Write {
        path: path.display().to_string(),
        source,
    })?;
    use std::io::Write;
    file.write_all(bytes)
        .map_err(|source| PrepareRunError::Write {
            path: path.display().to_string(),
            source,
        })?;
    file.flush().map_err(|source| PrepareRunError::Write {
        path: path.display().to_string(),
        source,
    })?;
    file.sync_all().map_err(|source| PrepareRunError::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

fn fsync_directory(path: &Path) -> Result<(), PrepareRunError> {
    // Best effort: on some filesystems directory fsync is a no-op. Errors here
    // are surfaced so the caller can log them, but the atomic rename is what
    // guarantees durability of the publication step.
    let file = File::open(path).map_err(|source| PrepareRunError::Write {
        path: path.display().to_string(),
        source,
    })?;
    file.sync_all().map_err(|source| PrepareRunError::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

// ─── advisory lock ──────────────────────────────────────────────────────────

/// RAII guard around the OS advisory lock on `<prepared_root>/prepare.lock`.
#[derive(Debug)]
pub struct PrepareLock {
    file: File,
}

impl PrepareLock {
    pub fn acquire(prepared_root: &Path) -> Result<Self, PrepareRunError> {
        let path = prepared_root.join(PREPARE_LOCK_FILE);
        let file = File::options()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| PrepareRunError::CreateDir {
                path: path.display().to_string(),
                source,
            })?;
        file.try_lock_exclusive()
            .map_err(|_| PrepareRunError::LockContended {
                path: path.display().to_string(),
            })?;
        Ok(Self { file })
    }
}

impl Drop for PrepareLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

// ─── manifest discovery helpers ─────────────────────────────────────────────

/// Returns the on-disk path of the checked-in dependency manifest.
pub fn dependency_manifest_path(root: &Path) -> PathBuf {
    root.join(DEPENDENCY_MANIFEST_PATH)
}

/// Returns the on-disk path of a prepared set for `id` under `prepared_root`.
pub fn prepared_dir(prepared_root: &Path, id: &DependencyId) -> PathBuf {
    prepared_root.join(PREPARED_SUBDIR).join(id.as_str())
}

/// Returns the SHA-256 domain digest a caller can compare against for a
/// content-only check without re-hashing.
pub fn digest_from_bytes(bytes: &[u8]) -> ContentDigest {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out: [u8; 32] = hasher.finalize().into();
    ContentDigest::from_sha256_bytes(out)
}
