//! Persisted verification record model (spec §8, §10, §11).
//!
//! The domain owns normalized verdicts, capability snapshots, and the
//! discriminated state machine for a single latest verification attempt per
//! solution. Only publishable fields exist here: sessions, tokens, cookies,
//! headers, and raw OJ responses must never enter this model.

use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::library::{LanguageId, LibraryId, SolutionId};
use crate::online_judge::SubmissionCapabilities;

// ─── Errors ──────────────────────────────────────────────────────────────────

/// Validation failures raised while constructing the verification newtypes.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerificationError {
    #[error("attempt id must not be empty")]
    EmptyAttemptId,
    #[error("attempt id must be printable ASCII without whitespace: {value:?}")]
    InvalidAttemptId { value: String },
    #[error("content hash must match \"sha256:<64 lowercase hex>\": {value:?}")]
    InvalidContentHash { value: String },
    #[error("verify fingerprint must match \"sha256:<64 lowercase hex>\": {value:?}")]
    InvalidFingerprint { value: String },
}

// ─── Newtypes ────────────────────────────────────────────────────────────────

/// Opaque identifier for a single verification attempt (spec §8.1, §11).
///
/// Accepts any non-empty, printable-ASCII string (no whitespace, no control
/// characters) so both UUIDv4-style hex-with-dashes and shorter opaque slugs
/// remain valid.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AttemptId(String);

impl AttemptId {
    pub fn parse(value: &str) -> Result<Self, VerificationError> {
        if value.is_empty() {
            return Err(VerificationError::EmptyAttemptId);
        }
        if !value.chars().all(|c| c.is_ascii_graphic()) {
            return Err(VerificationError::InvalidAttemptId {
                value: value.to_string(),
            });
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AttemptId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for AttemptId {
    type Error = VerificationError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<AttemptId> for String {
    fn from(id: AttemptId) -> Self {
        id.0
    }
}

fn is_sha256_prefixed_hex(value: &str) -> bool {
    let prefix = "sha256:";
    let Some(rest) = value.strip_prefix(prefix) else {
        return false;
    };
    rest.len() == 64
        && rest
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// Lowercase SHA-256 content hash stored inline in verification records
/// (spec §11). Encoded as `sha256:<64 lowercase hex>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentHash(String);

impl ContentHash {
    pub fn parse(value: &str) -> Result<Self, VerificationError> {
        if !is_sha256_prefixed_hex(value) {
            return Err(VerificationError::InvalidContentHash {
                value: value.to_string(),
            });
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ContentHash {
    type Error = VerificationError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ContentHash> for String {
    fn from(hash: ContentHash) -> Self {
        hash.0
    }
}

/// Aggregate verify fingerprint (spec §11). Encoded as `sha256:<64 hex>`.
///
/// Distinct from `ContentHash` at the type level so an attempt cannot
/// accidentally use a raw file hash where the aggregate fingerprint is
/// required.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VerifyFingerprint(String);

impl VerifyFingerprint {
    pub fn parse(value: &str) -> Result<Self, VerificationError> {
        if !is_sha256_prefixed_hex(value) {
            return Err(VerificationError::InvalidFingerprint {
                value: value.to_string(),
            });
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for VerifyFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for VerifyFingerprint {
    type Error = VerificationError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<VerifyFingerprint> for String {
    fn from(fp: VerifyFingerprint) -> Self {
        fp.0
    }
}

// ─── Shared value objects ────────────────────────────────────────────────────

/// Resolved binding between the internal language ID and the concrete OJ
/// submission-language ID (spec §8.1, §11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageBinding {
    pub language_id: LanguageId,
    pub oj_language_id: String,
}

/// Handle that lets the platform resume tracking a submission across
/// processes (spec §8, §8.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionHandle {
    pub oj: String,
    pub submission_id: String,
    pub submission_url: String,
    pub locator: Option<String>,
    pub submitted_at: DateTime<FixedOffset>,
}

/// Normalized judgement kind (spec §11.1). Unknown OJ verdicts fall through to
/// `Other`; the original string is preserved on `Verdict::raw`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    Accepted,
    WrongAnswer,
    TimeLimitExceeded,
    MemoryLimitExceeded,
    RuntimeError,
    CompileError,
    OutputLimitExceeded,
    JudgeError,
    Cancelled,
    Other,
}

/// Judgement pair carrying the normalized kind and the OJ's raw verdict
/// string (spec §11.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verdict {
    pub kind: VerdictKind,
    pub raw: String,
}

/// Aggregate run metrics (spec §11.1). Values that the OJ does not expose
/// stay `None`; distinct from `Some(0)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionSummary {
    pub max_execution_time_ms: Option<u64>,
    pub max_memory_bytes: Option<u64>,
}

/// Per-case metric row (spec §11.1). `name` may be `None` when the OJ omits
/// case IDs; the numeric fields follow the same optionality rule as the
/// summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestCaseResult {
    pub name: Option<String>,
    pub verdict: Verdict,
    pub execution_time_ms: Option<u64>,
    pub memory_bytes: Option<u64>,
}

/// Allow-listed value kinds that may appear inside `CompletedState.extra`.
///
/// Modeled as a closed enum (spec §11 "公開 allowlist を通した `extra`") so raw
/// JSON structures such as arrays or nested objects never leak into the
/// public record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum PublicExtraValue {
    String(String),
    Integer(i64),
    Bool(bool),
}

// ─── State bodies ────────────────────────────────────────────────────────────

/// Attempt persisted before any OJ contact (spec §8.1, §8.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartingState {
    pub plan_hash: ContentHash,
    pub submitted_source_hash: ContentHash,
    pub language: LanguageBinding,
    pub started_at: DateTime<FixedOffset>,
}

/// Attempt whose submission request may have been accepted but for which no
/// handle was captured (spec §8.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceUnknownState {
    pub plan_hash: ContentHash,
    pub submitted_source_hash: ContentHash,
    pub language: LanguageBinding,
    pub started_at: DateTime<FixedOffset>,
    pub observed_at: DateTime<FixedOffset>,
    pub summary: String,
}

/// Attempt for which the OJ has returned a submission handle (spec §8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmittedState {
    pub handle: SubmissionHandle,
    pub submitted_at: DateTime<FixedOffset>,
}

/// Attempt in queued or judging state, shared by `Queued` and `Judging`
/// variants (spec §8, §10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingState {
    pub handle: SubmissionHandle,
    pub observed_at: DateTime<FixedOffset>,
}

/// Failure stage recorded on `InfrastructureFailure` (spec §8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureStage {
    Prepare,
    Start,
    Poll,
}

/// Sanitized classification of the underlying failure (spec §8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Network,
    RateLimited,
    ServiceUnavailable,
    CredentialsMissing,
    AuthenticationRejected,
    InvalidResponse,
    SchemaError,
    Other,
}

/// Non-terminal operational failure state (spec §8.3).
///
/// Persisted in bot draft PRs only; never merged as a terminal main result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InfrastructureFailure {
    pub stage: FailureStage,
    pub error_kind: ErrorKind,
    pub retryable: bool,
    pub retry_count: u32,
    pub next_retry_at: Option<DateTime<FixedOffset>>,
    pub updated_at: DateTime<FixedOffset>,
    pub summary: String,
    pub plan_hash: Option<ContentHash>,
    pub handle: Option<SubmissionHandle>,
}

/// Terminal completed result (spec §10, §11, §11.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletedState {
    pub verdict: Verdict,
    pub verified_libraries: Vec<LibraryId>,
    pub language: LanguageBinding,
    pub verified_at: DateTime<FixedOffset>,
    pub capabilities: SubmissionCapabilities,
    pub submitted_source_hash: ContentHash,
    /// Per-input content hash map (spec §11 "各入力の content hash"): the
    /// submitted source and every library file (direct or transitive) that
    /// contributed to the fingerprint.
    pub input_hashes: BTreeMap<String, ContentHash>,
    pub summary: SubmissionSummary,
    pub test_cases: Option<Vec<TestCaseResult>>,
    pub handle: SubmissionHandle,
    pub extra: BTreeMap<String, PublicExtraValue>,
}

impl CompletedState {
    /// Recompute the run summary from the case list (spec §11.1).
    ///
    /// When `test_cases` is `Some`, the summary maxima are derived from the
    /// per-case metrics; otherwise the stored summary is returned unchanged.
    /// The recomputation ignores per-case `None`s so partial detail levels
    /// still produce a valid summary.
    pub fn recomputed_summary(&self) -> SubmissionSummary {
        match &self.test_cases {
            Some(cases) => SubmissionSummary {
                max_execution_time_ms: cases.iter().filter_map(|c| c.execution_time_ms).max(),
                max_memory_bytes: cases.iter().filter_map(|c| c.memory_bytes).max(),
            },
            None => self.summary.clone(),
        }
    }
}

/// Terminal reason the attempt cannot succeed under the current inputs
/// (spec §9, §10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    InteractiveUntrackable,
    UnsupportedMode,
    OjUnsupported,
    ProblemMismatch,
    LanguageMismatch,
}

/// Terminal "cannot verify" state (spec §9, §10, §11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnavailableState {
    pub reason: UnavailableReason,
    pub capabilities: SubmissionCapabilities,
    pub observed_at: DateTime<FixedOffset>,
    pub summary: String,
}

// ─── Discriminated state ─────────────────────────────────────────────────────

/// State machine for a single verification attempt (spec §8, §10).
///
/// Serializes adjacently-tagged: the outer object carries `kind` and the
/// state-specific fields live under `data`. This keeps inner structs strict
/// with `#[serde(deny_unknown_fields)]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum VerificationState {
    Starting(StartingState),
    AcceptanceUnknown(AcceptanceUnknownState),
    Submitted(SubmittedState),
    Queued(PendingState),
    Judging(PendingState),
    InfrastructureFailure(InfrastructureFailure),
    Completed(CompletedState),
    Unavailable(UnavailableState),
}

// ─── Record ──────────────────────────────────────────────────────────────────

/// Persisted latest verification record for a single solution (spec §11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationRecord {
    pub schema_version: u32,
    pub solution_id: SolutionId,
    pub attempt_id: AttemptId,
    pub replaces_attempt_id: Option<AttemptId>,
    pub fingerprint: VerifyFingerprint,
    pub state: VerificationState,
    /// Frozen plan context (spec §11 "record が十分な証跡を持つ") carried
    /// across every state transition so `CompletedState` can quote the
    /// original `language` and `submitted_source_hash` even from
    /// `Submitted/Queued/Judging/InfrastructureFailure`, which drop the
    /// `Starting` body once the OJ hands back a submission handle.
    ///
    /// `None` is allowed for backward compatibility with records written
    /// before this field existed; new records always populate it.
    #[serde(default)]
    pub plan_context: Option<PlanContext>,
}

/// Frozen plan facts required by every terminal record (spec §11).
///
/// Populated when the `Starting` record is first persisted; preserved by
/// [`apply_transition`] through every forward move, so downstream states
/// (`Submitted`, `Queued`, `Judging`, `InfrastructureFailure`, `Completed`)
/// can quote it without reaching back into the discarded `Starting` body.
///
/// `verify_libraries` freezes the sorted, deduplicated `[verify].libraries`
/// list the plan pinned so `CompletedState.verified_libraries` (spec §11
/// "result は提出時の direct `verified_libraries` を ID 順で保存する") is
/// populated from the plan rather than being lost between `Starting` and
/// `Completed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanContext {
    pub language: LanguageBinding,
    pub submitted_source_hash: ContentHash,
    /// Direct `[verify].libraries` from the plan, sorted by ID (spec §8.1).
    /// `#[serde(default)]` keeps records written before this field existed
    /// loadable — they simply deserialize with an empty vec.
    #[serde(default)]
    pub verify_libraries: Vec<LibraryId>,
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_id_rejects_empty_and_control_characters() {
        assert_eq!(
            AttemptId::parse("").unwrap_err(),
            VerificationError::EmptyAttemptId,
        );
        assert!(AttemptId::parse("has whitespace").is_err());
        assert!(AttemptId::parse("has\ttab").is_err());
        assert!(AttemptId::parse("with\nnewline").is_err());
    }

    #[test]
    fn attempt_id_accepts_uuidv4_and_opaque_slugs() {
        assert_eq!(
            AttemptId::parse("01931aae-3a48-7fb4-9c62-1f89a0a5f001")
                .unwrap()
                .as_str(),
            "01931aae-3a48-7fb4-9c62-1f89a0a5f001",
        );
        assert_eq!(AttemptId::parse("abc-42").unwrap().as_str(), "abc-42");
    }

    #[test]
    fn content_hash_requires_sha256_prefix_and_hex() {
        assert!(ContentHash::parse("sha256:not-hex").is_err());
        assert!(ContentHash::parse("sha1:0000000000000000000000000000000000000000").is_err());
        assert!(
            ContentHash::parse(
                "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            )
            .is_err()
        );
        assert!(
            ContentHash::parse(
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            )
            .is_ok()
        );
    }

    #[test]
    fn fingerprint_shape_matches_content_hash() {
        assert!(VerifyFingerprint::parse("sha1:abc").is_err());
        assert!(VerifyFingerprint::parse("").is_err());
        let ok = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
        assert_eq!(VerifyFingerprint::parse(ok).unwrap().as_str(), ok);
    }

    #[test]
    fn recomputed_summary_derives_from_cases_when_available() {
        let handle = SubmissionHandle {
            oj: "librarychecker".into(),
            submission_id: "1".into(),
            submission_url: "https://example.test/1".into(),
            locator: None,
            submitted_at: DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap(),
        };
        let completed = CompletedState {
            verdict: Verdict {
                kind: VerdictKind::Accepted,
                raw: "AC".into(),
            },
            verified_libraries: vec![],
            language: LanguageBinding {
                language_id: LanguageId::parse("rust").unwrap(),
                oj_language_id: "rust".into(),
            },
            verified_at: DateTime::parse_from_rfc3339("2026-08-10T10:00:00+00:00").unwrap(),
            capabilities: SubmissionCapabilities {
                submission_mode: crate::online_judge::SubmissionMode::UnattendedTrackable,
                result_detail: crate::online_judge::ResultDetail::TestcaseDetails,
                recovery_mode: crate::online_judge::RecoveryMode::BestEffort,
            },
            submitted_source_hash: ContentHash::parse(
                "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            )
            .unwrap(),
            input_hashes: BTreeMap::new(),
            summary: SubmissionSummary {
                max_execution_time_ms: Some(0),
                max_memory_bytes: Some(0),
            },
            test_cases: Some(vec![
                TestCaseResult {
                    name: None,
                    verdict: Verdict {
                        kind: VerdictKind::Accepted,
                        raw: "AC".into(),
                    },
                    execution_time_ms: Some(11),
                    memory_bytes: Some(22),
                },
                TestCaseResult {
                    name: None,
                    verdict: Verdict {
                        kind: VerdictKind::Accepted,
                        raw: "AC".into(),
                    },
                    execution_time_ms: Some(3),
                    memory_bytes: Some(99),
                },
            ]),
            handle,
            extra: BTreeMap::new(),
        };
        let recomputed = completed.recomputed_summary();
        assert_eq!(recomputed.max_execution_time_ms, Some(11));
        assert_eq!(recomputed.max_memory_bytes, Some(99));
    }
}
