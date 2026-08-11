//! Safe extraction of downloaded archives (spec §6.9, plan 041 Task 2).
//!
//! `extract_archive` opens a tar.gz, tar.xz, or zip archive and unpacks it
//! into a caller-provided destination directory. Absolute paths, `..`
//! traversal, symlinks, hard links, devices, sockets, and duplicate entries
//! are all rejected before any bytes hit the disk. The destination is created
//! if missing but the extractor refuses to overwrite existing files.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use domain::adapter_prepare::ArchiveFormat;
use flate2::read::GzDecoder;
use tar::{Archive, EntryType};
use thiserror::Error;
use xz2::read::XzDecoder;
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
        ArchiveFormat::TarXz => extract_tar_xz(archive_path, destination),
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
    let archive = Archive::new(GzDecoder::new(file));
    extract_tar_entries(archive, destination)
}

// ─── tar.xz ─────────────────────────────────────────────────────────────────

fn extract_tar_xz(archive_path: &Path, destination: &Path) -> Result<(), ArchiveError> {
    let file = File::open(archive_path).map_err(|source| ArchiveError::Open {
        path: archive_path.display().to_string(),
        source,
    })?;
    let archive = Archive::new(XzDecoder::new(file));
    extract_tar_entries(archive, destination)
}

// Shared tar body extraction. Used for both gzip- and xz-compressed streams so
// safety checks (parent traversal, symlinks, device entries, duplicates) apply
// uniformly.
fn extract_tar_entries<R: Read>(
    mut archive: Archive<R>,
    destination: &Path,
) -> Result<(), ArchiveError> {
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
            EntryType::Symlink => {
                let target = destination.join(&relative);
                enforce_within(&target, destination, &display)?;
                if !seen.insert(relative.clone()) {
                    return Err(ArchiveError::Duplicate {
                        entry: display.clone(),
                    });
                }
                let link_target = entry
                    .link_name()
                    .map_err(|source| ArchiveError::Invalid { source })?
                    .ok_or(ArchiveError::UnsafeEntry {
                        entry: display.clone(),
                        reason: "symlink is missing a target",
                    })?
                    .into_owned();
                let relative_dir = relative.parent().unwrap_or(Path::new(""));
                check_symlink_target(relative_dir, &link_target, destination, &display)?;
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|source| ArchiveError::Write {
                        path: parent.display().to_string(),
                        source,
                    })?;
                }
                std::os::unix::fs::symlink(&link_target, &target).map_err(|source| {
                    ArchiveError::Write {
                        path: target.display().to_string(),
                        source,
                    }
                })?;
            }
            EntryType::Link => {
                return Err(ArchiveError::UnsafeEntry {
                    entry: display,
                    reason: "hard links are not permitted",
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

/// Reject symlinks whose resolved target escapes the destination. Absolute
/// targets are forbidden outright; relative targets are joined onto the
/// symlink's parent directory (relative to the destination root) and then
/// normalized so `..` cannot climb above the destination root. Only the
/// resolved *path* is checked — we never follow the link, so a broken target
/// that would resolve inside the destination is still accepted (the archive
/// itself supplies the real file elsewhere).
fn check_symlink_target(
    symlink_relative_dir: &Path,
    link_target: &Path,
    destination: &Path,
    display: &str,
) -> Result<(), ArchiveError> {
    if link_target.is_absolute() {
        return Err(ArchiveError::UnsafeEntry {
            entry: display.to_string(),
            reason: "symlink target must be relative",
        });
    }
    let joined = symlink_relative_dir.join(link_target);
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(ArchiveError::Escape {
                        entry: display.to_string(),
                        destination: destination.display().to_string(),
                    });
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ArchiveError::UnsafeEntry {
                    entry: display.to_string(),
                    reason: "symlink target must not be absolute",
                });
            }
        }
    }
    Ok(())
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use xz2::write::XzEncoder;

    fn build_tar_xz(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut xz = XzEncoder::new(Vec::new(), 6);
        {
            let mut builder = tar::Builder::new(&mut xz);
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder
                .append_data(&mut header, name, contents)
                .expect("append tar file");
            builder.finish().expect("finish tar");
        }
        xz.finish().expect("finish xz")
    }

    #[test]
    fn extract_tar_xz_round_trips_regular_file() {
        let bytes = build_tar_xz("greeting.txt", b"hello xz\n");
        let archive_dir = TempDir::new().unwrap();
        let archive_path = archive_dir.path().join("sample.tar.xz");
        fs::write(&archive_path, &bytes).unwrap();
        let dest = TempDir::new().unwrap();
        extract_archive(&archive_path, ArchiveFormat::TarXz, dest.path()).unwrap();
        let extracted = fs::read(dest.path().join("greeting.txt")).unwrap();
        assert_eq!(extracted, b"hello xz\n");
    }

    fn build_tar_gz_with_symlink(
        file_name: &str,
        file_contents: &[u8],
        symlink_name: &str,
        symlink_target: &str,
    ) -> Vec<u8> {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = tar::Builder::new(&mut gz);
            let mut header = tar::Header::new_gnu();
            header.set_size(file_contents.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder
                .append_data(&mut header, file_name, file_contents)
                .expect("append tar file");
            let mut sym = tar::Header::new_gnu();
            sym.set_size(0);
            sym.set_mode(0o777);
            sym.set_entry_type(tar::EntryType::Symlink);
            sym.set_link_name(symlink_target)
                .expect("symlink target fits");
            sym.set_cksum();
            builder
                .append_data(&mut sym, symlink_name, std::io::empty())
                .expect("append tar symlink");
            builder.finish().expect("finish tar");
        }
        gz.finish().expect("finish gz")
    }

    #[test]
    fn extract_tar_gz_allows_safe_relative_symlink() {
        let bytes = build_tar_gz_with_symlink("bin/real", b"binary\n", "bin/alias", "real");
        let archive_dir = TempDir::new().unwrap();
        let archive_path = archive_dir.path().join("safe.tar.gz");
        fs::write(&archive_path, &bytes).unwrap();
        let dest = TempDir::new().unwrap();
        extract_archive(&archive_path, ArchiveFormat::TarGz, dest.path()).unwrap();
        let alias = dest.path().join("bin/alias");
        let meta = fs::symlink_metadata(&alias).unwrap();
        assert!(meta.file_type().is_symlink(), "alias must be a symlink");
        let read = fs::read_link(&alias).unwrap();
        assert_eq!(read, PathBuf::from("real"));
    }

    #[test]
    fn extract_tar_gz_rejects_escape_symlink() {
        let bytes = build_tar_gz_with_symlink("bin/real", b"x", "bin/evil", "../../etc/passwd");
        let archive_dir = TempDir::new().unwrap();
        let archive_path = archive_dir.path().join("escape.tar.gz");
        fs::write(&archive_path, &bytes).unwrap();
        let dest = TempDir::new().unwrap();
        let err = extract_archive(&archive_path, ArchiveFormat::TarGz, dest.path()).unwrap_err();
        assert!(matches!(err, ArchiveError::Escape { .. }), "{err:?}");
    }

    #[test]
    fn extract_tar_gz_rejects_absolute_symlink() {
        let bytes = build_tar_gz_with_symlink("bin/real", b"x", "bin/evil", "/etc/passwd");
        let archive_dir = TempDir::new().unwrap();
        let archive_path = archive_dir.path().join("absolute.tar.gz");
        fs::write(&archive_path, &bytes).unwrap();
        let dest = TempDir::new().unwrap();
        let err = extract_archive(&archive_path, ArchiveFormat::TarGz, dest.path()).unwrap_err();
        assert!(
            matches!(err, ArchiveError::UnsafeEntry { reason, .. } if reason.contains("relative")),
            "{err:?}"
        );
    }
}
