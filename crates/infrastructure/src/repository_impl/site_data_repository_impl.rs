//! Atomic-write filesystem implementation of [`SiteDataRepository`].
//!
//! The reader (web CI, preview server) sees either the previous version or
//! the new version and never a partial mix. Staging happens inside the parent
//! of `output_dir` so the final `rename` stays on the same filesystem and is
//! atomic under POSIX semantics; the old directory, if any, is atomically
//! swapped out and then cleaned up.

use std::fs::{self, File};
use std::io::Write;
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

        // Persist the staging directory into place: rename to output_dir.
        // If output_dir already exists, swap by renaming it aside first, then
        // renaming staging into place, then removing the old dir. On Linux
        // `renameat2(RENAME_EXCHANGE)` would be preferable but is not portable,
        // so we accept a brief window where the target does not exist.
        let staging_path = staging.keep();
        let backup = backup_path(output_dir);

        if output_dir.exists() {
            if backup.exists() {
                fs::remove_dir_all(&backup).with_context(|| {
                    format!("failed to clean stale backup {}", backup.display())
                })?;
            }
            fs::rename(output_dir, &backup).with_context(|| {
                format!(
                    "failed to move existing output {} aside to {}",
                    output_dir.display(),
                    backup.display()
                )
            })?;
        }

        if let Err(err) = fs::rename(&staging_path, output_dir) {
            // Restore the backup on failure so we don't leave the reader with
            // no directory at all.
            if backup.exists() {
                let _ = fs::rename(&backup, output_dir);
            }
            // Cleanup staging leftovers.
            let _ = fs::remove_dir_all(&staging_path);
            return Err(anyhow::Error::new(err).context(format!(
                "failed to move staged site data into {}",
                output_dir.display()
            )));
        }

        if backup.exists() {
            fs::remove_dir_all(&backup).with_context(|| {
                format!("failed to remove old output backup {}", backup.display())
            })?;
        }

        Ok(())
    }
}

fn write_site_data(target: &Path, data: &SiteData) -> Result<()> {
    let json = serde_json::to_vec_pretty(data).context("failed to serialize SiteData to JSON")?;
    let file_path = target.join(SITE_DATA_FILENAME);
    let mut file = File::create(&file_path)
        .with_context(|| format!("failed to create {}", file_path.display()))?;
    file.write_all(&json)
        .with_context(|| format!("failed to write {}", file_path.display()))?;
    file.write_all(b"\n").ok();
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
