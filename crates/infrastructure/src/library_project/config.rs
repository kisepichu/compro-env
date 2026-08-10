//! Strict TOML loader for the project-local `[library]` section (spec §6.1).
//!
//! `<repository_root>/config.toml` is parsed with `deny_unknown_fields` on
//! every raw struct so a typo or new key surfaces as a config error. The user
//! global `config.toml` at `~/.config/ce/` (or `CE_CONFIG_DIR`) is never read
//! from this loader: the library platform intentionally does not merge with
//! the existing CLI configuration.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use domain::library::{
    AnalyzerConfig, DEFAULT_TIMEOUT_SECONDS, ExpectedToolchain, LanguageConfig, LanguageId,
    LibraryProjectConfig, OnlineJudgeLanguageMapping, SiteConfig,
};
use serde::Deserialize;

/// Entry point for the strict library project loader.
pub struct ProjectLibraryConfigLoader;

impl ProjectLibraryConfigLoader {
    /// Loads `<repository_root>/config.toml` and returns the validated
    /// `[library]` section. Returns an error if the file is missing, contains
    /// unknown keys, or fails one of the semantic constraints from spec §6.1.
    pub fn load(repository_root: &Path) -> anyhow::Result<LibraryProjectConfig> {
        let path = repository_root.join("config.toml");
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Self::load_from_str(&contents, &path)
    }

    fn load_from_str(contents: &str, source: &Path) -> anyhow::Result<LibraryProjectConfig> {
        let raw: RawRoot = toml::from_str(contents)
            .with_context(|| format!("failed to parse {}", source.display()))?;
        let raw_library = raw
            .library
            .ok_or_else(|| anyhow!("{} is missing the [library] section", source.display()))?;
        raw_library.into_domain(source)
    }
}

// ─── Raw deserialization structs (private) ───────────────────────────────────

// Non-library keys are accepted at the root because the same file also hosts
// the existing CLI configuration (`[default]`, `[language.*]`, etc.). Strict
// unknown-key checking only applies inside `[library]`.
#[derive(Debug, Deserialize)]
struct RawRoot {
    library: Option<RawLibrary>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLibrary {
    site: Option<RawSite>,
    #[serde(default)]
    languages: BTreeMap<String, RawLanguage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSite {
    title: String,
    description: String,
    language: String,
    repository_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLanguage {
    display_name: Option<String>,
    root: Option<PathBuf>,
    include: Option<Vec<String>>,
    #[serde(default)]
    exclude: Vec<String>,
    check_command: Option<String>,
    check_timeout_seconds: Option<u32>,
    syntax_highlight: Option<String>,
    analyzer: Option<RawAnalyzer>,
    #[serde(default)]
    expected_toolchains: Vec<RawToolchain>,
    #[serde(default)]
    online_judges: BTreeMap<String, RawOnlineJudge>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAnalyzer {
    command: Vec<String>,
    timeout_seconds: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawToolchain {
    name: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOnlineJudge {
    language_id: String,
}

// ─── Validation into domain types ────────────────────────────────────────────

impl RawLibrary {
    fn into_domain(self, source: &Path) -> anyhow::Result<LibraryProjectConfig> {
        let RawLibrary { site, languages } = self;

        let site = site
            .map(|s| validate_site(s, source))
            .transpose()?
            .ok_or_else(|| {
                anyhow!(
                    "{}: [library.site] is required (title, description, language, repository_url)",
                    source.display()
                )
            })?;

        let mut typed_languages = BTreeMap::new();
        for (raw_id, raw_lang) in languages {
            let language_id = LanguageId::parse(&raw_id).with_context(|| {
                format!(
                    "{}: [library.languages.{}] is not a valid language id",
                    source.display(),
                    raw_id
                )
            })?;
            let language = validate_language(&language_id, raw_lang, source)?;
            typed_languages.insert(language_id, language);
        }

        Ok(LibraryProjectConfig {
            languages: typed_languages,
            site: Some(site),
        })
    }
}

fn validate_site(raw: RawSite, source: &Path) -> anyhow::Result<SiteConfig> {
    let RawSite {
        title,
        description,
        language,
        repository_url,
    } = raw;
    let title = non_empty("library.site.title", &title, source)?;
    let description = non_empty("library.site.description", &description, source)?;
    let language = non_empty("library.site.language", &language, source)?;
    let repository_url = non_empty("library.site.repository_url", &repository_url, source)?;
    Ok(SiteConfig {
        title,
        description,
        language,
        repository_url,
    })
}

fn validate_language(
    id: &LanguageId,
    raw: RawLanguage,
    source: &Path,
) -> anyhow::Result<LanguageConfig> {
    let RawLanguage {
        display_name,
        root,
        include,
        exclude,
        check_command,
        check_timeout_seconds,
        syntax_highlight,
        analyzer,
        expected_toolchains,
        online_judges,
    } = raw;

    let root_path = root.ok_or_else(|| {
        anyhow!(
            "{}: [library.languages.{}].root is required",
            source.display(),
            id
        )
    })?;
    let root = root_path
        .into_os_string()
        .into_string()
        .map_err(|_| anyhow!("[library.languages.{id}].root is not valid UTF-8"))?;
    if root.is_empty() {
        bail!(
            "{}: [library.languages.{}].root is empty",
            source.display(),
            id
        );
    }

    let include = include.ok_or_else(|| {
        anyhow!(
            "{}: [library.languages.{}].include is required",
            source.display(),
            id
        )
    })?;
    if include.is_empty() {
        bail!(
            "{}: [library.languages.{}].include must contain at least one glob pattern",
            source.display(),
            id
        );
    }
    for pat in &include {
        if pat.trim().is_empty() {
            bail!(
                "{}: [library.languages.{}].include contains an empty pattern",
                source.display(),
                id
            );
        }
    }

    let check_timeout_seconds = validate_timeout(
        check_timeout_seconds,
        &format!("[library.languages.{id}].check_timeout_seconds"),
        source,
    )?
    .unwrap_or(DEFAULT_TIMEOUT_SECONDS);

    let analyzer_raw = analyzer.ok_or_else(|| {
        anyhow!(
            "{}: [library.languages.{}].analyzer.command is required",
            source.display(),
            id
        )
    })?;
    let analyzer = validate_analyzer(id, analyzer_raw, source)?;

    let mut typed_toolchains = Vec::with_capacity(expected_toolchains.len());
    let mut seen_toolchain_names = BTreeSet::new();
    for tc in expected_toolchains {
        let name = non_empty(
            &format!("[library.languages.{id}].expected_toolchains[].name"),
            &tc.name,
            source,
        )?;
        let version = non_empty(
            &format!("[library.languages.{id}].expected_toolchains[].version"),
            &tc.version,
            source,
        )?;
        if !seen_toolchain_names.insert(name.clone()) {
            bail!(
                "{}: [library.languages.{}].expected_toolchains contains duplicate toolchain name {:?}",
                source.display(),
                id,
                name
            );
        }
        typed_toolchains.push(ExpectedToolchain { name, version });
    }

    let mut typed_online_judges = BTreeMap::new();
    for (oj, mapping) in online_judges {
        if oj.trim().is_empty() {
            bail!(
                "{}: [library.languages.{}].online_judges has an empty key",
                source.display(),
                id
            );
        }
        let language_id = non_empty(
            &format!("[library.languages.{id}].online_judges.{oj}.language_id"),
            &mapping.language_id,
            source,
        )?;
        typed_online_judges.insert(oj, OnlineJudgeLanguageMapping { language_id });
    }

    Ok(LanguageConfig {
        id: id.clone(),
        display_name: display_name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        root,
        include,
        exclude,
        check_command: check_command
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        check_timeout_seconds,
        syntax_highlight: syntax_highlight
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        analyzer,
        expected_toolchains: typed_toolchains,
        online_judges: typed_online_judges,
    })
}

fn validate_analyzer(
    id: &LanguageId,
    raw: RawAnalyzer,
    source: &Path,
) -> anyhow::Result<AnalyzerConfig> {
    let RawAnalyzer {
        command,
        timeout_seconds,
    } = raw;
    if command.is_empty() {
        bail!(
            "{}: [library.languages.{}].analyzer.command must be a non-empty argv array",
            source.display(),
            id
        );
    }
    for arg in &command {
        if arg.trim().is_empty() {
            bail!(
                "{}: [library.languages.{}].analyzer.command contains an empty argument",
                source.display(),
                id
            );
        }
    }
    let timeout_seconds = validate_timeout(
        timeout_seconds,
        &format!("[library.languages.{id}].analyzer.timeout_seconds"),
        source,
    )?
    .unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    Ok(AnalyzerConfig {
        command,
        timeout_seconds,
    })
}

fn validate_timeout(value: Option<u32>, key: &str, source: &Path) -> anyhow::Result<Option<u32>> {
    match value {
        Some(0) => bail!("{}: {} must be a positive integer", source.display(), key),
        other => Ok(other),
    }
}

fn non_empty(key: &str, value: &str, source: &Path) -> anyhow::Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!(
            "{}: {} must not be empty (trim-empty is treated as missing)",
            source.display(),
            key
        );
    }
    Ok(trimmed.to_string())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use domain::library::LanguageId;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("library-project")
    }

    fn load_str(toml: &str) -> anyhow::Result<LibraryProjectConfig> {
        ProjectLibraryConfigLoader::load_from_str(toml, Path::new("test-fixture"))
    }

    #[test]
    fn loads_valid_three_language_config() {
        let contents = std::fs::read_to_string(fixture_root().join("config-valid.toml")).unwrap();
        let config = load_str(&contents).unwrap();

        assert_eq!(
            config
                .languages
                .keys()
                .map(LanguageId::as_str)
                .collect::<Vec<_>>(),
            vec!["cpp", "lean", "rust"]
        );

        let rust = &config.languages[&LanguageId::parse("rust").unwrap()];
        assert_eq!(rust.root, "libraries/rust");
        assert_eq!(rust.analyzer.timeout_seconds, 600);
        assert_eq!(rust.check_timeout_seconds, 600);
        assert_eq!(rust.effective_syntax_highlight(), "rust");
        assert_eq!(rust.expected_toolchains.len(), 2);
        assert_eq!(rust.online_judges["librarychecker"].language_id, "rust");

        let cpp = &config.languages[&LanguageId::parse("cpp").unwrap()];
        assert_eq!(cpp.effective_display_name(), "C++");
        // analyzer.timeout_seconds omitted -> default 600.
        assert_eq!(cpp.analyzer.timeout_seconds, 600);
        // check_command omitted.
        assert!(cpp.check_command.is_none());

        let site = config.site.as_ref().unwrap();
        assert_eq!(site.language, "en");
    }

    #[test]
    fn rejects_missing_root() {
        let err = load_str(
            r#"
[library.site]
title = "t"
description = "d"
language = "en"
repository_url = "https://example.com"

[library.languages.rust]
include = ["**/*.rs"]

[library.languages.rust.analyzer]
command = ["./bin/rust"]
"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("root"), "{err:#}");
    }

    #[test]
    fn rejects_missing_include() {
        let err = load_str(
            r#"
[library.site]
title = "t"
description = "d"
language = "en"
repository_url = "https://example.com"

[library.languages.rust]
root = "libraries/rust"

[library.languages.rust.analyzer]
command = ["./bin/rust"]
"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("include"), "{err:#}");
    }

    #[test]
    fn rejects_missing_analyzer_command() {
        let err = load_str(
            r#"
[library.site]
title = "t"
description = "d"
language = "en"
repository_url = "https://example.com"

[library.languages.rust]
root = "libraries/rust"
include = ["**/*.rs"]
"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("analyzer.command"), "{err:#}");
    }

    #[test]
    fn rejects_empty_analyzer_command() {
        let err = load_str(
            r#"
[library.site]
title = "t"
description = "d"
language = "en"
repository_url = "https://example.com"

[library.languages.rust]
root = "libraries/rust"
include = ["**/*.rs"]

[library.languages.rust.analyzer]
command = []
"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("analyzer.command"), "{err:#}");
    }

    #[test]
    fn rejects_unknown_key() {
        let contents =
            std::fs::read_to_string(fixture_root().join("config-unknown-key.toml")).unwrap();
        let err = load_str(&contents).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("unknown"), "{msg}");
    }

    #[test]
    fn rejects_invalid_language_id() {
        let err = load_str(
            r#"
[library.site]
title = "t"
description = "d"
language = "en"
repository_url = "https://example.com"

[library.languages.Rust]
root = "libraries/rust"
include = ["**/*.rs"]

[library.languages.Rust.analyzer]
command = ["./bin/rust"]
"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("Rust"), "{err:#}");
    }

    #[test]
    fn rejects_invalid_timeout() {
        let err = load_str(
            r#"
[library.site]
title = "t"
description = "d"
language = "en"
repository_url = "https://example.com"

[library.languages.rust]
root = "libraries/rust"
include = ["**/*.rs"]
check_timeout_seconds = 0

[library.languages.rust.analyzer]
command = ["./bin/rust"]
"#,
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("check_timeout_seconds"),
            "{err:#}"
        );
    }

    #[test]
    fn rejects_duplicate_toolchains() {
        let err = load_str(
            r#"
[library.site]
title = "t"
description = "d"
language = "en"
repository_url = "https://example.com"

[library.languages.rust]
root = "libraries/rust"
include = ["**/*.rs"]
expected_toolchains = [
  { name = "rustc", version = "1.92.0" },
  { name = "rustc", version = "1.92.1" },
]

[library.languages.rust.analyzer]
command = ["./bin/rust"]
"#,
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("duplicate toolchain"),
            "{err:#}"
        );
    }

    #[test]
    fn rejects_missing_site_metadata() {
        let err = load_str(
            r#"
[library.languages.rust]
root = "libraries/rust"
include = ["**/*.rs"]

[library.languages.rust.analyzer]
command = ["./bin/rust"]
"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("[library.site]"), "{err:#}");
    }

    #[test]
    fn rejects_blank_site_field() {
        let err = load_str(
            r#"
[library.site]
title = "   "
description = "d"
language = "en"
repository_url = "https://example.com"

[library.languages.rust]
root = "libraries/rust"
include = ["**/*.rs"]

[library.languages.rust.analyzer]
command = ["./bin/rust"]
"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("title"), "{err:#}");
    }

    #[test]
    fn allows_non_library_keys_at_root() {
        // The same file also hosts the existing CLI configuration.
        let contents = format!(
            "{}\n\n[default]\nlanguage = \"rust\"\n",
            std::fs::read_to_string(fixture_root().join("config-valid.toml")).unwrap()
        );
        let config = load_str(&contents).unwrap();
        assert_eq!(config.languages.len(), 3);
    }

    /// The loader must never read the user-global config (`CE_CONFIG_DIR` or
    /// `~/.config/ce/`). Setting `CE_CONFIG_DIR` to a directory containing a
    /// conflicting `[library]` section must not affect the repository result.
    #[test]
    #[serial_test::serial]
    fn ignores_user_global_config_dir() {
        let global_tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            global_tmp.path().join("config.toml"),
            r#"
[library.site]
title = "global"
description = "global"
language = "ja"
repository_url = "https://example.com/global"

[library.languages.rust]
root = "libraries/rust-global"
include = ["**/*.rs"]

[library.languages.rust.analyzer]
command = ["./bin/rust-global"]
"#,
        )
        .unwrap();

        let repo_tmp = tempfile::tempdir().unwrap();
        let repo_config = repo_tmp.path().join("config.toml");
        std::fs::copy(fixture_root().join("config-valid.toml"), &repo_config).unwrap();

        // Snapshot and restore CE_CONFIG_DIR so this test does not leak into others.
        let previous = std::env::var_os("CE_CONFIG_DIR");
        // SAFETY: single-threaded test process; only this test touches CE_CONFIG_DIR.
        unsafe { std::env::set_var("CE_CONFIG_DIR", global_tmp.path()) };

        let config = ProjectLibraryConfigLoader::load(repo_tmp.path()).unwrap();

        // SAFETY: same justification as above.
        unsafe {
            match previous {
                Some(v) => std::env::set_var("CE_CONFIG_DIR", v),
                None => std::env::remove_var("CE_CONFIG_DIR"),
            }
        }

        let rust = &config.languages[&LanguageId::parse("rust").unwrap()];
        assert_eq!(rust.root, "libraries/rust");
        assert_ne!(rust.root, "libraries/rust-global");
        assert_eq!(config.site.as_ref().unwrap().title, "compro-env");
    }
}
