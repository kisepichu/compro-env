//! Submission lifecycle ports.
//!
//! Spec: `docs/superpowers/specs/2026-08-10-library-platform-design.md` §§ 8, 9.
//!
//! `OnlineJudge` retains login/problem metadata; three capability-specific ports
//! model start / poll / recovery so `ce verify` and a future `ce submit --watch`
//! can share them without coupling polling to submission.
//!
//! The models are strict: adapters must declare their capabilities via
//! [`SubmissionAdapterDescriptor`], and registries verify that a starter's
//! declared `submission_mode` is consistent with the presence/absence of a
//! matching poller and recovery adapter.
//!
//! A transport failure after a submission request may have been transmitted is
//! reported as [`StartSubmissionError::AcceptanceUnknown`], never as a
//! retryable error — this is the safety invariant spec §8.2 pins down.

use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use domain::entity::{OJKind, Session};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─── Capability descriptors (spec §9) ─────────────────────────────────────

/// The submission mode an adapter declares. See spec §9.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionMode {
    /// Auto-submitted and auto-tracked (e.g. LibraryChecker via REST).
    UnattendedTrackable,
    /// User completes an interactive flow (e.g. userscript) but the result is
    /// still trackable via a submission ID. Not used by current MVP adapters.
    InteractiveTrackable,
    /// User completes an interactive flow and the submission ID is not
    /// reliably observable (e.g. AtCoder browser submit).
    InteractiveUntrackable,
    /// The adapter cannot submit this problem/language at all.
    Unsupported,
}

/// Result-detail level an adapter can surface for a completed submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultDetailLevel {
    OverallOnly,
    SummaryMetrics,
    TestcaseDetails,
}

/// How well an adapter can recover a lost handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryMode {
    /// Uniquely identify the attempt OR prove it never landed.
    Exact,
    /// Sometimes uniquely identifies the attempt; "no candidates" does NOT
    /// prove non-acceptance.
    BestEffort,
    /// Cannot recover a lost handle at all.
    None,
}

/// Structured description of a submission adapter's capabilities. Registries
/// check consistency between `starter` / `poller` / `recovery` descriptors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmissionAdapterDescriptor {
    pub name: String,
    pub version: String,
    pub submission_mode: SubmissionMode,
    pub result_detail: ResultDetailLevel,
    pub recovery_mode: RecoveryMode,
}

impl SubmissionAdapterDescriptor {
    /// `ce verify` requires an unattended trackable adapter.
    pub fn supports_unattended_verify(&self) -> bool {
        matches!(self.submission_mode, SubmissionMode::UnattendedTrackable)
    }
}

// ─── Handles + requests (spec §8) ─────────────────────────────────────────

/// Everything needed to resume tracking a submission across processes.
///
/// Persisted to disk between runs so `ce verify` / `ce submit --watch` can pick
/// back up on the next tick. Serde format is stable JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionHandle {
    pub online_judge: OJKind,
    pub submission_id: String,
    pub submission_url: String,
    /// Free-form OJ-specific locator (e.g. problem + language + submitted_at
    /// composite) used by recovery adapters. Not required for polling.
    #[serde(default)]
    pub locator: Option<String>,
    pub submitted_at: DateTime<Utc>,
}

/// Immutable inputs to a submission attempt.
///
/// The controller composes this from the `SubmissionPlan` (spec §8.1) — the
/// adapter never re-runs analysis, checks, or the preprocess hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionRequest {
    pub online_judge: OJKind,
    pub contest_id: String,
    pub problem_id: String,
    pub lang_id: String,
    /// The exact bytes to send. This is post-preprocess and is what fingerprint
    /// calculations must match.
    pub source: String,
}

/// Inputs to a recovery attempt: enough to search the OJ's submission history
/// for the plan that was `Starting` when the run crashed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryRequest {
    pub online_judge: OJKind,
    pub contest_id: String,
    pub problem_id: String,
    pub lang_id: String,
    /// Hash of the exact source bytes the `Starting` record was pinned to.
    /// Adapters use this + problem/language + `submitted_at_lower_bound` as the
    /// "was this attempt accepted" query.
    pub source_hash: String,
    /// Lower bound for the submission timestamp; excludes historical submissions
    /// that predate the `Starting` record.
    pub submitted_at_lower_bound: Option<DateTime<Utc>>,
}

impl RecoveryRequest {
    /// Convenience for tests and simple call sites: derive a recovery request
    /// from the submission request. The hash uses SHA-256 of the source bytes.
    pub fn from_request(request: &SubmissionRequest) -> Self {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(request.source.as_bytes());
        let hash = hex_encode(&hasher.finalize());
        Self {
            online_judge: request.online_judge.clone(),
            contest_id: request.contest_id.clone(),
            problem_id: request.problem_id.clone(),
            lang_id: request.lang_id.clone(),
            source_hash: format!("sha256:{hash}"),
            submitted_at_lower_bound: None,
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// ─── Outcomes ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmissionStart {
    /// Adapter started the submission and returned a handle that a poller can
    /// resume from. Emitted by `unattended_trackable` and (future)
    /// `interactive_trackable` adapters.
    Trackable { handle: SubmissionHandle },
    /// The user must complete a browser flow. Emitted by
    /// `interactive_untrackable` adapters (e.g. AtCoder via userscript).
    UserActionRequired { url: String },
    /// Adapter cannot serve this request and cannot at the current
    /// configuration. Used by `unsupported` combinations.
    Unavailable { reason: UnavailableReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnavailableReason {
    /// AtCoder-shaped OJs whose submission mode is interactive-untrackable, but
    /// the caller (e.g. `ce verify`) needs an unattended trackable adapter.
    InteractiveUntrackable,
    /// The OJ/problem/language triple is fundamentally unsupported.
    UnsupportedProblemOrLanguage { detail: String },
    /// The starter recognises the request but its mode is `unsupported`.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollObservation {
    /// The submission is queued and no judge worker has picked it up yet.
    Queued,
    /// A worker is running the tests.
    Judging,
    Completed(JudgeResult),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeResult {
    pub verdict: JudgeVerdict,
    /// Per-testcase details when the adapter's `result_detail` is
    /// `TestcaseDetails`. Empty otherwise.
    pub testcases: Vec<TestcaseOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JudgeVerdict {
    Accepted,
    WrongAnswer,
    TimeLimitExceeded,
    MemoryLimitExceeded,
    RuntimeError,
    CompilationError,
    InternalError,
    /// Adapters may report OJ-specific verdicts as a free-form string; the
    /// public schema normalises these downstream.
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestcaseOutcome {
    pub name: String,
    pub verdict: JudgeVerdict,
    pub time_ms: Option<u32>,
    pub memory_kib: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// The adapter uniquely identified the attempt and returns a handle.
    Recovered { handle: SubmissionHandle },
    /// The OJ confirms the attempt never landed. Safe to discard and re-plan.
    ConfirmedNotAccepted,
    /// Ambiguous or unproven: multiple candidates, zero candidates on a
    /// `best_effort` adapter, or the recovery API was unavailable. Never a
    /// safe retry — leave the attempt as `AcceptanceUnknown` for an operator.
    AcceptanceUnknown,
    /// The adapter's `recovery_mode` is `None`, or the OJ has no recovery API.
    Unsupported,
}

impl RecoveryOutcome {
    /// Only `ConfirmedNotAccepted` allows the caller to discard the attempt
    /// and re-plan. `AcceptanceUnknown` and `Unsupported` keep the operator in
    /// the loop; `Recovered` transitions to normal polling.
    pub fn is_safe_to_discard_attempt(&self) -> bool {
        matches!(self, RecoveryOutcome::ConfirmedNotAccepted)
    }
}

// ─── Errors ───────────────────────────────────────────────────────────────

/// Errors emitted by `SubmissionStarter`.
///
/// The distinction between `AcceptanceUnknown` and `ConfirmedNotAccepted` is
/// load-bearing: only `ConfirmedNotAccepted` is safe to retry, per spec §8.2.
#[derive(Debug, Clone, Error)]
pub enum StartSubmissionError {
    /// The submission request may have been transmitted. The caller MUST NOT
    /// retry automatically. Persist `AcceptanceUnknown` and surface an
    /// operator-repairable state.
    ///
    /// `summary` is a sanitized short description safe to write to a public
    /// draft PR. Never include raw responses, headers, credentials, or tokens.
    #[error("acceptance unknown: {summary}")]
    AcceptanceUnknown { summary: String },
    /// The OJ actively refused the request before accepting it (e.g. schema
    /// error on our side, 400 response). Safe to discard the attempt and
    /// re-plan.
    #[error("confirmed not accepted: {summary}")]
    ConfirmedNotAccepted { summary: String },
    /// The adapter cannot serve this request in the current configuration.
    /// Fixed for the same inputs — do not repeatedly retry.
    #[error("submission unavailable: {reason:?}")]
    Unavailable { reason: UnavailableReason },
    /// Operational failure (missing credentials, auth rejected, rate limit,
    /// invalid response, ...). Classification lives in `kind`.
    #[error("infrastructure error [{kind:?}]: {summary}")]
    Infrastructure {
        kind: InfrastructureErrorKind,
        summary: String,
    },
}

/// Errors emitted by `SubmissionPoller`.
#[derive(Debug, Clone, Error)]
pub enum PollSubmissionError {
    #[error("infrastructure error [{kind:?}]: {summary}")]
    Infrastructure {
        kind: InfrastructureErrorKind,
        summary: String,
    },
    /// The OJ says this handle no longer exists (submission ID unknown).
    #[error("handle not found: {summary}")]
    HandleNotFound { summary: String },
}

/// Errors emitted by `SubmissionRecovery`.
#[derive(Debug, Clone, Error)]
pub enum RecoverSubmissionError {
    #[error("infrastructure error [{kind:?}]: {summary}")]
    Infrastructure {
        kind: InfrastructureErrorKind,
        summary: String,
    },
}

/// Classification for infrastructure failures. Matches spec §8.3 `error_kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfrastructureErrorKind {
    Network,
    RateLimited,
    ServiceUnavailable,
    CredentialsMissing,
    AuthenticationRejected,
    InvalidResponse,
    SchemaError,
    Other,
}

impl StartSubmissionError {
    /// Convenience constructor for the acceptance-unknown safety invariant.
    /// A transport failure after any bytes may have been sent MUST map here.
    pub fn from_transport_after_send(summary: impl Into<String>) -> Self {
        Self::AcceptanceUnknown {
            summary: sanitize_summary(&summary.into()),
        }
    }

    /// Only `ConfirmedNotAccepted` allows the caller to re-plan the attempt on
    /// the next tick without operator intervention.
    pub fn is_safe_to_retry(&self) -> bool {
        matches!(self, Self::ConfirmedNotAccepted { .. })
    }

    /// The sanitized human-readable summary safe for draft PR output.
    pub fn summary(&self) -> &str {
        match self {
            Self::AcceptanceUnknown { summary } => summary,
            Self::ConfirmedNotAccepted { summary } => summary,
            Self::Unavailable { .. } => "submission unavailable",
            Self::Infrastructure { summary, .. } => summary,
        }
    }
}

/// Strips credential-like substrings from a free-form error string so adapters
/// that forget to sanitize cannot leak by accident. Spec §8.3 forbids raw
/// responses, credentials, cookies, tokens, and request/response headers in
/// summaries.
///
/// Not a security boundary against malicious adapters — a defensive default
/// that catches accidental leaks (e.g. verbose reqwest errors that echo the URL
/// with a `?token=...` query).
pub fn sanitize_summary(input: &str) -> String {
    let placeholder = "[REDACTED]";
    let keywords = [
        "bearer ",
        "authorization:",
        "authorization=",
        "cookie:",
        "cookie=",
        "set-cookie:",
        "revel_session",
        "id_token",
        "idtoken",
        "refresh_token",
        "refreshtoken",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "token=",
    ];
    // Case-insensitive match using `to_ascii_lowercase` so `lower` and `input`
    // share the same byte layout — indexing by `i` into either is safe.
    // Full Unicode `to_lowercase` can change the string's length (e.g. `İ` →
    // `i̇`, two chars), which would break byte-indexed slicing.
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        // `i` is always on a UTF-8 char boundary of `input` (loop invariant).
        let mut matched_len = 0usize;
        for kw in &keywords {
            // Byte-safe: the keyword is ASCII, so a match starts at a char
            // boundary in `input` too.
            if lower[i..].starts_with(kw) {
                matched_len = kw.len();
                break;
            }
        }
        if matched_len > 0 {
            out.push_str(placeholder);
            // Scan the value that follows the keyword up to the next
            // whitespace or ASCII separator. We look at bytes of `lower` (safe
            // because we only compare against ASCII terminators), then snap
            // `j` forward to a char boundary before resuming outer iteration.
            let bytes = lower.as_bytes();
            let mut j = i + matched_len;
            while j < input.len() {
                let b = bytes[j];
                if b.is_ascii_whitespace() || matches!(b, b';' | b',' | b'"' | b'\'' | b')' | b']')
                {
                    break;
                }
                j += 1;
            }
            while j < input.len() && !input.is_char_boundary(j) {
                j += 1;
            }
            i = j;
        } else {
            // Copy one UTF-8 scalar so `i` stays on a char boundary.
            let ch_end = input[i..]
                .char_indices()
                .nth(1)
                .map(|(off, _)| i + off)
                .unwrap_or(input.len());
            out.push_str(&input[i..ch_end]);
            i = ch_end;
        }
    }
    out
}

// ─── Ports ────────────────────────────────────────────────────────────────

/// Starts a submission for a given `SubmissionRequest`.
///
/// Adapters must not run analysis, checks, or the preprocess hook — the
/// controller passes an immutable request whose `source` is already the exact
/// bytes to send (spec §8.1).
pub trait SubmissionStarter {
    fn descriptor(&self) -> SubmissionAdapterDescriptor;
    fn start_submission(
        &self,
        request: &SubmissionRequest,
        session: Option<&Session>,
    ) -> Result<SubmissionStart, StartSubmissionError>;
}

/// Polls the OJ for the current status of a submission.
pub trait SubmissionPoller {
    fn descriptor(&self) -> SubmissionAdapterDescriptor;
    fn poll_submission(
        &self,
        handle: &SubmissionHandle,
        session: Option<&Session>,
    ) -> Result<PollObservation, PollSubmissionError>;
}

/// Recovers a lost handle from the OJ's submission history.
pub trait SubmissionRecovery {
    fn descriptor(&self) -> SubmissionAdapterDescriptor;
    fn recover_submission(
        &self,
        request: &RecoveryRequest,
        session: Option<&Session>,
    ) -> Result<RecoveryOutcome, RecoverSubmissionError>;
}

// ─── Registries (one per port) ────────────────────────────────────────────

/// Registry of `SubmissionStarter` implementations, keyed by `OJKind`.
///
/// Kept separate from `PollerRegistry`/`RecoveryRegistry` because the three
/// capabilities are not equivalent: `interactive_untrackable` OJs (AtCoder)
/// register only a starter, and future OJs may register a poller without a
/// recovery adapter.
pub struct StarterRegistry {
    entries: HashMap<OJKind, Box<dyn SubmissionStarter>>,
}

impl StarterRegistry {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn register(&mut self, oj: OJKind, starter: Box<dyn SubmissionStarter>) {
        self.entries.insert(oj, starter);
    }

    pub fn get(&self, oj: &OJKind) -> Result<&dyn SubmissionStarter> {
        self.entries
            .get(oj)
            .map(|b| b.as_ref())
            .ok_or_else(|| anyhow::anyhow!("no submission starter registered for {}", oj.as_str()))
    }

    pub fn contains(&self, oj: &OJKind) -> bool {
        self.entries.contains_key(oj)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&OJKind, &dyn SubmissionStarter)> {
        self.entries.iter().map(|(k, v)| (k, v.as_ref()))
    }
}

impl Default for StarterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Registry of `SubmissionPoller` implementations, keyed by `OJKind`.
pub struct PollerRegistry {
    entries: HashMap<OJKind, Box<dyn SubmissionPoller>>,
}

impl PollerRegistry {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn register(&mut self, oj: OJKind, poller: Box<dyn SubmissionPoller>) {
        self.entries.insert(oj, poller);
    }

    pub fn get(&self, oj: &OJKind) -> Result<&dyn SubmissionPoller> {
        self.entries
            .get(oj)
            .map(|b| b.as_ref())
            .ok_or_else(|| anyhow::anyhow!("no submission poller registered for {}", oj.as_str()))
    }

    pub fn contains(&self, oj: &OJKind) -> bool {
        self.entries.contains_key(oj)
    }
}

impl Default for PollerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Registry of `SubmissionRecovery` implementations, keyed by `OJKind`.
pub struct RecoveryRegistry {
    entries: HashMap<OJKind, Box<dyn SubmissionRecovery>>,
}

impl RecoveryRegistry {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn register(&mut self, oj: OJKind, recovery: Box<dyn SubmissionRecovery>) {
        self.entries.insert(oj, recovery);
    }

    pub fn get(&self, oj: &OJKind) -> Result<&dyn SubmissionRecovery> {
        self.entries
            .get(oj)
            .map(|b| b.as_ref())
            .ok_or_else(|| anyhow::anyhow!("no submission recovery registered for {}", oj.as_str()))
    }

    pub fn contains(&self, oj: &OJKind) -> bool {
        self.entries.contains_key(oj)
    }
}

impl Default for RecoveryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Verifies that the three registries are internally consistent.
///
/// - Every `unattended_trackable` starter must have a poller and a recovery
///   adapter (else `AcceptanceUnknown` attempts would be unrecoverable).
/// - The recovery adapter's `recovery_mode` must not be `None` for
///   `unattended_trackable` starters.
/// - `interactive_untrackable` and `unsupported` starters need neither.
pub fn verify_registry_consistency(
    starters: &StarterRegistry,
    pollers: Option<&PollerRegistry>,
    recovery: Option<&RecoveryRegistry>,
) -> Result<()> {
    for (oj, starter) in starters.iter() {
        let d = starter.descriptor();
        match d.submission_mode {
            SubmissionMode::UnattendedTrackable | SubmissionMode::InteractiveTrackable => {
                let pollers = pollers.ok_or_else(|| {
                    anyhow::anyhow!(
                        "starter for {} declares {:?} but no poller registry was provided",
                        oj.as_str(),
                        d.submission_mode
                    )
                })?;
                if !pollers.contains(oj) {
                    anyhow::bail!(
                        "starter for {} declares {:?} but no poller is registered",
                        oj.as_str(),
                        d.submission_mode
                    );
                }
                let recovery = recovery.ok_or_else(|| {
                    anyhow::anyhow!(
                        "starter for {} declares {:?} but no recovery registry was provided",
                        oj.as_str(),
                        d.submission_mode
                    )
                })?;
                let rec = recovery.get(oj).map_err(|_| {
                    anyhow::anyhow!(
                        "starter for {} declares {:?} but no recovery adapter is registered",
                        oj.as_str(),
                        d.submission_mode
                    )
                })?;
                if matches!(rec.descriptor().recovery_mode, RecoveryMode::None) {
                    anyhow::bail!(
                        "starter for {} declares {:?} but its recovery adapter's recovery_mode is None",
                        oj.as_str(),
                        d.submission_mode
                    );
                }
            }
            SubmissionMode::InteractiveUntrackable | SubmissionMode::Unsupported => {
                // No poller or recovery required — nothing to check.
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_summary_leaves_benign_text_intact() {
        assert_eq!(
            sanitize_summary("connection reset before response headers"),
            "connection reset before response headers"
        );
    }

    #[test]
    fn sanitize_summary_scrubs_bearer_and_cookie() {
        // Dummy values (`TOKEN123`, `RS_abc123`) are non-secrets that still
        // trigger the keyword + trailing-token scrub.
        let out =
            sanitize_summary("auth failed: Bearer TOKEN123; cookie=REVEL_SESSION=RS_abc123; done");
        let lower = out.to_lowercase();
        assert!(!lower.contains("bearer "));
        assert!(!lower.contains("cookie="));
        assert!(!out.contains("TOKEN123"));
        assert!(!out.contains("REVEL_SESSION=RS_abc123"));
    }

    #[test]
    fn recovery_request_from_request_uses_sha256() {
        let r = SubmissionRequest {
            online_judge: OJKind::LibraryChecker,
            contest_id: "librarychecker-aplusb".to_string(),
            problem_id: "aplusb".to_string(),
            lang_id: "cpp".to_string(),
            source: "int main(){}".to_string(),
        };
        let rr = RecoveryRequest::from_request(&r);
        assert!(rr.source_hash.starts_with("sha256:"));
        assert_eq!(rr.source_hash.len(), 7 + 64);
    }

    #[test]
    fn start_error_from_transport_after_send_sanitizes_input() {
        let err =
            StartSubmissionError::from_transport_after_send("write failed after Bearer TOKEN123");
        let s = err.summary();
        assert!(!s.to_lowercase().contains("bearer "));
        assert!(!s.contains("TOKEN123"));
    }

    /// Regression: no panic on multi-byte UTF-8 either from the outer scan
    /// or from the trailing-token skip, and non-ASCII surroundings are kept.
    #[test]
    fn sanitize_summary_multibyte_utf8_is_lossless_and_safe() {
        let raw = "認証エラー: Bearer TOKEN123 が拒否されました 🚫";
        let out = sanitize_summary(raw);
        assert!(!out.to_lowercase().contains("bearer"));
        assert!(!out.contains("TOKEN123"));
        assert!(out.contains("認証エラー"));
        assert!(out.contains("🚫"));
    }
}
