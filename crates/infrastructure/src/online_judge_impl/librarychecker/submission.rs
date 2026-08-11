//! LibraryChecker [`SubmissionStarter`] and [`SubmissionPoller`] — posts the
//! source to the LibraryChecker REST API and polls for results.
//!
//! Spec § 9: LibraryChecker is `UnattendedTrackable / TestcaseDetails / BestEffort`.

use anyhow::{Context, Result};
use chrono::Utc;
use domain::entity::{OJKind, Session};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use usecases::submission::{
    InfrastructureErrorKind, JudgeResult, JudgeVerdict, PollObservation, PollSubmissionError,
    RecoveryMode, ResultDetailLevel, StartSubmissionError, SubmissionAdapterDescriptor,
    SubmissionHandle, SubmissionMode, SubmissionPoller, SubmissionRequest, SubmissionStart,
    SubmissionStarter, TestcaseOutcome,
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
