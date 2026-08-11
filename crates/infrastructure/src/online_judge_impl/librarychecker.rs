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

use anyhow::{Context, Result};
use domain::entity::{Language, OJKind, Problem, Sample, Session};
use serde::Deserialize;
use usecases::online_judge::{ContestMeta, CredentialKind, Credentials, OnlineJudge};

const REST_BASE: &str = "https://v3.api.judge.yosupo.jp";
const STORAGE_BASE: &str = "https://storage.googleapis.com/v2-prod-library-checker-data-public";
/// Public Firebase web API key (from the frontend's `.env.production`; not a secret).
const FIREBASE_API_KEY: &str = "AIzaSyCmpkoMVbKRDm2H0MJHB0iZ43uQtSqiLV0";

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

// ── Pure helpers (URL builders + response parsers) ─────────────────────────────

fn problem_info_url(name: &str) -> String {
    format!("{REST_BASE}/problems/{name}")
}

fn info_toml_url(name: &str, overall_version: &str) -> String {
    format!("{STORAGE_BASE}/v4/files/{name}/{overall_version}/{name}/info.toml")
}

fn example_in_url(name: &str, testcases_version: &str, idx: usize) -> String {
    // Example files are zero-padded to two digits (example_00, …, example_10, …).
    format!("{STORAGE_BASE}/v4/examples/{name}/{testcases_version}/in/example_{idx:02}.in")
}

fn example_out_url(name: &str, testcases_version: &str, idx: usize) -> String {
    format!("{STORAGE_BASE}/v4/examples/{name}/{testcases_version}/out/example_{idx:02}.out")
}

fn task_md_url(name: &str, overall_version: &str) -> String {
    format!("{STORAGE_BASE}/v4/files/{name}/{overall_version}/{name}/task.md")
}

/// Strips the "librarychecker-" namespace prefix to recover the bare problem name.
fn bare_problem_name(contest_id: &str) -> &str {
    contest_id
        .strip_prefix("librarychecker-")
        .unwrap_or(contest_id)
}

/// Extracts the input format from a task.md statement source.
///
/// The statement has a `## @{keyword.input}` heading followed by a fenced code block
/// holding the layout (e.g. `$A$ $B$`). We strip `$` so the result matches the
/// `$`-free format the input parser expects (e.g. `A B`, `N\nA_1 \dots A_N`).
/// Returns None if no input block is found or it is empty.
fn extract_input_format(task_md: &str) -> Option<String> {
    let block = fenced_block_after(task_md, "@{keyword.input}")?;
    let cleaned = block.replace('$', "");
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Extracts the constraints section from task.md, resolving `@{param.NAME}`
/// placeholders against the `[params]` table in info.toml and stripping `$`.
/// Returns None if no constraints section is found.
fn extract_constraints(task_md: &str, info_toml: &str) -> Option<String> {
    let section = section_after_heading(task_md, "@{keyword.constraints}")?;
    let resolved = resolve_params(section.trim(), info_toml).replace('$', "");
    let trimmed = resolved.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Returns the content of the first fenced ```` ``` ```` block that appears after a
/// line containing `heading_marker`.
fn fenced_block_after(md: &str, heading_marker: &str) -> Option<String> {
    let after_heading = &md[md.find(heading_marker)? + heading_marker.len()..];
    let fence_start = after_heading.find("```")?;
    let after_open = &after_heading[fence_start + 3..];
    // Skip to the end of the opening fence line (handles ```text etc.).
    let body_start = after_open.find('\n')? + 1;
    let body = &after_open[body_start..];
    let fence_end = body.find("```")?;
    Some(body[..fence_end].to_string())
}

/// Returns the text from just after the line containing `heading_marker` up to the
/// next `##` heading (or end of document).
fn section_after_heading(md: &str, heading_marker: &str) -> Option<String> {
    let pos = md.find(heading_marker)?;
    let after = &md[pos + heading_marker.len()..];
    // Skip the rest of the heading line.
    let nl = after.find('\n')?;
    let body = &after[nl + 1..];
    let end = body.find("\n##").unwrap_or(body.len());
    Some(body[..end].to_string())
}

/// Replaces `@{param.NAME}` placeholders with their values from the info.toml
/// `[params]` table. Unknown placeholders are left untouched.
fn resolve_params(text: &str, info_toml: &str) -> String {
    let params = match toml::from_str::<toml::Table>(info_toml) {
        Ok(t) => t,
        Err(_) => return text.to_string(),
    };
    let Some(params) = params.get("params").and_then(|v| v.as_table()) else {
        return text.to_string();
    };
    let mut out = text.to_string();
    for (name, value) in params {
        let needle = format!("@{{param.{name}}}");
        if out.contains(&needle) {
            let replacement = match value {
                toml::Value::Integer(i) => i.to_string(),
                toml::Value::Float(f) => f.to_string(),
                toml::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            out = out.replace(&needle, &replacement);
        }
    }
    out
}

struct ProblemInfo {
    title: String,
    overall_version: String,
    testcases_version: String,
}

#[derive(Deserialize)]
struct ProblemInfoResponse {
    title: String,
    overall_version: String,
    testcases_version: String,
}

fn parse_problem_info(json: &str) -> Result<ProblemInfo> {
    let r: ProblemInfoResponse =
        serde_json::from_str(json).context("failed to parse problem info response")?;
    Ok(ProblemInfo {
        title: r.title,
        overall_version: r.overall_version,
        testcases_version: r.testcases_version,
    })
}

/// Counts examples from info.toml: the `[[tests]]` entry named `example.in` has a
/// `number` field giving the example count. Returns 0 if absent.
fn count_examples(info_toml: &str) -> usize {
    let table: toml::Table = match toml::from_str(info_toml) {
        Ok(t) => t,
        Err(_) => return 0,
    };
    table
        .get("tests")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .find(|entry| entry.get("name").and_then(|n| n.as_str()) == Some("example.in"))
        .and_then(|entry| entry.get("number"))
        .and_then(|n| n.as_integer())
        .map(|n| n.max(0) as usize)
        .unwrap_or(0)
}

#[derive(Deserialize)]
struct SignInResponse {
    #[serde(rename = "idToken")]
    id_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: String,
}

fn parse_signin_response(json: &str) -> Result<(String, String)> {
    let r: SignInResponse =
        serde_json::from_str(json).context("failed to parse Firebase sign-in response")?;
    Ok((r.id_token, r.refresh_token))
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

fn parse_current_username(json: &str) -> Result<String> {
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

    #[test]
    fn url_builders_match_frontend_layout() {
        assert_eq!(
            problem_info_url("aplusb"),
            "https://v3.api.judge.yosupo.jp/problems/aplusb"
        );
        assert_eq!(
            info_toml_url("aplusb", "OV"),
            "https://storage.googleapis.com/v2-prod-library-checker-data-public/v4/files/aplusb/OV/aplusb/info.toml"
        );
        assert_eq!(
            example_in_url("aplusb", "TCV", 0),
            "https://storage.googleapis.com/v2-prod-library-checker-data-public/v4/examples/aplusb/TCV/in/example_00.in"
        );
        assert_eq!(
            example_out_url("aplusb", "TCV", 1),
            "https://storage.googleapis.com/v2-prod-library-checker-data-public/v4/examples/aplusb/TCV/out/example_01.out"
        );
        // Two-digit zero padding: idx >= 10 must not become a 3-digit "example_010".
        assert_eq!(
            example_in_url("aplusb", "TCV", 10),
            "https://storage.googleapis.com/v2-prod-library-checker-data-public/v4/examples/aplusb/TCV/in/example_10.in"
        );
    }

    #[test]
    fn count_examples_reads_example_in_number() {
        let info = r#"
[[tests]]
    name = "example.in"
    number = 2
[[tests]]
    name = "random.cpp"
    number = 10
"#;
        assert_eq!(count_examples(info), 2);
    }

    #[test]
    fn count_examples_absent_is_zero() {
        let info = "[[tests]]\n    name = \"random.cpp\"\n    number = 10\n";
        assert_eq!(count_examples(info), 0);
        assert_eq!(count_examples("not valid toml ["), 0);
    }

    #[test]
    fn parse_problem_info_extracts_versions_and_title() {
        let json = r#"{"title":"A + B","source_url":"https://x","time_limit":2,
            "version":"V","overall_version":"OV","testcases_version":"TCV"}"#;
        let info = parse_problem_info(json).expect("should parse");
        assert_eq!(info.title, "A + B");
        assert_eq!(info.overall_version, "OV");
        assert_eq!(info.testcases_version, "TCV");
    }

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
    fn bare_problem_name_strips_namespace() {
        assert_eq!(bare_problem_name("librarychecker-aplusb"), "aplusb");
        // Already-bare names (or other shapes) pass through unchanged.
        assert_eq!(bare_problem_name("aplusb"), "aplusb");
    }

    #[test]
    fn extract_input_format_strips_dollars() {
        let task = "## @{keyword.input}\n\n\n```\n$A$ $B$\n```\n\n## @{keyword.output}\n";
        assert_eq!(extract_input_format(task).as_deref(), Some("A B"));
    }

    #[test]
    fn extract_input_format_multiline_array() {
        let task = "## @{keyword.input}\n\n```\n$N$\n$A_1$ $A_2$ $\\dots$ $A_N$\n```\n## @{keyword.output}\n";
        assert_eq!(
            extract_input_format(task).as_deref(),
            Some("N\nA_1 A_2 \\dots A_N")
        );
    }

    #[test]
    fn extract_input_format_absent_is_none() {
        assert_eq!(extract_input_format("no input section here"), None);
    }

    #[test]
    fn extract_constraints_resolves_params_and_strips_dollars() {
        let task = "## @{keyword.constraints}\n\n- $0 \\leq A, B \\leq @{param.A_AND_B_MAX}$\n\n## @{keyword.input}\n";
        let info = "[params]\n    A_AND_B_MAX = 1_000_000_000\n";
        assert_eq!(
            extract_constraints(task, info).as_deref(),
            Some("- 0 \\leq A, B \\leq 1000000000")
        );
    }

    #[test]
    fn extract_constraints_absent_is_none() {
        assert_eq!(extract_constraints("nothing", "").as_deref(), None);
    }

    #[test]
    fn default_lang_id_uses_language_name() {
        let lc = LibraryChecker::new().expect("constructs");
        assert_eq!(
            lc.default_lang_id(&"rust".parse::<Language>().unwrap())
                .as_deref(),
            Some("rust")
        );
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
