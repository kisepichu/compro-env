//! Fan-out over language adapters via [`LibraryAdapterRunner`] (spec §6.9).
//!
//! Constructs one [`AnalysisRequest`] per language from the discovery
//! manifest, invokes the adapter runner, and returns the map of responses to
//! [`crate::library_analysis`]-style consumers.
//!
//! The adapter executables must be present on disk; `find_adapter_executable`
//! resolves each language's declared `analyzer.command[0]` relative to the
//! repository root. A missing executable surfaces as a clear error rather
//! than a silent skip.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use domain::analysis::DiscoveryManifest;
use domain::library::{LanguageId, LibraryProjectConfig};
use library_adapter_protocol::{
    AnalysisRequest, AnalysisResponse, LibraryTarget, SCHEMA_VERSION, SolutionTarget,
};
use usecases::library_adapter::LibraryAdapterRunner;
use usecases::library_analyzer::LibraryAnalyzer;

pub struct ProcessLibraryAnalyzer<R: LibraryAdapterRunner> {
    runner: R,
    config: LibraryProjectConfig,
    /// One sanitized environment per registered language. `analyze_all`
    /// looks up the language ID here so Lean picks up `CE_LEAN_ROOT` and
    /// its bin/lib prepends while Rust/C++ receive the shared allowlist.
    envs: BTreeMap<LanguageId, BTreeMap<String, String>>,
}

impl<R: LibraryAdapterRunner> ProcessLibraryAnalyzer<R> {
    pub fn new(
        runner: R,
        config: LibraryProjectConfig,
        envs: BTreeMap<LanguageId, BTreeMap<String, String>>,
    ) -> Self {
        Self {
            runner,
            config,
            envs,
        }
    }
}

impl<R: LibraryAdapterRunner> LibraryAnalyzer for ProcessLibraryAnalyzer<R> {
    fn analyze_all(
        &self,
        repository_root: &Path,
        manifest: &DiscoveryManifest,
    ) -> Result<BTreeMap<LanguageId, AnalysisResponse>> {
        let mut out = BTreeMap::new();
        for language_id in manifest.languages.keys() {
            let request = build_request(repository_root, language_id, manifest)?;
            let executable =
                resolve_adapter_executable(repository_root, &self.config, language_id)?;
            let language_cfg = self.config.languages.get(language_id).ok_or_else(|| {
                anyhow!("no [library.languages.{}] in project config", language_id)
            })?;
            let env = self.envs.get(language_id).ok_or_else(|| {
                anyhow!(
                    "no analyzer environment configured for language `{}`",
                    language_id.as_str()
                )
            })?;
            let response = self
                .runner
                .analyze(
                    &executable,
                    &request,
                    Duration::from_secs(u64::from(language_cfg.analyzer.timeout_seconds)),
                    env,
                )
                .map_err(|err| {
                    anyhow!(
                        "adapter for language `{}` failed: {}",
                        language_id.as_str(),
                        err
                    )
                })?;
            out.insert(language_id.clone(), response);
        }
        Ok(out)
    }
}

fn build_request(
    repo_root: &Path,
    language: &LanguageId,
    manifest: &DiscoveryManifest,
) -> Result<AnalysisRequest> {
    let libraries: Vec<LibraryTarget> = manifest
        .libraries
        .iter()
        .filter(|lib| &lib.language == language)
        .map(|lib| LibraryTarget {
            path: lib.source_path.clone(),
        })
        .collect();
    let solutions: Vec<SolutionTarget> = manifest
        .solutions
        .iter()
        .filter(|sol| &sol.language == language)
        .map(|sol| SolutionTarget {
            id: sol.id.as_str().to_string(),
            root: sol.root.clone(),
            entry: sol.entry.clone(),
        })
        .collect();
    Ok(AnalysisRequest {
        schema_version: SCHEMA_VERSION,
        repository_root: repo_root.display().to_string(),
        language: language.as_str().to_string(),
        libraries,
        solutions,
    })
}

fn resolve_adapter_executable(
    repo_root: &Path,
    config: &LibraryProjectConfig,
    language: &LanguageId,
) -> Result<PathBuf> {
    let language_cfg = config
        .languages
        .get(language)
        .ok_or_else(|| anyhow!("no [library.languages.{}] in project config", language))?;
    let first = language_cfg
        .analyzer
        .command
        .first()
        .ok_or_else(|| anyhow!("language `{}` has an empty analyzer.command", language))?;
    let candidate = PathBuf::from(first);
    let full = if candidate.is_absolute() {
        candidate
    } else {
        repo_root.join(&candidate)
    };
    if !full.exists() {
        bail!(
            "adapter executable for language `{}` not found at {}",
            language,
            full.display()
        );
    }
    Ok(full)
}
