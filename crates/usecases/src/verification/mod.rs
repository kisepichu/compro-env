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

pub mod fingerprint;
pub mod status;

pub use fingerprint::{
    FINGERPRINT_SCHEMA_VERSION, FingerprintError, FingerprintMaterial, FingerprintSource,
    OjBinding, calculate_fingerprint, verification_closure,
};
pub use status::{VerificationStatus, classify_library_status, classify_solution_status};
