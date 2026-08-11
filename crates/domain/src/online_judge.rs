//! Online judge capability declarations (spec §9).
//!
//! Capabilities are represented as small enums so incompatible combinations
//! cannot be expressed. `ce verify` requires `unattended_trackable`; future
//! commands such as `ce submit --watch` may also accept `interactive_trackable`.

use serde::{Deserialize, Serialize};

/// How submissions are dispatched and later tracked (spec §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionMode {
    UnattendedTrackable,
    InteractiveTrackable,
    InteractiveUntrackable,
    Unsupported,
}

/// Detail level the OJ exposes about the judged result (spec §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultDetail {
    OverallOnly,
    SummaryMetrics,
    TestcaseDetails,
}

/// Ability to recover a handle for an attempt that lost its `Starting`
/// context (spec §8.2, §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryMode {
    Exact,
    BestEffort,
    None,
}

/// Declared capabilities of an OJ adapter (spec §9).
///
/// Stored on `CompletedState` and `UnavailableState` so a stale adapter
/// upgrade can be detected without inspecting infrastructure code paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionCapabilities {
    pub submission_mode: SubmissionMode,
    pub result_detail: ResultDetail,
    pub recovery_mode: RecoveryMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submission_mode_serialises_to_snake_case() {
        let value = SubmissionMode::UnattendedTrackable;
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(json, "\"unattended_trackable\"");
    }

    #[test]
    fn result_detail_and_recovery_mode_use_snake_case() {
        assert_eq!(
            serde_json::to_string(&ResultDetail::TestcaseDetails).unwrap(),
            "\"testcase_details\"",
        );
        assert_eq!(
            serde_json::to_string(&RecoveryMode::BestEffort).unwrap(),
            "\"best_effort\"",
        );
    }

    #[test]
    fn submission_capabilities_reject_unknown_fields() {
        let json = r#"{
            "submission_mode": "unattended_trackable",
            "result_detail": "overall_only",
            "recovery_mode": "exact",
            "extra": true
        }"#;
        let parsed: Result<SubmissionCapabilities, _> = serde_json::from_str(json);
        assert!(parsed.is_err());
    }
}
