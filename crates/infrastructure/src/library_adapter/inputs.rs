//! Deterministic input digest for adapter build sets (spec §6.9).
//!
//! `load_build_inputs` reads the strict TOML declaration and rejects any path
//! that is not repository-relative, duplicated, or that overlaps another
//! declared directory. `calculate_input_digest` walks the declared inputs,
//! rejects symlinks and out-of-tree resolutions, and produces a SHA-256 digest
//! framed with the target platform so different platforms cannot share a
//! `build-id`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use domain::adapter_build::{
    BuildInputEntry, BuildInputKind, BuildInputs, ContentDigest, TargetPlatform,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use walkdir::WalkDir;

/// Repository-relative location of the input declaration.
pub const BUILD_INPUTS_CONFIG_PATH: &str = "tools/library-analyzers/build-inputs.toml";

/// Domain separator woven into every digest so unrelated hashes cannot collide
/// with this framing.
const DOMAIN_SEPARATOR: &[u8] = b"compro-env/adapter-build-inputs/v1\n";

#[derive(Debug, Error)]
pub enum BuildInputError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Toml {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("input path must be relative to the repository root: {path:?}")]
    NotRelative { path: String },
    #[error("input path is not allowed: {path:?}")]
    InvalidPath { path: String },
    #[error("input {path:?} escapes the repository root")]
    RepositoryEscape { path: String },
    #[error("input {path:?} is a symlink; symlinks are not allowed as build inputs")]
    Symlink { path: String },
    #[error("input directory {path:?} contains a symlink at {relative:?}")]
    SymlinkInside { path: String, relative: String },
    #[error("input {path:?} does not exist")]
    Missing { path: String },
    #[error("input {path:?} is expected to be a {expected} but is a {actual}")]
    WrongKind {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("duplicate input path: {path:?}")]
    Duplicate { path: String },
    #[error("input directories overlap: {outer:?} contains {inner:?}")]
    Overlap { outer: String, inner: String },
    #[error("repository root {root:?} is not a directory")]
    RootNotDirectory { root: String },
}

// ─── TOML parsing ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildInputsFile {
    #[serde(default)]
    directories: Vec<String>,
    #[serde(default)]
    files: Vec<String>,
}

/// Parse `tools/library-analyzers/build-inputs.toml` under `root`.
///
/// The declaration is validated in isolation from the filesystem: paths must be
/// repository-relative, syntactically safe, unique across `directories` and
/// `files`, and free of overlapping directory ancestry.
pub fn load_build_inputs(root: &Path) -> Result<BuildInputs, BuildInputError> {
    let config_path = root.join(BUILD_INPUTS_CONFIG_PATH);
    let contents = fs::read_to_string(&config_path).map_err(|source| BuildInputError::Io {
        path: display_path(&config_path),
        source,
    })?;
    let parsed: BuildInputsFile =
        toml::from_str(&contents).map_err(|source| BuildInputError::Toml {
            path: display_path(&config_path),
            source,
        })?;

    let mut entries: Vec<BuildInputEntry> = Vec::new();
    let mut seen: BTreeMap<String, BuildInputKind> = BTreeMap::new();
    for path in parsed.directories {
        validate_toml_path(&path)?;
        if seen
            .insert(path.clone(), BuildInputKind::Directory)
            .is_some()
        {
            return Err(BuildInputError::Duplicate { path });
        }
        entries.push(BuildInputEntry {
            kind: BuildInputKind::Directory,
            path,
        });
    }
    for path in parsed.files {
        validate_toml_path(&path)?;
        if seen.insert(path.clone(), BuildInputKind::File).is_some() {
            return Err(BuildInputError::Duplicate { path });
        }
        entries.push(BuildInputEntry {
            kind: BuildInputKind::File,
            path,
        });
    }

    check_overlaps(&entries)?;

    Ok(BuildInputs { entries })
}

fn validate_toml_path(path: &str) -> Result<(), BuildInputError> {
    if path.is_empty() {
        return Err(BuildInputError::InvalidPath {
            path: path.to_string(),
        });
    }
    if path.starts_with('/') {
        return Err(BuildInputError::NotRelative {
            path: path.to_string(),
        });
    }
    if path.contains('\\') || path.contains('\0') || path.contains(':') {
        return Err(BuildInputError::InvalidPath {
            path: path.to_string(),
        });
    }
    for segment in path.split('/') {
        match segment {
            "" | "." | ".." => {
                return Err(BuildInputError::InvalidPath {
                    path: path.to_string(),
                });
            }
            _ => {}
        }
    }
    Ok(())
}

fn check_overlaps(entries: &[BuildInputEntry]) -> Result<(), BuildInputError> {
    let dirs: Vec<&str> = entries
        .iter()
        .filter(|e| e.kind == BuildInputKind::Directory)
        .map(|e| e.path.as_str())
        .collect();
    for outer in &dirs {
        let outer_prefix = format!("{outer}/");
        for entry in entries {
            if entry.path == *outer {
                continue;
            }
            if entry.path.starts_with(&outer_prefix) {
                return Err(BuildInputError::Overlap {
                    outer: outer.to_string(),
                    inner: entry.path.clone(),
                });
            }
        }
    }
    Ok(())
}

// ─── Digest ──────────────────────────────────────────────────────────────────

/// Compute the SHA-256 input digest for the declared build inputs.
///
/// Files are enumerated recursively for directory entries, sorted by their
/// repository-relative UTF-8 byte path, and framed with 64-bit big-endian
/// lengths so no distinct input tuple can produce the same byte stream.
pub fn calculate_input_digest(
    root: &Path,
    inputs: &BuildInputs,
    platform: &TargetPlatform,
) -> Result<ContentDigest, BuildInputError> {
    let root_canonical = root.canonicalize().map_err(|source| BuildInputError::Io {
        path: display_path(root),
        source,
    })?;
    let root_meta =
        fs::symlink_metadata(&root_canonical).map_err(|source| BuildInputError::Io {
            path: display_path(&root_canonical),
            source,
        })?;
    if !root_meta.file_type().is_dir() {
        return Err(BuildInputError::RootNotDirectory {
            root: display_path(&root_canonical),
        });
    }

    let mut collected: BTreeMap<String, PathBuf> = BTreeMap::new();
    for entry in &inputs.entries {
        collect_entry(&root_canonical, entry, &mut collected)?;
    }

    let mut hasher = Sha256::new();
    hasher.update(DOMAIN_SEPARATOR);
    write_framed(&mut hasher, platform.os.as_bytes());
    write_framed(&mut hasher, platform.arch.as_bytes());
    for (rel_path, absolute) in &collected {
        let bytes = fs::read(absolute).map_err(|source| BuildInputError::Io {
            path: display_path(absolute),
            source,
        })?;
        write_framed(&mut hasher, rel_path.as_bytes());
        write_framed(&mut hasher, &bytes);
    }
    let out: [u8; 32] = hasher.finalize().into();
    Ok(ContentDigest::from_sha256_bytes(out))
}

fn write_framed(hasher: &mut Sha256, data: &[u8]) {
    let len = data.len() as u64;
    hasher.update(len.to_be_bytes());
    hasher.update(data);
}

fn collect_entry(
    root: &Path,
    entry: &BuildInputEntry,
    into: &mut BTreeMap<String, PathBuf>,
) -> Result<(), BuildInputError> {
    let absolute = root.join(&entry.path);
    let meta = match fs::symlink_metadata(&absolute) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(BuildInputError::Missing {
                path: entry.path.clone(),
            });
        }
        Err(source) => {
            return Err(BuildInputError::Io {
                path: display_path(&absolute),
                source,
            });
        }
    };
    if meta.file_type().is_symlink() {
        return Err(BuildInputError::Symlink {
            path: entry.path.clone(),
        });
    }

    let canonical = absolute
        .canonicalize()
        .map_err(|source| BuildInputError::Io {
            path: display_path(&absolute),
            source,
        })?;
    let expected_canonical = root.join(&entry.path);
    if canonical != expected_canonical {
        // A parent segment resolved through a symlink, or the entry escaped
        // to a location outside its declared path.
        if !canonical.starts_with(root) {
            return Err(BuildInputError::RepositoryEscape {
                path: entry.path.clone(),
            });
        }
        return Err(BuildInputError::Symlink {
            path: entry.path.clone(),
        });
    }

    match entry.kind {
        BuildInputKind::File => {
            if !meta.file_type().is_file() {
                return Err(BuildInputError::WrongKind {
                    path: entry.path.clone(),
                    expected: BuildInputKind::File.to_string(),
                    actual: describe_type(meta.file_type()),
                });
            }
            insert_unique(into, entry.path.clone(), canonical)?;
        }
        BuildInputKind::Directory => {
            if !meta.file_type().is_dir() {
                return Err(BuildInputError::WrongKind {
                    path: entry.path.clone(),
                    expected: BuildInputKind::Directory.to_string(),
                    actual: describe_type(meta.file_type()),
                });
            }
            walk_directory(root, entry, &canonical, into)?;
        }
    }
    Ok(())
}

fn walk_directory(
    root: &Path,
    entry: &BuildInputEntry,
    absolute: &Path,
    into: &mut BTreeMap<String, PathBuf>,
) -> Result<(), BuildInputError> {
    for dirent in WalkDir::new(absolute)
        .follow_links(false)
        .sort_by_file_name()
    {
        let dirent = dirent.map_err(|e| {
            let path = e
                .path()
                .map(display_path)
                .unwrap_or_else(|| display_path(absolute));
            let source = e
                .into_io_error()
                .unwrap_or_else(|| std::io::Error::other("walkdir failure"));
            BuildInputError::Io { path, source }
        })?;
        let file_type = dirent.file_type();
        if file_type.is_symlink() {
            let relative = dirent
                .path()
                .strip_prefix(root)
                .map(display_path)
                .unwrap_or_else(|_| display_path(dirent.path()));
            return Err(BuildInputError::SymlinkInside {
                path: entry.path.clone(),
                relative,
            });
        }
        if !file_type.is_file() {
            continue;
        }
        let rel = dirent
            .path()
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .expect("walked path stays under the canonical repository root");
        let rel_str = rel
            .to_str()
            .ok_or_else(|| BuildInputError::InvalidPath {
                path: display_path(&rel),
            })?
            .replace('\\', "/");
        insert_unique(into, rel_str, dirent.path().to_path_buf())?;
    }
    Ok(())
}

fn insert_unique(
    into: &mut BTreeMap<String, PathBuf>,
    rel_path: String,
    absolute: PathBuf,
) -> Result<(), BuildInputError> {
    if into.insert(rel_path.clone(), absolute).is_some() {
        return Err(BuildInputError::Duplicate { path: rel_path });
    }
    Ok(())
}

fn describe_type(t: std::fs::FileType) -> String {
    if t.is_file() {
        "file".into()
    } else if t.is_dir() {
        "directory".into()
    } else if t.is_symlink() {
        "symlink".into()
    } else {
        "other".into()
    }
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}
