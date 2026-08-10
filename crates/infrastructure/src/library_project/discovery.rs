//! Filesystem enumeration of managed library sources (spec §6.1).
//!
//! Enumerates every non-symlink regular file inside each language root,
//! applies the include/exclude glob rules, parses any accompanying sidecar
//! metadata, and returns an ordered `DiscoveryManifest`.

use std::path::Path;

use anyhow::{Context, anyhow};
use domain::analysis::{
    DiscoveredLanguage, DiscoveryDiagnostic, DiscoveryManifest, DiscoverySeverity, LibraryFile,
};
use domain::library::{LibraryId, LibraryProjectConfig};
use globset::{Glob, GlobSetBuilder};

use crate::library_project::metadata::parse_library_sidecar;

pub struct LibraryDiscovery;

impl LibraryDiscovery {
    /// Enumerates managed library files under every configured language root.
    ///
    /// This step never spawns adapter processes and never mutates the
    /// repository. Solution discovery is layered on top in a later task; the
    /// `solutions` field of the returned manifest is empty here.
    pub fn discover(
        repository_root: &Path,
        config: &LibraryProjectConfig,
    ) -> anyhow::Result<DiscoveryManifest> {
        let mut diagnostics = Vec::new();
        let mut languages = std::collections::BTreeMap::new();
        let mut libraries: Vec<LibraryFile> = Vec::new();

        for (language_id, language) in &config.languages {
            let root = repository_root.join(&language.root);
            let root_metadata = match std::fs::symlink_metadata(&root) {
                Ok(m) => m,
                Err(err) => {
                    return Err(anyhow!(
                        "language `{}` root not found at {}: {}",
                        language_id,
                        root.display(),
                        err
                    ));
                }
            };
            if root_metadata.file_type().is_symlink() {
                return Err(anyhow!(
                    "language `{}` root {} is a symlink; symlinks are rejected",
                    language_id,
                    root.display()
                ));
            }
            if !root_metadata.is_dir() {
                return Err(anyhow!(
                    "language `{}` root {} is not a directory",
                    language_id,
                    root.display()
                ));
            }

            let include_set = build_globset(&language.include)
                .with_context(|| format!("language `{language_id}`: invalid glob in `include`"))?;
            let exclude_set = build_globset(&language.exclude)
                .with_context(|| format!("language `{language_id}`: invalid glob in `exclude`"))?;

            let discovered_language = DiscoveredLanguage {
                id: language_id.clone(),
                root: language.root.clone(),
                display_name: language.effective_display_name().to_string(),
                description_path: index_md_path(&language.root, &root),
            };
            languages.insert(language_id.clone(), discovered_language);

            let mut language_hits = 0usize;
            for entry in walkdir::WalkDir::new(&root)
                .follow_links(false)
                .sort_by(|a, b| a.file_name().cmp(b.file_name()))
                .into_iter()
            {
                let entry = entry
                    .with_context(|| format!("failed to walk language root {}", root.display()))?;
                let file_type = entry.file_type();
                if file_type.is_symlink() {
                    // Silently skip symlink candidates; they never enter the
                    // manifest per spec §6.1.
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }

                let abs_path = entry.path().to_path_buf();
                let rel_to_root = match abs_path.strip_prefix(&root) {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                let rel_to_root_str = match to_forward_slash(rel_to_root) {
                    Some(s) => s,
                    None => continue,
                };

                if !include_set.is_match(&rel_to_root_str) {
                    continue;
                }
                if exclude_set.is_match(&rel_to_root_str) {
                    continue;
                }
                // Filter out sidecar `.md` files: they describe a sibling
                // source, they are not library sources themselves.
                if rel_to_root_str.ends_with(".md") {
                    continue;
                }

                let repo_relative = join_forward_slash(&language.root, &rel_to_root_str);
                let library_id = LibraryId::parse(&repo_relative).with_context(|| {
                    format!("discovered path {repo_relative} is not a valid library id")
                })?;

                let sidecar_path = abs_path.with_extension({
                    let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if ext.is_empty() {
                        "md".to_string()
                    } else {
                        format!("{ext}.md")
                    }
                });
                let sidecar = parse_library_sidecar(&sidecar_path).with_context(|| {
                    format!("failed to parse sidecar {}", sidecar_path.display())
                })?;

                let description_path = if sidecar_path.exists() {
                    Some(rel_from_repo(repository_root, &sidecar_path))
                } else {
                    None
                };
                let source_path = rel_from_repo(repository_root, &abs_path);

                libraries.push(LibraryFile {
                    id: library_id,
                    language: language_id.clone(),
                    source_path,
                    description_path,
                    published: sidecar.publish,
                    managed: true,
                    title: sidecar.title,
                });
                language_hits += 1;
            }

            if language_hits == 0 {
                diagnostics.push(DiscoveryDiagnostic {
                    severity: DiscoverySeverity::Warning,
                    code: "empty_language".into(),
                    message: format!("language `{language_id}` has no files matching include"),
                    language: Some(language_id.clone()),
                    path: None,
                });
            }
        }

        libraries.sort_by(|a, b| a.id.cmp(&b.id));

        // Detect orphan sidecars: `.md` files whose source is missing.
        detect_orphan_sidecars(repository_root, config, &libraries, &mut diagnostics)?;

        Ok(DiscoveryManifest {
            languages,
            libraries,
            solutions: vec![],
            diagnostics,
        })
    }
}

fn build_globset(patterns: &[String]) -> anyhow::Result<globset::GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = Glob::new(pat).with_context(|| format!("invalid glob pattern {pat:?}"))?;
        builder.add(glob);
    }
    builder.build().context("failed to compile glob set")
}

fn to_forward_slash(path: &Path) -> Option<String> {
    let mut out = String::new();
    for (i, component) in path.components().enumerate() {
        let piece = match component {
            std::path::Component::Normal(s) => s.to_str()?,
            _ => return None,
        };
        if i > 0 {
            out.push('/');
        }
        out.push_str(piece);
    }
    Some(out)
}

fn join_forward_slash(base: &str, rest: &str) -> String {
    if base.is_empty() {
        return rest.to_string();
    }
    let trimmed = base.trim_end_matches('/');
    format!("{trimmed}/{rest}")
}

fn rel_from_repo(repo_root: &Path, absolute: &Path) -> String {
    match absolute.strip_prefix(repo_root) {
        Ok(rel) => to_forward_slash(rel).unwrap_or_else(|| absolute.display().to_string()),
        Err(_) => absolute.display().to_string(),
    }
}

fn index_md_path(language_root: &str, absolute_root: &Path) -> Option<String> {
    let candidate = absolute_root.join("_index.md");
    if candidate.exists() {
        Some(join_forward_slash(language_root, "_index.md"))
    } else {
        None
    }
}

fn detect_orphan_sidecars(
    repository_root: &Path,
    config: &LibraryProjectConfig,
    libraries: &[LibraryFile],
    diagnostics: &mut Vec<DiscoveryDiagnostic>,
) -> anyhow::Result<()> {
    let known: std::collections::BTreeSet<&str> = libraries
        .iter()
        .filter_map(|l| l.description_path.as_deref())
        .collect();
    for (language_id, language) in &config.languages {
        let root = repository_root.join(&language.root);
        for entry in walkdir::WalkDir::new(&root).follow_links(false).into_iter() {
            let entry = entry
                .with_context(|| format!("failed to walk language root {}", root.display()))?;
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            let abs = entry.path();
            let file_name = match abs.file_name().and_then(|f| f.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if !file_name.ends_with(".md") {
                continue;
            }
            if file_name == "_index.md" {
                continue;
            }
            let rel = rel_from_repo(repository_root, abs);
            if known.contains(rel.as_str()) {
                continue;
            }
            // A sidecar must have a sibling source (name without the final .md).
            let base = file_name.trim_end_matches(".md");
            let sibling = abs.with_file_name(base);
            if !sibling.exists() {
                diagnostics.push(DiscoveryDiagnostic {
                    severity: DiscoverySeverity::Error,
                    code: "orphan_sidecar".into(),
                    message: format!("sidecar {} has no corresponding source file", rel),
                    language: Some(language_id.clone()),
                    path: Some(rel),
                });
            }
        }
    }
    Ok(())
}
