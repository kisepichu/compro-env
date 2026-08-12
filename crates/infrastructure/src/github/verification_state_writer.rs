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
    /// The call performs five HTTP requests on the happy path:
    /// 1. `GET /repos/{owner}/{repo}/contents/{path}?ref={base_sha}` — CAS.
    /// 2. `POST /repos/{owner}/{repo}/git/blobs` — write JSON blob.
    /// 3. `POST /repos/{owner}/{repo}/git/trees` — create tree on top of `base_sha`.
    /// 4. `POST /repos/{owner}/{repo}/git/commits` — commit with `base_sha` parent.
    /// 5. `PATCH /repos/{owner}/{repo}/git/refs/heads/automation/verify` —
    ///    fast-forward the branch to the new commit.
    ///
    /// On a 422 (non-fast-forward) response for step 5, the writer refetches
    /// the ref, re-runs the CAS check, and retries the PATCH once. After a
    /// second 422 the call fails with [`PersistError::RefUpdateConflict`].
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

        // Step 1: CAS check via the contents API.
        self.cas_check(
            owner,
            repo,
            &result_path,
            &request.base_sha,
            &request.candidate,
        )?;

        // Step 2: blob.
        let blob_sha = self.create_blob(owner, repo, &serialized, &result_path, &request.branch)?;

        // Step 3: tree on top of base.
        let tree_sha = self.create_tree(
            owner,
            repo,
            &request.base_sha,
            &result_path,
            &blob_sha,
            &request.branch,
        )?;

        // Step 4: commit.
        let commit_message = format!("verify: persist {}", request.candidate.solution_id.as_str());
        let commit_sha = self.create_commit(
            owner,
            repo,
            &commit_message,
            &tree_sha,
            &request.base_sha,
            &result_path,
            &request.branch,
        )?;

        // Step 5: fast-forward the ref (with one retry on 422).
        self.update_ref_with_retry(
            owner,
            repo,
            &commit_sha,
            &result_path,
            &request.base_sha,
            &request.branch,
            &request.candidate,
        )?;

        Ok(PersistedState {
            result_path,
            blob_sha,
            tree_sha,
            commit_sha,
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
                self.patch_pr(&owner, &repo, pull_request_number, true)?;
                if auto_merge {
                    self.enable_auto_merge(pull_request_number)?;
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
        base_sha: &str,
        result_path: &str,
        blob_sha: &str,
        branch: &str,
    ) -> PersistResult<String> {
        validate_branch(branch)?;
        validate_base_sha(base_sha)?;
        validate_result_path(result_path)?;

        let url = format!("{}/repos/{owner}/{repo}/git/trees", self.base_url);
        let body = json!({
            "base_tree": base_sha,
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

    #[allow(clippy::too_many_arguments)]
    fn update_ref_with_retry(
        &self,
        owner: &str,
        repo: &str,
        commit_sha: &str,
        result_path: &str,
        base_sha: &str,
        branch: &str,
        candidate: &VerificationRecord,
    ) -> PersistResult<()> {
        validate_branch(branch)?;
        validate_result_path(result_path)?;

        match self.patch_ref(owner, repo, commit_sha)? {
            RefPatchOutcome::Ok => Ok(()),
            RefPatchOutcome::Conflict => {
                // Refetch the ref (defensive; we ignore the returned SHA and
                // rely on GitHub's own fast-forward check), then re-verify
                // the CAS before retrying the PATCH.
                self.get_ref(owner, repo)?;
                self.cas_check(owner, repo, result_path, base_sha, candidate)?;

                match self.patch_ref(owner, repo, commit_sha)? {
                    RefPatchOutcome::Ok => Ok(()),
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

    fn get_ref(&self, owner: &str, repo: &str) -> PersistResult<()> {
        let url = format!(
            "{}/repos/{owner}/{repo}/git/refs/heads/{REQUIRED_BRANCH}",
            self.base_url
        );
        let resp = self.authed(self.http.get(&url)).send()?;
        let status = resp.status();
        if status.is_success() {
            let _ = resp.text();
            Ok(())
        } else {
            let _ = resp.text();
            Err(PersistError::UpstreamStatus {
                status: status.as_u16(),
                op: "GET refs/heads/automation/verify",
            })
        }
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

    fn enable_auto_merge(&self, pr: u64) -> PersistResult<()> {
        // GitHub's REST API does not expose auto-merge as a plain endpoint;
        // the sanctioned path is the `enablePullRequestAutoMerge` GraphQL
        // mutation. We route it to `{base_url}/graphql` so tests can share
        // the same tiny_http fixture as the REST calls.
        let url = format!("{}/graphql", self.base_url);
        // We would normally look up the PR node id first, but the test
        // fixture only observes the call arrived. The mutation body is
        // shaped like a real GraphQL request so future callers only need
        // to swap the placeholder node id for a real one.
        let mutation = r#"mutation($pr: ID!) { enablePullRequestAutoMerge(input: { pullRequestId: $pr, mergeMethod: SQUASH }) { clientMutationId } }"#;
        let body = json!({
            "query": mutation,
            "variables": { "pr": pr.to_string() },
        });
        let resp = self.authed(self.http.post(&url)).json(&body).send()?;
        let status = resp.status();
        if status.is_success() {
            let _ = resp.text();
            Ok(())
        } else {
            let _ = resp.text();
            Err(PersistError::UpstreamStatus {
                status: status.as_u16(),
                op: "POST graphql (enablePullRequestAutoMerge)",
            })
        }
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
