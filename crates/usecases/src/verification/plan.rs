//! Immutable submission plans and their canonical hash (spec §8.1).
//!
//! A [`SubmissionPlan`] freezes the exact bytes we intend to submit plus the
//! attempt identity, so the later start and poll jobs can prove they are
//! honoring the same content that was fingerprinted. The plan body is
//! canonicalized into a stable JSON projection and hashed once at
//! construction; the hash never changes even if working-tree files change.
//!
//! The module also produces the [`domain::verification::StartingState`] that
//! must be persisted before any OJ contact (spec §8.2). Prepare-side
//! serialization helpers live here; they are hidden from the public CLI and
//! only exposed to internal wiring.

use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset};
use domain::library::{LibraryId, SolutionId};
use domain::verification::{
    AttemptId, ContentHash, LanguageBinding, StartingState, VerificationRecord, VerificationState,
    VerifyFingerprint,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::clock::Clock;
use crate::id_generator::AttemptIdGenerator;
use crate::verification::fingerprint::FingerprintSource;

/// Plan schema version stamped into every [`SubmissionPlan`] (spec §8.1).
pub const PLAN_SCHEMA_VERSION: u32 = 1;

/// Immutable submission plan (spec §8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionPlan {
    pub body: SubmissionPlanBody,
    pub plan_hash: ContentHash,
}

/// Frozen body of a [`SubmissionPlan`] (spec §8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionPlanBody {
    pub schema_version: u32,
    pub solution_id: SolutionId,
    pub attempt_id: AttemptId,
    pub replaces_attempt_id: Option<AttemptId>,
    pub oj: String,
    pub contest_id: String,
    pub problem_code: String,
    pub language: LanguageBinding,
    pub submitted_source_path: String,
    pub submitted_source_bytes: Vec<u8>,
    pub submitted_source_hash: ContentHash,
    pub fingerprint: VerifyFingerprint,
    pub verifies: Vec<LibraryId>,
    pub started_at: DateTime<FixedOffset>,
}

impl SubmissionPlan {
    /// Serialize the frozen plan body to its canonical JSON representation.
    ///
    /// The exact bytes hashed by `plan_hash` are returned so consumers can
    /// round-trip the plan through disk (used by the hidden
    /// `internal verify-prepare` / `verify-start` CI boundary).
    pub fn to_canonical_json_bytes(&self) -> Vec<u8> {
        canonical_plan_bytes(&self.body)
    }

    /// Reconstruct a plan from canonical JSON bytes produced by
    /// [`Self::to_canonical_json_bytes`]. Recomputes the plan hash from the
    /// reconstructed body so `start_prepared_plan` can validate integrity.
    pub fn from_canonical_json_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        use anyhow::{Context, anyhow};
        use chrono::DateTime;
        use domain::library::{LanguageId, LibraryId};
        use serde_json::Value;

        let v: Value = serde_json::from_slice(bytes).context("plan JSON parse failed")?;
        let schema_version = v["schema_version"]
            .as_u64()
            .ok_or_else(|| anyhow!("plan missing schema_version"))?
            as u32;
        if schema_version != PLAN_SCHEMA_VERSION {
            return Err(anyhow!(
                "plan schema_version {schema_version} != {PLAN_SCHEMA_VERSION}"
            ));
        }
        let solution_id = SolutionId::parse(
            v["solution_id"]
                .as_str()
                .ok_or_else(|| anyhow!("plan missing solution_id"))?,
        )
        .map_err(|e| anyhow!("plan solution_id invalid: {e}"))?;
        let attempt_id = AttemptId::parse(
            v["attempt_id"]
                .as_str()
                .ok_or_else(|| anyhow!("plan missing attempt_id"))?,
        )
        .map_err(|e| anyhow!("plan attempt_id invalid: {e}"))?;
        let replaces_attempt_id = v["replaces_attempt_id"]
            .as_str()
            .map(|s| AttemptId::parse(s).map_err(|e| anyhow!("plan replaces_attempt_id: {e}")))
            .transpose()?;
        let oj = v["oj"]
            .as_str()
            .ok_or_else(|| anyhow!("plan missing oj"))?
            .to_string();
        let contest_id = v["contest_id"]
            .as_str()
            .ok_or_else(|| anyhow!("plan missing contest_id"))?
            .to_string();
        let problem_code = v["problem_code"]
            .as_str()
            .ok_or_else(|| anyhow!("plan missing problem_code"))?
            .to_string();
        let language = LanguageBinding {
            language_id: LanguageId::parse(
                v["language"]["language_id"]
                    .as_str()
                    .ok_or_else(|| anyhow!("plan missing language.language_id"))?,
            )
            .map_err(|e| anyhow!("plan language_id invalid: {e}"))?,
            oj_language_id: v["language"]["oj_language_id"]
                .as_str()
                .ok_or_else(|| anyhow!("plan missing language.oj_language_id"))?
                .to_string(),
        };
        let submitted_source_path = v["submitted_source"]["path"]
            .as_str()
            .ok_or_else(|| anyhow!("plan missing submitted_source.path"))?
            .to_string();
        let bytes_b64 = v["submitted_source"]["bytes_b64"]
            .as_str()
            .ok_or_else(|| anyhow!("plan missing submitted_source.bytes_b64"))?;
        let submitted_source_bytes = base64_decode(bytes_b64)?;
        let submitted_source_hash = ContentHash::parse(
            v["submitted_source"]["hash"]
                .as_str()
                .ok_or_else(|| anyhow!("plan missing submitted_source.hash"))?,
        )
        .map_err(|e| anyhow!("plan submitted_source.hash invalid: {e}"))?;
        let fingerprint = VerifyFingerprint::parse(
            v["fingerprint"]
                .as_str()
                .ok_or_else(|| anyhow!("plan missing fingerprint"))?,
        )
        .map_err(|e| anyhow!("plan fingerprint invalid: {e}"))?;
        let verifies_arr = v["verifies"]
            .as_array()
            .ok_or_else(|| anyhow!("plan missing verifies"))?;
        let mut verifies = Vec::with_capacity(verifies_arr.len());
        for item in verifies_arr {
            let s = item
                .as_str()
                .ok_or_else(|| anyhow!("plan verifies entry not a string"))?;
            verifies.push(
                LibraryId::parse(s).map_err(|e| anyhow!("plan verifies entry invalid: {e}"))?,
            );
        }
        let started_at = DateTime::parse_from_rfc3339(
            v["started_at"]
                .as_str()
                .ok_or_else(|| anyhow!("plan missing started_at"))?,
        )
        .map_err(|e| anyhow!("plan started_at invalid: {e}"))?;

        let body = SubmissionPlanBody {
            schema_version,
            solution_id,
            attempt_id,
            replaces_attempt_id,
            oj,
            contest_id,
            problem_code,
            language,
            submitted_source_path,
            submitted_source_bytes,
            submitted_source_hash,
            fingerprint,
            verifies,
            started_at,
        };
        let plan_hash = compute_plan_hash(&body);
        Ok(SubmissionPlan { body, plan_hash })
    }

    /// Build the [`StartingState`] that must be persisted before any OJ
    /// contact (spec §8.2).
    pub fn starting_state(&self) -> StartingState {
        StartingState {
            plan_hash: self.plan_hash.clone(),
            submitted_source_hash: self.body.submitted_source_hash.clone(),
            language: self.body.language.clone(),
            started_at: self.body.started_at,
        }
    }

    /// Wrap the plan in a `VerificationRecord` sitting in the `Starting`
    /// state (spec §8.1 → §8.2). This is the first record persisted by the
    /// verify pipeline.
    pub fn as_starting_record(&self) -> VerificationRecord {
        VerificationRecord {
            schema_version: 1,
            solution_id: self.body.solution_id.clone(),
            attempt_id: self.body.attempt_id.clone(),
            replaces_attempt_id: self.body.replaces_attempt_id.clone(),
            fingerprint: self.body.fingerprint.clone(),
            state: VerificationState::Starting(self.starting_state()),
            plan_context: Some(domain::verification::PlanContext {
                language: self.body.language.clone(),
                submitted_source_hash: self.body.submitted_source_hash.clone(),
                verify_libraries: self.body.verifies.clone(),
            }),
        }
    }
}

/// Prepare-time input passed to [`build_submission_plan`] (spec §8.1).
///
/// The caller has already resolved discovery, dependency analysis, and
/// preprocessing; this struct just carries the frozen submission material.
#[derive(Debug, Clone)]
pub struct PrepareVerificationInput<'a> {
    pub solution_id: &'a SolutionId,
    pub oj: String,
    pub contest_id: String,
    pub problem_code: String,
    pub language: LanguageBinding,
    pub submitted_source: FingerprintSource,
    pub fingerprint: VerifyFingerprint,
    pub verifies: Vec<LibraryId>,
    pub previous_attempt_id: Option<AttemptId>,
}

/// Failure modes rejected by [`build_submission_plan`] (spec §8.1).
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum PlanError {
    #[error("submitted source bytes are empty; plan would freeze an empty submission")]
    EmptySubmittedSource,
    #[error("verify list is not sorted or contains duplicates")]
    UnsortedOrDuplicateVerifyList,
}

/// Freeze a plan for the given prepare input (spec §8.1).
///
/// The returned plan has a stable canonical JSON projection: two calls with
/// the same input (deterministic clock and generator) yield the same
/// `plan_hash`. `verifies` must be strictly sorted by ID to keep the plan
/// canonical and free from duplicate library entries.
pub fn build_submission_plan(
    input: PrepareVerificationInput<'_>,
    clock: &dyn Clock,
    ids: &dyn AttemptIdGenerator,
) -> Result<SubmissionPlan, PlanError> {
    if input.submitted_source.bytes.is_empty() {
        return Err(PlanError::EmptySubmittedSource);
    }
    for pair in input.verifies.windows(2) {
        if pair[0] >= pair[1] {
            return Err(PlanError::UnsortedOrDuplicateVerifyList);
        }
    }
    let submitted_source_hash = input.submitted_source.hash();
    let body = SubmissionPlanBody {
        schema_version: PLAN_SCHEMA_VERSION,
        solution_id: input.solution_id.clone(),
        attempt_id: ids.generate(),
        replaces_attempt_id: input.previous_attempt_id,
        oj: input.oj,
        contest_id: input.contest_id,
        problem_code: input.problem_code,
        language: input.language,
        submitted_source_path: input.submitted_source.path,
        submitted_source_bytes: input.submitted_source.bytes,
        submitted_source_hash,
        fingerprint: input.fingerprint,
        verifies: input.verifies,
        started_at: clock.now(),
    };
    let plan_hash = compute_plan_hash(&body);
    Ok(SubmissionPlan { body, plan_hash })
}

fn compute_plan_hash(body: &SubmissionPlanBody) -> ContentHash {
    let payload = canonical_plan_projection(body);
    let bytes = canonical_json(&payload);
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let hex = format!("sha256:{:x}", hasher.finalize());
    ContentHash::parse(&hex).expect("sha256_hex always emits a valid content hash")
}

/// Canonical JSON projection used by both `plan_hash` and the hidden
/// prepare-side serialization helpers.
///
/// `submitted_source_bytes` are base64-encoded so the projection remains a
/// valid UTF-8 JSON string even for binary payloads while still uniquely
/// identifying the bytes (base64 is injective on the byte set).
fn canonical_plan_projection(body: &SubmissionPlanBody) -> serde_json::Value {
    let mut source_map: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    source_map.insert(
        "path".into(),
        serde_json::Value::String(body.submitted_source_path.clone()),
    );
    source_map.insert(
        "bytes_b64".into(),
        serde_json::Value::String(base64_encode(&body.submitted_source_bytes)),
    );
    source_map.insert(
        "hash".into(),
        serde_json::Value::String(body.submitted_source_hash.to_string()),
    );
    serde_json::json!({
        "schema_version": body.schema_version,
        "solution_id": body.solution_id.as_str(),
        "attempt_id": body.attempt_id.as_str(),
        "replaces_attempt_id": body.replaces_attempt_id.as_ref().map(|a| a.as_str().to_string()),
        "oj": body.oj,
        "contest_id": body.contest_id,
        "problem_code": body.problem_code,
        "language": {
            "language_id": body.language.language_id.as_str(),
            "oj_language_id": body.language.oj_language_id,
        },
        "submitted_source": source_map,
        "fingerprint": body.fingerprint.as_str(),
        "verifies": body
            .verifies
            .iter()
            .map(|l| l.as_str().to_string())
            .collect::<Vec<_>>(),
        "started_at": body.started_at.to_rfc3339(),
    })
}

/// Hidden serialization helper: return the canonical projection JSON bytes
/// used by the plan hash. Kept internal (`pub(crate)`) so tests and future
/// prepare/start wiring can round-trip the plan without exposing a public
/// CLI surface.
#[allow(dead_code)]
pub(crate) fn canonical_plan_bytes(body: &SubmissionPlanBody) -> Vec<u8> {
    canonical_json(&canonical_plan_projection(body))
}

fn canonical_json(value: &serde_json::Value) -> Vec<u8> {
    let text = serde_json::to_string(value).expect("Value serializes as JSON");
    let reparsed: serde_json::Value = serde_json::from_str(&text).expect("just serialized JSON");
    let mut out = Vec::with_capacity(text.len());
    serde_json::to_writer(&mut out, &reparsed).expect("compact serializer never fails");
    out
}

/// Minimal standard-alphabet base64 encoder without padding hooks.
///
/// Pulling `base64` as a dependency just for this hidden helper is overkill,
/// and the encoder is stable, dependency-free, and fully covered by the
/// deterministic-plan tests below.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let b0 = bytes[i];
        let b1 = bytes[i + 1];
        let b2 = bytes[i + 2];
        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        out.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        out.push(ALPHABET[(b2 & 0x3f) as usize] as char);
        i += 3;
    }
    match bytes.len() - i {
        0 => {}
        1 => {
            let b0 = bytes[i];
            out.push(ALPHABET[(b0 >> 2) as usize] as char);
            out.push(ALPHABET[((b0 & 0x03) << 4) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let b0 = bytes[i];
            let b1 = bytes[i + 1];
            out.push(ALPHABET[(b0 >> 2) as usize] as char);
            out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            out.push(ALPHABET[((b1 & 0x0f) << 2) as usize] as char);
            out.push('=');
        }
        _ => unreachable!(),
    }
    out
}

/// Companion to [`base64_encode`]: decode a standard-alphabet padded base64
/// string back to bytes. Kept dependency-free like the encoder above.
fn base64_decode(input: &str) -> anyhow::Result<Vec<u8>> {
    use anyhow::anyhow;
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (i, b) in ALPHABET.iter().enumerate() {
        lookup[*b as usize] = i as u8;
    }
    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(anyhow!("base64 payload not a multiple of 4"));
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut i = 0;
    while i < bytes.len() {
        let (c0, c1, c2, c3) = (bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]);
        let b0 = lookup[c0 as usize];
        let b1 = lookup[c1 as usize];
        if b0 == 255 || b1 == 255 {
            return Err(anyhow!("invalid base64 character"));
        }
        out.push((b0 << 2) | (b1 >> 4));
        // Padding is only valid in the final quartet; any `=` before the last
        // group means the payload had a run of ignored bytes after it.
        let is_last_quartet = i + 4 == bytes.len();
        if c2 == b'=' {
            if c3 != b'=' || !is_last_quartet {
                return Err(anyhow!("misplaced base64 padding"));
            }
            break;
        }
        let b2 = lookup[c2 as usize];
        if b2 == 255 {
            return Err(anyhow!("invalid base64 character"));
        }
        out.push(((b1 & 0x0f) << 4) | (b2 >> 2));
        if c3 == b'=' {
            if !is_last_quartet {
                return Err(anyhow!("misplaced base64 padding"));
            }
            break;
        }
        let b3 = lookup[c3 as usize];
        if b3 == 255 {
            return Err(anyhow!("invalid base64 character"));
        }
        out.push(((b2 & 0x03) << 6) | b3);
        i += 4;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use crate::id_generator::SequenceIdGenerator;
    use crate::verification::fingerprint::FingerprintSource;
    use domain::library::{LanguageId, LibraryId, SolutionId};
    use domain::verification::LanguageBinding;

    fn fixed_now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap()
    }

    fn fingerprint() -> VerifyFingerprint {
        VerifyFingerprint::parse(
            "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        )
        .unwrap()
    }

    fn binding() -> LanguageBinding {
        LanguageBinding {
            language_id: LanguageId::parse("rust").unwrap(),
            oj_language_id: "rust".into(),
        }
    }

    fn input(prev: Option<AttemptId>) -> PrepareVerificationInput<'static> {
        static ID: std::sync::OnceLock<SolutionId> = std::sync::OnceLock::new();
        let id = ID.get_or_init(|| SolutionId::parse("abc999/a/main").unwrap());
        PrepareVerificationInput {
            solution_id: id,
            oj: "librarychecker".into(),
            contest_id: "aplusb".into(),
            problem_code: "aplusb".into(),
            language: binding(),
            submitted_source: FingerprintSource {
                path: "solutions/abc999/a/main/src/main.rs".into(),
                bytes: b"fn main() {}".to_vec(),
            },
            fingerprint: fingerprint(),
            verifies: vec![
                LibraryId::parse("libraries/rust/a.rs").unwrap(),
                LibraryId::parse("libraries/rust/b.rs").unwrap(),
            ],
            previous_attempt_id: prev,
        }
    }

    #[test]
    fn plan_hash_and_body_are_deterministic() {
        let clock = FixedClock(fixed_now());
        let a =
            build_submission_plan(input(None), &clock, &SequenceIdGenerator::new("run1")).unwrap();
        let b =
            build_submission_plan(input(None), &clock, &SequenceIdGenerator::new("run1")).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.plan_hash, b.plan_hash);
    }

    #[test]
    fn plan_json_projection_is_canonical() {
        let clock = FixedClock(fixed_now());
        let plan =
            build_submission_plan(input(None), &clock, &SequenceIdGenerator::new("x")).unwrap();
        let bytes = canonical_plan_bytes(&plan.body);
        // reparse and dump again through the same canonicalizer — bytes must match.
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let round_trip = canonical_json(&value);
        assert_eq!(bytes, round_trip);
        // key ordering is enforced (BTreeMap serialization).
        let text = std::str::from_utf8(&bytes).unwrap();
        let attempt_pos = text.find("\"attempt_id\"").unwrap();
        let fingerprint_pos = text.find("\"fingerprint\"").unwrap();
        let started_pos = text.find("\"started_at\"").unwrap();
        assert!(attempt_pos < fingerprint_pos);
        assert!(fingerprint_pos < started_pos);
    }

    #[test]
    fn plan_freezes_submitted_source_bytes() {
        let clock = FixedClock(fixed_now());
        let mut input = input(None);
        input.submitted_source.bytes = b"first".to_vec();
        let plan = build_submission_plan(input, &clock, &SequenceIdGenerator::new("x")).unwrap();
        assert_eq!(plan.body.submitted_source_bytes, b"first");
        // hash is stable and refers exactly to those bytes.
        let expected_hash = FingerprintSource {
            path: plan.body.submitted_source_path.clone(),
            bytes: b"first".to_vec(),
        }
        .hash();
        assert_eq!(plan.body.submitted_source_hash, expected_hash);
    }

    #[test]
    fn plan_carries_replaces_attempt_id_from_input() {
        let clock = FixedClock(fixed_now());
        let prev = AttemptId::parse("prev-attempt").unwrap();
        let plan = build_submission_plan(
            input(Some(prev.clone())),
            &clock,
            &SequenceIdGenerator::new("x"),
        )
        .unwrap();
        assert_eq!(plan.body.replaces_attempt_id.as_ref(), Some(&prev));

        // With no previous attempt, the field is None and the hash differs.
        let plan_no_prev =
            build_submission_plan(input(None), &clock, &SequenceIdGenerator::new("x")).unwrap();
        assert_eq!(plan_no_prev.body.replaces_attempt_id, None);
        assert_ne!(plan.plan_hash, plan_no_prev.plan_hash);
    }

    #[test]
    fn plan_hash_reflects_source_bytes_and_language() {
        let clock = FixedClock(fixed_now());
        let plan_a =
            build_submission_plan(input(None), &clock, &SequenceIdGenerator::new("x")).unwrap();

        let mut mutated = input(None);
        mutated.submitted_source.bytes = b"a-different-body".to_vec();
        let plan_b =
            build_submission_plan(mutated, &clock, &SequenceIdGenerator::new("x")).unwrap();
        assert_ne!(plan_a.plan_hash, plan_b.plan_hash);

        let mut mutated_lang = input(None);
        mutated_lang.language.oj_language_id = "rust-2024".into();
        let plan_c =
            build_submission_plan(mutated_lang, &clock, &SequenceIdGenerator::new("x")).unwrap();
        assert_ne!(plan_a.plan_hash, plan_c.plan_hash);
    }

    #[test]
    fn plan_empty_submission_bytes_error() {
        let clock = FixedClock(fixed_now());
        let mut i = input(None);
        i.submitted_source.bytes.clear();
        let err = build_submission_plan(i, &clock, &SequenceIdGenerator::new("x")).unwrap_err();
        assert_eq!(err, PlanError::EmptySubmittedSource);
    }

    #[test]
    fn plan_verify_list_must_be_sorted_and_unique() {
        let clock = FixedClock(fixed_now());
        let mut i = input(None);
        i.verifies = vec![
            LibraryId::parse("libraries/rust/b.rs").unwrap(),
            LibraryId::parse("libraries/rust/a.rs").unwrap(),
        ];
        assert_eq!(
            build_submission_plan(i, &clock, &SequenceIdGenerator::new("x")).unwrap_err(),
            PlanError::UnsortedOrDuplicateVerifyList,
        );

        let mut i2 = input(None);
        i2.verifies = vec![
            LibraryId::parse("libraries/rust/a.rs").unwrap(),
            LibraryId::parse("libraries/rust/a.rs").unwrap(),
        ];
        assert_eq!(
            build_submission_plan(i2, &clock, &SequenceIdGenerator::new("x")).unwrap_err(),
            PlanError::UnsortedOrDuplicateVerifyList,
        );
    }

    #[test]
    fn plan_starting_record_carries_frozen_metadata() {
        let clock = FixedClock(fixed_now());
        let plan =
            build_submission_plan(input(None), &clock, &SequenceIdGenerator::new("x")).unwrap();
        let record = plan.as_starting_record();
        assert_eq!(record.solution_id, plan.body.solution_id);
        assert_eq!(record.attempt_id, plan.body.attempt_id);
        assert_eq!(record.fingerprint, plan.body.fingerprint);
        match record.state {
            VerificationState::Starting(s) => {
                assert_eq!(s.plan_hash, plan.plan_hash);
                assert_eq!(s.submitted_source_hash, plan.body.submitted_source_hash);
                assert_eq!(s.language, plan.body.language);
                assert_eq!(s.started_at, plan.body.started_at);
            }
            _ => panic!("expected Starting state"),
        }
    }

    #[test]
    fn base64_encode_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_decode_rejects_padding_before_final_quartet() {
        // Padding is only legal in the last quartet — `TQ==AAAA` used to be
        // accepted and silently drop the `AAAA` tail. Regression guard.
        assert!(base64_decode("TQ==AAAA").is_err());
        assert!(base64_decode("Zm8=Zm8=").is_err());
        // Valid inputs still round-trip.
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
    }
}
