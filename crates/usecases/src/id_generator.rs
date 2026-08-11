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
}
