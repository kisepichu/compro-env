//! Safe extraction of downloaded archives (spec §6.9, plan 041 Task 2).
//!
//! `extract_archive` opens a tar.gz or zip archive and unpacks it into a
//! staging directory that stays entirely inside the caller-provided root.
//! Absolute paths, `..` traversal, symlinks, hard links, devices, sockets,
//! and duplicate entries are all rejected before any bytes hit the disk.
//!
//! Extraction is filled in as part of Task 2; Task 1 only wires up the
//! module so the shared error type has a home.

use std::path::{Path, PathBuf};

use domain::adapter_prepare::ArchiveFormat;
use thiserror::Error;

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
    #[error("archive format {format:?} is not supported by extractor")]
    UnsupportedFormat { format: String },
}

/// Placeholder extraction entry point.
///
/// Task 2 will fill this in with tar.gz + zip readers and safety checks.
#[allow(dead_code)]
pub fn extract_archive(
    _archive_path: &Path,
    _format: ArchiveFormat,
    _destination: &Path,
) -> Result<PathBuf, ArchiveError> {
    Err(ArchiveError::UnsupportedFormat {
        format: "extraction is implemented in plan 041 Task 2".into(),
    })
}
