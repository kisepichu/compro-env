//! Opaque attempt-id generation port used by the verification planning use case.
//!
//! Kept as a trait so tests can inject deterministic IDs and production code
//! can plug in a UUIDv4 (or similar opaque) generator without infecting the
//! use-case layer with a random-source dependency.

use domain::verification::AttemptId;

/// Emits a fresh, unique [`AttemptId`] on every call.
pub trait AttemptIdGenerator {
    fn generate(&self) -> AttemptId;
}

/// Test-only [`AttemptIdGenerator`] that yields a deterministic sequence.
pub struct SequenceIdGenerator {
    prefix: String,
    counter: std::sync::atomic::AtomicU64,
}

impl SequenceIdGenerator {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            counter: std::sync::atomic::AtomicU64::new(1),
        }
    }
}

impl AttemptIdGenerator for SequenceIdGenerator {
    fn generate(&self) -> AttemptId {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        AttemptId::parse(&format!("{}-{}", self.prefix, n))
            .expect("generated ids are printable ASCII")
    }
}

/// Production [`AttemptIdGenerator`] emitting monotonically-increasing opaque
/// identifiers derived from the current wall-clock nanoseconds plus a per-call
/// counter. The workspace does not depend on `uuid`; this stitched form still
/// meets the schema contract (non-empty printable ASCII, no whitespace) and
/// yields unique IDs even when the wall clock is coarse-grained.
pub struct MonotonicAttemptIdGenerator {
    counter: std::sync::atomic::AtomicU64,
}

impl MonotonicAttemptIdGenerator {
    pub fn new() -> Self {
        Self {
            counter: std::sync::atomic::AtomicU64::new(1),
        }
    }
}

impl Default for MonotonicAttemptIdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl AttemptIdGenerator for MonotonicAttemptIdGenerator {
    fn generate(&self) -> AttemptId {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let now = chrono::Utc::now();
        // `attempt-<epoch-nanos>-<counter>`: 32-hex-ish is unnecessary; we
        // just need a non-empty printable-ASCII slug the schema accepts.
        let raw = format!(
            "attempt-{}-{n:x}",
            now.timestamp_nanos_opt().unwrap_or(now.timestamp())
        );
        AttemptId::parse(&raw).expect("generated attempt id is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_generator_returns_distinct_ids() {
        let generator = SequenceIdGenerator::new("test");
        let a = generator.generate();
        let b = generator.generate();
        assert_ne!(a, b);
        assert_eq!(a.as_str(), "test-1");
        assert_eq!(b.as_str(), "test-2");
    }

    #[test]
    fn monotonic_generator_returns_distinct_ids() {
        let generator = MonotonicAttemptIdGenerator::new();
        let a = generator.generate();
        let b = generator.generate();
        assert_ne!(a, b);
    }
}
