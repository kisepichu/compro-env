//! Filesystem enumeration of managed library sources (spec §6.1).
//!
//! Enumerates every non-symlink regular file inside each language root,
//! applies the include/exclude glob rules, parses any accompanying sidecar
//! metadata, and returns an ordered `DiscoveryManifest`.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use domain::analysis::{
    DiscoveredLanguage, DiscoveryDiagnostic, DiscoveryManifest, DiscoverySeverity, LibraryFile,
};
use domain::library::{LanguageConfig, LanguageId, LibraryId, LibraryProjectConfig, SolutionId};
use domain::solution::{PublishedSolution, VerifySpec};
use globset::{Glob, GlobSetBuilder};

use crate::library_project::metadata::parse_library_sidecar;
use crate::library_project::solution_metadata::{
    SolutionCeToml, parse_contest_ce_toml, parse_solution_ce_toml, resolve_oj_language_id,
};

pub struct LibraryDiscovery;

impl LibraryDiscovery {
    /// Enumerates managed library files under every configured language root
    /// and every publishable solution under `solutions/`.
    ///
    /// This step never spawns adapter processes and never mutates the
    /// repository.
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

        let published_library_ids: std::collections::BTreeSet<LibraryId> = libraries
            .iter()
            .filter(|l| l.published)
            .map(|l| l.id.clone())
            .collect();

        let mut solutions = discover_solutions(repository_root, config, &published_library_ids)?;
        solutions.sort_by(|a, b| a.id.cmp(&b.id));

        Ok(DiscoveryManifest {
            languages,
            libraries,
            solutions,
            diagnostics,
        })
    }
}

// ─── Solution discovery ─────────────────────────────────────────────────────

fn discover_solutions(
    repository_root: &Path,
    config: &LibraryProjectConfig,
    published_library_ids: &std::collections::BTreeSet<LibraryId>,
) -> anyhow::Result<Vec<PublishedSolution>> {
    let solutions_root = repository_root.join("solutions");
    if !solutions_root.exists() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for contest_entry in read_children_sorted(&solutions_root)? {
        if !contest_entry.file_type()?.is_dir() {
            continue;
        }
        let contest_dir = contest_entry.path();
        // Parse contest .ce.toml if present. We don't require any keys but do
        // want a parse failure to surface immediately.
        let contest_meta_path = contest_dir.join(".ce.toml");
        if contest_meta_path.exists() {
            parse_contest_ce_toml(&contest_meta_path)?;
        }

        for problem_entry in read_children_sorted(&contest_dir)? {
            if !problem_entry.file_type()?.is_dir() {
                continue;
            }
            let problem_dir = problem_entry.path();
            for solution_entry in read_children_sorted(&problem_dir)? {
                if !solution_entry.file_type()?.is_dir() {
                    continue;
                }
                let solution_dir = solution_entry.path();
                let ce_toml_path = solution_dir.join("ce.toml");
                if !ce_toml_path.exists() {
                    continue;
                }
                let ce_toml = parse_solution_ce_toml(&ce_toml_path)?;
                if !ce_toml.publish {
                    // Private solutions never appear in the manifest passed
                    // to adapters (spec §6.2).
                    continue;
                }
                let published = build_published_solution(
                    &solution_dir,
                    &contest_dir,
                    &problem_dir,
                    &ce_toml,
                    config,
                    published_library_ids,
                    repository_root,
                )?;
                out.push(published);
            }
        }
    }
    Ok(out)
}

fn build_published_solution(
    solution_dir: &Path,
    contest_dir: &Path,
    problem_dir: &Path,
    ce_toml: &SolutionCeToml,
    config: &LibraryProjectConfig,
    published_library_ids: &std::collections::BTreeSet<LibraryId>,
    repository_root: &Path,
) -> anyhow::Result<PublishedSolution> {
    let contest_id = dir_name(contest_dir)?;
    let problem_code = dir_name(problem_dir)?;
    let solution_name = dir_name(solution_dir)?;
    let composite = format!("{contest_id}/{problem_code}/{solution_name}");
    let solution_id = SolutionId::parse(&composite)
        .with_context(|| format!("solution directory {composite:?} is not a valid solution id"))?;

    let language = LanguageId::parse(&ce_toml.language).with_context(|| {
        format!(
            "solution {} references invalid language id {:?}",
            solution_id, ce_toml.language
        )
    })?;
    let language_cfg: &LanguageConfig = config.languages.get(&language).ok_or_else(|| {
        anyhow!(
            "solution {} references language `{}` that is not declared under `[library.languages]`",
            solution_id,
            language
        )
    })?;

    let solved_at = ce_toml.solved_at.ok_or_else(|| {
        anyhow!(
            "solution {} is `publish = true` but has no `solved_at` timestamp",
            solution_id
        )
    })?;

    let verify = match &ce_toml.verify {
        None => None,
        Some(v) => {
            let mut libraries = Vec::with_capacity(v.libraries.len());
            for lib_str in &v.libraries {
                let library_id = LibraryId::parse(lib_str).with_context(|| {
                    format!(
                        "solution {} has invalid `[verify].libraries` entry {:?}",
                        solution_id, lib_str
                    )
                })?;
                if !published_library_ids.contains(&library_id) {
                    return Err(anyhow!(
                        "solution {} verifies library `{}` which is not a public discovered library",
                        solution_id,
                        library_id
                    ));
                }
                libraries.push(library_id);
            }
            let contest_id_str = solution_id.contest_id().to_string();
            let oj_key = detect_oj_key(&contest_id_str);
            let oj_language_id =
                resolve_oj_language_id(v, &oj_key, language_cfg).with_context(|| {
                    format!("solution {} could not resolve OJ language ID", solution_id)
                })?;

            // Also perform a structural check for orphan verify result paths:
            // if a stale result JSON exists for a solution that no longer has
            // a verify spec, it must be removed in the same change. Here we
            // only prepare the result path so higher layers can use it (plan
            // §5 Step 4).
            let _result_path = verify_result_path(repository_root, &solution_id);
            Some(VerifySpec {
                libraries,
                oj_language_id,
            })
        }
    };

    let entry = ce_toml
        .verify
        .as_ref()
        .map(|_| language_cfg.entry_file.clone())
        .unwrap_or_else(|| language_cfg.entry_file.clone());

    let root = repo_relative_dir(repository_root, solution_dir)?;

    Ok(PublishedSolution {
        id: solution_id,
        language,
        root,
        entry,
        solved_at,
        test_command: ce_toml.test_command.clone(),
        test_timeout_seconds: ce_toml.test_timeout_seconds,
        verify,
    })
}

fn detect_oj_key(contest_id: &str) -> String {
    // Solutions namespaces used across `ce` follow the existing OJKind::detect
    // convention: `librarychecker-*` contests target Library Checker, and
    // everything else defaults to atcoder. We keep this local so the
    // library-platform pipeline does not import the existing `OJKind`.
    if contest_id.starts_with("librarychecker-") {
        "librarychecker".to_string()
    } else {
        "atcoder".to_string()
    }
}

fn verify_result_path(repository_root: &Path, solution_id: &SolutionId) -> PathBuf {
    let mut path = repository_root.join("verification").join("results");
    path.push(solution_id.contest_id());
    path.push(solution_id.problem_code());
    path.push(format!("{}.json", solution_id.solution_name()));
    path
}

fn read_children_sorted(dir: &Path) -> anyhow::Result<Vec<std::fs::DirEntry>> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read directory {}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    Ok(entries)
}

fn dir_name(path: &Path) -> anyhow::Result<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("directory has no unicode-safe name: {}", path.display()))
}

fn repo_relative_dir(repo_root: &Path, dir: &Path) -> anyhow::Result<String> {
    let rel = dir
        .strip_prefix(repo_root)
        .with_context(|| format!("{} is outside repository root", dir.display()))?;
    to_forward_slash(rel)
        .ok_or_else(|| anyhow!("{} contains non-unicode path components", dir.display()))
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
