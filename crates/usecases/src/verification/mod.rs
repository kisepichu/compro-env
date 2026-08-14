//! Verification planning use-case modules (spec §8, §10, §11).
//!
//! Pure functions that consume the immutable [`AnalysisSnapshot`] plus the
//! saved [`VerificationRecord`] and produce dependency closures, canonical
//! fingerprints, and public verification statuses. Nothing in this module
//! touches the filesystem or the network; every input is provided by the
//! caller so plan creation is deterministic and reproducible.
//!
//! [`AnalysisSnapshot`]: domain::analysis::AnalysisSnapshot
//! [`VerificationRecord`]: domain::verification::VerificationRecord

pub mod candidate;
pub mod fingerprint;
pub mod plan;
pub mod status;
pub mod transition;

pub use candidate::select_next_candidate;
pub use fingerprint::{
    FINGERPRINT_SCHEMA_VERSION, FingerprintError, FingerprintMaterial, FingerprintSource,
    OjBinding, calculate_fingerprint, verification_closure,
};
pub use plan::{
    PLAN_SCHEMA_VERSION, PlanError, PrepareVerificationInput, SubmissionPlan, SubmissionPlanBody,
    build_submission_plan,
};
pub use status::{VerificationStatus, classify_library_status, classify_solution_status};
pub use transition::{InvalidTransition, VerificationEvent, apply_transition};
