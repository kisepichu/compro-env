//! LibraryChecker (judge.yosupo.jp) `OnlineJudge` implementation.
//!
//! LibraryChecker has no contest concept, so each problem is modelled as a
//! single-problem "contest". The `contest_id` is namespaced (`librarychecker-aplusb`);
//! the bare problem name (`aplusb`) is recovered for API/bucket calls and submission.
//!
//! Auth uses Firebase (Bearer JWT): login exchanges email+password for an
//! `idToken` + `refreshToken`; Bearer calls refresh the short-lived `idToken`
//! on demand via the `refreshToken`. See docs/online_judges/librarychecker.md.
//!
//! Samples come from the public data bucket (the website itself is a SPA with no
//! server-rendered samples). The per-example `in`/`out` files are small — this is
//! NOT the full official test set — and are exactly what the official frontend uses.

pub mod auth;
pub mod problem;
pub mod schema;
pub mod submission;

use anyhow::{Context, Result};
use domain::entity::{Language, OJKind, Problem, Sample, Session};
use usecases::online_judge::{ContestMeta, CredentialKind, Credentials, OnlineJudge};

use auth::{
    FIREBASE_API_KEY, firebase_tokens, parse_current_username, parse_refresh_response,
    parse_signin_response,
};
use problem::{
    bare_problem_name, count_examples, example_in_url, example_out_url, extract_constraints,
    extract_input_format, info_toml_url, parse_problem_info, problem_info_url, task_md_url,
};

const REST_BASE: &str = "https://v3.api.judge.yosupo.jp";

pub struct LibraryChecker {
    client: reqwest::blocking::Client,
}

impl LibraryChecker {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder().build()?,
        })
    }

    /// Exchanges a refresh token for a fresh idToken via Firebase secure-token endpoint.
    fn refresh_id_token(&self, refresh_token: &str) -> Result<String> {
        let url = format!("https://securetoken.googleapis.com/v1/token?key={FIREBASE_API_KEY}");
        let body = self
            .client
            .post(&url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .send()?
            .error_for_status()
            .context(
                "session expired and token refresh failed. Run `ce login librarychecker` again.",
            )?
            .text()?;
        parse_refresh_response(&body)
    }

    /// Sends an authenticated request, retrying once with a refreshed token on 401/403.
    ///
    /// `build` is called with the bearer token to produce a fresh request; it may be
    /// invoked twice (original attempt + retry after refresh).
    fn send_authed(
        &self,
        session: &Session,
        build: impl Fn(&str) -> reqwest::blocking::RequestBuilder,
    ) -> Result<reqwest::blocking::Response> {
        let (id_token, refresh_token) = firebase_tokens(session)?;
        let resp = build(id_token).send()?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            || resp.status() == reqwest::StatusCode::FORBIDDEN
        {
            let fresh = self.refresh_id_token(refresh_token)?;
            return Ok(build(&fresh).send()?);
        }
        Ok(resp)
    }
}

impl OnlineJudge for LibraryChecker {
    fn name(&self) -> &str {
        "librarychecker"
    }

    fn credential_kind(&self) -> CredentialKind {
        CredentialKind::EmailPassword
    }

    fn default_lang_id(&self, language: &Language) -> Option<String> {
        // LibraryChecker's `lang` ids match common language names (e.g. "rust", "cpp"),
        // so default to the language name. Languages whose LC id differs (e.g. Python →
        // "python3"/"pypy3") must be set explicitly via config.toml. Invalid ids are
        // rejected by the submit API with a clear error.
        Some(language.as_str().to_string())
    }

    fn login(&self, credentials: &Credentials) -> Result<Session> {
        let (email, password) = match credentials {
            Credentials::Password {
                identifier,
                password,
            } => (identifier, password),
            Credentials::Cookie(_) => {
                anyhow::bail!("LibraryChecker login expects an email and password, not a cookie")
            }
        };
        let url = format!(
            "https://identitytoolkit.googleapis.com/v1/accounts:signInWithPassword?key={FIREBASE_API_KEY}"
        );
        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "email": email,
                "password": password,
                "returnSecureToken": true,
            }))
            .send()?;
        if !resp.status().is_success() {
            anyhow::bail!("LibraryChecker login failed: check your email and password");
        }
        let body = resp.text()?;
        let (id_token, refresh_token) = parse_signin_response(&body)?;
        Ok(Session::Firebase {
            online_judge: OJKind::LibraryChecker,
            id_token,
            refresh_token,
        })
    }

    fn whoami(&self, session: &Session) -> Result<String> {
        let url = format!("{REST_BASE}/auth/current_user");
        let resp = self.send_authed(session, |token| self.client.get(&url).bearer_auth(token))?;
        // After send_authed's refresh-and-retry, a remaining 401/403 means re-login is
        // needed. Other failures (5xx, 404, …) keep their original status so the message
        // is not misleadingly attributed to an expired session.
        let status = resp.status();
        let body = resp
            .error_for_status()
            .map_err(|e| {
                if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    anyhow::anyhow!("session expired. Run `ce login librarychecker` again.")
                } else {
                    anyhow::anyhow!(e)
                }
            })?
            .text()?;
        parse_current_username(&body)
    }

    fn get_contest_meta(&self, _contest_id: &str) -> Result<ContestMeta> {
        // LibraryChecker has no contest concept: no start time, no id hints.
        Ok(ContestMeta {
            start_time: None,
            problem_id_hints: vec![],
        })
    }

    fn get_problems_detail(
        &self,
        contest_id: &str,
        _session: Option<&Session>,
        _problem_id_hints: &[(String, String)],
    ) -> Result<Vec<Problem>> {
        // contest_id is namespaced ("librarychecker-aplusb"); the API/bucket use the
        // bare problem name ("aplusb").
        let name = bare_problem_name(contest_id);

        // Problem metadata (public; no auth).
        let info_body = self
            .client
            .get(problem_info_url(name))
            .send()?
            .error_for_status()
            .with_context(|| format!("problem \"{name}\" not found on LibraryChecker"))?
            .text()?;
        let info = parse_problem_info(&info_body)?;

        // info.toml gives the example count and statement parameters.
        let info_toml = self
            .client
            .get(info_toml_url(name, &info.overall_version))
            .send()?
            .error_for_status()
            .context("failed to fetch info.toml")?
            .text()?;
        let example_count = count_examples(&info_toml);

        // The statement source (task.md) holds the input format and constraints. The
        // rendered problem page is a client-side SPA, so we parse the Markdown source
        // directly instead of scraping HTML. Best-effort: fall back to None on failure.
        let (input_format_raw, constraints_raw) = match self
            .client
            .get(task_md_url(name, &info.overall_version))
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.text())
        {
            Ok(task_md) => (
                extract_input_format(&task_md),
                extract_constraints(&task_md, &info_toml),
            ),
            Err(_) => (None, None),
        };

        // Each example's in/out is a small file in the public bucket (NOT the full test set).
        let mut samples = Vec::new();
        for idx in 0..example_count {
            let input = self
                .client
                .get(example_in_url(name, &info.testcases_version, idx))
                .send()?
                .error_for_status()
                .with_context(|| format!("failed to fetch example_{idx:02}.in"))?
                .text()?;
            let output = self
                .client
                .get(example_out_url(name, &info.testcases_version, idx))
                .send()?
                .error_for_status()
                .with_context(|| format!("failed to fetch example_{idx:02}.out"))?
                .text()?;
            samples.push(Sample { input, output });
        }

        Ok(vec![Problem {
            // Bare problem name: used as the directory code and as the `problem` field
            // at submit time.
            id: name.to_string(),
            code: name.to_string(),
            title: info.title,
            samples,
            input_format_raw,
            constraints_raw,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_lang_id_uses_language_name() {
        let lc = LibraryChecker::new().expect("constructs");
        assert_eq!(
            lc.default_lang_id(&"rust".parse::<Language>().unwrap())
                .as_deref(),
            Some("rust")
        );
    }
}
