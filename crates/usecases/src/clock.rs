//! Wall-clock abstraction used by the verification planning use case.
//!
//! Kept minimal so tests can inject a deterministic clock without pulling in
//! a full timekeeping crate.

use chrono::{DateTime, FixedOffset};

/// Returns the current wall clock time.
pub trait Clock {
    fn now(&self) -> DateTime<FixedOffset>;
}

/// [`Clock`] that returns a fixed timestamp for every call.
///
/// Used by tests to keep plan hashes and record timestamps deterministic.
pub struct FixedClock(pub DateTime<FixedOffset>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    #[test]
    fn fixed_clock_returns_same_value_every_call() {
        let stamp = DateTime::parse_from_rfc3339("2026-08-10T09:00:00+00:00").unwrap();
        let clock = FixedClock(stamp);
        assert_eq!(clock.now(), stamp);
        assert_eq!(clock.now(), stamp);
    }
}
