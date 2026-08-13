//! Constrained GitHub state writer for verification records (spec §15.1,
//! §15.4).
//!
//! This writer is the only bridge between the internal `verify` pipeline and
//! the remote repository. It has three explicit contracts:
//!
//! 1. Never touch anything outside `verification/results/**/*.json`.
//! 2. Never mutate anything other than the sole `automation/verify` branch and
//!    its bot draft PR.
//! 3. Never let the App installation token appear in `Debug`, `Display`,
//!    error variants, log output, or a Git credential file.
//!
//! Every mutating API call re-runs the branch, path, and CAS guard clauses;
//! guard clauses run BEFORE the App token is ever attached to a request.
//! The writer uses HTTP APIs (`reqwest::blocking`) and never spawns `git`, so
//! there is no credential-file surface to worry about.
//!
//! See spec §15.1 (bot PR lifecycle), §15.3 (result-only push classification),
//! and §15.4 (credential separation) for the higher-level constraints.
//!
//! Test surface: the writer targets a caller-provided `base_url` (usually
//! `https://api.github.com` in production, `http://127.0.0.1:<port>` under
//! test). All network-facing methods route their GraphQL requests to
//! `{base_url}/graphql`, so a single tiny_http server can observe every side
//! effect.

use std::fmt;
use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use domain::verification::VerificationRecord;
use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

// ─── Public types ────────────────────────────────────────────────────────────

/// Request payload for [`GitHubVerificationStateWriter::persist`] (spec §15.1,
/// §15.4).
///
/// The writer refuses to contact GitHub until every field passes the local
/// guard clauses: `branch` must be exactly `automation/verify`, `base_sha`
/// must be a canonical 40-hex commit id, `repository` must be `owner/repo`,
/// and the derived result path must live under `verification/results/`.
#[derive(Debug, Clone)]
pub struct PersistStateRequest {
    /// `owner/repo` slug used as the API path prefix.
    pub repository: String,
    /// Commit SHA the plan and CAS were built against (spec §15.4).
    pub base_sha: String,
    /// Must be exactly `automation/verify` (spec §15.1).
    pub branch: String,
    /// Serialized verification record to persist. `solution_id` is validated
    /// by the domain layer; the writer additionally enforces the result-path
    /// allowlist as defense in depth.
    pub candidate: VerificationRecord,
}

/// Outcome of a successful [`GitHubVerificationStateWriter::persist`] call.
///
/// Callers can log or forward the returned SHAs (they are not secret). The
/// `pull_request_number` field is populated by [`set_pull_request_state`]
/// invocations; `persist` itself never opens or looks up the bot PR, so
/// this field is `0` when `persist` returns without a subsequent PR
/// update call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedState {
    /// Repository-relative JSON path that was written, always under
    /// `verification/results/**`.
    pub result_path: String,
    /// Blob SHA returned by the Git Data API.
    pub blob_sha: String,
    /// Tree SHA returned by the Git Data API.
    pub tree_sha: String,
    /// Commit SHA returned by the Git Data API.
    pub commit_sha: String,
    /// Bot PR number, or `0` when no PR has been observed yet (spec §15.1).
    pub pull_request_number: u64,
}

/// Target state of the bot PR (spec §15.1, §15.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BotPullRequestState {
    /// Keep the PR as a draft while polling / recovering.
    Draft { pull_request_number: u64 },
    /// Mark the PR ready for review; optionally enable auto-merge for
    /// terminal results.
    Ready {
        pull_request_number: u64,
        auto_merge: bool,
    },
}

/// Handle to the single long-lived automation PR returned by
/// [`GitHubVerificationStateWriter::find_or_open_bot_pr`].
///
/// `is_draft` is captured so callers can skip a no-op state update or
/// route through the appropriate direction-specific API: REST supports
/// Draft → Ready via `PATCH {draft: false}` but Ready → Draft requires
/// the `convertPullRequestToDraft` GraphQL mutation (spec §15.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotPullRequestRef {
    pub number: u64,
    pub is_draft: bool,
}

/// All failure modes surfaced by the writer.
///
/// Non-success HTTP responses are collapsed into `UpstreamStatus`; the raw
/// upstream body is never carried through so a leaked GitHub error page
/// cannot echo the App token back to a log line (spec §15.4).
#[derive(Debug, Error)]
pub enum PersistError {
    #[error(
        "verify state writer refuses branch {branch:?}: only automation/verify is permitted (spec §15.1)"
    )]
    WrongBranch { branch: String },

    #[error("base_sha must be exactly 40 lowercase hex characters (spec §15.4)")]
    InvalidBaseSha,

    #[error("repository must be owner/repo (got {value:?})")]
    InvalidRepository { value: String },

    #[error("result path {path:?} escapes the verification/results/ allowlist (spec §15.1, §15.4)")]
    InvalidResultPath { path: String },

    #[error(
        "attempt CAS mismatch: replaces_attempt_id={expected:?} but remote has {actual:?} (spec §15.1)"
    )]
    AttemptCasMismatch {
        expected: Option<String>,
        actual: Option<String>,
    },

    #[error(
        "PATCH refs/heads/automation/verify remained non-fast-forward after one retry (spec §15.1)"
    )]
    RefUpdateConflict,

    #[error("failed to serialize verification record: {source}")]
    Serialization {
        #[from]
        source: serde_json::Error,
    },

    #[error("HTTP transport error while contacting GitHub API")]
    Transport {
        #[from]
        source: reqwest::Error,
    },

    #[error(
        "GitHub API responded with HTTP {status} during {op}; body redacted to avoid leaking secrets"
    )]
    UpstreamStatus { status: u16, op: &'static str },

    #[error(
        "GitHub GraphQL mutation for {op} reported {count} error(s); details redacted to avoid leaking secrets"
    )]
    GraphqlError { op: &'static str, count: usize },

    #[error("GitHub API response for {op} was missing expected field {field}")]
    MalformedResponse {
        op: &'static str,
        field: &'static str,
    },
}

/// `Result` alias used across the writer's public surface.
pub type PersistResult<T> = std::result::Result<T, PersistError>;

// ─── Path allowlist (spec §15.4) ────────────────────────────────────────────

const RESULTS_PREFIX: &str = "verification/results/";
const REQUIRED_BRANCH: &str = "automation/verify";

/// Defense-in-depth guard that the caller has not supplied a path outside
/// `verification/results/**/*.json`.
///
/// The domain `SolutionId` newtype already forbids `..`, backslashes, and
/// path escapes; this function is a second layer that runs immediately
/// before every mutating API call so a future bug in path construction
/// cannot silently escape the allowlist.
pub fn validate_result_path(path: &str) -> PersistResult<()> {
    if !path.starts_with(RESULTS_PREFIX) {
        return Err(PersistError::InvalidResultPath {
            path: path.to_string(),
        });
    }
    if !path.ends_with(".json") {
        return Err(PersistError::InvalidResultPath {
            path: path.to_string(),
        });
    }
    // The leaf must have a non-empty stem — reject `.json` (bare extension).
    // Otherwise `verification/results/.json` would sail past the prefix and
    // suffix checks while `is_result_json_path` (the classifier) rejects it,
    // producing an inconsistent view between the two guards.
    let leaf = path.rsplit('/').next().unwrap_or("");
    if leaf.len() <= ".json".len() {
        return Err(PersistError::InvalidResultPath {
            path: path.to_string(),
        });
    }
    if path.contains("//") || path.contains("\\") || path.contains('\0') {
        return Err(PersistError::InvalidResultPath {
            path: path.to_string(),
        });
    }
    for segment in path.split('/') {
        if matches!(segment, "" | "." | "..") {
            return Err(PersistError::InvalidResultPath {
                path: path.to_string(),
            });
        }
    }
    Ok(())
}

fn compute_result_path(record: &VerificationRecord) -> String {
    format!("{RESULTS_PREFIX}{}.json", record.solution_id.as_str())
}

fn validate_branch(branch: &str) -> PersistResult<()> {
    if branch == REQUIRED_BRANCH {
        Ok(())
    } else {
        Err(PersistError::WrongBranch {
            branch: branch.to_string(),
        })
    }
}

fn validate_base_sha(sha: &str) -> PersistResult<()> {
    if sha.len() == 40
        && sha
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    {
        Ok(())
    } else {
        Err(PersistError::InvalidBaseSha)
    }
}

fn split_repository(repo: &str) -> PersistResult<(&str, &str)> {
    let mut parts = repo.split('/');
    let owner = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return Err(PersistError::InvalidRepository {
            value: repo.to_string(),
        });
    }
    Ok((owner, name))
}

// ─── Writer ──────────────────────────────────────────────────────────────────

/// GitHub verification-state writer.
///
/// The struct holds the base URL, a shared blocking HTTP client, the App
/// token wrapped in [`SecretString`], and a cached `owner/repo` slug that
/// `persist` populates so [`set_pull_request_state`] can reach the bot PR
/// without a second parameter. The token is never exposed via `Debug`
/// (`SecretString` prints `SecretBox<str>([REDACTED])`), never serialized,
/// and never included in error variants.
pub struct GitHubVerificationStateWriter {
    base_url: String,
    http: Client,
    token: SecretString,
    /// `owner/repo` remembered from the most recent successful validation
    /// step of [`persist`], or explicitly seeded via [`bind_repository`].
    bound_repository: Mutex<Option<(String, String)>>,
}

impl fmt::Debug for GitHubVerificationStateWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHubVerificationStateWriter")
            .field("base_url", &self.base_url)
            .field("token", &self.token)
            .field("bound_repository", &self.bound_repository)
            .finish()
    }
}

impl GitHubVerificationStateWriter {
    pub fn new(base_url: impl Into<String>, token: SecretString) -> Self {
        let http = Client::builder()
            .build()
            .expect("reqwest blocking client should build with default settings");
        Self {
            base_url: base_url.into(),
            http,
            token,
            bound_repository: Mutex::new(None),
        }
    }

    /// Explicitly bind `owner/repo` before invoking [`set_pull_request_state`]
    /// without going through [`persist`] first.
    ///
    /// `persist` binds automatically after passing its guard clauses; this
    /// helper exists so callers (and tests) that only need PR state updates
    /// can still target the correct repository. The guard clauses are the
    /// same as those inside `persist`, so a malformed slug is rejected
    /// before it is remembered.
    pub fn bind_repository(&self, repository: &str) -> PersistResult<()> {
        let (owner, repo) = split_repository(repository)?;
        *self
            .bound_repository
            .lock()
            .expect("bound_repository mutex poisoned") =
            Some((owner.to_string(), repo.to_string()));
        Ok(())
    }

    fn bound_owner_repo(&self) -> PersistResult<(String, String)> {
        self.bound_repository
            .lock()
            .expect("bound_repository mutex poisoned")
            .clone()
            .ok_or_else(|| PersistError::InvalidRepository {
                value: "no repository bound (call persist or bind_repository first)".to_string(),
            })
    }

    /// Persist a verification record to `verification/results/<solution_id>.json`
    /// on the sole `automation/verify` branch, using GitHub's Git Data API.
    ///
    /// The call performs seven HTTP requests on the happy path:
    /// 0. `GET /repos/{owner}/{repo}/git/refs/heads/automation/verify` —
    ///    resolve the state branch tip. Every subsequent read / write
    ///    anchors on this SHA, NOT on `request.base_sha`. `base_sha` is a
    ///    `main@base_sha` tamper-evidence anchor (spec §15.1) but does not
    ///    point at a tree that carries `verification/results/**`.
    /// 1. `GET /repos/{owner}/{repo}/contents/{path}?ref={state_head}` — CAS.
    /// 2. `POST /repos/{owner}/{repo}/git/blobs` — write JSON blob.
    /// 3. `GET /repos/{owner}/{repo}/git/commits/{state_head}` — resolve the
    ///    tip to its tree SHA. GitHub's `POST /git/trees` expects a tree
    ///    SHA in `base_tree`, not a commit SHA, so we must resolve first.
    /// 4. `POST /repos/{owner}/{repo}/git/trees` — create tree on top of the
    ///    resolved `base_tree`.
    /// 5. `POST /repos/{owner}/{repo}/git/commits` — commit with the state
    ///    branch tip as parent.
    /// 6. `PATCH /repos/{owner}/{repo}/git/refs/heads/automation/verify` —
    ///    fast-forward the branch to the new commit.
    ///
    /// On a 422 (non-fast-forward) response for the PATCH the writer rebuilds
    /// against whichever commit now sits at HEAD: it fetches the new head SHA,
    /// re-runs the CAS check against that head, resolves the new head's tree,
    /// posts a fresh tree (reusing the blob it already created) and a fresh
    /// commit with the new head as parent, then retries the PATCH once. If
    /// the CAS re-check reveals a divergent attempt id, the call fails with
    /// [`PersistError::AttemptCasMismatch`] (the state has genuinely diverged).
    /// If the rebuilt PATCH also fails with 422, the call fails with
    /// [`PersistError::RefUpdateConflict`] and no further retry is attempted.
    ///
    /// The writer never creates or updates a pull request as part of
    /// `persist`; use [`set_pull_request_state`] for that. The returned
    /// `pull_request_number` field is therefore `0` (spec §15.1 delegates
    /// PR management to a separate, explicit step).
    pub fn persist(&self, request: &PersistStateRequest) -> PersistResult<PersistedState> {
        // Guard clauses — cheap, run before any HTTP contact so we never
        // attach the App token to a request the writer would refuse to send.
        validate_branch(&request.branch)?;
        validate_base_sha(&request.base_sha)?;
        let (owner, repo) = split_repository(&request.repository)?;

        let result_path = compute_result_path(&request.candidate);
        validate_result_path(&result_path)?;

        let serialized = serde_json::to_string(&request.candidate)?;

        // Remember owner/repo so a later `set_pull_request_state` call can
        // reach the bot PR without extra plumbing.
        *self
            .bound_repository
            .lock()
            .expect("bound_repository mutex poisoned") =
            Some((owner.to_string(), repo.to_string()));

        // Fetch `automation/verify`'s current tip up front. `base_sha` is a
        // `main@base_sha` anchor for tamper-evidence (spec §15.1) but does
        // NOT point at a tree that carries `verification/results/**` — that
        // delta lives only on the state branch. Every subsequent HTTP call
        // that reads / writes existing records or fast-forwards the branch
        // must therefore anchor on the state branch tip, not on `base_sha`.
        // Step 6's retry path already does this on 422; doing it up front
        // keeps steps 1, 3, and 5 aligned with the same head so the initial
        // PATCH is a straight fast-forward on the happy path.
        let state_head_sha = self.get_ref_sha(owner, repo)?;

        // Step 1: CAS check via the contents API, anchored on the state
        // branch tip.
        self.cas_check(
            owner,
            repo,
            &result_path,
            &state_head_sha,
            &request.candidate,
        )?;

        // Step 2: blob.
        let blob_sha = self.create_blob(owner, repo, &serialized, &result_path, &request.branch)?;

        // Step 3: resolve the state branch tip to its tree SHA. GitHub's
        // `POST /git/trees` endpoint documents `base_tree` as the SHA of an
        // existing tree object, not a commit — even though the API sometimes
        // tolerates a commit SHA in practice, we do not rely on undocumented
        // behavior.
        let base_tree_sha = self.resolve_commit_tree(owner, repo, &state_head_sha)?;

        // Step 4: tree on top of the resolved base tree.
        let tree_sha = self.create_tree(
            owner,
            repo,
            &base_tree_sha,
            &result_path,
            &blob_sha,
            &request.branch,
        )?;

        // Step 5: commit whose parent is the state branch tip so PATCH
        // succeeds on the happy path.
        let commit_message = format!("verify: persist {}", request.candidate.solution_id.as_str());
        let commit_sha = self.create_commit(
            owner,
            repo,
            &commit_message,
            &tree_sha,
            &state_head_sha,
            &result_path,
            &request.branch,
        )?;

        // Step 6: fast-forward the ref, rebuilding once on 422 conflict.
        let (final_tree_sha, final_commit_sha) = self.update_ref_with_retry(
            owner,
            repo,
            &commit_sha,
            &tree_sha,
            &blob_sha,
            &commit_message,
            &result_path,
            &request.branch,
            &request.candidate,
        )?;

        Ok(PersistedState {
            result_path,
            blob_sha,
            tree_sha: final_tree_sha,
            commit_sha: final_commit_sha,
            pull_request_number: 0,
        })
    }

    /// Toggle the bot PR between draft and ready-for-review, optionally
    /// enabling auto-merge for terminal results (spec §15.1, §15.2).
    ///
    /// Draft/ready is toggled via `PATCH /repos/{owner}/{repo}/pulls/{n}`
    /// with a `{ "draft": bool }` body. Auto-merge is enabled by posting
    /// the `enablePullRequestAutoMerge` GraphQL mutation to
    /// `{base_url}/graphql`.
    ///
    /// GitHub's GraphQL mutation requires the PR's opaque base64-shaped
    /// **node id** (e.g. `PR_kwDO...`), not the numeric PR number, so when
    /// `auto_merge` is requested the writer first resolves the number to
    /// its node id via `GET /repos/{owner}/{repo}/pulls/{n}` (reading the
    /// `.node_id` field) before performing the PATCH and the mutation. Only
    /// the auto-merge path incurs the extra REST call.
    ///
    /// The writer must have a bound repository before this call — either
    /// through a prior [`persist`] or an explicit [`bind_repository`]. That
    /// way the exact-match signature stays free of extra parameters while
    /// the writer still targets the correct repository.
    pub fn set_pull_request_state(&self, state: BotPullRequestState) -> PersistResult<()> {
        let (owner, repo) = self.bound_owner_repo()?;

        match state {
            BotPullRequestState::Draft {
                pull_request_number,
            } => {
                self.patch_pr(&owner, &repo, pull_request_number, false)?;
            }
            BotPullRequestState::Ready {
                pull_request_number,
                auto_merge,
            } => {
                if auto_merge {
                    // Resolve the node id BEFORE we mutate anything so a
                    // lookup failure does not leave the PR half-marked ready.
                    let node_id = self.resolve_pr_node_id(&owner, &repo, pull_request_number)?;
                    self.patch_pr(&owner, &repo, pull_request_number, true)?;
                    self.enable_auto_merge(&node_id)?;
                } else {
                    self.patch_pr(&owner, &repo, pull_request_number, true)?;
                }
            }
        }
        Ok(())
    }

    // ── helpers ────────────────────────────────────────────────────────────

    fn authed(&self, mut req: RequestBuilder) -> RequestBuilder {
        // The token is only exposed here, at the moment we hand it to
        // reqwest's header builder. It never enters a String or format!
        // outside of this scope.
        let token = self.token.expose_secret();
        req = req.header("Authorization", format!("Bearer {token}"));
        req = req.header("User-Agent", "ce-verify-state-writer/0.1");
        req = req.header("Accept", "application/vnd.github+json");
        req = req.header("X-GitHub-Api-Version", "2022-11-28");
        req
    }

    fn cas_check(
        &self,
        owner: &str,
        repo: &str,
        result_path: &str,
        base_sha: &str,
        candidate: &VerificationRecord,
    ) -> PersistResult<()> {
        // Re-verify guards even for the read call.
        validate_base_sha(base_sha)?;
        validate_result_path(result_path)?;

        let url = format!(
            "{}/repos/{owner}/{repo}/contents/{result_path}?ref={base_sha}",
            self.base_url
        );
        let resp = self.authed(self.http.get(&url)).send()?;
        let status = resp.status();

        let expected = candidate
            .replaces_attempt_id
            .as_ref()
            .map(|id| id.as_str().to_string());

        if status == StatusCode::NOT_FOUND {
            // No existing result. CAS ok iff we don't expect a predecessor.
            if expected.is_none() {
                return Ok(());
            }
            return Err(PersistError::AttemptCasMismatch {
                expected,
                actual: None,
            });
        }

        if !status.is_success() {
            let _ = resp.text();
            return Err(PersistError::UpstreamStatus {
                status: status.as_u16(),
                op: "GET contents (cas)",
            });
        }

        #[derive(Deserialize)]
        struct Contents {
            content: String,
            encoding: String,
        }
        let body: Contents = resp.json().map_err(PersistError::from)?;
        let raw = if body.encoding == "base64" {
            let cleaned: String = body
                .content
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            BASE64
                .decode(cleaned.as_bytes())
                .map_err(|_| PersistError::MalformedResponse {
                    op: "GET contents (cas)",
                    field: "content",
                })?
        } else {
            body.content.into_bytes()
        };

        let existing: VerificationRecord =
            serde_json::from_slice(&raw).map_err(PersistError::from)?;

        let actual = Some(existing.attempt_id.as_str().to_string());
        if actual == expected {
            Ok(())
        } else {
            Err(PersistError::AttemptCasMismatch { expected, actual })
        }
    }

    fn create_blob(
        &self,
        owner: &str,
        repo: &str,
        content: &str,
        result_path: &str,
        branch: &str,
    ) -> PersistResult<String> {
        validate_branch(branch)?;
        validate_result_path(result_path)?;

        let url = format!("{}/repos/{owner}/{repo}/git/blobs", self.base_url);
        let body = json!({
            "content": content,
            "encoding": "utf-8",
        });
        let resp = self.authed(self.http.post(&url)).json(&body).send()?;
        Self::read_sha(resp, "POST blobs")
    }

    fn create_tree(
        &self,
        owner: &str,
        repo: &str,
        base_tree_sha: &str,
        result_path: &str,
        blob_sha: &str,
        branch: &str,
    ) -> PersistResult<String> {
        validate_branch(branch)?;
        // Tree SHAs are 40-hex, same shape as commit SHAs — the guard is
        // still meaningful defense in depth.
        validate_base_sha(base_tree_sha)?;
        validate_result_path(result_path)?;

        let url = format!("{}/repos/{owner}/{repo}/git/trees", self.base_url);
        let body = json!({
            "base_tree": base_tree_sha,
            "tree": [{
                "path": result_path,
                "mode": "100644",
                "type": "blob",
                "sha": blob_sha,
            }],
        });
        let resp = self.authed(self.http.post(&url)).json(&body).send()?;
        Self::read_sha(resp, "POST trees")
    }

    /// Fetch `.tree.sha` from `GET /repos/{owner}/{repo}/git/commits/{sha}`.
    ///
    /// Used to translate a base commit SHA (what callers know) into the
    /// tree SHA that GitHub's Git Data API expects for `base_tree` when
    /// creating a new tree.
    fn resolve_commit_tree(
        &self,
        owner: &str,
        repo: &str,
        commit_sha: &str,
    ) -> PersistResult<String> {
        validate_base_sha(commit_sha)?;

        let url = format!(
            "{}/repos/{owner}/{repo}/git/commits/{commit_sha}",
            self.base_url
        );
        let resp = self.authed(self.http.get(&url)).send()?;
        let status = resp.status();
        if !status.is_success() {
            let _ = resp.text();
            return Err(PersistError::UpstreamStatus {
                status: status.as_u16(),
                op: "GET git/commits/{sha}",
            });
        }
        #[derive(Deserialize)]
        struct TreeRef {
            sha: String,
        }
        #[derive(Deserialize)]
        struct CommitBody {
            tree: TreeRef,
        }
        let body: CommitBody = resp.json().map_err(PersistError::from)?;
        Ok(body.tree.sha)
    }

    #[allow(clippy::too_many_arguments)]
    fn create_commit(
        &self,
        owner: &str,
        repo: &str,
        message: &str,
        tree_sha: &str,
        base_sha: &str,
        result_path: &str,
        branch: &str,
    ) -> PersistResult<String> {
        validate_branch(branch)?;
        validate_base_sha(base_sha)?;
        validate_result_path(result_path)?;

        let url = format!("{}/repos/{owner}/{repo}/git/commits", self.base_url);
        let body = json!({
            "message": message,
            "tree": tree_sha,
            "parents": [base_sha],
        });
        let resp = self.authed(self.http.post(&url)).json(&body).send()?;
        Self::read_sha(resp, "POST commits")
    }

    /// PATCH the ref, and if GitHub reports a non-fast-forward conflict,
    /// rebuild the tree and commit against the branch's current HEAD before
    /// retrying exactly once.
    ///
    /// Returns the tree SHA and commit SHA that ultimately became the branch
    /// tip — these may differ from the initial ones when the rebuild path
    /// runs. The blob is never re-created; the same JSON payload is reused.
    ///
    /// A naive "retry the same commit SHA" is guaranteed to 422 again in any
    /// real concurrent-write scenario because that commit's parent still
    /// points at the stale base, so the retry is only useful once we rebase
    /// on the new head.
    #[allow(clippy::too_many_arguments)]
    fn update_ref_with_retry(
        &self,
        owner: &str,
        repo: &str,
        initial_commit_sha: &str,
        initial_tree_sha: &str,
        blob_sha: &str,
        commit_message: &str,
        result_path: &str,
        branch: &str,
        candidate: &VerificationRecord,
    ) -> PersistResult<(String, String)> {
        validate_branch(branch)?;
        validate_result_path(result_path)?;

        match self.patch_ref(owner, repo, initial_commit_sha)? {
            RefPatchOutcome::Ok => {
                Ok((initial_tree_sha.to_string(), initial_commit_sha.to_string()))
            }
            RefPatchOutcome::Conflict => {
                // Someone pushed a new commit to automation/verify ahead of
                // us. Resolve the current head and rebuild on top of it.
                let new_head = self.get_ref_sha(owner, repo)?;

                // Re-validate the CAS invariant against the new head. If the
                // stored attempt id has diverged (either differs from what
                // we planned to replace, or a record now exists where none
                // did, or a record we expected has been deleted), that is a
                // genuine attempt collision — not a race we can rebuild
                // through — so we surface it as AttemptCasMismatch.
                self.cas_check(owner, repo, result_path, &new_head, candidate)?;

                // Resolve the new head to its tree SHA so we can layer our
                // blob onto it.
                let new_base_tree = self.resolve_commit_tree(owner, repo, &new_head)?;

                // Rebuild the tree with the same blob (blob content is
                // deterministic in the record we're persisting).
                let new_tree_sha =
                    self.create_tree(owner, repo, &new_base_tree, result_path, blob_sha, branch)?;

                // Rebuild the commit with the new head as parent.
                let new_commit_sha = self.create_commit(
                    owner,
                    repo,
                    commit_message,
                    &new_tree_sha,
                    &new_head,
                    result_path,
                    branch,
                )?;

                match self.patch_ref(owner, repo, &new_commit_sha)? {
                    RefPatchOutcome::Ok => Ok((new_tree_sha, new_commit_sha)),
                    RefPatchOutcome::Conflict => Err(PersistError::RefUpdateConflict),
                }
            }
        }
    }

    fn patch_ref(
        &self,
        owner: &str,
        repo: &str,
        commit_sha: &str,
    ) -> PersistResult<RefPatchOutcome> {
        let url = format!(
            "{}/repos/{owner}/{repo}/git/refs/heads/{REQUIRED_BRANCH}",
            self.base_url
        );
        let body = json!({
            "sha": commit_sha,
            "force": false,
        });
        let resp = self.authed(self.http.patch(&url)).json(&body).send()?;
        let status = resp.status();
        if status.is_success() {
            let _ = resp.text();
            return Ok(RefPatchOutcome::Ok);
        }
        if status == StatusCode::UNPROCESSABLE_ENTITY {
            let _ = resp.text();
            return Ok(RefPatchOutcome::Conflict);
        }
        let _ = resp.text();
        Err(PersistError::UpstreamStatus {
            status: status.as_u16(),
            op: "PATCH refs/heads/automation/verify",
        })
    }

    /// Fetch the current tip SHA of `refs/heads/automation/verify`.
    fn get_ref_sha(&self, owner: &str, repo: &str) -> PersistResult<String> {
        let url = format!(
            "{}/repos/{owner}/{repo}/git/refs/heads/{REQUIRED_BRANCH}",
            self.base_url
        );
        let resp = self.authed(self.http.get(&url)).send()?;
        let status = resp.status();
        if !status.is_success() {
            let _ = resp.text();
            return Err(PersistError::UpstreamStatus {
                status: status.as_u16(),
                op: "GET refs/heads/automation/verify",
            });
        }
        #[derive(Deserialize)]
        struct RefObject {
            sha: String,
        }
        #[derive(Deserialize)]
        struct RefBody {
            object: RefObject,
        }
        let body: RefBody = resp.json().map_err(PersistError::from)?;
        Ok(body.object.sha)
    }

    /// Return a handle to the single long-lived bot PR from `head` into
    /// `base` (spec §15.1 "最大 1 本の automation/verify draft PR"). Opens a
    /// fresh draft PR when none is open.
    ///
    /// The GitHub REST list endpoint returns an array of PR summaries filtered
    /// by `head=<owner>:<branch>` and `state=open`. Query values are percent-
    /// encoded via `reqwest::RequestBuilder::query` — passing a branch name
    /// containing `&` or `=` (which git itself does not forbid) would
    /// otherwise silently corrupt the query string.
    ///
    /// When more than one PR matches the filter (should never happen given
    /// the "at most one" rule), the caller-observable behaviour is to reuse
    /// the first entry — the writer will never open a duplicate.
    ///
    /// TOCTOU race: two concurrent runs can both observe an empty list and
    /// both attempt `POST /pulls`; the second POST fails with 422 ("A pull
    /// request already exists for this head branch"). The writer detects
    /// that status and refetches the list; if the list is now non-empty the
    /// existing PR is returned as if it had been observed on the first GET.
    ///
    /// The returned [`BotPullRequestRef`] carries the PR's current
    /// `is_draft` flag so callers can pick the correct direction-specific
    /// API for a subsequent state change (spec §15.1 — REST cannot convert
    /// Ready → Draft; that path lives in
    /// [`convert_pr_to_draft`](Self::convert_pr_to_draft)).
    pub fn find_or_open_bot_pr(
        &self,
        owner: &str,
        repo: &str,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> PersistResult<BotPullRequestRef> {
        #[derive(Deserialize)]
        struct PullSummary {
            number: Option<u64>,
            #[serde(default)]
            draft: Option<bool>,
        }

        let list_url = format!("{}/repos/{owner}/{repo}/pulls", self.base_url);
        let head_qualified = format!("{owner}:{head}");
        let query_params = [
            ("head", head_qualified.as_str()),
            ("state", "open"),
            ("base", base),
        ];

        let list = || -> PersistResult<Vec<PullSummary>> {
            let resp = self
                .authed(self.http.get(&list_url))
                .query(&query_params)
                .send()?;
            let status = resp.status();
            if !status.is_success() {
                let _ = resp.text();
                return Err(PersistError::UpstreamStatus {
                    status: status.as_u16(),
                    op: "GET pulls?head (find bot pr)",
                });
            }
            resp.json().map_err(PersistError::from)
        };

        if let Some(first) = list()?.into_iter().next() {
            let number = first.number.ok_or(PersistError::MalformedResponse {
                op: "GET pulls?head (find bot pr)",
                field: "number",
            })?;
            return Ok(BotPullRequestRef {
                number,
                is_draft: first.draft.unwrap_or(false),
            });
        }

        let open_url = format!("{}/repos/{owner}/{repo}/pulls", self.base_url);
        let payload = json!({
            "title": title,
            "body": body,
            "head": head,
            "base": base,
            "draft": true,
        });
        let resp = self
            .authed(self.http.post(&open_url))
            .json(&payload)
            .send()?;
        let status = resp.status();
        if !status.is_success() {
            let _ = resp.text();
            // TOCTOU race: a concurrent run may have opened the PR between
            // our GET and POST. GitHub returns 422 in that case; refetch
            // and reuse it before propagating the error.
            if status == StatusCode::UNPROCESSABLE_ENTITY
                && let Some(first) = list()?.into_iter().next()
            {
                let number = first.number.ok_or(PersistError::MalformedResponse {
                    op: "GET pulls?head (find bot pr, retry after 422)",
                    field: "number",
                })?;
                return Ok(BotPullRequestRef {
                    number,
                    is_draft: first.draft.unwrap_or(false),
                });
            }
            return Err(PersistError::UpstreamStatus {
                status: status.as_u16(),
                op: "POST pulls (open bot pr)",
            });
        }
        let created: PullSummary = resp.json().map_err(PersistError::from)?;
        let number = created.number.ok_or(PersistError::MalformedResponse {
            op: "POST pulls (open bot pr)",
            field: "number",
        })?;
        Ok(BotPullRequestRef {
            number,
            // We asked for `draft: true`; the server echoes the field back.
            // Fall back to `true` if the server omitted it — matches the
            // request we just sent.
            is_draft: created.draft.unwrap_or(true),
        })
    }

    /// Convert a currently-open Ready PR back to draft via GitHub's
    /// `convertPullRequestToDraft` GraphQL mutation (spec §15.1).
    ///
    /// REST's `PATCH /pulls/{n}` supports `{draft: false}` (Draft → Ready)
    /// but rejects `{draft: true}` on a non-draft PR with 422 — the only
    /// supported reverse direction is the GraphQL mutation used here.
    /// Callers should pre-check the PR's current draft state via
    /// [`find_or_open_bot_pr`]'s [`BotPullRequestRef::is_draft`] and skip
    /// this call when it is already `true`; the mutation may error on
    /// an already-draft PR depending on GitHub's server-side behaviour.
    pub fn convert_pr_to_draft(&self, pull_request_number: u64) -> PersistResult<()> {
        let (owner, repo) = self.bound_owner_repo()?;
        let node_id = self.resolve_pr_node_id(&owner, &repo, pull_request_number)?;
        let url = format!("{}/graphql", self.base_url);
        let mutation = r#"mutation($pullRequestId: ID!) { convertPullRequestToDraft(input: { pullRequestId: $pullRequestId }) { clientMutationId } }"#;
        let payload = json!({
            "query": mutation,
            "variables": { "pullRequestId": node_id },
        });
        let resp = self.authed(self.http.post(&url)).json(&payload).send()?;
        let status = resp.status();
        if !status.is_success() {
            let _ = resp.text();
            return Err(PersistError::UpstreamStatus {
                status: status.as_u16(),
                op: "POST graphql (convertPullRequestToDraft)",
            });
        }
        let text = resp.text().map_err(PersistError::from)?;
        #[derive(Deserialize)]
        struct GraphqlBody {
            #[serde(default)]
            errors: Option<Vec<serde_json::Value>>,
        }
        let parsed: GraphqlBody = serde_json::from_str(&text).map_err(PersistError::from)?;
        if let Some(errs) = parsed.errors
            && !errs.is_empty()
        {
            return Err(PersistError::GraphqlError {
                op: "POST graphql (convertPullRequestToDraft)",
                count: errs.len(),
            });
        }
        Ok(())
    }

    fn patch_pr(&self, owner: &str, repo: &str, pr: u64, ready: bool) -> PersistResult<()> {
        let url = format!("{}/repos/{owner}/{repo}/pulls/{pr}", self.base_url);
        let body = json!({ "draft": !ready });
        let resp = self.authed(self.http.patch(&url)).json(&body).send()?;
        let status = resp.status();
        if status.is_success() {
            let _ = resp.text();
            Ok(())
        } else {
            let _ = resp.text();
            Err(PersistError::UpstreamStatus {
                status: status.as_u16(),
                op: "PATCH pulls/{n}",
            })
        }
    }

    /// Resolve a numeric PR number to GitHub's opaque node id.
    ///
    /// GraphQL identifies pull requests by their base64-shaped node id
    /// (e.g. `PR_kwDO...`); the numeric number is only usable through the
    /// REST endpoints. Reads `.node_id` from
    /// `GET /repos/{owner}/{repo}/pulls/{pr}`.
    fn resolve_pr_node_id(&self, owner: &str, repo: &str, pr: u64) -> PersistResult<String> {
        let url = format!("{}/repos/{owner}/{repo}/pulls/{pr}", self.base_url);
        let resp = self.authed(self.http.get(&url)).send()?;
        let status = resp.status();
        if !status.is_success() {
            let _ = resp.text();
            return Err(PersistError::UpstreamStatus {
                status: status.as_u16(),
                op: "GET pulls/{n} (resolve node_id)",
            });
        }
        #[derive(Deserialize)]
        struct PullResponse {
            node_id: Option<String>,
        }
        let body: PullResponse = resp.json().map_err(PersistError::from)?;
        body.node_id.ok_or(PersistError::MalformedResponse {
            op: "GET pulls/{n} (resolve node_id)",
            field: "node_id",
        })
    }

    fn enable_auto_merge(&self, pull_request_node_id: &str) -> PersistResult<()> {
        // GitHub's REST API does not expose auto-merge as a plain endpoint;
        // the sanctioned path is the `enablePullRequestAutoMerge` GraphQL
        // mutation. We route it to `{base_url}/graphql` so tests can share
        // the same tiny_http fixture as the REST calls.
        let url = format!("{}/graphql", self.base_url);
        let mutation = r#"mutation($pullRequestId: ID!) { enablePullRequestAutoMerge(input: { pullRequestId: $pullRequestId, mergeMethod: SQUASH }) { clientMutationId } }"#;
        let body = json!({
            "query": mutation,
            "variables": { "pullRequestId": pull_request_node_id },
        });
        let resp = self.authed(self.http.post(&url)).json(&body).send()?;
        let status = resp.status();
        // GraphQL uniformly returns 200 on protocol success even when the
        // mutation itself failed; the real signal lives in the response body's
        // top-level `errors` array. We must NOT surface the raw body — it can
        // echo internal repo/token metadata — so only the count of errors is
        // exposed to the caller.
        if !status.is_success() {
            let _ = resp.text();
            return Err(PersistError::UpstreamStatus {
                status: status.as_u16(),
                op: "POST graphql (enablePullRequestAutoMerge)",
            });
        }
        let text = resp.text().map_err(PersistError::from)?;
        #[derive(Deserialize)]
        struct GraphqlBody {
            #[serde(default)]
            errors: Option<Vec<serde_json::Value>>,
        }
        let parsed: GraphqlBody = serde_json::from_str(&text).map_err(PersistError::from)?;
        if let Some(errs) = parsed.errors
            && !errs.is_empty()
        {
            return Err(PersistError::GraphqlError {
                op: "POST graphql (enablePullRequestAutoMerge)",
                count: errs.len(),
            });
        }
        Ok(())
    }

    fn read_sha(resp: reqwest::blocking::Response, op: &'static str) -> PersistResult<String> {
        let status = resp.status();
        if !status.is_success() {
            let _ = resp.text();
            return Err(PersistError::UpstreamStatus {
                status: status.as_u16(),
                op,
            });
        }
        #[derive(Deserialize)]
        struct ShaOnly {
            sha: String,
        }
        let body: ShaOnly = resp.json().map_err(PersistError::from)?;
        Ok(body.sha)
    }
}

enum RefPatchOutcome {
    Ok,
    Conflict,
}
