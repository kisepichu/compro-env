//! LibraryChecker [`SubmissionStarter`] and [`SubmissionPoller`] — posts the
//! source to the LibraryChecker REST API and polls for results.
//!
//! Spec § 9: LibraryChecker is `UnattendedTrackable / TestcaseDetails / BestEffort`.

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use domain::entity::{OJKind, Session};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use usecases::submission::{
    InfrastructureErrorKind, JudgeResult, JudgeVerdict, PollObservation, PollSubmissionError,
    RecoverSubmissionError, RecoveryMode, RecoveryOutcome, RecoveryRequest, ResultDetailLevel,
    StartSubmissionError, SubmissionAdapterDescriptor, SubmissionHandle, SubmissionMode,
    SubmissionPoller, SubmissionRecovery, SubmissionRequest, SubmissionStart, SubmissionStarter,
    TestcaseOutcome,
};

use super::schema;

use super::auth::{FIREBASE_API_KEY, firebase_tokens};

const REST_BASE: &str = "https://v3.api.judge.yosupo.jp";
const SUBMISSION_BASE: &str = "https://judge.yosupo.jp/submission";

pub struct LibraryCheckerStarter {
    client: reqwest::blocking::Client,
}

impl LibraryCheckerStarter {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder().build()?,
        })
    }

    /// Exchanges a refresh token for a fresh idToken via Firebase secure-token
    /// endpoint. Kept private to this crate so tokens do not escape.
    fn refresh_id_token(&self, refresh_token: &str) -> Result<String, StartSubmissionError> {
        let url = format!("https://securetoken.googleapis.com/v1/token?key={FIREBASE_API_KEY}");
        let resp = self
            .client
            .post(&url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .send()
            .map_err(|e| StartSubmissionError::Infrastructure {
                kind: InfrastructureErrorKind::Network,
                summary: sanitize(&format!("token refresh failed: {e}")),
            })?;
        if !resp.status().is_success() {
            return Err(StartSubmissionError::Infrastructure {
                kind: InfrastructureErrorKind::AuthenticationRejected,
                summary: "session expired and token refresh failed; run `ce login librarychecker`"
                    .to_string(),
            });
        }
        let body = resp
            .text()
            .map_err(|e| StartSubmissionError::Infrastructure {
                kind: InfrastructureErrorKind::InvalidResponse,
                summary: sanitize(&format!("read token refresh body: {e}")),
            })?;
        parse_refresh_response(&body).map_err(|e| StartSubmissionError::Infrastructure {
            kind: InfrastructureErrorKind::SchemaError,
            summary: sanitize(&format!("{e}")),
        })
    }
}

impl SubmissionStarter for LibraryCheckerStarter {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        SubmissionAdapterDescriptor {
            name: "librarychecker".to_string(),
            version: "1".to_string(),
            submission_mode: SubmissionMode::UnattendedTrackable,
            result_detail: ResultDetailLevel::TestcaseDetails,
            recovery_mode: RecoveryMode::BestEffort,
        }
    }

    fn start_submission(
        &self,
        request: &SubmissionRequest,
        session: Option<&Session>,
    ) -> Result<SubmissionStart, StartSubmissionError> {
        let session = session.ok_or(StartSubmissionError::Infrastructure {
            kind: InfrastructureErrorKind::CredentialsMissing,
            summary: "LibraryChecker submission requires login. Run `ce login librarychecker`."
                .to_string(),
        })?;

        let (id_token, refresh_token) =
            firebase_tokens(session).map_err(|e| StartSubmissionError::Infrastructure {
                kind: InfrastructureErrorKind::CredentialsMissing,
                summary: sanitize(&format!("{e}")),
            })?;

        let url = format!("{REST_BASE}/submit");
        let payload = serde_json::json!({
            "problem": &request.problem_id,
            "source": &request.source,
            "lang": &request.lang_id,
        });

        // First attempt with the cached idToken.
        let response = self
            .client
            .post(&url)
            .bearer_auth(id_token)
            .json(&payload)
            .send();
        // A send error may or may not have transmitted bytes — treat as
        // `AcceptanceUnknown` (spec §8.2).
        let response = match response {
            Ok(r) => r,
            Err(e) => {
                return Err(StartSubmissionError::from_transport_after_send(format!(
                    "submit request failed: {e}"
                )));
            }
        };

        let response = if response.status() == reqwest::StatusCode::UNAUTHORIZED
            || response.status() == reqwest::StatusCode::FORBIDDEN
        {
            // Refresh + retry once. After refresh, another send-time failure is
            // still `AcceptanceUnknown`.
            let fresh = self.refresh_id_token(refresh_token)?;
            self.client
                .post(&url)
                .bearer_auth(fresh)
                .json(&payload)
                .send()
                .map_err(|e| {
                    StartSubmissionError::from_transport_after_send(format!(
                        "submit request failed after refresh: {e}"
                    ))
                })?
        } else {
            response
        };

        let status = response.status();
        if !status.is_success() {
            // 4xx pre-accept: the API rejected the payload before queuing.
            // Treat 4xx (except 401/403 handled above) as ConfirmedNotAccepted;
            // 5xx as Infrastructure so the caller can retry.
            let body = response.text().unwrap_or_default();
            if status.is_client_error() {
                return Err(StartSubmissionError::ConfirmedNotAccepted {
                    summary: sanitize(&format!(
                        "LibraryChecker rejected submission ({status}): {body}"
                    )),
                });
            }
            return Err(StartSubmissionError::Infrastructure {
                kind: InfrastructureErrorKind::ServiceUnavailable,
                summary: sanitize(&format!("LibraryChecker {status}: {body}")),
            });
        }

        // 2xx: we own the acceptance. Read the ID + build the URL.
        let body = response.text().map_err(|e| {
            // The server accepted (2xx headers received) but we can't read the
            // body. The submission may exist without our knowing its ID —
            // `AcceptanceUnknown` per spec §8.2.
            StartSubmissionError::from_transport_after_send(format!("read submit body: {e}"))
        })?;
        let id = parse_submit_id(&body).map_err(|e| StartSubmissionError::Infrastructure {
            kind: InfrastructureErrorKind::SchemaError,
            summary: sanitize(&format!("{e}")),
        })?;

        Ok(SubmissionStart::Trackable {
            handle: SubmissionHandle {
                online_judge: OJKind::LibraryChecker,
                submission_id: id.to_string(),
                submission_url: submission_url(id),
                locator: Some(build_locator(
                    &request.problem_id,
                    &request.lang_id,
                    &request.source,
                )),
                submitted_at: Utc::now(),
            },
        })
    }
}

// ─── SubmissionPoller ─────────────────────────────────────────────────────

pub struct LibraryCheckerPoller {
    client: reqwest::blocking::Client,
    base_url: String,
}

impl LibraryCheckerPoller {
    pub fn new() -> Result<Self> {
        Self::with_base_url(REST_BASE.to_string())
    }

    /// Override the base URL — used in tests to point at a local fixture server.
    pub fn with_base_url(base: String) -> Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder().build()?,
            base_url: base,
        })
    }
}

impl SubmissionPoller for LibraryCheckerPoller {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        SubmissionAdapterDescriptor {
            name: "librarychecker".to_string(),
            version: "1".to_string(),
            submission_mode: SubmissionMode::UnattendedTrackable,
            result_detail: ResultDetailLevel::TestcaseDetails,
            recovery_mode: RecoveryMode::BestEffort,
        }
    }

    fn poll_submission(
        &self,
        handle: &SubmissionHandle,
        // GET /submissions/{id} has no security requirement in the OpenAPI schema,
        // so we do not send a bearer token and do not need to refresh credentials.
        _session: Option<&Session>,
    ) -> Result<PollObservation, PollSubmissionError> {
        let url = format!("{}/submissions/{}", self.base_url, handle.submission_id);
        let response =
            self.client
                .get(&url)
                .send()
                .map_err(|e| PollSubmissionError::Infrastructure {
                    kind: InfrastructureErrorKind::Network,
                    summary: sanitize(&format!("network error: {e}")),
                })?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(PollSubmissionError::HandleNotFound {
                summary: sanitize(&format!("submission {} not found", handle.submission_id)),
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            // The public poll endpoint should not require auth, but if the OJ
            // starts gating it, classify as AuthenticationRejected so upstream
            // stops the retry loop and surfaces an operator-repairable state.
            return Err(PollSubmissionError::Infrastructure {
                kind: InfrastructureErrorKind::AuthenticationRejected,
                summary: sanitize(&format!(
                    "LibraryChecker rejected poll auth status={}",
                    status.as_u16()
                )),
            });
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());
            let summary = if let Some(secs) = retry_after {
                sanitize(&format!("rate limited (retry-after: {secs})"))
            } else {
                sanitize("rate limited")
            };
            return Err(PollSubmissionError::Infrastructure {
                kind: InfrastructureErrorKind::RateLimited,
                summary,
            });
        }
        if status.is_server_error() {
            return Err(PollSubmissionError::Infrastructure {
                kind: InfrastructureErrorKind::ServiceUnavailable,
                summary: sanitize(&format!("LibraryChecker status={}", status.as_u16())),
            });
        }
        if !status.is_success() {
            return Err(PollSubmissionError::Infrastructure {
                kind: InfrastructureErrorKind::InvalidResponse,
                summary: sanitize(&format!(
                    "unexpected client error status={}",
                    status.as_u16()
                )),
            });
        }

        let body = response
            .text()
            .map_err(|e| PollSubmissionError::Infrastructure {
                kind: InfrastructureErrorKind::Network,
                summary: sanitize(&format!("network error: {e}")),
            })?;

        let info: schema::SubmissionInfoResponse =
            serde_json::from_str(&body).map_err(|_| PollSubmissionError::Infrastructure {
                kind: InfrastructureErrorKind::SchemaError,
                summary: sanitize("malformed submission info response"),
            })?;

        map_observation(info)
    }
}

fn map_verdict(status: &str) -> JudgeVerdict {
    match status {
        "AC" => JudgeVerdict::Accepted,
        "WA" => JudgeVerdict::WrongAnswer,
        "TLE" => JudgeVerdict::TimeLimitExceeded,
        "MLE" => JudgeVerdict::MemoryLimitExceeded,
        "RE" => JudgeVerdict::RuntimeError,
        "CE" => JudgeVerdict::CompilationError,
        "IE" => JudgeVerdict::InternalError,
        other => JudgeVerdict::Other(other.to_string()),
    }
}

/// Converts seconds (f32) to milliseconds, rounding up. Returns None for
/// negative or non-finite values (LC sends -1 as a sentinel before judging).
fn map_time_ms(time: f32) -> Option<u32> {
    if !time.is_finite() || time < 0.0 {
        return None;
    }
    Some((time * 1000.0_f32).ceil() as u32)
}

/// Converts bytes (i64) to KiB, rounding up. Returns None for negative values.
fn map_memory_kib(memory: i64) -> Option<u32> {
    if memory < 0 {
        return None;
    }
    Some(((memory as f64) / 1024.0).ceil() as u32)
}

fn map_observation(
    info: schema::SubmissionInfoResponse,
) -> Result<PollObservation, PollSubmissionError> {
    match info.overview.status.as_str() {
        "WJ" => return Ok(PollObservation::Queued),
        "Judging" | "J" => return Ok(PollObservation::Judging),
        _ => {}
    }
    let verdict = map_verdict(&info.overview.status);
    let testcases = info
        .case_results
        .unwrap_or_default()
        .into_iter()
        .map(|c| TestcaseOutcome {
            name: c.case,
            verdict: map_verdict(&c.status),
            time_ms: map_time_ms(c.time),
            memory_kib: map_memory_kib(c.memory),
        })
        .collect();
    Ok(PollObservation::Completed(JudgeResult {
        verdict,
        testcases,
    }))
}

// ─── SubmissionRecovery ───────────────────────────────────────────────────

/// Maximum number of `GET /submissions` list pages to fetch per recovery attempt.
/// An operational safety net to prevent runaway iteration (spec §8.2 / plan 058).
const MAX_RECOVERY_PAGES: u32 = 3;

/// Submissions per page. LibraryChecker's API maximum is 1000; 100 is conservative.
const PAGE_SIZE: i32 = 100;

/// Grace window in seconds: submissions this many seconds before
/// `submitted_at_lower_bound` are still considered potentially in-window.
const GRACE_SECS: i64 = 60;

pub struct LibraryCheckerRecovery {
    client: reqwest::blocking::Client,
    base_url: String,
}

impl LibraryCheckerRecovery {
    pub fn new() -> Result<Self> {
        Self::with_base_url(REST_BASE.to_string())
    }

    /// Override the base URL — used in tests to point at a local fixture server.
    pub fn with_base_url(base: String) -> Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder().build()?,
            base_url: base,
        })
    }

    /// Fetches the currently-authenticated user's username.
    /// Returns `None` on any transport or parse failure (best-effort).
    fn get_current_username(&self, id_token: &str) -> Option<String> {
        let url = format!("{}/auth/current_user", self.base_url);
        let resp = self.client.get(&url).bearer_auth(id_token).send().ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body = resp.text().ok()?;
        let r: schema::CurrentUserInfoResponse = serde_json::from_str(&body).ok()?;
        r.user.map(|u| u.name)
    }

    /// Fetches one page of the submission list. Returns `None` on failure.
    fn list_page(
        &self,
        problem_id: &str,
        lang_id: &str,
        user: &str,
        skip: i32,
    ) -> Option<schema::SubmissionListResponse> {
        let url = format!("{}/submissions", self.base_url);
        // Delegate URL encoding to reqwest so problem/language/username values
        // that contain '+', '@', or spaces round-trip safely.
        let resp = self
            .client
            .get(&url)
            .query(&[
                ("problem", problem_id),
                ("lang", lang_id),
                ("user", user),
                ("order", "-id"),
                ("limit", &PAGE_SIZE.to_string()),
                ("skip", &skip.to_string()),
            ])
            .send()
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body = resp.text().ok()?;
        serde_json::from_str(&body).ok()
    }

    /// Fetches the full detail for one submission. Returns `None` on failure.
    fn fetch_detail(&self, id: i32) -> Option<schema::SubmissionInfoResponse> {
        let url = format!("{}/submissions/{}", self.base_url, id);
        let resp = self.client.get(&url).send().ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body = resp.text().ok()?;
        serde_json::from_str(&body).ok()
    }
}

impl SubmissionRecovery for LibraryCheckerRecovery {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        SubmissionAdapterDescriptor {
            name: "librarychecker".to_string(),
            version: "1".to_string(),
            submission_mode: SubmissionMode::UnattendedTrackable,
            result_detail: ResultDetailLevel::TestcaseDetails,
            recovery_mode: RecoveryMode::BestEffort,
        }
    }

    fn recover_submission(
        &self,
        request: &RecoveryRequest,
        session: Option<&Session>,
    ) -> Result<RecoveryOutcome, RecoverSubmissionError> {
        let session = session.ok_or_else(|| RecoverSubmissionError::Infrastructure {
            kind: InfrastructureErrorKind::CredentialsMissing,
            summary: sanitize(
                "LibraryChecker recovery requires a session. Run `ce login librarychecker`.",
            ),
        })?;

        // Non-Firebase sessions cannot provide a Bearer token for /auth/current_user.
        let id_token = match firebase_tokens(session) {
            Ok((id, _)) => id,
            Err(_) => return Ok(RecoveryOutcome::AcceptanceUnknown),
        };

        let current_user = match self.get_current_username(id_token) {
            Some(u) => u,
            None => return Ok(RecoveryOutcome::AcceptanceUnknown),
        };

        let lower_bound = request.submitted_at_lower_bound;
        let mut seen_ids: HashSet<i32> = HashSet::new();
        // (id, submission_time_rfc3339)
        let mut matches: Vec<(i32, Option<String>)> = Vec::new();
        let mut stop_pagination = false;

        for page in 0..MAX_RECOVERY_PAGES {
            let skip = page as i32 * PAGE_SIZE;
            let list =
                match self.list_page(&request.problem_id, &request.lang_id, &current_user, skip) {
                    Some(l) => l,
                    None => return Ok(RecoveryOutcome::AcceptanceUnknown),
                };

            if list.submissions.is_empty() {
                break;
            }

            for overview in &list.submissions {
                // Time-based pagination cutoff checked before per-row field guards so
                // any old submission (even with wrong problem/lang) stops the scan.
                if let Some(lb) = lower_bound
                    && let Some(time_str) = &overview.submission_time
                    && let Ok(sub_time) = DateTime::parse_from_rfc3339(time_str)
                {
                    let sub_time: DateTime<Utc> = sub_time.with_timezone(&Utc);
                    if sub_time < lb - Duration::seconds(GRACE_SECS) {
                        stop_pagination = true;
                        break;
                    }
                }

                // Per-row field guards — defence against server bugs or stale cache.
                if overview.user_name.as_deref() != Some(current_user.as_str()) {
                    continue;
                }
                if overview.problem_name != request.problem_id {
                    continue;
                }
                if overview.lang != request.lang_id {
                    continue;
                }

                // Deduplicate by submission ID (list may return duplicates).
                if !seen_ids.insert(overview.id) {
                    continue;
                }

                // Fetch detail and compare source hash. Transport failures abort
                // recovery: zero or more candidates is always AcceptanceUnknown.
                let detail = match self.fetch_detail(overview.id) {
                    Some(d) => d,
                    None => return Ok(RecoveryOutcome::AcceptanceUnknown),
                };

                if detail.source_sha256_hash() == request.source_hash {
                    matches.push((overview.id, overview.submission_time.clone()));
                }
            }

            if stop_pagination {
                break;
            }
        }

        if matches.len() == 1 {
            let (id, sub_time_str) = &matches[0];
            let submitted_at = sub_time_str
                .as_deref()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);
            Ok(RecoveryOutcome::Recovered {
                handle: SubmissionHandle {
                    online_judge: OJKind::LibraryChecker,
                    submission_id: id.to_string(),
                    submission_url: format!("{SUBMISSION_BASE}/{id}"),
                    locator: Some(format!(
                        "{}:{}:{}",
                        request.problem_id, request.lang_id, request.source_hash
                    )),
                    submitted_at,
                },
            })
        } else {
            Ok(RecoveryOutcome::AcceptanceUnknown)
        }
    }
}

// ─── URL helpers ──────────────────────────────────────────────────────────

fn submission_url(id: i64) -> String {
    format!("{SUBMISSION_BASE}/{id}")
}

/// Composite locator used by the future recovery adapter (plan 058): problem +
/// language + submitted-source hash. Stable across restarts.
fn build_locator(problem_id: &str, lang_id: &str, source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    let hash = hasher.finalize();
    let mut hex = String::with_capacity(hash.len() * 2);
    for b in hash {
        hex.push_str(&format!("{b:02x}"));
    }
    format!("{problem_id}:{lang_id}:sha256:{hex}")
}

fn sanitize(input: &str) -> String {
    usecases::submission::sanitize_summary(input)
}

#[derive(Deserialize)]
struct RefreshResponse {
    id_token: String,
}

fn parse_refresh_response(json: &str) -> Result<String> {
    let r: RefreshResponse =
        serde_json::from_str(json).context("failed to parse Firebase token-refresh response")?;
    Ok(r.id_token)
}

fn parse_submit_id(json: &str) -> Result<i64> {
    let v: serde_json::Value =
        serde_json::from_str(json).context("failed to parse submit response")?;
    v.get("id")
        .and_then(|id| id.as_i64())
        .ok_or_else(|| anyhow::anyhow!("submit response missing `id`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Characterization: submission URLs remain `SUBMISSION_BASE/{id}`.
    #[test]
    fn submission_url_shape_matches_pre_migration() {
        assert_eq!(submission_url(42), "https://judge.yosupo.jp/submission/42");
    }

    #[test]
    fn descriptor_declares_unattended_trackable() {
        let starter = LibraryCheckerStarter::new().unwrap();
        let d = starter.descriptor();
        assert_eq!(d.submission_mode, SubmissionMode::UnattendedTrackable);
        assert_eq!(d.recovery_mode, RecoveryMode::BestEffort);
        assert_eq!(d.result_detail, ResultDetailLevel::TestcaseDetails);
        assert!(d.supports_unattended_verify());
    }

    #[test]
    fn parse_submit_id_reads_id() {
        assert_eq!(parse_submit_id(r#"{"id":12345}"#).expect("parse"), 12345);
        assert!(parse_submit_id("{}").is_err());
    }

    #[test]
    fn build_locator_includes_hash_prefix() {
        let locator = build_locator("aplusb", "cpp", "int main(){}");
        assert!(locator.starts_with("aplusb:cpp:sha256:"));
        // sha256 hex is 64 chars.
        assert_eq!(locator.len(), "aplusb:cpp:sha256:".len() + 64);
    }
}
