//! LibraryChecker [`SubmissionStarter`] — posts the source to the LibraryChecker
//! REST API and returns a trackable handle.
//!
//! Spec § 9: LibraryChecker is `UnattendedTrackable / TestcaseDetails / BestEffort`.
//! The pollers and recovery adapters land with plan 058; this starter's job is
//! to enter `Trackable` state with the submission ID.

use anyhow::{Context, Result};
use chrono::Utc;
use domain::entity::{OJKind, Session};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use usecases::submission::{
    InfrastructureErrorKind, RecoveryMode, ResultDetailLevel, StartSubmissionError,
    SubmissionAdapterDescriptor, SubmissionHandle, SubmissionMode, SubmissionRequest,
    SubmissionStart, SubmissionStarter,
};

const REST_BASE: &str = "https://v3.api.judge.yosupo.jp";
const SUBMISSION_BASE: &str = "https://judge.yosupo.jp/submission";
/// Public Firebase web API key (from the frontend's `.env.production`; not a secret).
const FIREBASE_API_KEY: &str = "AIzaSyCmpkoMVbKRDm2H0MJHB0iZ43uQtSqiLV0";

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

/// Extracts the Firebase (id_token, refresh_token) pair from a session.
fn firebase_tokens(session: &Session) -> Result<(&str, &str)> {
    match session {
        Session::Firebase {
            id_token,
            refresh_token,
            ..
        } => Ok((id_token, refresh_token)),
        _ => anyhow::bail!("LibraryChecker requires a Firebase session"),
    }
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
