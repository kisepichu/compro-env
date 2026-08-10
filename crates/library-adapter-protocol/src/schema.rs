//! JSON Schema generation for the analyzer protocol.
//!
//! The schema is generated from the strongly typed `AnalysisRequest` and
//! `AnalysisResponse` in `lib.rs` and checked into
//! `tools/library-analyzers/protocol/analysis-v1.schema.json` so external
//! adapters can validate their input and output against a single normative file.

use std::path::Path;

use anyhow::Context;
use schemars::{Schema, SchemaGenerator};
use serde_json::json;

use crate::{AnalysisRequest, AnalysisResponse, SCHEMA_VERSION};

/// Builds the combined JSON Schema describing the request and response documents
/// used at protocol version `SCHEMA_VERSION`.
pub fn analysis_schema() -> Schema {
    let mut generator = SchemaGenerator::default();
    let request = generator.subschema_for::<AnalysisRequest>();
    let response = generator.subschema_for::<AnalysisResponse>();

    let mut root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": format!("Library adapter protocol v{SCHEMA_VERSION}"),
        "description": "Combined JSON Schema for AnalysisRequest and AnalysisResponse.",
        "type": "object",
        "properties": {
            "request": request,
            "response": response,
        },
    });
    if let serde_json::Value::Object(ref mut map) = root {
        map.insert(
            "$defs".to_string(),
            serde_json::Value::Object(generator.definitions().clone()),
        );
    }

    Schema::try_from(root).expect("analysis schema is a valid JSON Schema object")
}

/// Writes the canonical schema to `path`, creating any missing parent directories.
pub fn write_analysis_schema(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating parent directory of {}", path.display()))?;
    }
    let schema = analysis_schema();
    let bytes = serialize_schema(&schema);
    std::fs::write(path, bytes)
        .with_context(|| format!("writing analysis schema to {}", path.display()))?;
    Ok(())
}

/// Serializes the schema deterministically (pretty-printed, trailing newline).
pub fn serialize_schema(schema: &Schema) -> Vec<u8> {
    let mut bytes =
        serde_json::to_vec_pretty(schema).expect("Schema serializes to JSON without errors");
    bytes.push(b'\n');
    bytes
}
