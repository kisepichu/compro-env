//! Firebase authentication helpers for LibraryChecker.

use anyhow::{Context, Result};
use domain::entity::Session;
use serde::Deserialize;

/// Public Firebase web API key (from the frontend's `.env.production`; not a secret).
pub(super) const FIREBASE_API_KEY: &str = "AIzaSyCmpkoMVbKRDm2H0MJHB0iZ43uQtSqiLV0";

/// Extracts the Firebase (id_token, refresh_token) pair from a session.
pub(super) fn firebase_tokens(session: &Session) -> Result<(&str, &str)> {
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
struct SignInResponse {
    #[serde(rename = "idToken")]
    id_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: String,
}

pub(super) fn parse_signin_response(json: &str) -> Result<(String, String)> {
    let r: SignInResponse =
        serde_json::from_str(json).context("failed to parse Firebase sign-in response")?;
    Ok((r.id_token, r.refresh_token))
}

#[derive(Deserialize)]
pub(super) struct RefreshResponse {
    pub(super) id_token: String,
}

pub(super) fn parse_refresh_response(json: &str) -> Result<String> {
    let r: RefreshResponse =
        serde_json::from_str(json).context("failed to parse Firebase token-refresh response")?;
    Ok(r.id_token)
}

pub(super) fn parse_current_username(json: &str) -> Result<String> {
    let v: serde_json::Value =
        serde_json::from_str(json).context("failed to parse current_user response")?;
    v.get("user")
        .and_then(|u| u.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("not logged in. Run `ce login librarychecker`."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::entity::OJKind;

    #[test]
    fn parse_signin_response_extracts_tokens() {
        let json = r#"{"idToken":"ID","refreshToken":"RF","expiresIn":"3600"}"#;
        let (id, rf) = parse_signin_response(json).expect("should parse");
        assert_eq!(id, "ID");
        assert_eq!(rf, "RF");
    }

    #[test]
    fn parse_refresh_response_uses_snake_case() {
        // The secure-token endpoint returns snake_case, unlike sign-in.
        let json = r#"{"id_token":"NEW","refresh_token":"RF","expires_in":"3600"}"#;
        assert_eq!(parse_refresh_response(json).expect("parse"), "NEW");
    }

    #[test]
    fn parse_current_username_reads_nested_name() {
        let json = r#"{"user":{"name":"alice","is_developer":false}}"#;
        assert_eq!(parse_current_username(json).expect("parse"), "alice");
    }

    #[test]
    fn parse_current_username_errors_when_no_user() {
        assert!(parse_current_username("{}").is_err());
    }

    #[test]
    fn firebase_tokens_rejects_cookie_session() {
        let s = Session::Cookie {
            online_judge: OJKind::AtCoder,
            cookie: "c".to_string(),
        };
        assert!(firebase_tokens(&s).is_err());
    }
}
