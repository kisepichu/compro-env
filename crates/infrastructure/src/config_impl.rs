use anyhow::{Context as _, Result};
use domain::entity::{Language, OJKind};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use usecases::config::Config;

/// Filesystem-backed [`Config`] implementation.
///
/// Holds the repository root so `submit_preprocess()` can resolve project-local
/// relative paths (`hooks/expand-libraries.sh`) against it. Other methods read
/// the global `~/.config/ce/config.toml` (or the path in `CE_CONFIG_DIR`) via
/// `Self::config_toml_path()` and ignore the project root.
pub struct ConfigImpl {
    project_root: PathBuf,
}

impl ConfigImpl {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    /// Returns the config directory path.
    /// Uses the `CE_CONFIG_DIR` environment variable if set to a non-empty, non-whitespace value;
    /// otherwise falls back to `~/.config/ce/`.
    fn config_dir() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("CE_CONFIG_DIR")
            && !dir.trim().is_empty()
        {
            return Ok(PathBuf::from(dir));
        }
        let home = std::env::var("HOME").map_err(|_| {
            anyhow::anyhow!(
                "HOME environment variable is not set; cannot determine config directory"
            )
        })?;
        Ok(PathBuf::from(home).join(".config").join("ce"))
    }

    fn config_toml_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    fn project_config_toml_path(&self) -> PathBuf {
        self.project_root.join("config.toml")
    }

    /// Reads project-local `[submit].preprocess` and returns a resolved value.
    ///
    /// Empty/whitespace-only values collapse to `None`, letting the caller fall
    /// back to the global config. Relative bare paths (no whitespace) are
    /// absolutised against `project_root`; absolute/tilde-prefixed values and
    /// whitespace-bearing commands (arguments present) are returned verbatim
    /// so the shell can honour tilde expansion and argument parsing.
    fn read_project_local_preprocess(&self) -> Option<String> {
        let path = self.project_config_toml_path();
        if !path.exists() {
            return None;
        }
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| eprintln!("warning: failed to read {}: {e}", path.display()))
            .ok()?;
        let table: toml::Table = toml::from_str(&contents)
            .map_err(|e| eprintln!("warning: failed to parse {}: {e}", path.display()))
            .ok()?;
        let raw = table
            .get("submit")
            .and_then(|v| v.get("preprocess"))
            .and_then(|v| v.as_str())?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(resolve_project_local_preprocess(trimmed, &self.project_root))
    }

    /// Reads global `[submit].preprocess` and returns the raw value unchanged
    /// (the shell handles tilde expansion, cwd-relative resolution, etc.).
    fn read_global_preprocess(&self) -> Option<String> {
        let path = Self::config_toml_path().ok()?;
        if !path.exists() {
            return None;
        }
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| eprintln!("warning: failed to read {}: {e}", path.display()))
            .ok()?;
        let table: toml::Table = toml::from_str(&contents)
            .map_err(|e| eprintln!("warning: failed to parse {}: {e}", path.display()))
            .ok()?;
        table
            .get("submit")
            .and_then(|v| v.get("preprocess"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

/// Resolves the trimmed, non-empty value of project-local `[submit].preprocess`
/// per docs/commands/submit.md.
///
/// - Absolute path (`/…`) → verbatim (shell will run it directly).
/// - Tilde-prefixed (`~/…`) → verbatim (shell will expand `~`).
/// - Contains ASCII whitespace → verbatim (treated as a shell command with
///   arguments; scripts should use `$CE_PROJECT_ROOT` to self-resolve).
/// - Otherwise (bare relative path) → `<project_root>/<value>` absolute path.
fn resolve_project_local_preprocess(trimmed: &str, project_root: &Path) -> String {
    debug_assert!(
        !trimmed.is_empty() && trimmed == trimmed.trim(),
        "caller must pass a trimmed, non-empty value",
    );
    if trimmed.starts_with('/')
        || trimmed.starts_with('~')
        || trimmed.chars().any(|c: char| c.is_ascii_whitespace())
    {
        return trimmed.to_string();
    }
    project_root.join(trimmed).to_string_lossy().into_owned()
}

impl Config for ConfigImpl {
    fn default_language(&self) -> Result<Language> {
        let path = Self::config_toml_path()?;
        if !path.exists() {
            return Err(anyhow::anyhow!(
                "default language is not set. Add the following to {}:\n  [default]\n  language = \"...\"",
                path.display()
            ));
        }
        let contents = std::fs::read_to_string(&path)?;
        let table: toml::Table = toml::from_str(&contents)?;
        let lang_str = table
            .get("default")
            .and_then(|v| v.get("language"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "default language is not set. Add the following to {}:\n  [default]\n  language = \"...\"",
                    path.display()
                )
            })?;
        Language::from_str(lang_str).map_err(|e| anyhow::anyhow!(e))
    }

    fn default_online_judge(&self) -> OJKind {
        OJKind::AtCoder
    }

    fn submit_file(&self, lang: &Language) -> String {
        let result: Result<Option<String>> = (|| {
            let path = Self::config_toml_path()?;
            if !path.exists() {
                return Ok(None);
            }
            let contents = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let table: toml::Table = toml::from_str(&contents)
                .with_context(|| format!("failed to parse {}", path.display()))?;
            Ok(table
                .get("language")
                .and_then(|v| v.get(lang.as_str()))
                .and_then(|v| v.get("solution_file"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()))
        })();
        match result {
            Ok(Some(val)) => val,
            Ok(None) => "src/main.rs".to_string(),
            Err(e) => {
                eprintln!("warning: {e}");
                "src/main.rs".to_string()
            }
        }
    }

    fn submit_preprocess(&self) -> Option<String> {
        // Empty/whitespace values collapse to `None` inside
        // `read_project_local_preprocess`, so no additional filter here.
        self.read_project_local_preprocess()
            .or_else(|| self.read_global_preprocess())
    }

    fn project_root(&self) -> &Path {
        &self.project_root
    }

    fn lang_id(&self, lang: &Language, oj: &OJKind) -> Option<String> {
        let path = Self::config_toml_path().ok()?;
        if !path.exists() {
            return None;
        }
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| eprintln!("warning: failed to read {}: {e}", path.display()))
            .ok()?;
        let table: toml::Table = toml::from_str(&contents)
            .map_err(|e| eprintln!("warning: failed to parse {}: {e}", path.display()))
            .ok()?;
        table
            .get("language")
            .and_then(|v| v.get(lang.as_str()))
            .and_then(|v| v.get(oj.as_str()))
            .and_then(|v| v.get("lang_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::entity::{Language, OJKind};
    use serial_test::serial;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            unsafe { std::env::set_var(key, value) }; // safe: tests using this guard are #[serial]
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                unsafe { std::env::set_var(self.key, previous) }; // safe: tests using this guard are #[serial]
            } else {
                unsafe { std::env::remove_var(self.key) }; // safe: tests using this guard are #[serial]
            }
        }
    }

    /// A tempdir with no `config.toml` inside — used as `project_root` when the
    /// caller only exercises global-side behaviour and wants project-local to be
    /// silently `None`.
    fn tmp_root_without_config() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn config_at(project_root: &Path) -> ConfigImpl {
        ConfigImpl::new(project_root.to_path_buf())
    }

    #[test]
    #[serial]
    fn default_online_judge_returns_atcoder() {
        let root = tmp_root_without_config();
        assert_eq!(config_at(root.path()).default_online_judge(), OJKind::AtCoder);
    }

    /// When config.toml contains `[default]\nlanguage = "rust"`, default_language() returns Ok(Language::new("rust")).
    #[test]
    #[serial]
    fn default_language_returns_rust_when_config_has_rust() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        std::fs::write(
            tmp.path().join("config.toml"),
            "[default]\nlanguage = \"rust\"\n",
        )
        .expect("failed to write config.toml");
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).default_language();
        assert_eq!(
            result.expect("expected Ok(Language::new(\"rust\"))"),
            Language::new("rust"),
        );
    }

    /// When config.toml exists but has no `language` key, default_language() returns Err.
    #[test]
    #[serial]
    fn default_language_returns_error_when_language_not_set() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        std::fs::write(tmp.path().join("config.toml"), "# no language key here\n")
            .expect("failed to write config.toml");
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).default_language();
        assert!(
            result.is_err(),
            "expected Err when language key is absent, got: {:?}",
            result,
        );
    }

    /// When config.toml does not exist, default_language() returns Err.
    #[test]
    #[serial]
    fn default_language_returns_error_when_config_not_found() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        // Deliberately do NOT create config.toml
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).default_language();
        assert!(
            result.is_err(),
            "expected Err when config.toml is missing, got: {:?}",
            result,
        );
    }

    /// submit_file returns the configured value when [language.rust].solution_file is set.
    #[test]
    #[serial]
    fn submit_file_returns_configured_value() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "[language.rust]\nsolution_file = \"src/lib.rs\"\n",
        )
        .unwrap();
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).submit_file(&Language::new("rust"));
        assert_eq!(result, "src/lib.rs");
    }

    /// submit_file returns "src/main.rs" when [language.rust].solution_file is absent.
    #[test]
    #[serial]
    fn submit_file_returns_default_when_not_configured() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "[default]\nlanguage = \"rust\"\n",
        )
        .unwrap();
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).submit_file(&Language::new("rust"));
        assert_eq!(result, "src/main.rs");
    }

    /// submit_file returns "src/main.rs" when config.toml does not exist.
    #[test]
    #[serial]
    fn submit_file_returns_default_when_no_config() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).submit_file(&Language::new("rust"));
        assert_eq!(result, "src/main.rs");
    }

    /// lang_id returns the configured lang_id when [language.rust.atcoder].lang_id is set.
    #[test]
    #[serial]
    fn lang_id_returns_configured_value() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "[language.rust.atcoder]\nlang_id = \"5054\"\n",
        )
        .unwrap();
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).lang_id(&Language::new("rust"), &OJKind::AtCoder);
        assert_eq!(result, Some("5054".to_string()));
    }

    /// lang_id resolves the LibraryChecker section: [language.rust.librarychecker].lang_id.
    #[test]
    #[serial]
    fn lang_id_returns_librarychecker_value() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "[language.rust.librarychecker]\nlang_id = \"rust\"\n",
        )
        .unwrap();
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).lang_id(&Language::new("rust"), &OJKind::LibraryChecker);
        assert_eq!(result, Some("rust".to_string()));
    }

    /// The OJ key discriminates sections: an atcoder-only config yields None for LibraryChecker.
    #[test]
    #[serial]
    fn lang_id_is_scoped_per_oj() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "[language.rust.atcoder]\nlang_id = \"5054\"\n",
        )
        .unwrap();
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).lang_id(&Language::new("rust"), &OJKind::LibraryChecker);
        assert_eq!(result, None);
    }

    /// lang_id returns None when [language.rust.atcoder].lang_id is absent.
    #[test]
    #[serial]
    fn lang_id_returns_none_when_not_configured() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.toml"), "language = \"rust\"\n").unwrap();
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).lang_id(&Language::new("rust"), &OJKind::AtCoder);
        assert_eq!(result, None);
    }

    /// lang_id returns None when config.toml does not exist.
    #[test]
    #[serial]
    fn lang_id_returns_none_when_no_config() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).lang_id(&Language::new("rust"), &OJKind::AtCoder);
        assert_eq!(result, None);
    }

    /// submit_preprocess returns the configured command from [submit].preprocess.
    #[test]
    #[serial]
    fn submit_preprocess_returns_configured_value() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "[submit]\npreprocess = \"~/.config/ce/hooks/pre.sh\"\n",
        )
        .unwrap();
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).submit_preprocess();
        assert_eq!(result, Some("~/.config/ce/hooks/pre.sh".to_string()));
    }

    /// submit_preprocess returns None when [submit].preprocess is absent.
    #[test]
    #[serial]
    fn submit_preprocess_returns_none_when_not_configured() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "[default]\nlanguage = \"rust\"\n",
        )
        .unwrap();
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).submit_preprocess();
        assert_eq!(result, None);
    }

    /// submit_preprocess returns None when config.toml does not exist.
    #[test]
    #[serial]
    fn submit_preprocess_returns_none_when_no_config() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let root = tmp_root_without_config();
        let result = config_at(root.path()).submit_preprocess();
        assert_eq!(result, None);
    }

    /// project-local [submit].preprocess が global を上書きする。
    #[test]
    #[serial]
    fn submit_preprocess_project_local_overrides_global() {
        let global_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            global_dir.path().join("config.toml"),
            "[submit]\npreprocess = \"~/.config/ce/hooks/global.sh\"\n",
        )
        .unwrap();
        let _guard_home = EnvVarGuard::set("CE_CONFIG_DIR", global_dir.path());

        let project_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            project_dir.path().join("config.toml"),
            "[submit]\npreprocess = \"hooks/expand-libraries.sh\"\n",
        )
        .unwrap();

        let config = ConfigImpl::new(project_dir.path().to_path_buf());
        let expected = project_dir
            .path()
            .join("hooks/expand-libraries.sh")
            .to_string_lossy()
            .into_owned();
        assert_eq!(config.submit_preprocess(), Some(expected));
    }

    /// project-local だけに書いてある場合、絶対パスに resolve される。
    #[test]
    #[serial]
    fn submit_preprocess_project_local_only_resolves_to_absolute() {
        let global_dir = tempfile::tempdir().unwrap();
        let _guard_home = EnvVarGuard::set("CE_CONFIG_DIR", global_dir.path());

        let project_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            project_dir.path().join("config.toml"),
            "[submit]\npreprocess = \"hooks/expand-libraries.sh\"\n",
        )
        .unwrap();

        let config = ConfigImpl::new(project_dir.path().to_path_buf());
        let expected = project_dir
            .path()
            .join("hooks/expand-libraries.sh")
            .to_string_lossy()
            .into_owned();
        assert_eq!(config.submit_preprocess(), Some(expected));
    }

    /// project-local に無ければ global にフォールバックし、global 値はそのまま返る (tilde 保存)。
    #[test]
    #[serial]
    fn submit_preprocess_falls_back_to_global_verbatim() {
        let global_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            global_dir.path().join("config.toml"),
            "[submit]\npreprocess = \"~/.config/ce/hooks/global.sh\"\n",
        )
        .unwrap();
        let _guard_home = EnvVarGuard::set("CE_CONFIG_DIR", global_dir.path());

        let project_dir = tempfile::tempdir().unwrap();
        // project-local config.toml は作らない

        let config = ConfigImpl::new(project_dir.path().to_path_buf());
        assert_eq!(
            config.submit_preprocess(),
            Some("~/.config/ce/hooks/global.sh".to_string())
        );
    }

    /// project-local の絶対パスと tilde 始まりはそのまま返る (resolve しない)。
    #[test]
    #[serial]
    fn submit_preprocess_project_local_absolute_and_tilde_pass_through() {
        let global_dir = tempfile::tempdir().unwrap();
        let _guard_home = EnvVarGuard::set("CE_CONFIG_DIR", global_dir.path());

        let project_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            project_dir.path().join("config.toml"),
            "[submit]\npreprocess = \"/opt/ce/hooks/x.sh\"\n",
        )
        .unwrap();

        let config = ConfigImpl::new(project_dir.path().to_path_buf());
        assert_eq!(
            config.submit_preprocess(),
            Some("/opt/ce/hooks/x.sh".to_string())
        );

        std::fs::write(
            project_dir.path().join("config.toml"),
            "[submit]\npreprocess = \"~/foo/x.sh\"\n",
        )
        .unwrap();
        let config = ConfigImpl::new(project_dir.path().to_path_buf());
        assert_eq!(
            config.submit_preprocess(),
            Some("~/foo/x.sh".to_string())
        );
    }

    /// project-local が空白を含む (引数付きコマンド) 場合はそのまま返る (絶対パス化しない)。
    #[test]
    #[serial]
    fn submit_preprocess_project_local_command_with_args_passes_through() {
        let global_dir = tempfile::tempdir().unwrap();
        let _guard_home = EnvVarGuard::set("CE_CONFIG_DIR", global_dir.path());

        let project_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            project_dir.path().join("config.toml"),
            "[submit]\npreprocess = \"hooks/expand-libraries.sh --debug\"\n",
        )
        .unwrap();

        let config = ConfigImpl::new(project_dir.path().to_path_buf());
        assert_eq!(
            config.submit_preprocess(),
            Some("hooks/expand-libraries.sh --debug".to_string())
        );
    }

    /// project-local `preprocess = ""` (空文字 / 空白のみ) は「未設定」扱いで global に fallback する。
    #[test]
    #[serial]
    fn submit_preprocess_project_local_empty_value_falls_back_to_global() {
        let global_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            global_dir.path().join("config.toml"),
            "[submit]\npreprocess = \"~/.config/ce/hooks/global.sh\"\n",
        )
        .unwrap();
        let _guard_home = EnvVarGuard::set("CE_CONFIG_DIR", global_dir.path());

        let project_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            project_dir.path().join("config.toml"),
            "[submit]\npreprocess = \"   \"\n",
        )
        .unwrap();

        let config = ConfigImpl::new(project_dir.path().to_path_buf());
        assert_eq!(
            config.submit_preprocess(),
            Some("~/.config/ce/hooks/global.sh".to_string()),
        );
    }
}
