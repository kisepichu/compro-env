use library_adapter_protocol::schema::{analysis_schema, serialize_schema, write_analysis_schema};
use library_adapter_protocol::{
    AnalysisRequest, AnalysisResponse, ProtocolVersionError, SCHEMA_VERSION, validate_version,
};

const EMPTY_REQUEST: &str =
    include_str!("../../../tools/library-analyzers/protocol/fixtures/empty-request.json");
const EMPTY_RESPONSE: &str =
    include_str!("../../../tools/library-analyzers/protocol/fixtures/empty-response.json");
const INVALID_VERSION_RESPONSE: &str = include_str!(
    "../../../tools/library-analyzers/protocol/fixtures/invalid-version-response.json"
);
const CHECKED_IN_SCHEMA: &str =
    include_str!("../../../tools/library-analyzers/protocol/analysis-v1.schema.json");

#[test]
fn empty_protocol_fixture_round_trips() {
    let request: AnalysisRequest = serde_json::from_str(EMPTY_REQUEST).unwrap();
    assert_eq!(request.schema_version, SCHEMA_VERSION);
    assert!(request.libraries.is_empty());
    assert!(request.solutions.is_empty());

    let response: AnalysisResponse = serde_json::from_str(EMPTY_RESPONSE).unwrap();
    assert_eq!(response.schema_version, request.schema_version);
    assert!(response.libraries.is_empty());
    assert!(response.solutions.is_empty());
    validate_version(response.schema_version).unwrap();
}

#[test]
fn invalid_version_fixture_deserializes_but_fails_validation() {
    // The fixture still parses because the version is a valid `u32`.
    let response: AnalysisResponse = serde_json::from_str(INVALID_VERSION_RESPONSE).unwrap();
    assert_ne!(response.schema_version, SCHEMA_VERSION);
    let err = validate_version(response.schema_version).unwrap_err();
    assert_eq!(
        err,
        ProtocolVersionError {
            actual: 2,
            expected: SCHEMA_VERSION,
        }
    );
}

#[test]
fn unknown_top_level_field_is_rejected() {
    let bad = r#"{
        "schema_version": 1,
        "repository_root": ".",
        "language": "rust",
        "libraries": [],
        "solutions": [],
        "extra_key": true
    }"#;
    let err = serde_json::from_str::<AnalysisRequest>(bad).unwrap_err();
    assert!(
        err.to_string().contains("extra_key"),
        "expected `extra_key` in serde error, got: {err}"
    );
}

#[test]
fn unknown_dependency_field_is_rejected() {
    let bad = r#"{
        "schema_version": 1,
        "adapter": { "name": "a", "version": "0", "toolchains": [] },
        "libraries": [{
            "path": "libraries/rust/x.rs",
            "dependency_analysis": {
                "state": "complete",
                "dependencies": [
                    { "kind": "internal", "path": "libraries/rust/y.rs", "extra": 1 }
                ]
            },
            "symbol_analysis": { "state": "complete", "symbols": [] },
            "diagnostics": []
        }],
        "solutions": []
    }"#;
    let err = serde_json::from_str::<AnalysisResponse>(bad).unwrap_err();
    assert!(
        err.to_string().contains("extra"),
        "expected `extra` in serde error, got: {err}"
    );
}

#[test]
fn checked_in_schema_matches_generated_schema() {
    let schema = analysis_schema();
    let generated = serialize_schema(&schema);
    let checked_in = CHECKED_IN_SCHEMA.as_bytes();
    if generated != checked_in {
        // Write the drift into a tempfile for easy manual inspection.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        write_analysis_schema(tmp.path()).unwrap();
        panic!(
            "analysis-v1.schema.json is out of date; regenerate with \
             `REGEN_ANALYSIS_SCHEMA=1 cargo test -p library-adapter-protocol` \
             (drift written to {})",
            tmp.path().display()
        );
    }
}

/// Opt-in helper that rewrites the checked-in schema file. Set
/// `REGEN_ANALYSIS_SCHEMA=1` when the protocol types intentionally change so
/// the checked-in JSON tracks them exactly.
#[test]
fn regenerate_checked_in_schema_when_requested() {
    if std::env::var_os("REGEN_ANALYSIS_SCHEMA").is_none() {
        return;
    }
    let repo_root = env!("CARGO_MANIFEST_DIR");
    let path = std::path::Path::new(repo_root)
        .join("..")
        .join("..")
        .join("tools")
        .join("library-analyzers")
        .join("protocol")
        .join("analysis-v1.schema.json");
    write_analysis_schema(&path).unwrap();
}
