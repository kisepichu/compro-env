//! Port and error taxonomy for running language adapter processes (spec §6.9).
//!
//! Use cases depend on this trait so the pipeline can swap in test fakes or a
//! future in-process runner without pulling `std::process` into the domain or
//! use-case layers.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use library_adapter_protocol::{AnalysisRequest, AnalysisResponse, ProtocolVersionError};
use thiserror::Error;

/// Runs an adapter executable and returns its parsed response.
pub trait LibraryAdapterRunner {
    fn analyze(
        &self,
        executable: &Path,
        request: &AnalysisRequest,
        timeout: Duration,
        environment: &BTreeMap<String, String>,
    ) -> Result<AnalysisResponse, AdapterRunError>;
}

/// Failure modes documented in spec §6.3 and §6.9.
#[derive(Debug, Error)]
pub enum AdapterRunError {
    #[error("failed to spawn adapter {command:?}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "adapter {command:?} did not finish within {timeout_ms}ms\n\
         stderr tail:\n{stderr_tail}"
    )]
    Timeout {
        command: String,
        timeout_ms: u128,
        stderr_tail: String,
    },

    #[error("adapter {command:?} exited with {status}\nstderr tail:\n{stderr_tail}")]
    NonZeroExit {
        command: String,
        status: String,
        stderr_tail: String,
    },

    #[error(
        "adapter {command:?} exceeded the {limit_bytes}-byte stdout limit\n\
         stderr tail:\n{stderr_tail}"
    )]
    StdoutLimit {
        command: String,
        limit_bytes: usize,
        stderr_tail: String,
    },

    #[error("adapter {command:?} produced non-UTF-8 stdout\nstderr tail:\n{stderr_tail}")]
    StdoutNotUtf8 {
        command: String,
        stderr_tail: String,
    },

    #[error("failed to serialize adapter request to JSON: {source}")]
    RequestSerialization {
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "adapter {command:?} produced invalid JSON on stdout: {source}\n\
         stderr tail:\n{stderr_tail}"
    )]
    InvalidJson {
        command: String,
        #[source]
        source: serde_json::Error,
        stderr_tail: String,
    },

    #[error(
        "adapter {command:?} reported {source}\n\
         stderr tail:\n{stderr_tail}"
    )]
    ProtocolVersion {
        command: String,
        #[source]
        source: ProtocolVersionError,
        stderr_tail: String,
    },

    #[error("adapter {command:?} I/O failure: {source}")]
    Io {
        command: String,
        #[source]
        source: std::io::Error,
    },
}
