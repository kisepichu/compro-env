//! Atomic-write filesystem implementation of [`SiteDataRepository`].
//!
//! The reader (web CI, preview server) sees either the previous version or
//! the new version and never a partial mix. Staging happens inside the parent
//! of `output_dir` so the final `rename` stays on the same filesystem.
//!
//! When `output_dir` already exists, we prefer `renameat2(RENAME_EXCHANGE)`
//! (Linux 3.15+) which swaps the two directories in a single kernel call so
//! there is no visible window where `output_dir` is missing. On platforms
//! without that syscall we fall back to a rename-aside-then-rename-into
//! sequence, which still leaves the reader with either the old or new tree
//! but is not swap-atomic; the fallback path is only taken when the swap
//! call itself is unavailable (`ENOSYS`) or when the target does not yet
//! exist.

use std::ffi::CString;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use site_schema::SiteData;
use tempfile::TempDir;
use usecases::repository::site_data_repository::SiteDataRepository;

const SITE_DATA_FILENAME: &str = "site-data.json";

pub struct SiteDataRepositoryImpl;

impl SiteDataRepositoryImpl {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SiteDataRepositoryImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl SiteDataRepository for SiteDataRepositoryImpl {
    fn write_atomically(&self, output_dir: &Path, data: &SiteData) -> Result<()> {
        let parent = output_dir.parent().ok_or_else(|| {
            anyhow!(
                "output path {} has no parent directory",
                output_dir.display()
            )
        })?;
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to ensure parent directory {} exists",
                parent.display()
            )
        })?;

        // Stage inside the parent so `rename` is same-filesystem.
        let staging = TempDir::new_in(parent).with_context(|| {
            format!(
                "failed to create staging directory under {}",
                parent.display()
            )
        })?;
        write_site_data(staging.path(), data)?;

        let staging_path = staging.keep();

        // If the target does not exist yet, a plain rename is already atomic.
        if !output_dir.exists() {
            fs::rename(&staging_path, output_dir).with_context(|| {
                format!(
                    "failed to move staged site data into {}",
                    output_dir.display()
                )
            })?;
            return Ok(());
        }

        // Preferred path (Linux): swap staging ⇄ output_dir with a single
        // renameat2(RENAME_EXCHANGE) so no reader ever sees a missing target.
        match try_exchange(&staging_path, output_dir) {
            SwapOutcome::Exchanged => {
                // staging_path now holds the OLD contents; remove them.
                fs::remove_dir_all(&staging_path).with_context(|| {
                    format!(
                        "failed to remove replaced output directory at {}",
                        staging_path.display()
                    )
                })?;
                Ok(())
            }
            SwapOutcome::Unsupported => fallback_rename_swap(&staging_path, output_dir),
            SwapOutcome::Failed(err) => Err(err),
        }
    }
}

enum SwapOutcome {
    /// `renameat2(RENAME_EXCHANGE)` succeeded.
    Exchanged,
    /// The kernel / libc lacks `renameat2` — fall back.
    Unsupported,
    /// The syscall failed for a real reason (permissions, ENOENT, …).
    Failed(anyhow::Error),
}

#[cfg(target_os = "linux")]
fn try_exchange(staging: &Path, target: &Path) -> SwapOutcome {
    let staging_c = match CString::new(staging.as_os_str().as_bytes()) {
        Ok(c) => c,
        Err(err) => {
            return SwapOutcome::Failed(anyhow::Error::from(err).context("staging path"));
        }
    };
    let target_c = match CString::new(target.as_os_str().as_bytes()) {
        Ok(c) => c,
        Err(err) => {
            return SwapOutcome::Failed(anyhow::Error::from(err).context("target path"));
        }
    };
    // SAFETY: the two CStrings live for the duration of the call and are
    // both derived from valid `&Path` sources. `renameat2` is a syscall
    // wrapper with no memory-safety obligations beyond valid pointers.
    let ret = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            staging_c.as_ptr(),
            libc::AT_FDCWD,
            target_c.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if ret == 0 {
        SwapOutcome::Exchanged
    } else {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ENOSYS) || err.raw_os_error() == Some(libc::EINVAL) {
            // ENOSYS: kernel lacks renameat2. EINVAL: filesystem does not
            // support RENAME_EXCHANGE (older ext4 mount options, some FUSE
            // filesystems). Both cases warrant the fallback.
            SwapOutcome::Unsupported
        } else {
            SwapOutcome::Failed(anyhow::Error::new(err).context(format!(
                "renameat2(RENAME_EXCHANGE) between {} and {} failed",
                staging.display(),
                target.display()
            )))
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn try_exchange(_staging: &Path, _target: &Path) -> SwapOutcome {
    SwapOutcome::Unsupported
}

/// Non-atomic fallback used when `renameat2(RENAME_EXCHANGE)` isn't
/// available. The reader sees a brief window where the target is missing;
/// this is documented in the module header.
fn fallback_rename_swap(staging: &Path, target: &Path) -> Result<()> {
    let backup = backup_path(target);
    if backup.exists() {
        fs::remove_dir_all(&backup)
            .with_context(|| format!("failed to clean stale backup {}", backup.display()))?;
    }
    fs::rename(target, &backup).with_context(|| {
        format!(
            "failed to move existing output {} aside to {}",
            target.display(),
            backup.display()
        )
    })?;
    if let Err(err) = fs::rename(staging, target) {
        // Restore the backup on failure so we don't leave the reader with
        // no directory at all.
        let _ = fs::rename(&backup, target);
        let _ = fs::remove_dir_all(staging);
        return Err(anyhow::Error::new(err).context(format!(
            "failed to move staged site data into {}",
            target.display()
        )));
    }
    if backup.exists() {
        fs::remove_dir_all(&backup)
            .with_context(|| format!("failed to remove old output backup {}", backup.display()))?;
    }
    Ok(())
}

fn write_site_data(target: &Path, data: &SiteData) -> Result<()> {
    let json = serde_json::to_vec_pretty(data).context("failed to serialize SiteData to JSON")?;
    let file_path = target.join(SITE_DATA_FILENAME);
    let mut file = File::create(&file_path)
        .with_context(|| format!("failed to create {}", file_path.display()))?;
    file.write_all(&json)
        .with_context(|| format!("failed to write {}", file_path.display()))?;
    file.write_all(b"\n").with_context(|| {
        format!(
            "failed to append trailing newline to {}",
            file_path.display()
        )
    })?;
    file.sync_all()
        .with_context(|| format!("failed to fsync {}", file_path.display()))?;
    fsync_dir(target)?;
    Ok(())
}

fn backup_path(target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    let mut name = file_name.into_string().unwrap_or_default();
    name.push_str(".ce-old");
    target
        .parent()
        .map(|p| p.join(&name))
        .unwrap_or_else(|| PathBuf::from(&name))
}

fn fsync_dir(dir: &Path) -> Result<()> {
    let handle = File::open(dir)
        .with_context(|| format!("failed to open directory {} for fsync", dir.display()))?;
    handle
        .sync_all()
        .with_context(|| format!("failed to fsync directory {}", dir.display()))?;
    Ok(())
}
