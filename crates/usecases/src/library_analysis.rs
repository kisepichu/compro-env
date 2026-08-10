//! Normalize adapter responses into an immutable analysis snapshot
//! (spec §6.4).
//!
//! The core validates every response against the discovery manifest and the
//! strict adapter protocol before producing a `AnalysisSnapshot`. The snapshot
//! is deterministic: shuffled input maps and dependency arrays produce the
//! same `snapshot_hash`. Adapter and toolchain identity are recorded but do
//! not participate in the hash so verify staleness stays tied to source
//! content.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, anyhow, bail};
use chrono::{DateTime, FixedOffset, Utc};
use domain::analysis::{
    AnalysisSnapshot, AnalysisState, DiagnosticSeverity, DiscoveryManifest, NormalizedDiagnostic,
    NormalizedLanguageAnalysis, NormalizedLibraryAnalysis, NormalizedSolutionAnalysis,
    NormalizedSymbol, SourceLocation, TargetAnalysisState,
};
use domain::library::{ExpectedToolchain, LanguageId, LibraryId, SolutionId};
use library_adapter_protocol::{
    AnalysisResponse, AnalysisState as ProtoState, Dependency, Diagnostic as ProtoDiagnostic,
    LibraryAnalysis as ProtoLibraryAnalysis, Location as ProtoLocation, Position as ProtoPosition,
    SCHEMA_VERSION, Severity as ProtoSeverity, SolutionAnalysis as ProtoSolutionAnalysis,
    Symbol as ProtoSymbol, ToolchainIdentity as ProtoToolchainIdentity, validate_version,
};
use sha2::{Digest, Sha256};

/// Snapshot schema version this pipeline emits.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Normalize adapter responses into an immutable snapshot.
///
/// * `manifest` is the discovery output the request was built from.
/// * `responses` must have exactly one entry per language in the manifest.
/// * `revision` is an opaque repository revision label recorded for audit.
/// * `source_bytes` maps repository-relative paths to raw file bytes; every
///   managed library and published solution entry file must be present.
pub fn normalize_analysis(
    manifest: &DiscoveryManifest,
    responses: BTreeMap<LanguageId, AnalysisResponse>,
    revision: &str,
    source_bytes: &BTreeMap<String, Vec<u8>>,
) -> anyhow::Result<AnalysisSnapshot> {
    validate_response_shape(manifest, &responses)?;

    let mut languages: BTreeMap<LanguageId, NormalizedLanguageAnalysis> = BTreeMap::new();
    for (language_id, response) in responses {
        let normalized = normalize_language(&language_id, response, manifest)?;
        languages.insert(language_id, normalized);
    }

    let source_hashes = build_source_hashes(source_bytes);
    let discovery_hash = compute_discovery_hash(manifest);
    let snapshot_hash = compute_snapshot_hash(
        SNAPSHOT_SCHEMA_VERSION,
        revision,
        &discovery_hash,
        &source_hashes,
        &languages,
    );

    Ok(AnalysisSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        repository_revision: revision.to_string(),
        created_at: created_at_utc(),
        discovery_hash,
        source_hashes,
        languages,
        snapshot_hash,
    })
}

fn created_at_utc() -> DateTime<FixedOffset> {
    // Deterministic timestamp anchor. The plan intentionally keeps
    // `created_at` an audit field only; it does not participate in
    // `snapshot_hash` (§6.4).
    Utc::now().fixed_offset()
}

// ─── Response shape validation ──────────────────────────────────────────────

fn validate_response_shape(
    manifest: &DiscoveryManifest,
    responses: &BTreeMap<LanguageId, AnalysisResponse>,
) -> anyhow::Result<()> {
    let manifest_langs: BTreeSet<&LanguageId> = manifest.languages.keys().collect();
    let response_langs: BTreeSet<&LanguageId> = responses.keys().collect();
    if manifest_langs != response_langs {
        return Err(anyhow!(
            "response language set mismatches manifest: manifest={:?} responses={:?}",
            manifest_langs
                .iter()
                .map(|l| l.as_str())
                .collect::<Vec<_>>(),
            response_langs
                .iter()
                .map(|l| l.as_str())
                .collect::<Vec<_>>(),
        ));
    }
    for (language_id, response) in responses {
        validate_version(response.schema_version).with_context(|| {
            format!("language `{language_id}` adapter returned unsupported schema_version")
        })?;
        if response.schema_version != SCHEMA_VERSION {
            bail!(
                "language `{language_id}` adapter returned schema_version {} but the pipeline uses {SCHEMA_VERSION}",
                response.schema_version
            );
        }
        if response.adapter.name.trim().is_empty() {
            bail!("language `{language_id}` adapter identity `name` is empty");
        }
        if response.adapter.version.trim().is_empty() {
            bail!("language `{language_id}` adapter identity `version` is empty");
        }
        let mut seen_names: BTreeSet<&str> = BTreeSet::new();
        for tc in &response.adapter.toolchains {
            if tc.name.trim().is_empty() || tc.version.trim().is_empty() {
                bail!(
                    "language `{language_id}` adapter reported a toolchain with an empty name or version"
                );
            }
            if !seen_names.insert(tc.name.as_str()) {
                bail!(
                    "language `{language_id}` adapter reported duplicate toolchain name `{}`",
                    tc.name
                );
            }
        }
    }
    Ok(())
}

// ─── Per-language normalization ────────────────────────────────────────────

fn normalize_language(
    language_id: &LanguageId,
    response: AnalysisResponse,
    manifest: &DiscoveryManifest,
) -> anyhow::Result<NormalizedLanguageAnalysis> {
    let manifest_libraries: BTreeSet<LibraryId> = manifest
        .libraries
        .iter()
        .filter(|l| &l.language == language_id)
        .map(|l| l.id.clone())
        .collect();
    let manifest_solutions: BTreeSet<SolutionId> = manifest
        .solutions
        .iter()
        .filter(|s| &s.language == language_id)
        .map(|s| s.id.clone())
        .collect();

    // libraries: must cover the exact set with no duplicates.
    let mut response_libs: BTreeMap<LibraryId, ProtoLibraryAnalysis> = BTreeMap::new();
    for lib in response.libraries {
        let id = LibraryId::parse(&lib.path).with_context(|| {
            format!(
                "language `{language_id}` adapter returned invalid library path {:?}",
                lib.path
            )
        })?;
        if !manifest_libraries.contains(&id) {
            bail!(
                "language `{language_id}` adapter returned library {} that is not in the manifest",
                id
            );
        }
        if response_libs.insert(id.clone(), lib).is_some() {
            bail!(
                "language `{language_id}` adapter returned duplicate library {}",
                id
            );
        }
    }
    let response_lib_keys: BTreeSet<LibraryId> = response_libs.keys().cloned().collect();
    let missing_libs: Vec<&LibraryId> = manifest_libraries.difference(&response_lib_keys).collect();
    if !missing_libs.is_empty() {
        bail!(
            "language `{language_id}` adapter is missing libraries: {:?}",
            missing_libs.iter().map(|i| i.as_str()).collect::<Vec<_>>()
        );
    }

    let mut response_solutions: BTreeMap<SolutionId, ProtoSolutionAnalysis> = BTreeMap::new();
    for solution in response.solutions {
        let id = SolutionId::parse(&solution.id).with_context(|| {
            format!(
                "language `{language_id}` adapter returned invalid solution id {:?}",
                solution.id
            )
        })?;
        if !manifest_solutions.contains(&id) {
            bail!(
                "language `{language_id}` adapter returned solution {} that is not in the manifest",
                id
            );
        }
        if response_solutions.insert(id.clone(), solution).is_some() {
            bail!(
                "language `{language_id}` adapter returned duplicate solution {}",
                id
            );
        }
    }
    let response_solution_keys: BTreeSet<SolutionId> = response_solutions.keys().cloned().collect();
    let missing_solutions: Vec<&SolutionId> = manifest_solutions
        .difference(&response_solution_keys)
        .collect();
    if !missing_solutions.is_empty() {
        bail!(
            "language `{language_id}` adapter is missing solutions: {:?}",
            missing_solutions
                .iter()
                .map(|i| i.as_str())
                .collect::<Vec<_>>()
        );
    }

    let mut libraries_out: BTreeMap<LibraryId, NormalizedLibraryAnalysis> = BTreeMap::new();
    for (id, lib) in response_libs {
        libraries_out.insert(
            id.clone(),
            normalize_library(&id, lib, &manifest_libraries)?,
        );
    }

    let mut solutions_out: BTreeMap<SolutionId, NormalizedSolutionAnalysis> = BTreeMap::new();
    for (id, sol) in response_solutions {
        solutions_out.insert(
            id.clone(),
            normalize_solution(&id, sol, &manifest_libraries)?,
        );
    }

    let observed_toolchains = response
        .adapter
        .toolchains
        .into_iter()
        .map(convert_toolchain)
        .collect();

    Ok(NormalizedLanguageAnalysis {
        language: language_id.clone(),
        adapter_name: response.adapter.name,
        adapter_version: response.adapter.version,
        observed_toolchains,
        libraries: libraries_out,
        solutions: solutions_out,
    })
}

fn convert_toolchain(t: ProtoToolchainIdentity) -> ExpectedToolchain {
    // The observed identity keeps target information via the version string
    // in later plans; for now we preserve just name/version (spec §6.1).
    let mut version = t.version;
    if let Some(target) = t.target {
        // Encode target as an audit suffix so it survives the round-trip.
        version = format!("{version} ({target})");
    }
    ExpectedToolchain {
        name: t.name,
        version,
    }
}

fn normalize_library(
    id: &LibraryId,
    lib: ProtoLibraryAnalysis,
    same_language_libraries: &BTreeSet<LibraryId>,
) -> anyhow::Result<NormalizedLibraryAnalysis> {
    let ProtoLibraryAnalysis {
        path: _,
        dependency_analysis,
        symbol_analysis,
        diagnostics,
    } = lib;

    let (dep_state, direct_edges) = normalize_dependencies(
        id.as_str(),
        dependency_analysis.state,
        dependency_analysis.dependencies,
        same_language_libraries,
    )?;

    let symbols: Vec<NormalizedSymbol> = symbol_analysis
        .symbols
        .into_iter()
        .map(convert_symbol)
        .collect();

    Ok(NormalizedLibraryAnalysis {
        id: id.clone(),
        state: TargetAnalysisState {
            dependency_state: dep_state,
            symbol_state: convert_state(symbol_analysis.state),
        },
        direct_dependencies: direct_edges,
        symbols,
        diagnostics: diagnostics.into_iter().map(convert_diagnostic).collect(),
    })
}

fn normalize_solution(
    id: &SolutionId,
    solution: ProtoSolutionAnalysis,
    same_language_libraries: &BTreeSet<LibraryId>,
) -> anyhow::Result<NormalizedSolutionAnalysis> {
    let ProtoSolutionAnalysis {
        id: _,
        dependency_analysis,
        diagnostics,
    } = solution;
    let (dep_state, direct_edges) = normalize_dependencies(
        id.as_str(),
        dependency_analysis.state,
        dependency_analysis.dependencies,
        same_language_libraries,
    )?;
    Ok(NormalizedSolutionAnalysis {
        solution_id: id.clone(),
        dependency_state: dep_state,
        direct_dependencies: direct_edges,
        diagnostics: diagnostics.into_iter().map(convert_diagnostic).collect(),
    })
}

fn normalize_dependencies(
    context: &str,
    state: ProtoState,
    dependencies: Vec<Dependency>,
    same_language_libraries: &BTreeSet<LibraryId>,
) -> anyhow::Result<(AnalysisState, Vec<LibraryId>)> {
    let dep_state = convert_state(state);
    let mut internal: BTreeSet<LibraryId> = BTreeSet::new();
    for dep in dependencies {
        match dep {
            Dependency::Internal { path, .. } => {
                let target = LibraryId::parse(&path).with_context(|| {
                    format!("{context}: internal dependency path {path:?} is invalid")
                })?;
                if !same_language_libraries.contains(&target) {
                    bail!(
                        "{context}: internal dependency `{target}` is not in the same language manifest"
                    );
                }
                internal.insert(target);
            }
            // External/Unresolved edges do not become direct internal edges
            // and are dropped from the normalized snapshot for now. Future
            // plans will preserve them for site rendering.
            Dependency::External { .. } | Dependency::Unresolved { .. } => {}
        }
    }
    Ok((dep_state, internal.into_iter().collect()))
}

fn convert_state(s: ProtoState) -> AnalysisState {
    match s {
        ProtoState::Complete => AnalysisState::Complete,
        ProtoState::Partial => AnalysisState::Partial,
        ProtoState::Failed => AnalysisState::Failed,
    }
}

fn convert_symbol(s: ProtoSymbol) -> NormalizedSymbol {
    let ProtoSymbol {
        name,
        kind,
        qualified_name,
        search_names,
        signature,
        location,
    } = s;
    NormalizedSymbol {
        name,
        kind,
        qualified_name,
        search_names,
        signature,
        location: location.map(convert_location),
    }
}

fn convert_diagnostic(d: ProtoDiagnostic) -> NormalizedDiagnostic {
    let ProtoDiagnostic {
        severity,
        code,
        message,
        location,
    } = d;
    NormalizedDiagnostic {
        severity: match severity {
            ProtoSeverity::Info => DiagnosticSeverity::Info,
            ProtoSeverity::Warning => DiagnosticSeverity::Warning,
            ProtoSeverity::Error => DiagnosticSeverity::Error,
        },
        code,
        message,
        location: location.map(convert_location),
    }
}

fn convert_location(l: ProtoLocation) -> SourceLocation {
    let ProtoLocation { path, start, end } = l;
    let ProtoPosition {
        line: start_line,
        column: start_column,
    } = start;
    let (end_line, end_column) = match end {
        Some(ProtoPosition { line, column }) => (Some(line), column),
        None => (None, None),
    };
    SourceLocation {
        path,
        start_line,
        start_column,
        end_line,
        end_column,
    }
}

// ─── Hashing (canonical JSON + SHA-256) ─────────────────────────────────────

fn build_source_hashes(source_bytes: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, String> {
    source_bytes
        .iter()
        .map(|(path, bytes)| (path.clone(), sha256_hex(bytes)))
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn compute_discovery_hash(manifest: &DiscoveryManifest) -> String {
    let value = serde_json::json!({
        "languages": manifest
            .languages
            .iter()
            .map(|(id, lang)| serde_json::json!({
                "id": id.as_str(),
                "root": lang.root,
                "display_name": lang.display_name,
                "description_path": lang.description_path,
            }))
            .collect::<Vec<_>>(),
        "libraries": manifest
            .libraries
            .iter()
            .map(|l| serde_json::json!({
                "id": l.id.as_str(),
                "language": l.language.as_str(),
                "source_path": l.source_path,
                "description_path": l.description_path,
                "published": l.published,
                "managed": l.managed,
                "title": l.title,
            }))
            .collect::<Vec<_>>(),
        "solutions": manifest
            .solutions
            .iter()
            .map(|s| serde_json::json!({
                "id": s.id.as_str(),
                "language": s.language.as_str(),
                "entry": s.entry,
                "root": s.root,
                "solved_at": s.solved_at.to_rfc3339(),
                "verify": s.verify.as_ref().map(|v| serde_json::json!({
                    "libraries": v.libraries.iter().map(|l| l.as_str()).collect::<Vec<_>>(),
                    "oj_language_id": v.oj_language_id,
                })),
            }))
            .collect::<Vec<_>>(),
    });
    sha256_hex(&canonical_json(&value))
}

fn compute_snapshot_hash(
    schema_version: u32,
    revision: &str,
    discovery_hash: &str,
    source_hashes: &BTreeMap<String, String>,
    languages: &BTreeMap<LanguageId, NormalizedLanguageAnalysis>,
) -> String {
    let hashable = serde_json::json!({
        "schema_version": schema_version,
        "repository_revision": revision,
        "discovery_hash": discovery_hash,
        "source_hashes": source_hashes,
        "languages": languages
            .iter()
            .map(|(id, lang)| (id.as_str().to_string(), language_hashable_projection(lang)))
            .collect::<serde_json::Map<String, serde_json::Value>>(),
    });
    sha256_hex(&canonical_json(&hashable))
}

/// Returns the projection of a language analysis that participates in
/// `snapshot_hash`. Adapter and toolchain identity are deliberately excluded
/// so a mere identity bump does not invalidate downstream verify state.
fn language_hashable_projection(lang: &NormalizedLanguageAnalysis) -> serde_json::Value {
    serde_json::json!({
        "libraries": lang
            .libraries
            .iter()
            .map(|(id, analysis)| serde_json::json!({
                "id": id.as_str(),
                "dependency_state": state_label(analysis.state.dependency_state),
                "symbol_state": state_label(analysis.state.symbol_state),
                "direct_dependencies": analysis
                    .direct_dependencies
                    .iter()
                    .map(|d| d.as_str())
                    .collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
        "solutions": lang
            .solutions
            .iter()
            .map(|(id, analysis)| serde_json::json!({
                "id": id.as_str(),
                "dependency_state": state_label(analysis.dependency_state),
                "direct_dependencies": analysis
                    .direct_dependencies
                    .iter()
                    .map(|d| d.as_str())
                    .collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
    })
}

fn state_label(s: AnalysisState) -> &'static str {
    match s {
        AnalysisState::Complete => "complete",
        AnalysisState::Partial => "partial",
        AnalysisState::Failed => "failed",
    }
}

/// Serializes a `serde_json::Value` deterministically:
/// - object keys sorted (default `serde_json::Map` without `preserve_order`
///   feature is a `BTreeMap`, so `to_writer` already iterates sorted)
/// - no whitespace between tokens (default `to_writer` is compact)
///
/// We double-check by reparsing and re-serialising through the same path so
/// the output does not depend on how the input was constructed.
fn canonical_json(value: &serde_json::Value) -> Vec<u8> {
    // Round-trip through the compact serializer so nested structures always
    // sort by BTreeMap key order regardless of how the caller assembled them.
    let text = serde_json::to_string(value).expect("Value serializes as JSON");
    let reparsed: serde_json::Value = serde_json::from_str(&text).expect("just serialized JSON");
    let mut out = Vec::with_capacity(text.len());
    serde_json::to_writer(&mut out, &reparsed).expect("compact serializer never fails");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_is_key_sorted_compact() {
        let value = serde_json::json!({
            "b": 1,
            "a": [{ "y": 2, "x": 1 }],
        });
        let bytes = canonical_json(&value);
        let text = std::str::from_utf8(&bytes).unwrap();
        assert_eq!(text, r#"{"a":[{"x":1,"y":2}],"b":1}"#);
    }

    #[test]
    fn sha256_hex_is_deterministic() {
        assert_eq!(sha256_hex(b"abc"), sha256_hex(b"abc"));
    }
}
