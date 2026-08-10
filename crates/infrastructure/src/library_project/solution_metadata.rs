//! Strict TOML parsing for solution `ce.toml` files under `solutions/` and the
//! per-contest `.ce.toml` file used by the library platform (spec §4.2, §5,
//! §7.2). Existing CLI keys such as `language` and `test_command` are
//! preserved; new library-publication keys go through `deny_unknown_fields`.

use std::path::Path;

use anyhow::{Context, anyhow, bail};
use chrono::{DateTime, FixedOffset};
use serde::Deserialize;

/// Parsed contents of a solution `ce.toml`.
///
/// Existing CLI keys such as `language` and `test_command` remain part of the
/// schema, but any additional unknown key is rejected so we catch typos in the
/// new library-publication keys immediately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolutionCeToml {
    pub language: String,
    pub test_command: String,
    pub publish: bool,
    pub solved_at: Option<DateTime<FixedOffset>>,
    pub test_timeout_seconds: u32,
    pub verify: Option<VerifyBlock>,
}

/// `[verify]` block parsed structurally. Library ID validity, publication
/// checks, and OJ language mapping resolution belong to a higher layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyBlock {
    pub libraries: Vec<String>,
    pub language_id: Option<String>,
}

/// Parsed contents of a per-contest `.ce.toml`.
///
/// The library platform reads this file so it knows a contest directory
/// exists, but its schema is intentionally permissive: the existing CLI keys
/// (`online_judge`, `contest_id`, `problems`, ...) may or may not be present.
/// Only keys under the new `[library]` sub-table are strictly validated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContestCeToml {
    pub display_title: Option<String>,
}

// ─── Entry points ────────────────────────────────────────────────────────────

pub fn parse_solution_ce_toml(path: &Path) -> anyhow::Result<SolutionCeToml> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    parse_solution_ce_toml_from_str(&contents, path)
}

pub fn parse_contest_ce_toml(path: &Path) -> anyhow::Result<ContestCeToml> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    parse_contest_ce_toml_from_str(&contents, path)
}

fn parse_solution_ce_toml_from_str(
    contents: &str,
    source: &Path,
) -> anyhow::Result<SolutionCeToml> {
    let raw: RawSolutionCeToml = toml::from_str(contents)
        .with_context(|| format!("failed to parse {}", source.display()))?;
    raw.into_domain(source)
}

fn parse_contest_ce_toml_from_str(contents: &str, source: &Path) -> anyhow::Result<ContestCeToml> {
    let raw: RawContestCeToml = toml::from_str(contents)
        .with_context(|| format!("failed to parse {}", source.display()))?;
    Ok(ContestCeToml {
        display_title: raw.library.and_then(|l| l.title),
    })
}

// ─── Raw structs ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSolutionCeToml {
    language: String,
    test_command: String,
    #[serde(default)]
    publish: bool,
    solved_at: Option<String>,
    test_timeout_seconds: Option<u32>,
    verify: Option<RawVerifyBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVerifyBlock {
    libraries: Vec<String>,
    language_id: Option<String>,
}

/// Contest-level `.ce.toml`. Anything under `[library]` must match this
/// strict schema; other top-level keys are ignored so we do not conflict with
/// existing CLI contest metadata (pre-decided default #4).
#[derive(Debug, Default, Deserialize)]
struct RawContestCeToml {
    library: Option<RawContestLibrarySection>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContestLibrarySection {
    title: Option<String>,
}

impl RawSolutionCeToml {
    fn into_domain(self, source: &Path) -> anyhow::Result<SolutionCeToml> {
        let RawSolutionCeToml {
            language,
            test_command,
            publish,
            solved_at,
            test_timeout_seconds,
            verify,
        } = self;

        let language = language.trim().to_string();
        if language.is_empty() {
            bail!("{}: `language` must not be empty", source.display());
        }
        let test_command = test_command.trim().to_string();
        if test_command.is_empty() {
            bail!("{}: `test_command` must not be empty", source.display());
        }

        let solved_at = match solved_at {
            Some(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    bail!("{}: `solved_at` must not be empty", source.display());
                }
                let dt = DateTime::parse_from_rfc3339(trimmed).with_context(|| {
                    format!(
                        "{}: `solved_at` must be an RFC 3339 timestamp with a timezone",
                        source.display()
                    )
                })?;
                Some(dt)
            }
            None => None,
        };

        if publish && solved_at.is_none() {
            bail!(
                "{}: `publish = true` requires `solved_at` (RFC 3339 with timezone)",
                source.display()
            );
        }

        let test_timeout_seconds = match test_timeout_seconds {
            None => 600,
            Some(0) => bail!(
                "{}: `test_timeout_seconds` must be a positive integer",
                source.display()
            ),
            Some(v) => v,
        };

        let verify = match verify {
            None => None,
            Some(raw) => Some(validate_verify(raw, source)?),
        };

        if verify.is_some() && !publish {
            bail!(
                "{}: `[verify]` requires `publish = true` (spec §7.2)",
                source.display()
            );
        }

        if verify.is_some() && solved_at.is_none() {
            bail!(
                "{}: `[verify]` requires a `solved_at` timestamp",
                source.display()
            );
        }

        Ok(SolutionCeToml {
            language,
            test_command,
            publish,
            solved_at,
            test_timeout_seconds,
            verify,
        })
    }
}

fn validate_verify(raw: RawVerifyBlock, source: &Path) -> anyhow::Result<VerifyBlock> {
    let RawVerifyBlock {
        libraries,
        language_id,
    } = raw;
    if libraries.is_empty() {
        bail!(
            "{}: `[verify].libraries` must contain at least one library id",
            source.display()
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    for lib in &libraries {
        if lib.trim().is_empty() {
            bail!(
                "{}: `[verify].libraries` contains an empty entry",
                source.display()
            );
        }
        if !seen.insert(lib.as_str()) {
            bail!(
                "{}: `[verify].libraries` contains duplicate entry {:?}",
                source.display(),
                lib
            );
        }
    }
    let language_id = language_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if language_id.as_deref() == Some("") {
        bail!(
            "{}: `[verify].language_id` must not be empty when present",
            source.display()
        );
    }
    Ok(VerifyBlock {
        libraries,
        language_id,
    })
}

// A quick alias to signal that a value came from adapter mapping resolution.
pub type ResolvedOjLanguageId = String;

/// Result of resolving `[verify].language_id` for a solution (spec §7.2).
pub fn resolve_oj_language_id(
    verify: &VerifyBlock,
    oj: &str,
    library_config: &domain::library::LanguageConfig,
) -> anyhow::Result<ResolvedOjLanguageId> {
    if let Some(id) = &verify.language_id {
        return Ok(id.clone());
    }
    let mapping = library_config.online_judges.get(oj).ok_or_else(|| {
        anyhow!(
            "no `[verify].language_id` and no `[library.languages.{}].online_judges.{}]`",
            library_config.id,
            oj
        )
    })?;
    Ok(mapping.language_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> anyhow::Result<SolutionCeToml> {
        parse_solution_ce_toml_from_str(s, Path::new("test-fixture"))
    }

    #[test]
    fn parses_public_solution_with_verify() {
        let t = parse(
            r#"
language = "rust"
test_command = "./test.sh"
publish = true
solved_at = "2026-08-02T14:30:00+09:00"

[verify]
libraries = ["libraries/rust/a.rs"]
language_id = "rust"
"#,
        )
        .unwrap();
        assert!(t.publish);
        assert!(t.solved_at.is_some());
        assert!(t.verify.is_some());
        assert_eq!(t.test_timeout_seconds, 600);
    }

    #[test]
    fn parses_private_solution() {
        let t = parse(
            r#"
language = "rust"
test_command = "./test.sh"
"#,
        )
        .unwrap();
        assert!(!t.publish);
        assert!(t.verify.is_none());
    }

    #[test]
    fn public_solution_requires_solved_at() {
        let err = parse(
            r#"
language = "rust"
test_command = "./test.sh"
publish = true
"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("solved_at"), "{err:#}");
    }

    #[test]
    fn verify_on_private_solution_is_rejected() {
        let err = parse(
            r#"
language = "rust"
test_command = "./test.sh"
solved_at = "2026-08-02T14:30:00+09:00"

[verify]
libraries = ["libraries/rust/a.rs"]
language_id = "rust"
"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("publish = true"), "{err:#}");
    }

    #[test]
    fn verify_libraries_must_be_nonempty_and_unique() {
        let err = parse(
            r#"
language = "rust"
test_command = "./test.sh"
publish = true
solved_at = "2026-08-02T14:30:00+09:00"

[verify]
libraries = []
"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("libraries"), "{err:#}");

        let err2 = parse(
            r#"
language = "rust"
test_command = "./test.sh"
publish = true
solved_at = "2026-08-02T14:30:00+09:00"

[verify]
libraries = ["libraries/rust/a.rs", "libraries/rust/a.rs"]
"#,
        )
        .unwrap_err();
        assert!(format!("{err2:#}").contains("duplicate"), "{err2:#}");
    }

    #[test]
    fn solved_at_must_be_rfc3339_with_timezone() {
        let err = parse(
            r#"
language = "rust"
test_command = "./test.sh"
publish = true
solved_at = "2026-08-02T14:30:00"
"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("RFC 3339"), "{err:#}");
    }

    #[test]
    fn unknown_top_level_key_is_rejected() {
        let err = parse(
            r#"
language = "rust"
test_command = "./test.sh"
publish = true
solved_at = "2026-08-02T14:30:00+09:00"
mystery = "boom"
"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("mystery"), "{err:#}");
    }

    #[test]
    fn zero_timeout_is_rejected() {
        let err = parse(
            r#"
language = "rust"
test_command = "./test.sh"
test_timeout_seconds = 0
"#,
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("test_timeout_seconds"),
            "{err:#}"
        );
    }

    #[test]
    fn contest_ce_toml_accepts_existing_cli_keys() {
        let contents = r#"
online_judge = "librarychecker"
contest_id = "librarychecker-aplusb"

[[problems]]
id = "aplusb"
code = "aplusb"
title = "A + B"
"#;
        let contest = parse_contest_ce_toml_from_str(contents, Path::new("test-fixture")).unwrap();
        assert!(contest.display_title.is_none());
    }

    #[test]
    fn resolve_oj_language_id_prefers_solution_override() {
        use domain::library::{
            AnalyzerConfig, DEFAULT_TIMEOUT_SECONDS, LanguageConfig, LanguageId,
            OnlineJudgeLanguageMapping,
        };
        let mut oj_map = std::collections::BTreeMap::new();
        oj_map.insert(
            "librarychecker".to_string(),
            OnlineJudgeLanguageMapping {
                language_id: "rust".into(),
            },
        );
        let cfg = LanguageConfig {
            id: LanguageId::parse("rust").unwrap(),
            display_name: None,
            root: "libraries/rust".into(),
            include: vec!["**/*.rs".into()],
            exclude: vec![],
            check_command: None,
            check_timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            syntax_highlight: None,
            analyzer: AnalyzerConfig {
                command: vec!["./bin".into()],
                timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            },
            expected_toolchains: vec![],
            online_judges: oj_map,
            entry_file: "src/main.rs".into(),
        };
        let verify_override = VerifyBlock {
            libraries: vec!["libraries/rust/a.rs".into()],
            language_id: Some("rust-2024".into()),
        };
        assert_eq!(
            resolve_oj_language_id(&verify_override, "librarychecker", &cfg).unwrap(),
            "rust-2024"
        );
        let verify_none = VerifyBlock {
            libraries: vec!["libraries/rust/a.rs".into()],
            language_id: None,
        };
        assert_eq!(
            resolve_oj_language_id(&verify_none, "librarychecker", &cfg).unwrap(),
            "rust"
        );
        assert!(resolve_oj_language_id(&verify_none, "unknown-oj", &cfg).is_err());
    }
}
