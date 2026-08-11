//! Safe extraction of downloaded archives (spec §6.9, plan 041 Task 2).
//!
//! `extract_archive` opens a tar.gz or zip archive and unpacks it into a
//! caller-provided destination directory. Absolute paths, `..` traversal,
//! symlinks, hard links, devices, sockets, and duplicate entries are all
//! rejected before any bytes hit the disk. The destination is created if
//! missing but the extractor refuses to overwrite existing files.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use domain::adapter_prepare::ArchiveFormat;
use flate2::read::GzDecoder;
use tar::{Archive, EntryType};
use thiserror::Error;
use zip::ZipArchive;

#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("failed to open archive at {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("archive contains an unsafe entry {entry:?}: {reason}")]
    UnsafeEntry { entry: String, reason: &'static str },
    #[error("archive entry {entry:?} escapes destination {destination:?}")]
    Escape { entry: String, destination: String },
    #[error("failed to write extracted file at {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read archive entry {entry:?}: {source}")]
    Read {
        entry: String,
        #[source]
        source: std::io::Error,
    },
    #[error("archive is invalid: {source}")]
    Invalid {
        #[source]
        source: std::io::Error,
    },
    #[error("zip archive is invalid: {source}")]
    InvalidZip {
        #[source]
        source: zip::result::ZipError,
    },
    #[error("archive contained duplicate entry {entry:?}")]
    Duplicate { entry: String },
}

/// Extract `archive_path` (already downloaded and checksummed) into
/// `destination`. `destination` must not exist as a file. Directories are
/// created as needed. Returns `destination` on success.
pub fn extract_archive(
    archive_path: &Path,
    format: ArchiveFormat,
    destination: &Path,
) -> Result<PathBuf, ArchiveError> {
    fs::create_dir_all(destination).map_err(|source| ArchiveError::Write {
        path: destination.display().to_string(),
        source,
    })?;
    match format {
        ArchiveFormat::TarGz => extract_tar_gz(archive_path, destination),
        ArchiveFormat::Zip => extract_zip(archive_path, destination),
    }?;
    Ok(destination.to_path_buf())
}

// ─── tar.gz ─────────────────────────────────────────────────────────────────

fn extract_tar_gz(archive_path: &Path, destination: &Path) -> Result<(), ArchiveError> {
    let file = File::open(archive_path).map_err(|source| ArchiveError::Open {
        path: archive_path.display().to_string(),
        source,
    })?;
    let mut archive = Archive::new(GzDecoder::new(file));
    let mut seen: std::collections::BTreeSet<PathBuf> = Default::default();

    let entries = archive
        .entries()
        .map_err(|source| ArchiveError::Invalid { source })?;
    for entry in entries {
        let mut entry = entry.map_err(|source| ArchiveError::Invalid { source })?;
        let raw_path = entry
            .path()
            .map_err(|source| ArchiveError::Invalid { source })?;
        let display = raw_path.display().to_string();
        let relative = check_relative_path(&raw_path, &display)?;
        let entry_type = entry.header().entry_type();
        match entry_type {
            EntryType::Regular => {
                let target = destination.join(&relative);
                enforce_within(&target, destination, &display)?;
                if !seen.insert(relative.clone()) {
                    return Err(ArchiveError::Duplicate {
                        entry: display.clone(),
                    });
                }
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|source| ArchiveError::Write {
                        path: parent.display().to_string(),
                        source,
                    })?;
                }
                let mut file = File::create_new(&target).map_err(|source| ArchiveError::Write {
                    path: target.display().to_string(),
                    source,
                })?;
                io::copy(&mut entry, &mut file).map_err(|source| ArchiveError::Read {
                    entry: display.clone(),
                    source,
                })?;
                file.flush().map_err(|source| ArchiveError::Write {
                    path: target.display().to_string(),
                    source,
                })?;
                file.sync_all().map_err(|source| ArchiveError::Write {
                    path: target.display().to_string(),
                    source,
                })?;
            }
            EntryType::Directory => {
                let target = destination.join(&relative);
                enforce_within(&target, destination, &display)?;
                fs::create_dir_all(&target).map_err(|source| ArchiveError::Write {
                    path: target.display().to_string(),
                    source,
                })?;
            }
            EntryType::Symlink | EntryType::Link => {
                return Err(ArchiveError::UnsafeEntry {
                    entry: display,
                    reason: "symlinks and hard links are not permitted",
                });
            }
            EntryType::Char | EntryType::Block | EntryType::Fifo => {
                return Err(ArchiveError::UnsafeEntry {
                    entry: display,
                    reason: "device and FIFO entries are not permitted",
                });
            }
            EntryType::GNULongName
            | EntryType::GNULongLink
            | EntryType::GNUSparse
            | EntryType::XGlobalHeader
            | EntryType::XHeader => {
                // Extended headers describe the next entry — allow.
            }
            _ => {
                return Err(ArchiveError::UnsafeEntry {
                    entry: display,
                    reason: "unsupported tar entry type",
                });
            }
        }
    }
    Ok(())
}

// ─── zip ────────────────────────────────────────────────────────────────────

fn extract_zip(archive_path: &Path, destination: &Path) -> Result<(), ArchiveError> {
    let file = File::open(archive_path).map_err(|source| ArchiveError::Open {
        path: archive_path.display().to_string(),
        source,
    })?;
    let mut zip = ZipArchive::new(file).map_err(|source| ArchiveError::InvalidZip { source })?;
    let mut seen: std::collections::BTreeSet<PathBuf> = Default::default();
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|source| ArchiveError::InvalidZip { source })?;
        let raw_name = entry.name().to_string();
        // zip crate's `enclosed_name` rejects `..` and absolute; use it as a
        // first-line check but still enforce our own policy afterward.
        let raw_path = match entry.enclosed_name() {
            Some(p) => p,
            None => {
                return Err(ArchiveError::UnsafeEntry {
                    entry: raw_name,
                    reason: "path escapes the archive root",
                });
            }
        };
        let relative = check_relative_path(&raw_path, &raw_name)?;

        // Reject symlink zip entries (identified via the external attrs).
        if let Some(mode) = entry.unix_mode()
            && mode & 0o170000 == 0o120000
        {
            return Err(ArchiveError::UnsafeEntry {
                entry: raw_name,
                reason: "symlinks are not permitted",
            });
        }

        let target = destination.join(&relative);
        enforce_within(&target, destination, &raw_name)?;

        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|source| ArchiveError::Write {
                path: target.display().to_string(),
                source,
            })?;
            continue;
        }

        if !seen.insert(relative.clone()) {
            return Err(ArchiveError::Duplicate { entry: raw_name });
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|source| ArchiveError::Write {
                path: parent.display().to_string(),
                source,
            })?;
        }
        let mut file = File::create_new(&target).map_err(|source| ArchiveError::Write {
            path: target.display().to_string(),
            source,
        })?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = entry.read(&mut buf).map_err(|source| ArchiveError::Read {
                entry: raw_name.clone(),
                source,
            })?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .map_err(|source| ArchiveError::Write {
                    path: target.display().to_string(),
                    source,
                })?;
        }
        file.flush().map_err(|source| ArchiveError::Write {
            path: target.display().to_string(),
            source,
        })?;
        file.sync_all().map_err(|source| ArchiveError::Write {
            path: target.display().to_string(),
            source,
        })?;
    }
    Ok(())
}

// ─── shared safety helpers ──────────────────────────────────────────────────

fn check_relative_path(path: &Path, display: &str) -> Result<PathBuf, ArchiveError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(ArchiveError::UnsafeEntry {
                    entry: display.to_string(),
                    reason: "parent-directory traversal is not permitted",
                });
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ArchiveError::UnsafeEntry {
                    entry: display.to_string(),
                    reason: "absolute paths are not permitted",
                });
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(ArchiveError::UnsafeEntry {
            entry: display.to_string(),
            reason: "empty path",
        });
    }
    Ok(normalized)
}

fn enforce_within(target: &Path, destination: &Path, display: &str) -> Result<(), ArchiveError> {
    if !target.starts_with(destination) {
        return Err(ArchiveError::Escape {
            entry: display.to_string(),
            destination: destination.display().to_string(),
        });
    }
    Ok(())
}
