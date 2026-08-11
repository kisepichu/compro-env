//! Port for querying git history (spec §12.14, §6.4).
//!
//! The site-data build needs `updated_at` and the commit SHA that last touched
//! each managed source file. Wrapping the query in a port keeps the projection
//! testable without shelling out to `git`.

use std::collections::BTreeMap;

use anyhow::Result;
use chrono::{DateTime, FixedOffset};

/// Repository-wide git identity captured at build time (spec §12.14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositorySnapshot {
    pub commit_sha: String,
    pub short_sha: String,
    pub committed_at: DateTime<FixedOffset>,
    pub uncommitted_changes: bool,
}

/// Per-path last-touch info the projection needs (spec §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathUpdate {
    pub committer_time: DateTime<FixedOffset>,
    pub commit_sha: String,
}

pub trait GitHistory {
    /// Repository-level identity for the `BuildMetadata` block.
    fn head_snapshot(&self) -> Result<RepositorySnapshot>;

    /// Returns `PathUpdate` for every repository-relative path in `paths`.
    /// Paths without recorded git history are omitted from the result; callers
    /// decide whether that is an error.
    fn last_touched(&self, paths: &[&str]) -> Result<BTreeMap<String, PathUpdate>>;
}
