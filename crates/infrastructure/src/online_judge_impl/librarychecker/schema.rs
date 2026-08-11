//! Serde types for the LibraryChecker REST API responses.
//!
//! Field names follow the upstream OpenAPI schema pinned at commit
//! `9a9ee40f4b284e56615f123fa69f06943d0b710c` of `yosupo06/library-checker-judge`.
//!
//! Sensitive fields (`source`, `compile_error`, `stderr`, `checker_out`) are
//! captured by serde but intentionally not exposed via Debug or public accessors
//! so they cannot leak into error summaries or logs.
//!
//! The types here are defined ahead of use (Tasks 2-3 in plan 058 will consume
//! them), so dead_code is expected in Task 1.
#![allow(dead_code)]

use serde::Deserialize;

/// Overview fields included in list and detail responses.
///
/// `user_name` and `submission_time` are absent from the required set in the
/// OpenAPI schema so they are optional here.
#[derive(Deserialize)]
pub(super) struct SubmissionOverview {
    pub(super) id: i32,
    pub(super) problem_name: String,
    pub(super) lang: String,
    pub(super) is_latest: bool,
    pub(super) status: String,
    pub(super) time: f32,
    pub(super) memory: i64,
    #[serde(default)]
    pub(super) user_name: Option<String>,
    #[serde(default)]
    pub(super) submission_time: Option<String>,
}

/// Response from `GET /submissions`.
#[derive(Deserialize)]
pub(super) struct SubmissionListResponse {
    pub(super) submissions: Vec<SubmissionOverview>,
    pub(super) count: i32,
}

/// One test-case result entry within `SubmissionInfoResponse`.
///
/// `stderr` and `checker_out` are captured by serde but excluded from Debug to
/// prevent leakage into error messages.
pub(super) struct SubmissionCaseResult {
    pub(super) case: String,
    pub(super) status: String,
    pub(super) time: f32,
    pub(super) memory: i64,
    // Not exposed: must not appear in error summaries.
    #[allow(dead_code)]
    stderr: Option<String>,
    #[allow(dead_code)]
    checker_out: Option<String>,
}

impl<'de> Deserialize<'de> for SubmissionCaseResult {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            case: String,
            status: String,
            time: f32,
            memory: i64,
            #[serde(default)]
            stderr: Option<String>,
            #[serde(default)]
            checker_out: Option<String>,
        }
        let r = Raw::deserialize(deserializer)?;
        Ok(Self {
            case: r.case,
            status: r.status,
            time: r.time,
            memory: r.memory,
            stderr: r.stderr,
            checker_out: r.checker_out,
        })
    }
}

/// Response from `GET /submissions/{id}`.
///
/// `source` and `compile_error` are captured but not exposed via Debug or
/// public accessors so they cannot appear in error summaries or logs.
pub(super) struct SubmissionInfoResponse {
    pub(super) overview: SubmissionOverview,
    // Not exposed: must not appear in error summaries.
    source: String,
    #[allow(dead_code)]
    compile_error: Option<String>,
    pub(super) can_rejudge: bool,
    pub(super) case_results: Option<Vec<SubmissionCaseResult>>,
}

impl SubmissionInfoResponse {
    /// Returns `sha256:<lowercase-hex>` of the submission source without
    /// exposing the source string. Used by the recovery adapter to compare
    /// against `RecoveryRequest.source_hash`.
    pub(super) fn source_sha256_hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(self.source.as_bytes());
        let hash = h.finalize();
        let mut hex = String::with_capacity(hash.len() * 2);
        for b in hash {
            hex.push_str(&format!("{b:02x}"));
        }
        format!("sha256:{hex}")
    }
}

impl<'de> Deserialize<'de> for SubmissionInfoResponse {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            overview: SubmissionOverview,
            source: String,
            #[serde(default)]
            compile_error: Option<String>,
            can_rejudge: bool,
            #[serde(default)]
            case_results: Option<Vec<SubmissionCaseResult>>,
        }
        let r = Raw::deserialize(deserializer)?;
        Ok(Self {
            overview: r.overview,
            source: r.source,
            compile_error: r.compile_error,
            can_rejudge: r.can_rejudge,
            case_results: r.case_results,
        })
    }
}

/// A LibraryChecker user record.
#[derive(Deserialize)]
pub(super) struct User {
    pub(super) name: String,
    pub(super) library_url: String,
    pub(super) is_developer: bool,
}

/// Response from `GET /auth/current_user`.
///
/// `user` is absent from the required fields in the OpenAPI schema, so it is
/// optional here.
#[derive(Deserialize)]
pub(super) struct CurrentUserInfoResponse {
    #[serde(default)]
    pub(super) user: Option<User>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_submission_list_fixture() {
        let json = include_str!("../../../tests/fixtures/librarychecker/submission-list.json");
        let resp: SubmissionListResponse = serde_json::from_str(json).expect("parse");
        assert_eq!(resp.count, 2);
        assert_eq!(resp.submissions.len(), 2);
        assert_eq!(resp.submissions[0].id, 1234);
        assert_eq!(resp.submissions[0].status, "AC");
        assert_eq!(resp.submissions[0].problem_name, "aplusb");
        assert_eq!(resp.submissions[0].lang, "rust");
        assert!(resp.submissions[0].is_latest);
        assert_eq!(resp.submissions[1].id, 1235);
        assert_eq!(resp.submissions[1].status, "WA");
    }

    #[test]
    fn parse_submission_pending_fixture() {
        let json = include_str!("../../../tests/fixtures/librarychecker/submission-pending.json");
        let resp: SubmissionInfoResponse = serde_json::from_str(json).expect("parse");
        assert_eq!(resp.overview.id, 1236);
        assert_eq!(resp.overview.status, "WJ");
        assert!(!resp.can_rejudge);
        // No case results for a pending submission.
        assert!(resp.case_results.is_none() || resp.case_results.as_ref().unwrap().is_empty());
    }

    #[test]
    fn parse_submission_accepted_fixture() {
        let json = include_str!("../../../tests/fixtures/librarychecker/submission-accepted.json");
        let resp: SubmissionInfoResponse = serde_json::from_str(json).expect("parse");
        assert_eq!(resp.overview.id, 1234);
        assert_eq!(resp.overview.status, "AC");
        let cases = resp.case_results.as_ref().expect("has case results");
        assert_eq!(cases.len(), 3);
        assert!(cases.iter().all(|c| c.status == "AC"));
        assert_eq!(cases[0].case, "example_00");
    }

    #[test]
    fn parse_current_user_info_with_user() {
        let json = r#"{"user":{"name":"alice","library_url":"","is_developer":false}}"#;
        let resp: CurrentUserInfoResponse = serde_json::from_str(json).expect("parse");
        let user = resp.user.expect("user present");
        assert_eq!(user.name, "alice");
        assert!(!user.is_developer);
    }

    #[test]
    fn parse_current_user_info_empty_is_ok() {
        // The OpenAPI schema does not require `user` in the response.
        let resp: CurrentUserInfoResponse = serde_json::from_str("{}").expect("parse");
        assert!(resp.user.is_none());
    }
}
