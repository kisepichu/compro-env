//! Rust module-graph model used by the direct-dependency resolver
//! (plan 043 Task 2).
//!
//! The adapter has no cargo-metadata dependency; it reconstructs the graph
//! from the paths handed in via `AnalysisRequest` plus the on-disk files it
//! reaches through `mod` declarations. Every path stored on a `ModuleTree`
//! or `RustWorkspace` is expressed in repository-relative POSIX form because
//! the protocol contract (§6.3) requires that shape in emitted `Location`
//! and `Internal.path` fields.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use library_adapter_protocol::{AnalysisRequest, LibraryTarget, SolutionTarget};
use thiserror::Error;

/// Convenience alias for repo-relative POSIX-style paths.
pub type RepoPath = String;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("target path {path:?} escapes the repository root")]
    EscapedRoot { path: String },
    #[error("target path {path:?} is not UTF-8 or contains invalid characters")]
    BadPath { path: String },
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: syn::Error,
    },
}

/// One resolved crate root: the file that gets parsed first when a target is
/// analyzed. Libraries and solutions both fit this model — libraries are
/// single-file synthetic crates rooted at their listed file; solutions are
/// crates whose root is `<target.root>/<target.entry>`.
#[derive(Debug, Clone)]
pub struct RustCrate {
    /// Target identity. For libraries this equals `LibraryTarget.path`; for
    /// solutions it equals `SolutionTarget.id`.
    pub target_id: String,
    /// Repository-relative path to the crate root file.
    pub root_file: RepoPath,
    /// Absolute path to the crate root file, used only for filesystem access.
    pub root_file_abs: PathBuf,
    /// Repository-relative package name inferred from the crate root. Used
    /// when classifying `use crate_name::...` paths — if `crate_name` matches
    /// this, the path is treated as if it started with `crate::`.
    pub package_name: String,
    /// Kind of the target.
    pub kind: CrateKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrateKind {
    Library,
    Solution,
}

/// The workspace context shared across every target analyzed for a single
/// `AnalysisRequest`. It knows which repository-relative files this pipeline
/// considers "managed" — internal edges may point only at those files.
#[derive(Debug, Clone)]
pub struct RustWorkspace {
    pub repository_root: PathBuf,
    /// All repository-relative `.rs` paths this analysis owns. The union of:
    /// * every `LibraryTarget.path`,
    /// * every `SolutionTarget.root/entry`,
    /// * every `.rs` file reachable via a solution's mod chain.
    ///
    /// Any `mod`/`#[path]` reference outside this set is emitted as External
    /// (unmanaged path) or Unresolved (path does not exist).
    pub managed_files: BTreeSet<RepoPath>,
    /// Subset of `managed_files` that comes from `LibraryTarget`s. Emitted
    /// dependencies only surface as `Internal` when they land in a library
    /// target file — a solution's own submodules stay silent because they
    /// are not "dependencies" in the site-render sense.
    pub library_target_files: BTreeSet<RepoPath>,
    /// Crate roots keyed by their `target_id`.
    pub crates: BTreeMap<String, RustCrate>,
}

impl RustWorkspace {
    /// Build the workspace from an `AnalysisRequest`. Fails if any listed
    /// path escapes the repository root or is not UTF-8.
    ///
    /// The workspace's `managed_files` set is the union of:
    ///
    /// * every `LibraryTarget.path`, and
    /// * every `.rs` file reachable from each solution's entry via the
    ///   `mod`/`#[path]` chain. Solution submodules are considered part of
    ///   the solution's own crate; internal edges landing there stay
    ///   Internal instead of falling back to Unresolved.
    pub fn from_request(request: &AnalysisRequest) -> Result<Self, WorkspaceError> {
        let repository_root = PathBuf::from(&request.repository_root);
        let mut managed_files: BTreeSet<RepoPath> = BTreeSet::new();
        let mut library_target_files: BTreeSet<RepoPath> = BTreeSet::new();
        let mut crates: BTreeMap<String, RustCrate> = BTreeMap::new();

        for target in &request.libraries {
            let cr = crate_from_library(&repository_root, target)?;
            managed_files.insert(cr.root_file.clone());
            library_target_files.insert(cr.root_file.clone());
            crates.insert(cr.target_id.clone(), cr);
        }
        for target in &request.solutions {
            let cr = crate_from_solution(&repository_root, target)?;
            managed_files.insert(cr.root_file.clone());
            crates.insert(cr.target_id.clone(), cr);
        }

        // Second pass: expand `managed_files` with every file reachable via
        // `mod` from a solution entry. Libraries are single-file by contract,
        // so their submodule chains are not folded in.
        let mut probe = Self {
            repository_root: repository_root.clone(),
            managed_files: managed_files.clone(),
            library_target_files: library_target_files.clone(),
            crates: crates.clone(),
        };
        for cr in crates.values().filter(|c| c.kind == CrateKind::Solution) {
            expand_managed_from(&mut probe, &cr.root_file);
        }
        managed_files = probe.managed_files;

        Ok(Self {
            repository_root,
            managed_files,
            library_target_files,
            crates,
        })
    }

    /// True iff `repo_relative` is a listed `LibraryTarget.path`.
    pub fn is_library_target(&self, repo_relative: &str) -> bool {
        self.library_target_files.contains(repo_relative)
    }

    /// True iff `repo_relative` is one of the paths this analysis manages.
    pub fn is_managed(&self, repo_relative: &str) -> bool {
        self.managed_files.contains(repo_relative)
    }

    /// Absolute path for a repository-relative POSIX path.
    pub fn absolute(&self, repo_relative: &str) -> PathBuf {
        let mut p = self.repository_root.clone();
        for segment in repo_relative.split('/') {
            p.push(segment);
        }
        p
    }
}

fn crate_from_library(repo: &Path, target: &LibraryTarget) -> Result<RustCrate, WorkspaceError> {
    let repo_path = validate_repo_relative(&target.path)?;
    let abs = join_repo(repo, &repo_path);
    let package_name = package_name_from_root(&repo_path);
    Ok(RustCrate {
        target_id: repo_path.clone(),
        root_file: repo_path,
        root_file_abs: abs,
        package_name,
        kind: CrateKind::Library,
    })
}

fn crate_from_solution(repo: &Path, target: &SolutionTarget) -> Result<RustCrate, WorkspaceError> {
    let root = validate_repo_relative(&target.root)?;
    let entry = validate_repo_relative_component(&target.entry)?;
    let repo_path = if root.is_empty() {
        entry.clone()
    } else {
        format!("{root}/{entry}")
    };
    let repo_path = validate_repo_relative(&repo_path)?;
    let abs = join_repo(repo, &repo_path);
    let package_name = package_name_from_root(&repo_path);
    Ok(RustCrate {
        target_id: target.id.clone(),
        root_file: repo_path,
        root_file_abs: abs,
        package_name,
        kind: CrateKind::Solution,
    })
}

fn validate_repo_relative(path: &str) -> Result<RepoPath, WorkspaceError> {
    if path.is_empty() {
        return Ok(String::new());
    }
    if path.starts_with('/') || path.contains("//") {
        return Err(WorkspaceError::BadPath {
            path: path.to_string(),
        });
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." || segment.contains('\\') {
            return Err(WorkspaceError::EscapedRoot {
                path: path.to_string(),
            });
        }
    }
    Ok(path.replace('\\', "/"))
}

fn validate_repo_relative_component(path: &str) -> Result<RepoPath, WorkspaceError> {
    let sanitized = path.replace('\\', "/");
    validate_repo_relative(&sanitized)
}

fn join_repo(repo: &Path, repo_relative: &str) -> PathBuf {
    let mut p = repo.to_path_buf();
    for segment in repo_relative.split('/') {
        p.push(segment);
    }
    p
}

/// Extract the crate root's synthetic package name for `use <name>::…`
/// classification. For a solution rooted at `solutions/abc/A/main/src/main.rs`
/// the package name is `main`; for a library rooted at `libraries/rust/a.rs`
/// it is `a`. If the root file is `mod.rs` the parent directory name is used.
fn package_name_from_root(repo_relative: &str) -> String {
    let (parent, file_stem) = split_parent_stem(repo_relative);
    if file_stem == "main" || file_stem == "lib" || file_stem == "mod" {
        // For `src/main.rs`, the useful package label is the directory that
        // contains `src/`. Fall back to the file stem itself if that walk
        // is not possible.
        if let Some(parent_of_src) = parent
            .rsplit_once('/')
            .and_then(|(before_src, src)| (src == "src").then_some(before_src))
        {
            let last = parent_of_src
                .rsplit_once('/')
                .map(|(_, tail)| tail)
                .unwrap_or(parent_of_src);
            if !last.is_empty() {
                return sanitize_identifier(last);
            }
        }
        // `mod.rs` inside `foo/` becomes `foo`.
        if !parent.is_empty() {
            let last = parent
                .rsplit_once('/')
                .map(|(_, tail)| tail)
                .unwrap_or(&parent);
            if !last.is_empty() {
                return sanitize_identifier(last);
            }
        }
    }
    sanitize_identifier(&file_stem)
}

fn split_parent_stem(repo_relative: &str) -> (String, String) {
    let (parent, name) = repo_relative
        .rsplit_once('/')
        .map(|(p, n)| (p.to_string(), n.to_string()))
        .unwrap_or_else(|| (String::new(), repo_relative.to_string()));
    let stem = name
        .strip_suffix(".rs")
        .map(|s| s.to_string())
        .unwrap_or(name);
    (parent, stem)
}

fn sanitize_identifier(raw: &str) -> String {
    // Cargo package names allow `-`, but Rust identifiers do not — Cargo
    // rewrites `-` to `_` when synthesizing the crate name. Do the same so
    // `use my_crate::x` matches a package named `my-crate` or `my_crate`.
    raw.replace('-', "_")
}

// ─── Module resolution ──────────────────────────────────────────────────────

/// One parsed source file with its repo-relative path context. Held by value
/// because `syn::File` owns everything and there is no borrowing benefit.
#[derive(Debug)]
pub struct ParsedFile {
    /// Repository-relative POSIX path of this file. Emitted verbatim into
    /// `Location.path` and `Internal.path`.
    pub repo_relative: RepoPath,
    /// Absolute path used for logging and follow-up reads.
    pub absolute: PathBuf,
    /// Parsed AST. Note that `syn` records byte spans only when compiled with
    /// `proc-macro2/span-locations`, which this crate enables.
    pub file: syn::File,
}

/// Walk the mod chain starting at `entry`, inserting every reachable
/// repo-relative `.rs` file into `workspace.managed_files`. Library targets
/// terminate the walk: they are their own single-file crates and do not
/// contribute their submodules to the calling solution's tree.
fn expand_managed_from(workspace: &mut RustWorkspace, entry: &str) {
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = vec![entry.to_string()];
    while let Some(current) = queue.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        workspace.managed_files.insert(current.clone());
        // Library targets are single-file crates. Stop the walk so a
        // solution that `#[path]`-links into a library does not vacuum its
        // whole file tree into the solution's managed set.
        if workspace.is_library_target(&current) && current != entry {
            continue;
        }
        let parsed = match load_file(workspace, &current) {
            Ok(p) => p,
            Err(_) => continue,
        };
        for item in &parsed.file.items {
            if let syn::Item::Mod(m) = item
                && m.content.is_none()
            {
                let name = m.ident.to_string();
                let explicit = read_path_attribute_for_walk(&m.attrs);
                let resolution = resolve_mod(workspace, &current, &name, explicit.as_deref());
                match resolution {
                    ModResolution::ManagedFile { repo_relative }
                    | ModResolution::UnmanagedFile { repo_relative } => {
                        queue.push(repo_relative);
                    }
                    ModResolution::Ambiguous { candidates } => {
                        for c in candidates {
                            queue.push(c);
                        }
                    }
                    ModResolution::Missing { .. } => {}
                }
            }
        }
    }
}

fn read_path_attribute_for_walk(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if !attr.path().is_ident("path") {
            continue;
        }
        if let syn::Meta::NameValue(nv) = &attr.meta
            && let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
        {
            return Some(s.value());
        }
    }
    None
}

/// Load and parse the given repo-relative `.rs` file.
pub fn load_file(
    workspace: &RustWorkspace,
    repo_relative: &str,
) -> Result<ParsedFile, WorkspaceError> {
    let abs = workspace.absolute(repo_relative);
    let source = std::fs::read_to_string(&abs).map_err(|source| WorkspaceError::Io {
        path: abs.display().to_string(),
        source,
    })?;
    let file = syn::parse_file(&source).map_err(|source| WorkspaceError::Parse {
        path: abs.display().to_string(),
        source,
    })?;
    Ok(ParsedFile {
        repo_relative: repo_relative.to_string(),
        absolute: abs,
        file,
    })
}

/// Resolution outcome for a `mod foo;` (or `#[path="…"] mod foo;`) statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModResolution {
    /// Resolved to an internal managed file — Internal edge fodder.
    ManagedFile { repo_relative: RepoPath },
    /// Resolved to a file that exists but is not managed by this analysis.
    /// Callers usually emit Unresolved edges for these because they may hide
    /// symbols the pipeline cannot reason about.
    UnmanagedFile { repo_relative: RepoPath },
    /// No candidate file exists on disk. Common when the module is fully
    /// generated by a macro or lives on a cfg-inactive path.
    Missing { candidates: Vec<RepoPath> },
    /// Ambiguous — both `<parent>/<name>.rs` and `<parent>/<name>/mod.rs`
    /// exist, which is a compile error in Rust 2018+ but useful to surface
    /// deterministically instead of picking one silently.
    Ambiguous { candidates: Vec<RepoPath> },
}

/// Resolve `mod name;` relative to `containing_file`. Applies Rust 2018+
/// filename rules: adjacent `<name>.rs` first, then `<parent_dir>/<name>/mod.rs`.
/// When the parent file is itself named `mod.rs` (or the crate root), lookups
/// start from that file's parent directory instead of `<parent_dir>/<file_stem>`.
pub fn resolve_mod(
    workspace: &RustWorkspace,
    containing_file: &str,
    module_name: &str,
    explicit_path: Option<&str>,
) -> ModResolution {
    let (parent_dir, file_stem) = split_parent_stem(containing_file);
    let module_base: String = if file_stem == "mod" || file_stem == "lib" || file_stem == "main" {
        parent_dir.clone()
    } else if parent_dir.is_empty() {
        file_stem.clone()
    } else {
        format!("{parent_dir}/{file_stem}")
    };

    let candidates: Vec<RepoPath> = if let Some(explicit) = explicit_path {
        // `#[path="foo/bar.rs"] mod x;` — path is relative to `containing_file`'s
        // directory. Rust silently rewrites backslashes on Windows, but we're
        // POSIX-only here; reject anything that escapes the repo.
        let joined = join_relative_posix(&parent_dir, explicit);
        match joined {
            Some(p) => vec![p],
            None => vec![],
        }
    } else {
        let sibling = if module_base.is_empty() {
            format!("{module_name}.rs")
        } else {
            format!("{module_base}/{module_name}.rs")
        };
        let mod_rs = if module_base.is_empty() {
            format!("{module_name}/mod.rs")
        } else {
            format!("{module_base}/{module_name}/mod.rs")
        };
        vec![sibling, mod_rs]
    };

    let present: Vec<&RepoPath> = candidates
        .iter()
        .filter(|c| workspace.absolute(c).exists())
        .collect();

    match present.len() {
        0 => ModResolution::Missing { candidates },
        1 => {
            let chosen = present[0].clone();
            if workspace.is_managed(&chosen) {
                ModResolution::ManagedFile {
                    repo_relative: chosen,
                }
            } else {
                ModResolution::UnmanagedFile {
                    repo_relative: chosen,
                }
            }
        }
        _ => ModResolution::Ambiguous {
            candidates: present.iter().map(|s| (*s).clone()).collect(),
        },
    }
}

/// Join `base` (a repo-relative POSIX path, may be empty) with `relative`
/// (which may contain `..` segments) into a normalized repo-relative path,
/// or `None` if the result escapes the repository root.
fn join_relative_posix(base: &str, relative: &str) -> Option<RepoPath> {
    let mut components: Vec<&str> = if base.is_empty() {
        Vec::new()
    } else {
        base.split('/').collect()
    };
    let normalized = relative.replace('\\', "/");
    for segment in normalized.split('/') {
        match segment {
            "" | "." => continue,
            ".." => {
                components.pop()?;
            }
            other => components.push(other),
        }
    }
    Some(components.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use library_adapter_protocol::{AnalysisRequest, SCHEMA_VERSION};

    fn tmp_repo(files: &[(&str, &str)]) -> tempfile::TempDir {
        let td = tempfile::tempdir().unwrap();
        for (rel, body) in files {
            let path = td.path().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, body).unwrap();
        }
        td
    }

    #[test]
    fn workspace_gathers_library_and_solution_paths() {
        let request = AnalysisRequest {
            schema_version: SCHEMA_VERSION,
            repository_root: "/tmp/x".into(),
            language: "rust".into(),
            libraries: vec![LibraryTarget {
                path: "libraries/rust/a.rs".into(),
            }],
            solutions: vec![SolutionTarget {
                id: "abc/A/main".into(),
                root: "solutions/abc/A/main".into(),
                entry: "src/main.rs".into(),
            }],
        };
        let ws = RustWorkspace::from_request(&request).unwrap();
        assert!(ws.is_managed("libraries/rust/a.rs"));
        assert!(ws.is_managed("solutions/abc/A/main/src/main.rs"));
        assert_eq!(ws.crates.get("abc/A/main").unwrap().package_name, "main");
        assert_eq!(
            ws.crates.get("libraries/rust/a.rs").unwrap().package_name,
            "a"
        );
    }

    #[test]
    fn resolve_mod_finds_sibling_rs() {
        let td = tmp_repo(&[
            ("libraries/rust/a.rs", "mod inner;"),
            ("libraries/rust/a/inner.rs", ""),
        ]);
        let request = AnalysisRequest {
            schema_version: SCHEMA_VERSION,
            repository_root: td.path().display().to_string(),
            language: "rust".into(),
            libraries: vec![
                LibraryTarget {
                    path: "libraries/rust/a.rs".into(),
                },
                LibraryTarget {
                    path: "libraries/rust/a/inner.rs".into(),
                },
            ],
            solutions: vec![],
        };
        let ws = RustWorkspace::from_request(&request).unwrap();
        match resolve_mod(&ws, "libraries/rust/a.rs", "inner", None) {
            ModResolution::ManagedFile { repo_relative } => {
                assert_eq!(repo_relative, "libraries/rust/a/inner.rs");
            }
            other => panic!("expected managed file, got {other:?}"),
        }
    }

    #[test]
    fn resolve_mod_finds_mod_rs() {
        let td = tmp_repo(&[
            ("libraries/rust/a.rs", "mod inner;"),
            ("libraries/rust/a/inner/mod.rs", ""),
        ]);
        let request = AnalysisRequest {
            schema_version: SCHEMA_VERSION,
            repository_root: td.path().display().to_string(),
            language: "rust".into(),
            libraries: vec![
                LibraryTarget {
                    path: "libraries/rust/a.rs".into(),
                },
                LibraryTarget {
                    path: "libraries/rust/a/inner/mod.rs".into(),
                },
            ],
            solutions: vec![],
        };
        let ws = RustWorkspace::from_request(&request).unwrap();
        match resolve_mod(&ws, "libraries/rust/a.rs", "inner", None) {
            ModResolution::ManagedFile { repo_relative } => {
                assert_eq!(repo_relative, "libraries/rust/a/inner/mod.rs");
            }
            other => panic!("expected managed file, got {other:?}"),
        }
    }

    #[test]
    fn resolve_mod_reports_ambiguous_when_both_exist() {
        let td = tmp_repo(&[
            ("libraries/rust/a.rs", "mod inner;"),
            ("libraries/rust/a/inner.rs", ""),
            ("libraries/rust/a/inner/mod.rs", ""),
        ]);
        let request = AnalysisRequest {
            schema_version: SCHEMA_VERSION,
            repository_root: td.path().display().to_string(),
            language: "rust".into(),
            libraries: vec![LibraryTarget {
                path: "libraries/rust/a.rs".into(),
            }],
            solutions: vec![],
        };
        let ws = RustWorkspace::from_request(&request).unwrap();
        match resolve_mod(&ws, "libraries/rust/a.rs", "inner", None) {
            ModResolution::Ambiguous { candidates } => {
                assert_eq!(candidates.len(), 2);
            }
            other => panic!("expected ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn resolve_mod_with_explicit_path_attribute() {
        let td = tmp_repo(&[
            (
                "libraries/rust/a.rs",
                "#[path=\"custom/where.rs\"] mod inner;",
            ),
            ("libraries/rust/custom/where.rs", ""),
        ]);
        let request = AnalysisRequest {
            schema_version: SCHEMA_VERSION,
            repository_root: td.path().display().to_string(),
            language: "rust".into(),
            libraries: vec![
                LibraryTarget {
                    path: "libraries/rust/a.rs".into(),
                },
                LibraryTarget {
                    path: "libraries/rust/custom/where.rs".into(),
                },
            ],
            solutions: vec![],
        };
        let ws = RustWorkspace::from_request(&request).unwrap();
        match resolve_mod(&ws, "libraries/rust/a.rs", "inner", Some("custom/where.rs")) {
            ModResolution::ManagedFile { repo_relative } => {
                assert_eq!(repo_relative, "libraries/rust/custom/where.rs");
            }
            other => panic!("expected managed file, got {other:?}"),
        }
    }

    #[test]
    fn resolve_mod_missing_returns_candidates() {
        let td = tmp_repo(&[("libraries/rust/a.rs", "mod inner;")]);
        let request = AnalysisRequest {
            schema_version: SCHEMA_VERSION,
            repository_root: td.path().display().to_string(),
            language: "rust".into(),
            libraries: vec![LibraryTarget {
                path: "libraries/rust/a.rs".into(),
            }],
            solutions: vec![],
        };
        let ws = RustWorkspace::from_request(&request).unwrap();
        match resolve_mod(&ws, "libraries/rust/a.rs", "inner", None) {
            ModResolution::Missing { candidates } => {
                assert_eq!(
                    candidates,
                    vec![
                        "libraries/rust/a/inner.rs".to_string(),
                        "libraries/rust/a/inner/mod.rs".to_string(),
                    ],
                );
            }
            other => panic!("expected missing, got {other:?}"),
        }
    }
}
