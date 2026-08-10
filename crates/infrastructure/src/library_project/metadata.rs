//! Strict Markdown-frontmatter parsers for library sidecars and directory
//! index files (spec §5, §5.1).
//!
//! Both file kinds use a `+++`-delimited TOML frontmatter followed by an
//! optional Markdown body. When the frontmatter is absent the file is still
//! valid (with default metadata) so contributors can add descriptions without
//! configuration.

use std::path::Path;

use anyhow::{Context, anyhow, bail};
use serde::Deserialize;

/// Metadata extracted from a library sidecar (`<source>.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryMetadata {
    pub title: Option<String>,
    /// Defaults to `true` when the file is absent or `publish` is unspecified.
    pub publish: bool,
    pub relations: Vec<Relation>,
    pub dependency_overrides: Vec<DependencyOverride>,
    /// Markdown body after the frontmatter fence (retained verbatim).
    pub body: String,
}

impl Default for LibraryMetadata {
    fn default() -> Self {
        Self {
            title: None,
            publish: true,
            relations: vec![],
            dependency_overrides: vec![],
            body: String::new(),
        }
    }
}

/// Metadata extracted from a directory `_index.md`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DirectoryMetadata {
    pub title: Option<String>,
    pub body: String,
}

/// A cross-source relation declared in a sidecar (spec §5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub kind: String,
    pub to: String,
}

/// A manual dependency override entry (spec §6.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyOverride {
    Add {
        to: String,
        reason: String,
    },
    Remove {
        to: String,
        reason: String,
    },
    Resolve {
        key: String,
        to: String,
        reason: String,
    },
    External {
        key: String,
        name: String,
        reason: String,
    },
}

// ─── Public entry points ─────────────────────────────────────────────────────

/// Parses a library sidecar file. Returns default metadata if the file does
/// not exist (a bare source without a sidecar is valid per spec §5.1).
pub fn parse_library_sidecar(path: &Path) -> anyhow::Result<LibraryMetadata> {
    if !path.exists() {
        return Ok(LibraryMetadata::default());
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read sidecar {}", path.display()))?;
    parse_library_sidecar_from_str(&contents, path)
}

/// Parses a directory `_index.md`. Returns default metadata if absent.
pub fn parse_directory_index(path: &Path) -> anyhow::Result<DirectoryMetadata> {
    if !path.exists() {
        return Ok(DirectoryMetadata::default());
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read _index.md {}", path.display()))?;
    parse_directory_index_from_str(&contents, path)
}

// ─── Internal parsers ────────────────────────────────────────────────────────

fn parse_library_sidecar_from_str(
    contents: &str,
    source: &Path,
) -> anyhow::Result<LibraryMetadata> {
    let (frontmatter, body) = split_frontmatter(contents, source)?;
    let raw: RawSidecar = match frontmatter {
        Some(fm) => toml::from_str(fm)
            .with_context(|| format!("malformed frontmatter in {}", source.display()))?,
        None => RawSidecar::default(),
    };
    raw.into_domain(source, body)
}

fn parse_directory_index_from_str(
    contents: &str,
    source: &Path,
) -> anyhow::Result<DirectoryMetadata> {
    let (frontmatter, body) = split_frontmatter(contents, source)?;
    let raw: RawIndex = match frontmatter {
        Some(fm) => toml::from_str(fm)
            .with_context(|| format!("malformed frontmatter in {}", source.display()))?,
        None => RawIndex::default(),
    };
    raw.into_domain(source, body)
}

/// Splits `+++`-delimited TOML frontmatter from the body.
///
/// Contracts (spec §5.1):
/// - Frontmatter is optional. When absent, the whole file is body.
/// - When a leading `+++` line exists it must be closed by another `+++` line.
fn split_frontmatter<'a>(
    contents: &'a str,
    source: &Path,
) -> anyhow::Result<(Option<&'a str>, String)> {
    let stripped = contents.trim_start_matches('\u{feff}');
    let mut lines = stripped.split_inclusive('\n');
    let first = lines.next().unwrap_or("");
    let first_trimmed = first.trim_end_matches(['\r', '\n']).trim();
    if first_trimmed != "+++" {
        return Ok((None, contents.to_string()));
    }
    // Track byte offsets so we can slice out the frontmatter.
    let start_offset = first.len();
    let mut end_offset: Option<usize> = None;
    let mut cursor = start_offset;
    for line in lines {
        let trimmed = line.trim_end_matches(['\r', '\n']).trim();
        if trimmed == "+++" {
            end_offset = Some(cursor);
            cursor += line.len();
            break;
        }
        cursor += line.len();
    }
    let end_offset = end_offset.ok_or_else(|| {
        anyhow!(
            "{}: frontmatter opened with `+++` but never closed",
            source.display()
        )
    })?;
    let frontmatter = &stripped[start_offset..end_offset];
    let body = stripped[cursor..].to_string();
    Ok((Some(frontmatter), body))
}

// ─── Raw structs (private) ───────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSidecar {
    title: Option<String>,
    publish: Option<bool>,
    #[serde(default)]
    relations: Vec<RawRelation>,
    #[serde(default)]
    dependency_overrides: Vec<RawOverride>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIndex {
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRelation {
    kind: String,
    to: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOverride {
    action: String,
    to: Option<String>,
    key: Option<String>,
    name: Option<String>,
    reason: String,
}

impl RawSidecar {
    fn into_domain(self, source: &Path, body: String) -> anyhow::Result<LibraryMetadata> {
        let title = normalise_title(self.title, source)?;
        let publish = self.publish.unwrap_or(true);

        let mut relations = Vec::with_capacity(self.relations.len());
        for raw in self.relations {
            relations.push(validate_relation(raw, source)?);
        }

        let mut overrides = Vec::with_capacity(self.dependency_overrides.len());
        for raw in self.dependency_overrides {
            overrides.push(validate_override(raw, source)?);
        }

        Ok(LibraryMetadata {
            title,
            publish,
            relations,
            dependency_overrides: overrides,
            body,
        })
    }
}

impl RawIndex {
    fn into_domain(self, source: &Path, body: String) -> anyhow::Result<DirectoryMetadata> {
        let title = normalise_title(self.title, source)?;
        Ok(DirectoryMetadata { title, body })
    }
}

fn normalise_title(value: Option<String>, source: &Path) -> anyhow::Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(t) => {
            let trimmed = t.trim();
            if trimmed.is_empty() {
                bail!(
                    "{}: `title` must not be empty (omit the key to inherit the default)",
                    source.display()
                );
            }
            Ok(Some(trimmed.to_string()))
        }
    }
}

fn validate_relation(raw: RawRelation, source: &Path) -> anyhow::Result<Relation> {
    let kind = raw.kind.trim().to_string();
    if kind.is_empty() {
        bail!("{}: relation kind must not be empty", source.display());
    }
    if !is_relation_kind(&kind) {
        bail!(
            "{}: relation kind {:?} must match [a-z][a-z0-9_-]*",
            source.display(),
            kind
        );
    }
    let to = raw.to.trim().to_string();
    if to.is_empty() {
        bail!("{}: relation.to must not be empty", source.display());
    }
    Ok(Relation { kind, to })
}

fn is_relation_kind(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn validate_override(raw: RawOverride, source: &Path) -> anyhow::Result<DependencyOverride> {
    let reason = raw.reason.trim().to_string();
    if reason.is_empty() {
        bail!(
            "{}: dependency_override.reason is required and must not be empty",
            source.display()
        );
    }
    match raw.action.as_str() {
        "add" => {
            let to = require_field(raw.to, "add", "to", source)?;
            ensure_absent(raw.key, "add", "key", source)?;
            ensure_absent(raw.name, "add", "name", source)?;
            Ok(DependencyOverride::Add { to, reason })
        }
        "remove" => {
            let to = require_field(raw.to, "remove", "to", source)?;
            ensure_absent(raw.key, "remove", "key", source)?;
            ensure_absent(raw.name, "remove", "name", source)?;
            Ok(DependencyOverride::Remove { to, reason })
        }
        "resolve" => {
            let key = require_field(raw.key, "resolve", "key", source)?;
            let to = require_field(raw.to, "resolve", "to", source)?;
            ensure_absent(raw.name, "resolve", "name", source)?;
            Ok(DependencyOverride::Resolve { key, to, reason })
        }
        "external" => {
            let key = require_field(raw.key, "external", "key", source)?;
            let name = require_field(raw.name, "external", "name", source)?;
            ensure_absent(raw.to, "external", "to", source)?;
            Ok(DependencyOverride::External { key, name, reason })
        }
        other => bail!(
            "{}: unknown dependency_override.action {:?}",
            source.display(),
            other
        ),
    }
}

fn require_field(
    value: Option<String>,
    action: &str,
    field: &str,
    source: &Path,
) -> anyhow::Result<String> {
    match value.map(|v| v.trim().to_string()) {
        Some(s) if !s.is_empty() => Ok(s),
        _ => bail!(
            "{}: dependency_override action = \"{}\" requires `{}`",
            source.display(),
            action,
            field
        ),
    }
}

fn ensure_absent(
    value: Option<String>,
    action: &str,
    field: &str,
    source: &Path,
) -> anyhow::Result<()> {
    if value.is_some() {
        bail!(
            "{}: dependency_override action = \"{}\" must not set `{}`",
            source.display(),
            action,
            field
        );
    }
    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_sidecar(s: &str) -> anyhow::Result<LibraryMetadata> {
        parse_library_sidecar_from_str(s, Path::new("test-fixture"))
    }

    fn parse_index(s: &str) -> anyhow::Result<DirectoryMetadata> {
        parse_directory_index_from_str(s, Path::new("test-fixture"))
    }

    #[test]
    fn missing_frontmatter_yields_defaults_with_body() {
        let m = parse_sidecar("Just a body.\n").unwrap();
        assert!(m.title.is_none());
        assert!(m.publish);
        assert_eq!(m.body, "Just a body.\n");
    }

    #[test]
    fn valid_sidecar_frontmatter() {
        let m = parse_sidecar(
            r#"+++
title = "Public marker"
publish = false

[[relations]]
kind = "companion"
to = "libraries/rust/private.rs"

[[dependency_overrides]]
action = "add"
to = "libraries/rust/other.rs"
reason = "macro"
+++
Body here.
"#,
        )
        .unwrap();
        assert_eq!(m.title.as_deref(), Some("Public marker"));
        assert!(!m.publish);
        assert_eq!(m.relations.len(), 1);
        assert_eq!(m.relations[0].kind, "companion");
        assert_eq!(m.dependency_overrides.len(), 1);
        assert!(m.body.starts_with("Body here."));
    }

    #[test]
    fn empty_title_is_rejected() {
        let err = parse_sidecar("+++\ntitle = \"   \"\n+++\n").unwrap_err();
        assert!(format!("{err:#}").contains("title"), "{err:#}");
    }

    #[test]
    fn unclosed_frontmatter_is_rejected() {
        let err = parse_sidecar("+++\ntitle = \"x\"\nbody").unwrap_err();
        assert!(format!("{err:#}").contains("never closed"), "{err:#}");
    }

    #[test]
    fn unknown_key_in_sidecar_is_rejected() {
        let err = parse_sidecar("+++\ntitle = \"x\"\nfoo = 1\n+++\n").unwrap_err();
        assert!(format!("{err:#}").contains("foo"), "{err:#}");
    }

    #[test]
    fn unknown_key_in_directory_index_is_rejected() {
        let err = parse_index("+++\ntitle = \"x\"\npublish = true\n+++\n").unwrap_err();
        assert!(format!("{err:#}").contains("publish"), "{err:#}");
    }

    #[test]
    fn relation_kind_must_match_pattern() {
        let err =
            parse_sidecar("+++\n[[relations]]\nkind = \"BadKind\"\nto = \"x\"\n+++\n").unwrap_err();
        assert!(format!("{err:#}").contains("BadKind"), "{err:#}");
    }

    #[test]
    fn dependency_override_actions_parse() {
        let m = parse_sidecar(
            r#"+++
[[dependency_overrides]]
action = "add"
to = "libraries/rust/a.rs"
reason = "r"

[[dependency_overrides]]
action = "remove"
to = "libraries/rust/b.rs"
reason = "r"

[[dependency_overrides]]
action = "resolve"
key = "use:crate::x"
to = "libraries/rust/c.rs"
reason = "r"

[[dependency_overrides]]
action = "external"
key = "import:Mathlib"
name = "Mathlib"
reason = "r"
+++
"#,
        )
        .unwrap();
        assert_eq!(m.dependency_overrides.len(), 4);
    }

    #[test]
    fn dependency_override_requires_reason() {
        let err = parse_sidecar(
            "+++\n[[dependency_overrides]]\naction = \"add\"\nto = \"x\"\nreason = \"\"\n+++\n",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("reason"), "{err:#}");
    }

    #[test]
    fn dependency_override_forbids_extraneous_fields() {
        let err = parse_sidecar(
            "+++\n[[dependency_overrides]]\naction = \"add\"\nto = \"x\"\nkey = \"y\"\nreason = \"r\"\n+++\n",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("key"), "{err:#}");
    }
}
