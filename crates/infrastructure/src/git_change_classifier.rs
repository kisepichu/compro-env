//! Classify the set of paths changed between two Git commits into one of
//! three buckets that gate whether automation may run without a human review.
//!
//! Spec: `docs/spec.md` §15.3. The classifier is intentionally the sole
//! source of truth used by the safe-automation workflow — GitHub Actions
//! `paths` filters are unreliable (path-list length caps, symlink
//! opacity) and get bypassed here.

use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

/// Result of comparing two commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeClass {
    /// Both commits point at the same tree — no paths differ.
    Empty,
    /// Every changed path is a regular file under
    /// `verification/results/**/*.json`. Symlinks and type changes count as
    /// `SourceOrConfig`.
    ResultOnly,
    /// Any other diff.
    SourceOrConfig,
}

/// Failure modes for [`classify_changes`]. Callers distinguish invalid SHA
/// (usually a caller bug) from Git-plumbing/process failures.
#[derive(Debug, Error)]
pub enum ChangeClassificationError {
    #[error("invalid git revision {revision:?}: {message}")]
    InvalidRevision { revision: String, message: String },
    #[error("git command failed: {0}")]
    Git(String),
    #[error("failed to spawn git: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("unexpected git output: {0}")]
    Parse(String),
}

/// Classify the paths that differ between `before` and `after` in `root`.
pub fn classify_changes(
    root: &Path,
    before: &str,
    after: &str,
) -> Result<ChangeClass, ChangeClassificationError> {
    let raw = run_git_raw(root, before, after)?;
    let records = parse_raw_records(&raw)?;

    if records.is_empty() {
        return Ok(ChangeClass::Empty);
    }

    let mut result_only = true;
    for record in &records {
        // A symlink on either side (or a type change through one) never
        // counts as result-only, regardless of the affected path.
        if record.src_mode == "120000" || record.dst_mode == "120000" {
            result_only = false;
            break;
        }
        // Because we pass `--no-renames`, each record has exactly one path
        // in `paths`. Defensive: if git ever hands us multiple paths, all
        // must be allow-listed.
        for path in &record.paths {
            if !is_result_json(path) {
                result_only = false;
                break;
            }
        }
        if !result_only {
            break;
        }
    }

    Ok(if result_only {
        ChangeClass::ResultOnly
    } else {
        ChangeClass::SourceOrConfig
    })
}

fn is_result_json(path: &Path) -> bool {
    // Require `verification/results/` prefix, at least one further component,
    // and a `.json` suffix on the final component.
    let mut comps = path.components();
    match comps.next().and_then(|c| c.as_os_str().to_str()) {
        Some("verification") => {}
        _ => return false,
    }
    match comps.next().and_then(|c| c.as_os_str().to_str()) {
        Some("results") => {}
        _ => return false,
    }
    // There must be at least one component after `results/`, and the leaf
    // must end in `.json`.
    let remainder: Vec<_> = comps.collect();
    if remainder.is_empty() {
        return false;
    }
    let leaf = match remainder.last().and_then(|c| c.as_os_str().to_str()) {
        Some(s) => s,
        None => return false,
    };
    // Reject a bare `.json` filename or any leaf without the suffix.
    leaf.len() > ".json".len() && leaf.ends_with(".json")
}

#[derive(Debug)]
struct RawRecord {
    src_mode: String,
    dst_mode: String,
    paths: Vec<PathBuf>,
}

fn run_git_raw(
    root: &Path,
    before: &str,
    after: &str,
) -> Result<Vec<u8>, ChangeClassificationError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--raw", "-z", "--no-abbrev", "--no-renames"])
        .arg(before)
        .arg(after)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let lower = stderr.to_lowercase();
        if lower.contains("unknown revision")
            || lower.contains("bad revision")
            || lower.contains("bad object")
            || lower.contains("ambiguous argument")
            || lower.contains("fatal: not a valid object name")
        {
            // Report whichever revision git could not resolve. If we cannot
            // tell, report both.
            let revision = if stderr.contains(before) && !stderr.contains(after) {
                before.to_string()
            } else if stderr.contains(after) && !stderr.contains(before) {
                after.to_string()
            } else {
                format!("{before}..{after}")
            };
            return Err(ChangeClassificationError::InvalidRevision {
                revision,
                message: stderr.trim().to_string(),
            });
        }
        return Err(ChangeClassificationError::Git(format!(
            "git diff --raw failed with {}: {}",
            output.status,
            stderr.trim()
        )));
    }
    Ok(output.stdout)
}

/// Parse `git diff --raw -z --no-abbrev --no-renames` output.
///
/// Format (repeating): `:mode_src mode_dst sha_src sha_dst status\0path\0`.
/// With `--no-renames` there is exactly one path per record (rename detection
/// off, so no second-path slot). Handles NUL-safe path bytes including those
/// containing newlines.
fn parse_raw_records(bytes: &[u8]) -> Result<Vec<RawRecord>, ChangeClassificationError> {
    let mut records = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        // Header portion ends at the first NUL; format:
        // ":100644 100644 sha_src sha_dst STATUS"
        let header_end = bytes[cursor..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| {
                ChangeClassificationError::Parse("missing NUL after diff --raw header".into())
            })?;
        let header_bytes = &bytes[cursor..cursor + header_end];
        cursor += header_end + 1;
        let header = std::str::from_utf8(header_bytes).map_err(|e| {
            ChangeClassificationError::Parse(format!("non-utf8 diff --raw header: {e}"))
        })?;
        if !header.starts_with(':') {
            return Err(ChangeClassificationError::Parse(format!(
                "expected ':' at start of raw header, got {header:?}"
            )));
        }
        let fields: Vec<&str> = header[1..].split_whitespace().collect();
        if fields.len() < 5 {
            return Err(ChangeClassificationError::Parse(format!(
                "unexpected raw header shape: {header:?}"
            )));
        }
        let src_mode = fields[0].to_string();
        let dst_mode = fields[1].to_string();
        let status = fields[4];

        // Path count: 1 for A/M/D/T (all we can see under --no-renames),
        // 2 for R/C — but we passed --no-renames so R/C should not appear.
        // Be defensive anyway.
        let path_slots = if status.starts_with('R') || status.starts_with('C') {
            2
        } else {
            1
        };

        let mut paths = Vec::with_capacity(path_slots);
        for _ in 0..path_slots {
            let end = bytes[cursor..]
                .iter()
                .position(|&b| b == 0)
                .ok_or_else(|| {
                    ChangeClassificationError::Parse("missing NUL after diff --raw path".into())
                })?;
            let path_bytes = &bytes[cursor..cursor + end];
            cursor += end + 1;
            paths.push(bytes_to_path(path_bytes));
        }

        records.push(RawRecord {
            src_mode,
            dst_mode,
            paths,
        });
    }
    Ok(records)
}

#[cfg(unix)]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    // Non-unix fallback: assume UTF-8. The safe-automation workflow only
    // targets Linux runners, so this branch is a compile-time convenience.
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}
