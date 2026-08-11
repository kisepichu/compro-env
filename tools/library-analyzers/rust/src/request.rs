//! Strict `AnalysisRequest` parsing for the ce-rust adapter (plan 043 Task 1).
//!
//! The Rust adapter accepts a single JSON document on stdin. Anything that
//! deviates from the shared protocol — an unknown schema version, an
//! unexpected language tag, or an unknown field — is rejected before any
//! analysis begins.

use library_adapter_protocol::{AnalysisRequest, SCHEMA_VERSION};
use thiserror::Error;

/// Language tag this adapter answers to. The core dispatches by this string.
pub const LANGUAGE: &str = "rust";

#[derive(Debug, Error)]
pub enum RequestError {
    #[error("failed to parse AnalysisRequest as JSON: {source}")]
    Parse {
        #[source]
        source: serde_json::Error,
    },
    #[error("unsupported schema_version {actual}; expected {expected}")]
    Version { actual: u32, expected: u32 },
    #[error("unexpected language {actual:?}; ce-rust only handles {expected:?}")]
    Language { actual: String, expected: String },
}

/// Parse and strictly validate an `AnalysisRequest`.
pub fn parse_request(input: &str) -> Result<AnalysisRequest, RequestError> {
    let request: AnalysisRequest =
        serde_json::from_str(input).map_err(|source| RequestError::Parse { source })?;
    if request.schema_version != SCHEMA_VERSION {
        return Err(RequestError::Version {
            actual: request.schema_version,
            expected: SCHEMA_VERSION,
        });
    }
    if request.language != LANGUAGE {
        return Err(RequestError::Language {
            actual: request.language,
            expected: LANGUAGE.to_string(),
        });
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty(language: &str) -> String {
        format!(
            r#"{{"schema_version":1,"repository_root":".","language":"{language}","libraries":[],"solutions":[]}}"#
        )
    }

    #[test]
    fn accepts_empty_rust_request() {
        let req = parse_request(&empty("rust")).unwrap();
        assert!(req.libraries.is_empty());
        assert!(req.solutions.is_empty());
    }

    #[test]
    fn rejects_wrong_language() {
        let err = parse_request(&empty("cpp")).unwrap_err();
        assert!(matches!(err, RequestError::Language { .. }));
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let raw = r#"{"schema_version":2,"repository_root":".","language":"rust","libraries":[],"solutions":[]}"#;
        let err = parse_request(raw).unwrap_err();
        assert!(matches!(err, RequestError::Version { .. }));
    }

    #[test]
    fn rejects_unknown_fields() {
        let raw = r#"{"schema_version":1,"repository_root":".","language":"rust","libraries":[],"solutions":[],"extra":true}"#;
        let err = parse_request(raw).unwrap_err();
        assert!(matches!(err, RequestError::Parse { .. }));
    }
}
