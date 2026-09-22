//! Cross-workflow retry backoff schedule (spec §8.3).
//!
//! Persisters call [`retry_delay`] with the new [`InfrastructureFailure`]
//! `retry_count` and add the result to `updated_at` to obtain
//! `next_retry_at`. The `Retry-After` hint (spec §8.3) is layered separately
//! in `submission_lifecycle::sleep_with_hint` for intra-command waits.
//!
//! [`InfrastructureFailure`]: domain::verification::InfrastructureFailure

use std::time::Duration;

/// Retry cap for cross-workflow retries (spec §8.3 "最終的な上限を 6 時間").
const RETRY_CAP: Duration = Duration::from_secs(6 * 60 * 60);
/// Base delay for the first retry (spec §8.3 "5 分から始め").
const RETRY_BASE_MINUTES: u64 = 5;

/// Delay from the persist timestamp to the next eligible retry (spec §8.3).
///
/// Follows the schedule `5 → 10 → 20 → 40 → 80 → …` minutes, doubling each
/// step and capped at 6 hours. `retry_count == 0` is treated as `1` so
/// callers that forget to bump the counter never emit a zero deadline.
pub fn retry_delay(retry_count: u32) -> Duration {
    let n = retry_count.max(1);
    // `5 * 2^20 min` already dwarfs the 6h cap; clamp the shift so `1u64
    // << shift` cannot overflow even for absurd `retry_count` values.
    let shift = (n - 1).min(20);
    let minutes = RETRY_BASE_MINUTES.saturating_mul(1u64 << shift);
    let raw = Duration::from_secs(minutes.saturating_mul(60));
    std::cmp::min(raw, RETRY_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_follows_spec_curve() {
        assert_eq!(retry_delay(1), Duration::from_secs(5 * 60));
        assert_eq!(retry_delay(2), Duration::from_secs(10 * 60));
        assert_eq!(retry_delay(3), Duration::from_secs(20 * 60));
        assert_eq!(retry_delay(4), Duration::from_secs(40 * 60));
        assert_eq!(retry_delay(5), Duration::from_secs(80 * 60));
    }

    #[test]
    fn cap_activates_when_doubling_exceeds_six_hours() {
        assert_eq!(retry_delay(6), Duration::from_secs(160 * 60));
        assert_eq!(retry_delay(7), Duration::from_secs(320 * 60));
        // 5 * 2^7 = 640 min > 360 min cap.
        assert_eq!(retry_delay(8), RETRY_CAP);
        assert_eq!(retry_delay(100), RETRY_CAP);
        assert_eq!(retry_delay(u32::MAX), RETRY_CAP);
    }

    #[test]
    fn zero_treated_as_one() {
        assert_eq!(retry_delay(0), retry_delay(1));
    }
}
