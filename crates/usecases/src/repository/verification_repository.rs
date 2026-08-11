//! Latest verification record repository port (spec §11).
//!
//! Implementations persist at most one record per solution, tie every update
//! to a prior `AttemptId`, and reject any record whose location does not match
//! its embedded `solution_id`.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use domain::library::SolutionId;
use domain::verification::{AttemptId, VerificationRecord};

pub trait VerificationRepository {
    /// Returns the latest record for `id`, or `Ok(None)` when no record is
    /// stored yet. Corrupt records surface as `Err`.
    fn load(&self, id: &SolutionId) -> Result<Option<VerificationRecord>>;

    /// Loads every stored record, cross-checking each against `discovered`
    /// so orphan records raise an error rather than silently disappear.
    fn load_all(
        &self,
        discovered: &BTreeSet<SolutionId>,
    ) -> Result<BTreeMap<SolutionId, VerificationRecord>>;

    /// Atomic replacement gated by the caller-supplied prior attempt.
    ///
    /// - `expected == None` requires the record to not yet exist.
    /// - `expected == Some(a)` requires the stored record's `attempt_id` to
    ///   equal `a`.
    /// - `next.replaces_attempt_id` must equal `expected.cloned()`.
    fn compare_and_swap(
        &self,
        id: &SolutionId,
        expected: Option<&AttemptId>,
        next: &VerificationRecord,
    ) -> Result<()>;

    /// Deletes the record for `id` iff its `attempt_id` matches `expected`.
    fn remove_if_attempt(&self, id: &SolutionId, expected: &AttemptId) -> Result<()>;
}
