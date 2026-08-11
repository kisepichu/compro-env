//! Public site-data DTO shared between the Rust CLI and the static Web build.
//!
//! Every type here is a *public projection*: sessions, tokens, cookies,
//! headers, raw OJ responses, private library paths, private sources, private
//! diagnostics, and internal dependency counts must never appear. New fields
//! must be added at the DTO layer so `site_data_schema()` and the checked-in
//! JSON Schema stay in sync (spec §12, §14).

pub mod model;
pub mod schema;

pub use model::*;

/// Public site-data schema version.
///
/// Breaking changes bump this integer; the Astro build refuses inputs whose
/// `schema_version` does not match the value the crate was built against.
pub const SITE_SCHEMA_VERSION: u32 = 1;
