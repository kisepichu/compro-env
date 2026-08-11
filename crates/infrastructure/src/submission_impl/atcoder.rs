//! AtCoder [`SubmissionStarter`] — builds the browser URL that the Tampermonkey
//! userscript picks up (see `docs/userscript.md`).
//!
//! Spec § 9: AtCoder is `InteractiveUntrackable / OverallOnly / None`. The
//! starter returns [`SubmissionStart::UserActionRequired`]; there is no poller
//! or recovery adapter.

use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use domain::entity::Session;
use usecases::submission::{
    RecoveryMode, ResultDetailLevel, StartSubmissionError, SubmissionAdapterDescriptor,
    SubmissionMode, SubmissionRequest, SubmissionStart, SubmissionStarter,
};

/// Maximum URL fragment size AtCoder will accept via a browser fragment.
///
/// Kept identical to the previous `OnlineJudge::submit` implementation so
/// characterization tests continue to pin the exact size limit.
const MAX_FRAGMENT_BYTES: usize = 32 * 1024;

pub struct AtCoderStarter;

impl AtCoderStarter {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
}

impl SubmissionStarter for AtCoderStarter {
    fn descriptor(&self) -> SubmissionAdapterDescriptor {
        SubmissionAdapterDescriptor {
            name: "atcoder".to_string(),
            version: "1".to_string(),
            submission_mode: SubmissionMode::InteractiveUntrackable,
            result_detail: ResultDetailLevel::OverallOnly,
            recovery_mode: RecoveryMode::None,
        }
    }

    fn start_submission(
        &self,
        request: &SubmissionRequest,
        _session: Option<&Session>,
    ) -> Result<SubmissionStart, StartSubmissionError> {
        Ok(SubmissionStart::UserActionRequired {
            url: build_submit_url(
                &request.contest_id,
                &request.problem_id,
                &request.lang_id,
                &request.source,
            )?,
        })
    }
}

/// Encodes `{lang_id, source}` as URL-safe base64 JSON and embeds it in the
/// URL fragment. The Tampermonkey userscript reads this fragment and auto-fills
/// the submit form (`docs/userscript.md`).
fn build_submit_url(
    contest_id: &str,
    problem_id: &str,
    lang_id: &str,
    source: &str,
) -> Result<String, StartSubmissionError> {
    let payload = serde_json::json!({
        "lang_id": lang_id,
        "source": source,
    })
    .to_string();
    let fragment = format!("ce={}", URL_SAFE.encode(payload.as_bytes()));

    // Measure the actual encoded fragment (JSON escaping of control characters
    // can expand a byte up to 6×, so a source-length estimate is unsafe).
    if fragment.len() > MAX_FRAGMENT_BYTES {
        return Err(StartSubmissionError::ConfirmedNotAccepted {
            summary: format!(
                "source file is too large to submit via URL fragment \
                 (fragment {} bytes, max {MAX_FRAGMENT_BYTES})",
                fragment.len()
            ),
        });
    }

    // Percent-encode contest_id/problem_id via reqwest::Url so URL-reserved
    // characters do not break the URL.
    let mut url = reqwest::Url::parse("https://atcoder.jp/").expect("base URL is valid");
    url.path_segments_mut()
        .expect("base URL is cannot-be-a-base")
        .push("contests")
        .push(contest_id)
        .push("submit");
    url.query_pairs_mut()
        .append_pair("taskScreenName", problem_id);
    url.set_fragment(Some(&fragment));
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::entity::OJKind;

    fn request() -> SubmissionRequest {
        SubmissionRequest {
            online_judge: OJKind::AtCoder,
            contest_id: "abc001".to_string(),
            problem_id: "abc001_a".to_string(),
            lang_id: "4026".to_string(),
            source: "fn main() {}".to_string(),
        }
    }

    /// Characterization: the URL matches the pre-migration `OnlineJudge::submit`
    /// output shape byte-for-byte — contest and problem segments, taskScreenName
    /// query, and a `#ce=<base64-JSON>` fragment.
    #[test]
    fn starter_url_shape_matches_pre_migration() {
        let starter = AtCoderStarter::new().unwrap();
        let start = starter
            .start_submission(&request(), None)
            .expect("start ok");
        let url = match start {
            SubmissionStart::UserActionRequired { url } => url,
            other => panic!("expected UserActionRequired, got {other:?}"),
        };
        assert!(url.starts_with("https://atcoder.jp/contests/abc001/submit?"));
        assert!(url.contains("taskScreenName=abc001_a"));
        let fragment = url.split('#').nth(1).expect("fragment present");
        let encoded = fragment
            .strip_prefix("ce=")
            .expect("fragment starts with ce=");
        let decoded = URL_SAFE.decode(encoded).expect("valid base64");
        let payload: serde_json::Value = serde_json::from_slice(&decoded).expect("valid JSON");
        assert_eq!(payload["lang_id"], "4026");
        assert_eq!(payload["source"], "fn main() {}");
    }

    #[test]
    fn descriptor_declares_interactive_untrackable() {
        let starter = AtCoderStarter::new().unwrap();
        let d = starter.descriptor();
        assert_eq!(d.submission_mode, SubmissionMode::InteractiveUntrackable);
        assert_eq!(d.recovery_mode, RecoveryMode::None);
        assert_eq!(d.result_detail, ResultDetailLevel::OverallOnly);
        assert!(!d.supports_unattended_verify());
    }

    /// Oversized sources map to `ConfirmedNotAccepted` (safe to bail; the OJ
    /// never sees the bytes), never `AcceptanceUnknown`.
    #[test]
    fn oversized_source_is_confirmed_not_accepted() {
        let starter = AtCoderStarter::new().unwrap();
        let request = SubmissionRequest {
            source: "x".repeat(200 * 1024),
            ..request()
        };
        let err = starter
            .start_submission(&request, None)
            .expect_err("oversized source should fail");
        match err {
            StartSubmissionError::ConfirmedNotAccepted { summary } => {
                assert!(summary.contains("too large"));
            }
            other => panic!("expected ConfirmedNotAccepted, got {other:?}"),
        }
    }
}
