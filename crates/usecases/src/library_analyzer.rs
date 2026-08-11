//! Fan-out over language adapters (spec §6.4, §6.9).
//!
//! Implementations invoke the per-language adapter (via
//! [`LibraryAdapterRunner`](crate::library_adapter::LibraryAdapterRunner) or an
//! in-process fake) and hand back one [`AnalysisResponse`] per language in the
//! discovery manifest.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use domain::analysis::DiscoveryManifest;
use domain::library::LanguageId;
use library_adapter_protocol::AnalysisResponse;

pub trait LibraryAnalyzer {
    /// Return one [`AnalysisResponse`] per language in `manifest`. Callers
    /// hand the map to [`crate::library_analysis::normalize_analysis`] to
    /// build an immutable snapshot.
    fn analyze_all(
        &self,
        repository_root: &Path,
        manifest: &DiscoveryManifest,
    ) -> Result<BTreeMap<LanguageId, AnalysisResponse>>;
}
