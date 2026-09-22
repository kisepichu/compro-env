//! Collects the projection inputs `ce site-data generate` cannot derive from
//! the analysis snapshot (spec §5.1, §6.5, §12).
//!
//! Discovery keeps only what language adapters need, so the sidecar body,
//! the declared relations, and the dependency overrides are dropped once a
//! `LibraryFile` is built. This module re-reads each sidecar named by
//! `LibraryFile::description_path` and pairs it with the two repository-level
//! facts the projection also needs: whether submissions run a preprocess hook,
//! and which online judge each contest belongs to.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, anyhow, bail};
use domain::analysis::{DiscoveryManifest, LibraryFile};
use domain::entity::OJKind;
use domain::library::{LibraryId, SolutionId};
use usecases::site_data::ProjectedRelation;

use crate::library_project::metadata::{DependencyOverride, Relation, parse_library_sidecar};
use crate::library_project::solution_metadata::parse_contest_ce_toml;

/// The caller-supplied half of [`usecases::site_data::PublicProjectionInput`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SiteDataInputs {
    /// Contest ID → online-judge name shown on solutions with no completed
    /// verification record. Contests we cannot attribute stay absent so the
    /// projection can fall back to `"unknown"`.
    pub oj_by_contest: BTreeMap<String, String>,
    pub relations: BTreeMap<LibraryId, Vec<ProjectedRelation>>,
    pub manual_dependency_edges: BTreeMap<LibraryId, BTreeSet<LibraryId>>,
    pub solution_has_preprocess: BTreeMap<SolutionId, bool>,
    /// Sidecar Markdown body (frontmatter stripped, trimmed). Libraries with
    /// no sidecar, or with a body-less sidecar, carry no entry.
    pub library_descriptions: BTreeMap<LibraryId, String>,
}

impl SiteDataInputs {
    /// Reads every sidecar referenced by `manifest` plus the per-contest
    /// `.ce.toml` files. `submit_preprocess` is the resolved
    /// `[submit].preprocess` hook: it is repository-wide, so it applies
    /// uniformly to every published solution.
    pub fn collect(
        repository_root: &Path,
        manifest: &DiscoveryManifest,
        submit_preprocess: Option<&str>,
    ) -> anyhow::Result<Self> {
        let mut inputs = Self {
            solution_has_preprocess: manifest
                .solutions
                .iter()
                .map(|solution| (solution.id.clone(), submit_preprocess.is_some()))
                .collect(),
            oj_by_contest: collect_oj_by_contest(repository_root, manifest)?,
            ..Self::default()
        };
        collect_sidecars(repository_root, manifest, &mut inputs)?;
        Ok(inputs)
    }
}

fn collect_sidecars(
    repository_root: &Path,
    manifest: &DiscoveryManifest,
    inputs: &mut SiteDataInputs,
) -> anyhow::Result<()> {
    let by_id: BTreeMap<&LibraryId, &LibraryFile> = manifest
        .libraries
        .iter()
        .map(|library| (&library.id, library))
        .collect();

    for library in &manifest.libraries {
        let Some(sidecar_path) = &library.description_path else {
            continue;
        };
        let absolute = repository_root.join(sidecar_path);
        let sidecar = parse_library_sidecar(&absolute)
            .with_context(|| format!("failed to parse sidecar {sidecar_path}"))?;

        let body = sidecar.body.trim();
        if !body.is_empty() {
            inputs
                .library_descriptions
                .insert(library.id.clone(), body.to_string());
        }

        let relations = collect_relations(sidecar.relations, library, sidecar_path, &by_id)?;
        if !relations.is_empty() {
            inputs.relations.insert(library.id.clone(), relations);
        }

        let edges =
            collect_manual_edges(sidecar.dependency_overrides, library, sidecar_path, &by_id)?;
        if !edges.is_empty() {
            inputs
                .manual_dependency_edges
                .insert(library.id.clone(), edges);
        }
    }
    Ok(())
}

fn collect_relations(
    declared: Vec<Relation>,
    library: &LibraryFile,
    sidecar_path: &str,
    by_id: &BTreeMap<&LibraryId, &LibraryFile>,
) -> anyhow::Result<Vec<ProjectedRelation>> {
    let mut out: Vec<ProjectedRelation> = Vec::with_capacity(declared.len());
    for relation in declared {
        let target = LibraryId::parse(&relation.to).with_context(|| {
            format!(
                "{sidecar_path}: relation `{}` target {:?} is not a valid library id",
                relation.kind, relation.to
            )
        })?;
        if target == library.id {
            bail!(
                "{sidecar_path}: relation `{}` points at itself; self relations are rejected (spec §5.1)",
                relation.kind
            );
        }
        if !by_id.contains_key(&target) {
            bail!(
                "{sidecar_path}: relation `{}` points at `{target}`, which is not a managed library (spec §5.1)",
                relation.kind
            );
        }
        if out
            .iter()
            .any(|seen| seen.kind == relation.kind && seen.target == target)
        {
            bail!(
                "{sidecar_path}: duplicate relation `{}` -> `{target}` (spec §5.1)",
                relation.kind
            );
        }
        out.push(ProjectedRelation {
            kind: relation.kind,
            target,
            // `manual` marks a dependency edge a contributor added by hand on
            // top of adapter output (spec §6.5). Relations only ever come from
            // frontmatter, so the badge would carry no information here.
            manual: false,
        });
    }
    Ok(out)
}

fn collect_manual_edges(
    overrides: Vec<DependencyOverride>,
    library: &LibraryFile,
    sidecar_path: &str,
    by_id: &BTreeMap<&LibraryId, &LibraryFile>,
) -> anyhow::Result<BTreeSet<LibraryId>> {
    let mut out = BTreeSet::new();
    for entry in overrides {
        let to = match &entry {
            DependencyOverride::Add { to, .. } => to,
            DependencyOverride::Remove { .. } => return Err(unsupported(sidecar_path, "remove")),
            DependencyOverride::Resolve { .. } => return Err(unsupported(sidecar_path, "resolve")),
            DependencyOverride::External { .. } => {
                return Err(unsupported(sidecar_path, "external"));
            }
        };
        let target = LibraryId::parse(to).with_context(|| {
            format!("{sidecar_path}: dependency override target {to:?} is not a valid library id")
        })?;
        if target == library.id {
            bail!("{sidecar_path}: dependency override adds `{target}` to itself");
        }
        let target_file = by_id.get(&target).ok_or_else(|| {
            anyhow!(
                "{sidecar_path}: dependency override adds `{target}`, which is not a managed library (spec §6.5)"
            )
        })?;
        if target_file.language != library.language {
            bail!(
                "{sidecar_path}: dependency override adds `{target}` from language `{}`, but `add` must stay inside the same language `{}` (spec §6.5)",
                target_file.language,
                library.language
            );
        }
        if !out.insert(target.clone()) {
            bail!("{sidecar_path}: duplicate dependency override adding `{target}` (spec §6.5)");
        }
    }
    Ok(out)
}

/// `remove` / `resolve` / `external` need adapter-reported edges and
/// unresolved dependency keys, neither of which the projection models today.
/// Refuse loudly instead of publishing a dependency graph that silently
/// ignores the override.
fn unsupported(sidecar_path: &str, action: &str) -> anyhow::Error {
    anyhow!(
        "{sidecar_path}: dependency override `action = \"{action}\"` is not supported by site-data generation yet; only `add` is applied"
    )
}

fn collect_oj_by_contest(
    repository_root: &Path,
    manifest: &DiscoveryManifest,
) -> anyhow::Result<BTreeMap<String, String>> {
    let contests: BTreeSet<&str> = manifest
        .solutions
        .iter()
        .map(|solution| solution.id.contest_id())
        .collect();
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for contest_id in contests {
        // `.ce.toml` is authoritative — it is what `ce submit` resolves the OJ
        // from. It is not committed for every contest, so fall back to the
        // contest-ID convention and leave unrecognised contests unmapped.
        let path = repository_root
            .join("solutions")
            .join(contest_id)
            .join(".ce.toml");
        let declared = if path.exists() {
            parse_contest_ce_toml(&path)?.online_judge
        } else {
            None
        };
        let resolved = declared
            .or_else(|| OJKind::detect(contest_id).map(|(kind, _)| kind.as_str().to_string()));
        if let Some(oj) = resolved {
            out.insert(contest_id.to_string(), oj);
        }
    }
    Ok(out)
}
