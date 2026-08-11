//! `git log`-based implementation of the [`GitHistory`] port.
//!
//! Uses porcelain subcommands that ship with every supported git build so we
//! avoid the plumbing cost of libgit2. All queries are wrapped in
//! `git -C <repo>` so the working directory of the caller is irrelevant.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, anyhow, bail};
use chrono::DateTime;
use usecases::git_history::{GitHistory, PathUpdate, RepositorySnapshot};

pub struct GitHistoryImpl {
    repo_root: PathBuf,
}

impl GitHistoryImpl {
    pub fn new(repo_root: PathBuf) -> Self {
        Self { repo_root }
    }

    fn run(&self, args: &[&str]) -> Result<Output> {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.repo_root);
        for arg in args {
            cmd.arg(arg);
        }
        let output = cmd
            .output()
            .with_context(|| format!("failed to spawn git {}", args.join(" ")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            bail!(
                "git {} failed with {}: {}",
                args.join(" "),
                output.status,
                stderr
            );
        }
        Ok(output)
    }
}

impl GitHistory for GitHistoryImpl {
    fn head_snapshot(&self) -> Result<RepositorySnapshot> {
        let sha = self.run(&["rev-parse", "HEAD"])?;
        let commit_sha = String::from_utf8_lossy(&sha.stdout).trim().to_string();
        if commit_sha.is_empty() {
            bail!("git rev-parse HEAD returned empty output");
        }
        let short = &commit_sha[..commit_sha.len().min(7)];

        let ts = self.run(&["show", "-s", "--format=%cI", "HEAD"])?;
        let iso = String::from_utf8_lossy(&ts.stdout).trim().to_string();
        let committed_at = DateTime::parse_from_rfc3339(&iso)
            .with_context(|| format!("could not parse git commit timestamp {iso:?}"))?;

        let status = self.run(&["status", "--porcelain"])?;
        let uncommitted_changes = !status.stdout.is_empty();

        Ok(RepositorySnapshot {
            commit_sha: commit_sha.clone(),
            short_sha: short.to_string(),
            committed_at,
            uncommitted_changes,
        })
    }

    fn last_touched(&self, paths: &[&str]) -> Result<BTreeMap<String, PathUpdate>> {
        let mut out = BTreeMap::new();
        for path in paths {
            let output = Command::new("git")
                .arg("-C")
                .arg(&self.repo_root)
                .args(["log", "-1", "--pretty=format:%H%x1f%cI", "--", path])
                .output()
                .with_context(|| format!("failed to spawn git log for {path}"))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(anyhow!(
                    "git log for {} failed with {}: {}",
                    path,
                    output.status,
                    stderr
                ));
            }
            let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if line.is_empty() {
                // No history recorded — omit from the map so the caller can
                // decide whether that is fatal.
                continue;
            }
            let (sha, ts) = line
                .split_once('\x1f')
                .ok_or_else(|| anyhow!("unexpected git log output shape: {line:?}"))?;
            let committer_time = DateTime::parse_from_rfc3339(ts)
                .with_context(|| format!("could not parse git commit timestamp {ts:?}"))?;
            out.insert(
                path.to_string(),
                PathUpdate {
                    committer_time,
                    commit_sha: sha.to_string(),
                },
            );
        }
        Ok(out)
    }
}

/// Repository root probe used by the CLI when no root has been established
/// upstream. Returns the first ancestor of `start` that contains a `.git`
/// entry, or an error if the walk exhausts.
pub fn find_repository_root(start: &Path) -> Result<PathBuf> {
    let mut cur = start.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize project root candidate {}",
            start.display()
        )
    })?;
    loop {
        if cur.join(".git").exists() {
            return Ok(cur);
        }
        match cur.parent() {
            Some(parent) => cur = parent.to_path_buf(),
            None => bail!("no git repository found at or above {}", start.display()),
        }
    }
}
