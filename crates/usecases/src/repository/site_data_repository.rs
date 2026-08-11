//! Port for persisting the generated public site data (spec §12, §14).
//!
//! The site-data build must be *atomic from the reader's perspective*: web
//! CI / preview servers watch a target directory and load JSON from it, so
//! partially-written trees can never appear at the target path. Implementations
//! stage output in a sibling temp directory and swap it into place with a
//! single rename.

use anyhow::Result;
use std::path::Path;

use site_schema::SiteData;

pub trait SiteDataRepository {
    /// Writes the projected `SiteData` beneath `output_dir` as
    /// `site-data.json` and any accompanying artifacts. Existing content at
    /// `output_dir` is replaced atomically; interrupted runs leave the target
    /// either fully old or fully new.
    fn write_atomically(&self, output_dir: &Path, data: &SiteData) -> Result<()>;
}
