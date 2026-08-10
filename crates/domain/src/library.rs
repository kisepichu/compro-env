//! Value objects and configuration models for the library platform.
//!
//! Only pure data lives here. Filesystem parsing (TOML, discovery) belongs in
//! the `infrastructure` crate.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default timeout applied to analyzer and check commands (see spec §6.1).
pub const DEFAULT_TIMEOUT_SECONDS: u32 = 600;

// ─── Errors ──────────────────────────────────────────────────────────────────

/// Reasons an ID string is rejected by validation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdError {
    #[error("id must not be empty")]
    Empty,
    #[error("id contains invalid character(s): {value:?}")]
    InvalidCharacter { value: String },
    #[error("path escapes the repository: {value:?}")]
    PathEscape { value: String },
    #[error("path must be relative: {value:?}")]
    NotRelative { value: String },
    #[error("path segment {segment:?} is not allowed")]
    InvalidSegment { segment: String },
    #[error("solution id must have three '/'-separated segments: {value:?}")]
    MalformedSolutionId { value: String },
}

// ─── LanguageId ──────────────────────────────────────────────────────────────

/// Stable slug identifying a library language (spec §6.1).
///
/// Must match `[a-z][a-z0-9-]*`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LanguageId(String);

impl LanguageId {
    pub fn parse(value: &str) -> Result<Self, IdError> {
        if value.is_empty() {
            return Err(IdError::Empty);
        }
        let mut chars = value.chars();
        let first = chars.next().unwrap();
        if !first.is_ascii_lowercase() {
            return Err(IdError::InvalidCharacter {
                value: value.to_string(),
            });
        }
        for c in chars {
            if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
                return Err(IdError::InvalidCharacter {
                    value: value.to_string(),
                });
            }
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LanguageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for LanguageId {
    type Error = IdError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<LanguageId> for String {
    fn from(id: LanguageId) -> Self {
        id.0
    }
}

// ─── Repository-relative path validation ────────────────────────────────────

fn validate_repo_relative_path(value: &str) -> Result<(), IdError> {
    if value.is_empty() {
        return Err(IdError::Empty);
    }
    if value.starts_with('/') {
        return Err(IdError::NotRelative {
            value: value.to_string(),
        });
    }
    if value.contains('\\') {
        return Err(IdError::InvalidCharacter {
            value: value.to_string(),
        });
    }
    if value.contains('\0') {
        return Err(IdError::InvalidCharacter {
            value: value.to_string(),
        });
    }
    for segment in value.split('/') {
        match segment {
            "" | "." | ".." => {
                return Err(IdError::InvalidSegment {
                    segment: segment.to_string(),
                });
            }
            _ => {}
        }
    }
    if value.split('/').any(|s| s.trim().is_empty()) {
        return Err(IdError::InvalidSegment {
            segment: value.to_string(),
        });
    }
    // Reject anything that would try to escape via drive letters or scheme prefixes.
    if value.contains(':') {
        return Err(IdError::PathEscape {
            value: value.to_string(),
        });
    }
    Ok(())
}

// ─── LibraryId ───────────────────────────────────────────────────────────────

/// Repository-relative source path identifying a `LibraryFile` (spec §4.1).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LibraryId(String);

impl LibraryId {
    pub fn parse(value: &str) -> Result<Self, IdError> {
        validate_repo_relative_path(value)?;
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LibraryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for LibraryId {
    type Error = IdError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<LibraryId> for String {
    fn from(id: LibraryId) -> Self {
        id.0
    }
}

// ─── SolutionId ──────────────────────────────────────────────────────────────

/// `{contest_id}/{problem_code}/{solution_name}` triple (spec §4.2, §5).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SolutionId {
    raw: String,
    contest_end: usize,
    problem_end: usize,
}

impl SolutionId {
    pub fn parse(value: &str) -> Result<Self, IdError> {
        validate_repo_relative_path(value)?;
        let parts: Vec<&str> = value.split('/').collect();
        if parts.len() != 3 {
            return Err(IdError::MalformedSolutionId {
                value: value.to_string(),
            });
        }
        let contest_end = parts[0].len();
        let problem_end = contest_end + 1 + parts[1].len();
        Ok(Self {
            raw: value.to_string(),
            contest_end,
            problem_end,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    pub fn contest_id(&self) -> &str {
        &self.raw[..self.contest_end]
    }

    pub fn problem_code(&self) -> &str {
        &self.raw[self.contest_end + 1..self.problem_end]
    }

    pub fn solution_name(&self) -> &str {
        &self.raw[self.problem_end + 1..]
    }
}

impl std::fmt::Display for SolutionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.raw)
    }
}

impl TryFrom<String> for SolutionId {
    type Error = IdError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<SolutionId> for String {
    fn from(id: SolutionId) -> Self {
        id.raw
    }
}

// ─── Configuration models ────────────────────────────────────────────────────

/// Full project-local `[library]` section (spec §6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryProjectConfig {
    pub languages: BTreeMap<LanguageId, LanguageConfig>,
    pub site: Option<SiteConfig>,
}

/// Per-language configuration (spec §6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageConfig {
    pub id: LanguageId,
    pub display_name: Option<String>,
    pub root: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub check_command: Option<String>,
    pub check_timeout_seconds: u32,
    pub syntax_highlight: Option<String>,
    pub analyzer: AnalyzerConfig,
    pub expected_toolchains: Vec<ExpectedToolchain>,
    pub online_judges: BTreeMap<String, OnlineJudgeLanguageMapping>,
    /// Solution-root-relative path of the file adapters and OJ submissions
    /// should treat as the entry point. Defaults to `src/main.rs` when
    /// omitted; kept configurable per language so the platform stays free of
    /// hard-coded language semantics.
    pub entry_file: String,
}

impl LanguageConfig {
    /// Effective display name: explicit `display_name` or the language id.
    pub fn effective_display_name(&self) -> &str {
        self.display_name
            .as_deref()
            .unwrap_or_else(|| self.id.as_str())
    }

    /// Effective syntax highlight token: explicit value or the language id.
    pub fn effective_syntax_highlight(&self) -> &str {
        self.syntax_highlight
            .as_deref()
            .unwrap_or_else(|| self.id.as_str())
    }
}

/// Analyzer command declaration (spec §6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyzerConfig {
    pub command: Vec<String>,
    pub timeout_seconds: u32,
}

/// Per-OJ language mapping (spec §6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnlineJudgeLanguageMapping {
    pub language_id: String,
}

/// Expected toolchain identity (spec §6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedToolchain {
    pub name: String,
    pub version: String,
}

/// Publication metadata for the static site (spec §6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteConfig {
    pub title: String,
    pub description: String,
    pub language: String,
    pub repository_url: String,
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_id_accepts_valid_slugs() {
        assert_eq!(LanguageId::parse("rust").unwrap().as_str(), "rust");
        assert_eq!(LanguageId::parse("cpp-23").unwrap().as_str(), "cpp-23");
        assert_eq!(LanguageId::parse("lean").unwrap().as_str(), "lean");
        assert_eq!(LanguageId::parse("a").unwrap().as_str(), "a");
    }

    #[test]
    fn language_id_rejects_invalid_slugs() {
        assert!(LanguageId::parse("").is_err());
        assert!(LanguageId::parse("C++").is_err());
        assert!(LanguageId::parse("Rust").is_err());
        assert!(LanguageId::parse("1cpp").is_err());
        assert!(LanguageId::parse("cpp_23").is_err());
        assert!(LanguageId::parse("cpp 23").is_err());
        assert!(LanguageId::parse("-cpp").is_err());
    }

    #[test]
    fn library_id_accepts_repo_relative_paths() {
        let id = LibraryId::parse("libraries/rust/a.rs").unwrap();
        assert_eq!(id.as_str(), "libraries/rust/a.rs");
    }

    #[test]
    fn library_id_rejects_unsafe_paths() {
        assert!(LibraryId::parse("").is_err());
        assert!(LibraryId::parse("/etc/passwd").is_err());
        assert!(LibraryId::parse("../private.rs").is_err());
        assert!(LibraryId::parse("a/../b").is_err());
        assert!(LibraryId::parse("./a").is_err());
        assert!(LibraryId::parse("a//b").is_err());
        assert!(LibraryId::parse("a\\b").is_err());
        assert!(LibraryId::parse("C:\\foo").is_err());
        assert!(LibraryId::parse("http://foo").is_err());
    }

    #[test]
    fn library_ids_sort_by_utf8_bytes() {
        let mut ids = vec![
            LibraryId::parse("libraries/rust/z.rs").unwrap(),
            LibraryId::parse("libraries/rust/a.rs").unwrap(),
            LibraryId::parse("libraries/cpp/a.hpp").unwrap(),
        ];
        ids.sort();
        assert_eq!(
            ids.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
            vec![
                "libraries/cpp/a.hpp",
                "libraries/rust/a.rs",
                "libraries/rust/z.rs",
            ]
        );
    }

    #[test]
    fn solution_id_parses_three_segments() {
        let id = SolutionId::parse("abc999/a/main").unwrap();
        assert_eq!(id.as_str(), "abc999/a/main");
        assert_eq!(id.contest_id(), "abc999");
        assert_eq!(id.problem_code(), "a");
        assert_eq!(id.solution_name(), "main");
    }

    #[test]
    fn solution_id_rejects_wrong_arity() {
        assert!(SolutionId::parse("abc999/a").is_err());
        assert!(SolutionId::parse("abc999/a/main/extra").is_err());
        assert!(SolutionId::parse("").is_err());
        assert!(SolutionId::parse("../a/b/c").is_err());
    }

    #[test]
    fn language_config_defaults_fill_in_display_name_and_highlight() {
        let cfg = LanguageConfig {
            id: LanguageId::parse("cpp").unwrap(),
            display_name: None,
            root: "libraries/cpp".into(),
            include: vec!["**/*.hpp".into()],
            exclude: vec![],
            check_command: None,
            check_timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            syntax_highlight: None,
            analyzer: AnalyzerConfig {
                command: vec!["./bin/cpp".into()],
                timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            },
            expected_toolchains: vec![],
            online_judges: BTreeMap::new(),
            entry_file: "src/main.rs".into(),
        };
        assert_eq!(cfg.effective_display_name(), "cpp");
        assert_eq!(cfg.effective_syntax_highlight(), "cpp");
    }
}
