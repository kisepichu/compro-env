//! JSON Schema generation for the public site-data model.
//!
//! The generated schema is checked into `web/schema/site-data-v1.schema.json`
//! and validated in CI so hand-written and generated views stay in sync
//! (spec §12).

use std::path::Path;

use anyhow::Context;
use schemars::{Schema, SchemaGenerator};
use serde_json::json;

use crate::{SITE_SCHEMA_VERSION, SiteData};

/// Builds the JSON Schema describing a `site-data.json` document at
/// [`SITE_SCHEMA_VERSION`].
pub fn site_data_schema() -> Schema {
    let mut generator = SchemaGenerator::default();
    let root_ref = generator.subschema_for::<SiteData>();

    let mut root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": format!("compro-env site-data v{SITE_SCHEMA_VERSION}"),
        "description": "Public projection of libraries, solutions, and verification state served by the static site.",
    });
    if let serde_json::Value::Object(ref mut map) = root {
        if let serde_json::Value::Object(root_ref_map) =
            serde_json::to_value(&root_ref).expect("subschema_for returns a JSON object")
        {
            for (key, value) in root_ref_map {
                map.insert(key, value);
            }
        }
        map.insert(
            "$defs".to_string(),
            serde_json::Value::Object(generator.definitions().clone()),
        );
    }

    Schema::try_from(root).expect("site_data_schema is a valid JSON Schema object")
}

/// Writes the canonical schema to `path`, creating any missing parent
/// directories.
pub fn write_site_data_schema(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating parent directory of {}", path.display()))?;
    }
    let bytes = serialize_schema(&site_data_schema());
    std::fs::write(path, bytes)
        .with_context(|| format!("writing site-data schema to {}", path.display()))?;
    Ok(())
}

/// Deterministic pretty JSON encoding with a trailing newline.
pub fn serialize_schema(schema: &Schema) -> Vec<u8> {
    let mut bytes =
        serde_json::to_vec_pretty(schema).expect("Schema serializes to JSON without errors");
    bytes.push(b'\n');
    bytes
}

/// Walks the object keys of an arbitrary JSON value and returns the first key
/// that matches [`forbidden_key`]. Used by the drift test to catch accidental
/// leaks of private fields.
pub fn find_forbidden_key(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if forbidden_key(key) {
                    return Some(key.clone());
                }
                if let Some(nested) = find_forbidden_key(child) {
                    return Some(nested);
                }
            }
            None
        }
        serde_json::Value::Array(items) => items.iter().find_map(find_forbidden_key),
        _ => None,
    }
}

/// Public-DTO denylist for keys that must never appear in a serialized
/// `SiteData` document (spec §12, §14).
pub fn forbidden_key(key: &str) -> bool {
    matches!(
        key,
        "private"
            | "private_dependencies"
            | "private_paths"
            | "token"
            | "tokens"
            | "session"
            | "sessions"
            | "cookie"
            | "cookies"
            | "authorization"
            | "raw_oj_response"
            | "raw_response"
            | "absolute_path"
            | "repository_absolute_path"
            | "internal_path"
            | "internal_paths"
    )
}
