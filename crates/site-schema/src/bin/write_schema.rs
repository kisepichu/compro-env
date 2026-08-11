//! Writes the canonical `site-data-v1.schema.json` file next to `web/schema/`.
//!
//! Invoked manually (or from CI) whenever `SITE_SCHEMA_VERSION` or a DTO
//! changes; the `checked_in_schema_matches_generated_schema` test fails until
//! the file is regenerated and committed.

use std::path::PathBuf;

use site_schema::schema::write_site_data_schema;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path: PathBuf = match args.next() {
        Some(arg) => PathBuf::from(arg),
        None => {
            let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
                .expect("CARGO_MANIFEST_DIR is set during cargo run");
            PathBuf::from(manifest)
                .join("..")
                .join("..")
                .join("web")
                .join("schema")
                .join("site-data-v1.schema.json")
        }
    };
    write_site_data_schema(&path)?;
    eprintln!("wrote {}", path.display());
    Ok(())
}
