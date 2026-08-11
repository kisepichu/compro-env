//! Filesystem-backed [`VerificationRepository`] (spec §11).
//!
//! Records live at `verification/results/{contest}/{problem}/{solution}.json`
//! under a caller-supplied root. Writes are atomic (temporary file in the same
//! directory, `fsync` on both the file and its parent) and gated by a
//! compare-and-swap on the prior `attempt_id`. The results tree tolerates only
//! canonical layouts: non-JSON entries, symlinks anywhere under the tree, and
//! records for undiscovered solutions all raise hard errors.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use domain::library::SolutionId;
use domain::verification::{AttemptId, VerificationRecord};
use thiserror::Error;
use usecases::repository::verification_repository::VerificationRepository;
use walkdir::WalkDir;

const RESULTS_DIRNAME: &str = "verification";
const RESULTS_SUBDIR: &str = "results";
const JSON_EXT: &str = "json";

/// Downcastable errors raised by [`VerificationRepositoryImpl`]. Returned via
/// `anyhow::Error` so call sites can decide whether to match on a variant or
/// forward the message.
#[derive(Debug, Error)]
pub enum VerificationRepositoryError {
    /// CAS with `expected == None` but a record already exists.
    #[error("verification record already exists for {id}")]
    AlreadyExists { id: SolutionId },

    /// CAS with `expected == Some(a)` but the stored record has a different
    /// `attempt_id`, or `remove_if_attempt` disagrees with the stored attempt.
    #[error("verification record for {id} has attempt {current}, not the expected attempt")]
    ConflictingAttempt { id: SolutionId, current: AttemptId },

    /// `next.replaces_attempt_id` disagrees with `expected`.
    #[error(
        "verification record for {id}: expected replaces_attempt_id {expected:?}, got {actual:?}"
    )]
    InconsistentReplacement {
        id: SolutionId,
        expected: Option<AttemptId>,
        actual: Option<AttemptId>,
    },

    /// `remove_if_attempt` on a missing record, or CAS with a specific
    /// expected attempt but no stored record.
    #[error("verification record not found: {id}")]
    NotFound { id: SolutionId },

    /// Stored `solution_id` disagrees with the record's on-disk path.
    #[error("verification record at {path} declares solution_id {stored}")]
    PathMismatch { path: String, stored: SolutionId },

    /// A file whose extension is not `.json` was found inside the tree.
    #[error("non-JSON entry in verification results tree: {path}")]
    NonJsonEntry { path: String },

    /// A symlink was encountered inside the results tree.
    #[error("symlink not allowed in verification results tree: {path}")]
    SymlinkNotAllowed { path: String },

    /// A stored record's `solution_id` is not in the discovered set.
    #[error("verification record for undiscovered solution: {id}")]
    OrphanRecord { id: SolutionId },

    /// A path inside the tree does not fit the canonical
    /// `{contest}/{problem}/{solution}.json` shape.
    #[error("unexpected path in verification results tree: {path}")]
    UnexpectedPath { path: String },
}

/// Filesystem-backed implementation. `root` is the repository root; the
/// results tree lives at `root/verification/results/`.
pub struct VerificationRepositoryImpl {
    root: PathBuf,
}

impl VerificationRepositoryImpl {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn results_root(&self) -> PathBuf {
        self.root.join(RESULTS_DIRNAME).join(RESULTS_SUBDIR)
    }

    /// Canonical on-disk location for a solution's latest record.
    fn record_path(&self, id: &SolutionId) -> PathBuf {
        self.results_root()
            .join(id.contest_id())
            .join(id.problem_code())
            .join(format!("{}.json", id.solution_name()))
    }

    /// Rejects any existing ancestor of `path` (up to and including the
    /// configured `root`) that is a symlink. Missing ancestors are tolerated;
    /// the caller will create them. Walking up to `root` (not just
    /// `results_root`) closes the hole where `root/verification` itself is a
    /// symlink pointing outside the caller's tree.
    fn reject_symlinked_ancestors(&self, path: &Path) -> Result<()> {
        let root = self.root.as_path();
        let mut chain: Vec<PathBuf> = Vec::new();
        let mut cur: Option<&Path> = Some(path);
        while let Some(p) = cur {
            chain.push(p.to_path_buf());
            if p == root {
                break;
            }
            cur = p.parent();
        }
        // Walk top-down so the earliest offender wins.
        for ancestor in chain.iter().rev() {
            match std::fs::symlink_metadata(ancestor) {
                Ok(md) if md.file_type().is_symlink() => {
                    return Err(VerificationRepositoryError::SymlinkNotAllowed {
                        path: ancestor.display().to_string(),
                    }
                    .into());
                }
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("failed to stat {}", ancestor.display()));
                }
            }
        }
        Ok(())
    }

    /// Reads a record if present. `Ok(None)` for a missing file, `Err` for a
    /// symlink, unreadable file, or corrupt JSON. Does not validate the
    /// record's stored `solution_id`; call [`Self::read_record_for`] when the
    /// expected id is known so a mismatched id surfaces as [`VerificationRepositoryError::PathMismatch`].
    fn read_record(&self, path: &Path) -> Result<Option<VerificationRecord>> {
        match std::fs::symlink_metadata(path) {
            Ok(md) => {
                if md.file_type().is_symlink() {
                    return Err(VerificationRepositoryError::SymlinkNotAllowed {
                        path: path.display().to_string(),
                    }
                    .into());
                }
                let contents = std::fs::read_to_string(path)
                    .with_context(|| format!("failed to read record: {}", path.display()))?;
                let record: VerificationRecord = serde_json::from_str(&contents)
                    .with_context(|| format!("failed to parse record: {}", path.display()))?;
                Ok(Some(record))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("failed to stat {}", path.display())),
        }
    }

    /// Reads a record and rejects it when the stored `solution_id` disagrees
    /// with `expected`. Guards single-id call sites (`load`, `compare_and_swap`,
    /// `remove_if_attempt`) against corrupted or tampered files whose contents
    /// no longer belong at their on-disk location.
    fn read_record_for(
        &self,
        expected: &SolutionId,
        path: &Path,
    ) -> Result<Option<VerificationRecord>> {
        let Some(record) = self.read_record(path)? else {
            return Ok(None);
        };
        if record.solution_id != *expected {
            return Err(VerificationRepositoryError::PathMismatch {
                path: path.display().to_string(),
                stored: record.solution_id,
            }
            .into());
        }
        Ok(Some(record))
    }

    /// Serializes `record` and writes it atomically to `path`. Fsyncs both the
    /// temporary file and the containing directory so the rename survives a
    /// crash.
    fn write_atomic(&self, path: &Path, record: &VerificationRecord) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("record path has no parent: {}", path.display()))?;
        self.reject_symlinked_ancestors(parent)?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent dir: {}", parent.display()))?;

        let contents = serde_json::to_vec_pretty(record)
            .with_context(|| "failed to serialize verification record")?;

        let mut tmp = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("failed to create temp file in {}", parent.display()))?;
        tmp.as_file_mut()
            .write_all(&contents)
            .with_context(|| "failed to write verification temp file")?;
        tmp.as_file_mut()
            .sync_all()
            .with_context(|| "failed to fsync verification temp file")?;
        tmp.persist(path).map_err(|e| {
            anyhow!(
                "failed to persist verification record over {}: {}",
                path.display(),
                e
            )
        })?;

        // fsync the containing directory so the rename hits disk (Linux).
        let dir = File::open(parent)
            .with_context(|| format!("failed to open dir for fsync: {}", parent.display()))?;
        dir.sync_all()
            .with_context(|| format!("failed to fsync dir: {}", parent.display()))?;
        Ok(())
    }
}

impl VerificationRepository for VerificationRepositoryImpl {
    fn load(&self, id: &SolutionId) -> Result<Option<VerificationRecord>> {
        let path = self.record_path(id);
        self.reject_symlinked_ancestors(&path)?;
        self.read_record_for(id, &path)
    }

    fn load_all(
        &self,
        discovered: &BTreeSet<SolutionId>,
    ) -> Result<BTreeMap<SolutionId, VerificationRecord>> {
        let results_root = self.results_root();

        // Reject symlinks anywhere between `self.root` and `results_root` before
        // testing existence — `Path::exists` follows symlinks, so a symlinked
        // `verification/` dir pointing outside the tree would otherwise appear
        // empty here even when it is not.
        self.reject_symlinked_ancestors(&results_root)?;
        if !results_root.exists() {
            return Ok(BTreeMap::new());
        }

        let mut out = BTreeMap::new();

        for entry in WalkDir::new(&results_root)
            .follow_links(false)
            .sort_by(|a, b| a.file_name().cmp(b.file_name()))
        {
            let entry =
                entry.with_context(|| format!("walk failed under {}", results_root.display()))?;
            if entry.path() == results_root {
                continue;
            }
            let file_type = entry.file_type();
            if file_type.is_symlink() {
                return Err(VerificationRepositoryError::SymlinkNotAllowed {
                    path: entry.path().display().to_string(),
                }
                .into());
            }
            if file_type.is_dir() {
                continue;
            }
            if !file_type.is_file() {
                return Err(VerificationRepositoryError::UnexpectedPath {
                    path: entry.path().display().to_string(),
                }
                .into());
            }

            let rel = entry
                .path()
                .strip_prefix(&results_root)
                .with_context(|| "failed to strip results_root prefix")?;
            let components: Vec<&std::ffi::OsStr> =
                rel.components().map(|c| c.as_os_str()).collect();
            if components.len() != 3 {
                return Err(VerificationRepositoryError::UnexpectedPath {
                    path: entry.path().display().to_string(),
                }
                .into());
            }

            let ext = entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if ext != JSON_EXT {
                return Err(VerificationRepositoryError::NonJsonEntry {
                    path: entry.path().display().to_string(),
                }
                .into());
            }

            let contest = components[0].to_str().ok_or_else(|| {
                anyhow::Error::from(VerificationRepositoryError::UnexpectedPath {
                    path: entry.path().display().to_string(),
                })
            })?;
            let problem = components[1].to_str().ok_or_else(|| {
                anyhow::Error::from(VerificationRepositoryError::UnexpectedPath {
                    path: entry.path().display().to_string(),
                })
            })?;
            let solution_name = entry
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| {
                    anyhow::Error::from(VerificationRepositoryError::UnexpectedPath {
                        path: entry.path().display().to_string(),
                    })
                })?;

            let id_from_path = SolutionId::parse(&format!("{contest}/{problem}/{solution_name}"))
                .map_err(|e| {
                anyhow::Error::from(VerificationRepositoryError::UnexpectedPath {
                    path: entry.path().display().to_string(),
                })
                .context(format!("invalid solution id from path: {e}"))
            })?;

            let record = self.read_record(entry.path())?.ok_or_else(|| {
                anyhow!("record disappeared during walk: {}", entry.path().display())
            })?;

            if record.solution_id != id_from_path {
                return Err(VerificationRepositoryError::PathMismatch {
                    path: entry.path().display().to_string(),
                    stored: record.solution_id.clone(),
                }
                .into());
            }
            if !discovered.contains(&id_from_path) {
                return Err(VerificationRepositoryError::OrphanRecord { id: id_from_path }.into());
            }

            out.insert(id_from_path, record);
        }

        Ok(out)
    }

    fn compare_and_swap(
        &self,
        id: &SolutionId,
        expected: Option<&AttemptId>,
        next: &VerificationRecord,
    ) -> Result<()> {
        let path = self.record_path(id);
        self.reject_symlinked_ancestors(&path)?;

        let existing = self.read_record_for(id, &path)?;

        match (expected, existing.as_ref()) {
            (None, Some(_)) => {
                return Err(VerificationRepositoryError::AlreadyExists { id: id.clone() }.into());
            }
            (Some(_), None) => {
                return Err(VerificationRepositoryError::NotFound { id: id.clone() }.into());
            }
            (Some(want), Some(rec)) => {
                if rec.attempt_id != *want {
                    return Err(VerificationRepositoryError::ConflictingAttempt {
                        id: id.clone(),
                        current: rec.attempt_id.clone(),
                    }
                    .into());
                }
            }
            (None, None) => {}
        }

        let expected_cloned = expected.cloned();
        if next.replaces_attempt_id != expected_cloned {
            return Err(VerificationRepositoryError::InconsistentReplacement {
                id: id.clone(),
                expected: expected_cloned,
                actual: next.replaces_attempt_id.clone(),
            }
            .into());
        }

        self.write_atomic(&path, next)
    }

    fn remove_if_attempt(&self, id: &SolutionId, expected: &AttemptId) -> Result<()> {
        let path = self.record_path(id);
        self.reject_symlinked_ancestors(&path)?;

        let Some(existing) = self.read_record_for(id, &path)? else {
            return Err(VerificationRepositoryError::NotFound { id: id.clone() }.into());
        };
        if existing.attempt_id != *expected {
            return Err(VerificationRepositoryError::ConflictingAttempt {
                id: id.clone(),
                current: existing.attempt_id,
            }
            .into());
        }
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
        if let Some(parent) = path.parent()
            && parent.exists()
        {
            let dir = File::open(parent)
                .with_context(|| format!("failed to open dir for fsync: {}", parent.display()))?;
            dir.sync_all()
                .with_context(|| format!("failed to fsync dir: {}", parent.display()))?;
        }
        Ok(())
    }
}
