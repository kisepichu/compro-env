//! Strict parsing and validation for adapter dependency preparation
//! (spec §6.9, plan 041 Task 1).
//!
//! `load_dependency_manifest` reads `tools/library-analyzers/dependencies.toml`
//! and rejects anything the specification forbids: HTTP/SSH/SCP-style URLs,
//! URL userinfo, mutable Git refs, non-hex digests, absolute or `..` local
//! paths, duplicate dependency names.
//!
//! `expected_dependency_id` recomputes a content-addressed identifier from the
//! normalized manifest plus target platform plus the actual content of any
//! declared local paths. This is deterministic and stable across process
//! restarts.
//!
//! `validate_prepared_set` reads `manifest.json` in a
//! `target/library-analyzers/prepared/<id>/` directory, checks the recorded
//! platform and id against the caller's expectations, and re-hashes every
//! declared artifact byte-for-byte before returning success.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use domain::adapter_build::{ContentDigest, ContentDigestError, TargetPlatform};
use domain::adapter_prepare::{
    ArchiveDependency, ArchiveFormat, DependencyId, DependencyManifest, ExpectedPreparedSet,
    GitDependency, LocalDependency, PreparedArtifact, PreparedArtifactKind, PreparedManifest,
    PreparedManifestError, PreparedSet, ToolchainPin, validate_prepared_manifest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use walkdir::WalkDir;

/// Repository-relative location of the dependency manifest.
pub const DEPENDENCY_MANIFEST_PATH: &str = "tools/library-analyzers/dependencies.toml";

/// File name of the prepared-set manifest inside `<prepared-root>/<id>/`.
pub const PREPARED_MANIFEST_FILE: &str = "manifest.json";

/// Domain separator woven into every dependency-id hash so unrelated hashes
/// cannot collide with this framing.
const DEPENDENCY_ID_DOMAIN: &[u8] = b"compro-env/adapter-prepare-id/v1\n";

// ─── Errors ─────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum PrepareError {
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
    #[error("failed to parse prepared manifest at {path}: {source}")]
    PreparedManifestParse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("path must be relative to the repository root: {path:?}")]
    NotRelative { path: String },
    #[error("path is not allowed: {path:?}")]
    InvalidPath { path: String },
    #[error("input {path:?} does not exist")]
    Missing { path: String },
    #[error("input {path:?} is a symlink")]
    Symlink { path: String },
    #[error("input directory {path:?} contains a symlink at {relative:?}")]
    SymlinkInside { path: String, relative: String },
    #[error("input {path:?} is expected to be a {expected} but is a {actual}")]
    WrongKind {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("dependency URL is not allowed ({reason}): {url:?}")]
    InvalidUrl { url: String, reason: &'static str },
    #[error("Git commit is not a full 40-character lowercase hex hash: {commit:?}")]
    InvalidGitCommit { commit: String },
    #[error("digest is not a valid content digest: {value:?} ({source})")]
    InvalidDigest {
        value: String,
        #[source]
        source: ContentDigestError,
    },
    #[error("unknown archive format: {value:?} (expected 'tar.gz' or 'zip')")]
    InvalidArchiveFormat { value: String },
    #[error("duplicate dependency name: {name:?}")]
    Duplicate { name: String },
    #[error("prepared manifest is missing: {path}")]
    PreparedManifestMissing { path: String },
    #[error("prepared directory is not a directory: {path}")]
    NotADirectory { path: String },
    #[error("prepared manifest mismatch: {source}")]
    PreparedManifestMismatch {
        #[source]
        source: PreparedManifestError,
    },
    #[error("prepared artifact {relative:?} is missing")]
    ArtifactMissing { relative: String },
    #[error("prepared artifact {relative:?} hash mismatch: expected {expected}, actual {actual}")]
    ArtifactHashMismatch {
        relative: String,
        expected: String,
        actual: String,
    },
    #[error("prepared artifact path escapes the prepared root: {relative:?}")]
    ArtifactPathEscape { relative: String },
}

// ─── TOML DTOs ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct DependencyManifestFile {
    #[serde(default)]
    archives: Vec<ArchiveDependencyFile>,
    #[serde(default)]
    git: Vec<GitDependencyFile>,
    #[serde(default)]
    locals: Vec<LocalDependencyFile>,
    #[serde(default)]
    toolchains: Vec<ToolchainPinFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveDependencyFile {
    name: String,
    url: String,
    sha256: String,
    format: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GitDependencyFile {
    name: String,
    url: String,
    commit: String,
    archive_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalDependencyFile {
    name: String,
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolchainPinFile {
    name: String,
    version: String,
    #[serde(default)]
    components: Vec<String>,
}

// ─── load_dependency_manifest ───────────────────────────────────────────────

pub fn load_dependency_manifest(root: &Path) -> Result<DependencyManifest, PrepareError> {
    let manifest_path = root.join(DEPENDENCY_MANIFEST_PATH);
    let contents = fs::read_to_string(&manifest_path).map_err(|source| PrepareError::Io {
        path: display_path(&manifest_path),
        source,
    })?;
    let parsed: DependencyManifestFile =
        toml::from_str(&contents).map_err(|source| PrepareError::Toml {
            path: display_path(&manifest_path),
            source,
        })?;

    let mut names: BTreeMap<String, &'static str> = BTreeMap::new();
    let mut record_name = |name: &str, kind: &'static str| -> Result<(), PrepareError> {
        if names.insert(name.to_string(), kind).is_some() {
            return Err(PrepareError::Duplicate {
                name: name.to_string(),
            });
        }
        Ok(())
    };

    let mut archives = Vec::with_capacity(parsed.archives.len());
    for a in parsed.archives {
        record_name(&a.name, "archive")?;
        validate_public_https(&a.url)?;
        let sha256 = parse_digest(&a.sha256)?;
        let format = parse_archive_format(&a.format)?;
        archives.push(ArchiveDependency {
            name: a.name,
            url: a.url,
            sha256,
            format,
        });
    }

    let mut git = Vec::with_capacity(parsed.git.len());
    for g in parsed.git {
        record_name(&g.name, "git")?;
        validate_public_https(&g.url)?;
        validate_git_commit(&g.commit)?;
        let archive_sha256 = parse_digest(&g.archive_sha256)?;
        git.push(GitDependency {
            name: g.name,
            url: g.url,
            commit: g.commit,
            archive_sha256,
        });
    }

    let mut locals = Vec::with_capacity(parsed.locals.len());
    for l in parsed.locals {
        record_name(&l.name, "local")?;
        validate_relative_path(&l.path)?;
        locals.push(LocalDependency {
            name: l.name,
            path: l.path,
        });
    }

    let mut toolchains = Vec::with_capacity(parsed.toolchains.len());
    for t in parsed.toolchains {
        record_name(&t.name, "toolchain")?;
        toolchains.push(ToolchainPin {
            name: t.name,
            version: t.version,
            components: t.components,
        });
    }

    Ok(DependencyManifest {
        archives,
        git,
        locals,
        toolchains,
    })
}

fn parse_digest(value: &str) -> Result<ContentDigest, PrepareError> {
    ContentDigest::from_hex(value).map_err(|source| PrepareError::InvalidDigest {
        value: value.to_string(),
        source,
    })
}

fn parse_archive_format(value: &str) -> Result<ArchiveFormat, PrepareError> {
    match value {
        "tar.gz" => Ok(ArchiveFormat::TarGz),
        "zip" => Ok(ArchiveFormat::Zip),
        _ => Err(PrepareError::InvalidArchiveFormat {
            value: value.to_string(),
        }),
    }
}

fn validate_git_commit(commit: &str) -> Result<(), PrepareError> {
    if commit.len() != 40
        || !commit
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    {
        return Err(PrepareError::InvalidGitCommit {
            commit: commit.to_string(),
        });
    }
    Ok(())
}

fn validate_public_https(url: &str) -> Result<(), PrepareError> {
    // Require an explicit scheme so SCP-style `git@host:path` is rejected here.
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(PrepareError::InvalidUrl {
            url: url.to_string(),
            reason: "missing scheme (use https://)",
        });
    };
    if scheme != "https" {
        return Err(PrepareError::InvalidUrl {
            url: url.to_string(),
            reason: "only https scheme is permitted",
        });
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() {
        return Err(PrepareError::InvalidUrl {
            url: url.to_string(),
            reason: "empty authority",
        });
    }
    if authority.contains('@') {
        return Err(PrepareError::InvalidUrl {
            url: url.to_string(),
            reason: "URL userinfo is not permitted",
        });
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), PrepareError> {
    if path.is_empty() {
        return Err(PrepareError::InvalidPath {
            path: path.to_string(),
        });
    }
    if path.starts_with('/') {
        return Err(PrepareError::NotRelative {
            path: path.to_string(),
        });
    }
    if path.contains('\\') || path.contains('\0') || path.contains(':') {
        return Err(PrepareError::InvalidPath {
            path: path.to_string(),
        });
    }
    for segment in path.split('/') {
        match segment {
            "" | "." | ".." => {
                return Err(PrepareError::InvalidPath {
                    path: path.to_string(),
                });
            }
            _ => {}
        }
    }
    Ok(())
}

// ─── expected_dependency_id ─────────────────────────────────────────────────

pub fn expected_dependency_id(
    root: &Path,
    manifest: &DependencyManifest,
    platform: &TargetPlatform,
) -> Result<DependencyId, PrepareError> {
    // Canonicalize root once so local paths cannot escape via parent symlinks.
    let root_canonical = root.canonicalize().map_err(|source| PrepareError::Io {
        path: display_path(root),
        source,
    })?;

    let mut hasher = Sha256::new();
    hasher.update(DEPENDENCY_ID_DOMAIN);
    write_framed(&mut hasher, platform.os.as_bytes());
    write_framed(&mut hasher, platform.arch.as_bytes());

    // Archives: sorted by name for stability.
    let mut archives = manifest.archives.clone();
    archives.sort_by(|a, b| a.name.cmp(&b.name));
    write_framed(&mut hasher, b"archives");
    for a in &archives {
        write_framed(&mut hasher, a.name.as_bytes());
        write_framed(&mut hasher, a.url.as_bytes());
        write_framed(&mut hasher, a.sha256.as_str().as_bytes());
        write_framed(&mut hasher, a.format.as_str().as_bytes());
    }

    // Git: sorted by name for stability.
    let mut git = manifest.git.clone();
    git.sort_by(|a, b| a.name.cmp(&b.name));
    write_framed(&mut hasher, b"git");
    for g in &git {
        write_framed(&mut hasher, g.name.as_bytes());
        write_framed(&mut hasher, g.url.as_bytes());
        write_framed(&mut hasher, g.commit.as_bytes());
        write_framed(&mut hasher, g.archive_sha256.as_str().as_bytes());
    }

    // Locals: sorted by name for stability, then hash content.
    let mut locals = manifest.locals.clone();
    locals.sort_by(|a, b| a.name.cmp(&b.name));
    write_framed(&mut hasher, b"locals");
    for l in &locals {
        write_framed(&mut hasher, l.name.as_bytes());
        write_framed(&mut hasher, l.path.as_bytes());
        hash_local_content(&root_canonical, &l.path, &mut hasher)?;
    }

    // Toolchains: sorted by name for stability.
    let mut toolchains = manifest.toolchains.clone();
    toolchains.sort_by(|a, b| a.name.cmp(&b.name));
    write_framed(&mut hasher, b"toolchains");
    for t in &toolchains {
        write_framed(&mut hasher, t.name.as_bytes());
        write_framed(&mut hasher, t.version.as_bytes());
        let mut components = t.components.clone();
        components.sort();
        write_framed(&mut hasher, format!("{}", components.len()).as_bytes());
        for c in components {
            write_framed(&mut hasher, c.as_bytes());
        }
    }

    let out: [u8; 32] = hasher.finalize().into();
    Ok(DependencyId::new(ContentDigest::from_sha256_bytes(out)))
}

fn hash_local_content(root: &Path, rel: &str, hasher: &mut Sha256) -> Result<(), PrepareError> {
    let absolute = root.join(rel);
    let meta = match fs::symlink_metadata(&absolute) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(PrepareError::Missing {
                path: rel.to_string(),
            });
        }
        Err(source) => {
            return Err(PrepareError::Io {
                path: display_path(&absolute),
                source,
            });
        }
    };
    if meta.file_type().is_symlink() {
        return Err(PrepareError::Symlink {
            path: rel.to_string(),
        });
    }
    if meta.file_type().is_file() {
        let bytes = fs::read(&absolute).map_err(|source| PrepareError::Io {
            path: display_path(&absolute),
            source,
        })?;
        write_framed(hasher, b"file");
        write_framed(hasher, rel.as_bytes());
        write_framed(hasher, &bytes);
        return Ok(());
    }
    if !meta.file_type().is_dir() {
        return Err(PrepareError::WrongKind {
            path: rel.to_string(),
            expected: "file or directory".into(),
            actual: describe_type(meta.file_type()),
        });
    }
    let mut entries: BTreeMap<String, PathBuf> = BTreeMap::new();
    for dirent in WalkDir::new(&absolute)
        .follow_links(false)
        .sort_by_file_name()
    {
        let dirent = dirent.map_err(|e| {
            let path = e
                .path()
                .map(display_path)
                .unwrap_or_else(|| display_path(&absolute));
            let source = e
                .into_io_error()
                .unwrap_or_else(|| std::io::Error::other("walkdir failure"));
            PrepareError::Io { path, source }
        })?;
        let file_type = dirent.file_type();
        if file_type.is_symlink() {
            let relative = dirent
                .path()
                .strip_prefix(root)
                .map(display_path)
                .unwrap_or_else(|_| display_path(dirent.path()));
            return Err(PrepareError::SymlinkInside {
                path: rel.to_string(),
                relative,
            });
        }
        if !file_type.is_file() {
            continue;
        }
        let rel_path = dirent
            .path()
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .expect("walked path stays under the repository root");
        let rel_str = rel_path
            .to_str()
            .ok_or_else(|| PrepareError::InvalidPath {
                path: display_path(&rel_path),
            })?
            .replace('\\', "/");
        entries.insert(rel_str, dirent.path().to_path_buf());
    }
    write_framed(hasher, b"dir");
    write_framed(hasher, rel.as_bytes());
    write_framed(hasher, format!("{}", entries.len()).as_bytes());
    for (rel_str, absolute) in entries {
        let bytes = fs::read(&absolute).map_err(|source| PrepareError::Io {
            path: display_path(&absolute),
            source,
        })?;
        write_framed(hasher, rel_str.as_bytes());
        write_framed(hasher, &bytes);
    }
    Ok(())
}

fn write_framed(hasher: &mut Sha256, data: &[u8]) {
    let len = data.len() as u64;
    hasher.update(len.to_be_bytes());
    hasher.update(data);
}

// ─── validate_prepared_set ──────────────────────────────────────────────────

pub fn validate_prepared_set(
    path: &Path,
    expected: &ExpectedPreparedSet,
) -> Result<PreparedSet, PrepareError> {
    let meta = fs::symlink_metadata(path).map_err(|source| PrepareError::Io {
        path: display_path(path),
        source,
    })?;
    if !meta.file_type().is_dir() {
        return Err(PrepareError::NotADirectory {
            path: display_path(path),
        });
    }
    let manifest_path = path.join(PREPARED_MANIFEST_FILE);
    let contents = match fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(PrepareError::PreparedManifestMissing {
                path: display_path(&manifest_path),
            });
        }
        Err(source) => {
            return Err(PrepareError::Io {
                path: display_path(&manifest_path),
                source,
            });
        }
    };
    let dto: PreparedManifestDto =
        serde_json::from_str(&contents).map_err(|source| PrepareError::PreparedManifestParse {
            path: display_path(&manifest_path),
            source,
        })?;
    let manifest = dto.into_domain()?;

    validate_prepared_manifest(expected, &manifest)
        .map_err(|source| PrepareError::PreparedManifestMismatch { source })?;

    // Byte-verify every artifact in a stable order.
    for artifact in &manifest.artifacts {
        validate_relative_path(&artifact.relative_path)?;
        let absolute = path.join(&artifact.relative_path);
        let canonical = absolute.canonicalize().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PrepareError::ArtifactMissing {
                    relative: artifact.relative_path.clone(),
                }
            } else {
                PrepareError::Io {
                    path: display_path(&absolute),
                    source: e,
                }
            }
        })?;
        let root_canonical = path.canonicalize().map_err(|source| PrepareError::Io {
            path: display_path(path),
            source,
        })?;
        if !canonical.starts_with(&root_canonical) {
            return Err(PrepareError::ArtifactPathEscape {
                relative: artifact.relative_path.clone(),
            });
        }
        let meta = fs::symlink_metadata(&absolute).map_err(|source| PrepareError::Io {
            path: display_path(&absolute),
            source,
        })?;
        if meta.file_type().is_symlink() {
            return Err(PrepareError::Symlink {
                path: artifact.relative_path.clone(),
            });
        }
        if !meta.file_type().is_file() {
            return Err(PrepareError::WrongKind {
                path: artifact.relative_path.clone(),
                expected: "file".into(),
                actual: describe_type(meta.file_type()),
            });
        }
        let bytes = fs::read(&absolute).map_err(|source| PrepareError::Io {
            path: display_path(&absolute),
            source,
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let out: [u8; 32] = hasher.finalize().into();
        let actual = ContentDigest::from_sha256_bytes(out);
        if actual != artifact.sha256 {
            return Err(PrepareError::ArtifactHashMismatch {
                relative: artifact.relative_path.clone(),
                expected: artifact.sha256.to_string(),
                actual: actual.to_string(),
            });
        }
    }

    Ok(PreparedSet {
        id: manifest.id.clone(),
        root: path.to_path_buf(),
        manifest,
    })
}

// ─── JSON DTO ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedManifestDto {
    id: String,
    target_platform: TargetPlatformDto,
    artifacts: Vec<PreparedArtifactDto>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetPlatformDto {
    os: String,
    arch: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedArtifactDto {
    name: String,
    kind: String,
    relative_path: String,
    sha256: String,
}

impl PreparedManifestDto {
    fn from_domain(manifest: &PreparedManifest) -> Self {
        Self {
            id: manifest.id.to_string(),
            target_platform: TargetPlatformDto {
                os: manifest.target_platform.os.clone(),
                arch: manifest.target_platform.arch.clone(),
            },
            artifacts: manifest
                .artifacts
                .iter()
                .map(|a| PreparedArtifactDto {
                    name: a.name.clone(),
                    kind: a.kind.to_string(),
                    relative_path: a.relative_path.clone(),
                    sha256: a.sha256.to_string(),
                })
                .collect(),
        }
    }

    fn into_domain(self) -> Result<PreparedManifest, PrepareError> {
        let id =
            ContentDigest::from_hex(&self.id).map_err(|source| PrepareError::InvalidDigest {
                value: self.id.clone(),
                source,
            })?;
        let mut artifacts = Vec::with_capacity(self.artifacts.len());
        for a in self.artifacts {
            let sha256 = ContentDigest::from_hex(&a.sha256).map_err(|source| {
                PrepareError::InvalidDigest {
                    value: a.sha256.clone(),
                    source,
                }
            })?;
            let kind = match a.kind.as_str() {
                "archive" => PreparedArtifactKind::Archive,
                "git" => PreparedArtifactKind::Git,
                "local" => PreparedArtifactKind::Local,
                "toolchain" => PreparedArtifactKind::Toolchain,
                _ => {
                    return Err(PrepareError::InvalidArchiveFormat { value: a.kind });
                }
            };
            artifacts.push(PreparedArtifact {
                name: a.name,
                kind,
                relative_path: a.relative_path,
                sha256,
            });
        }
        Ok(PreparedManifest {
            id: DependencyId::new(id),
            target_platform: TargetPlatform {
                os: self.target_platform.os,
                arch: self.target_platform.arch,
            },
            artifacts,
        })
    }
}

/// Serialize a `PreparedManifest` to canonical JSON (stable field order, no
/// trailing newline). Used by both `prepare` and the tests.
pub fn write_prepared_manifest_json(manifest: &PreparedManifest) -> String {
    let dto = PreparedManifestDto::from_domain(manifest);
    serde_json::to_string_pretty(&dto).expect("prepared manifest is always serializable")
}

// ─── helpers ────────────────────────────────────────────────────────────────

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
