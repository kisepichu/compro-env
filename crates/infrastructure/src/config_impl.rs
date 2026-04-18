use anyhow::Result;
use domain::entity::{Language, OJKind};
use std::path::PathBuf;
use std::str::FromStr;
use usecases::config::Config;

pub struct ConfigImpl;

impl ConfigImpl {
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
}

impl Config for ConfigImpl {
    fn default_language(&self) -> Result<Language> {
        let path = Self::config_toml_path()?;
        if !path.exists() {
            return Err(anyhow::anyhow!(
                "default language is not set. Add `language = \"...\"` to {}",
                path.display()
            ));
        }
        let contents = std::fs::read_to_string(&path)?;
        let table: toml::Table = toml::from_str(&contents)?;
        let lang_str = table
            .get("language")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "default language is not set. Add `language = \"...\"` to {}",
                    path.display()
                )
            })?;
        Language::from_str(lang_str).map_err(|e| anyhow::anyhow!(e))
    }

    fn default_online_judge(&self) -> OJKind {
        OJKind::AtCoder
    }

    fn submit_file(&self, _lang: &Language) -> String {
        todo!()
    }

    fn submit_preprocess(&self, _lang: &Language) -> String {
        todo!()
    }

    fn lang_id(&self, _lang: &Language, _oj: &OJKind) -> Option<String> {
        todo!()
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

    #[test]
    #[serial]
    fn default_online_judge_returns_atcoder() {
        let config = ConfigImpl;
        assert_eq!(config.default_online_judge(), OJKind::AtCoder);
    }

    /// When config.toml contains `language = "rust"`, default_language() returns Ok(Language::new("rust")).
    #[test]
    #[serial]
    fn default_language_returns_rust_when_config_has_rust() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        std::fs::write(tmp.path().join("config.toml"), "language = \"rust\"\n")
            .expect("failed to write config.toml");
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let result = ConfigImpl.default_language();
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

        let result = ConfigImpl.default_language();
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

        let result = ConfigImpl.default_language();
        assert!(
            result.is_err(),
            "expected Err when config.toml is missing, got: {:?}",
            result,
        );
    }
}
