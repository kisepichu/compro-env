use anyhow::Result;
use domain::entity::{OJKind, Session};
use std::path::{Path, PathBuf};
use toml;
use usecases::repository::session_repository::SessionRepository;

pub struct SessionRepositoryImpl;

impl SessionRepositoryImpl {
    /// Returns the config directory path.
    /// Uses the `CE_CONFIG_DIR` environment variable if set to a non-empty, non-whitespace value;
    /// otherwise falls back to `~/.config/ce/`.
    /// Returns an error if neither `CE_CONFIG_DIR` nor `HOME` is set.
    fn config_dir() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("CE_CONFIG_DIR") {
            if !dir.trim().is_empty() {
                return Ok(PathBuf::from(dir));
            }
        }
        let home = std::env::var("HOME").map_err(|_| {
            anyhow::anyhow!(
                "HOME environment variable is not set; cannot determine config directory"
            )
        })?;
        Ok(PathBuf::from(home).join(".config").join("ce"))
    }

    fn session_toml_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("session.toml"))
    }
}

impl SessionRepository for SessionRepositoryImpl {
    fn get(&self, oj: &OJKind) -> Result<Option<Session>> {
        let path = Self::session_toml_path()?;
        get_session_from_path(oj, &path)
    }

    fn save(&self, session: &Session) -> Result<()> {
        let path = Self::session_toml_path()?;
        save_session_to_path(session, &path)
    }

    fn delete(&self, oj: &OJKind) -> Result<bool> {
        let path = Self::session_toml_path()?;
        delete_session_from_path(oj, &path)
    }
}

/// Serializes a `Session` to TOML and writes it to `path`.
/// Reads the existing file first and updates only the relevant OJ section,
/// preserving sessions for other OJs. Parent directories are created if needed.
fn save_session_to_path(session: &Session, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut table: toml::Table = if path.exists() {
        let contents = std::fs::read_to_string(path)?;
        toml::from_str(&contents)?
    } else {
        toml::Table::new()
    };

    let section_key = match &session.online_judge {
        OJKind::AtCoder => "atcoder",
    };

    let mut section = toml::Table::new();
    section.insert(
        "revel_session".to_string(),
        toml::Value::String(session.cookie.clone()),
    );
    table.insert(section_key.to_string(), toml::Value::Table(section));

    std::fs::write(path, toml::to_string(&table)?)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

/// Removes the OJ's section from the TOML file at `path`.
/// Returns `Ok(true)` if the section was present, `Ok(false)` if the file didn't exist
/// or if the target OJ section was not present in the file.
/// If the file becomes empty (no sections remain), it is deleted entirely.
fn delete_session_from_path(oj: &OJKind, path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }

    let contents = std::fs::read_to_string(path)?;
    let mut table: toml::Table = toml::from_str(&contents)?;

    let section_key = match oj {
        OJKind::AtCoder => "atcoder",
    };

    if table.remove(section_key).is_none() {
        return Ok(false);
    }

    if table.is_empty() {
        std::fs::remove_file(path)?;
    } else {
        std::fs::write(path, toml::to_string(&table)?)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }

    Ok(true)
}

/// Reads a `Session` for the given OJ from a TOML file at `path`.
/// Returns `Ok(None)` if the file does not exist.
fn get_session_from_path(oj: &OJKind, path: &Path) -> Result<Option<Session>> {
    if !path.exists() {
        return Ok(None);
    }

    let contents = std::fs::read_to_string(path)?;
    let table: toml::Table = toml::from_str(&contents)?;

    let cookie = match oj {
        OJKind::AtCoder => table
            .get("atcoder")
            .and_then(|v| v.get("revel_session"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    };

    Ok(cookie.map(|c| Session {
        online_judge: oj.clone(),
        cookie: c,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::fs;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    /// Helper: create a Session for AtCoder with the given cookie value.
    fn atcoder_session(cookie: &str) -> Session {
        Session {
            online_judge: OJKind::AtCoder,
            cookie: cookie.to_string(),
        }
    }

    /// get() returns Some(Session) when session.toml exists with a valid [atcoder] section.
    ///
    /// Uses CE_CONFIG_DIR env override so that the file is read from a temp directory
    /// instead of the real ~/.config/ce/.
    #[test]
    #[serial]
    fn get_returns_session_when_file_exists() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let config_path = tmp.path().join("session.toml");

        // Write a valid session.toml manually.
        fs::write(
            &config_path,
            "[atcoder]\nrevel_session = \"my_cookie_value\"\n",
        )
        .expect("failed to write session.toml");

        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let repo = SessionRepositoryImpl;
        let result = repo
            .get(&OJKind::AtCoder)
            .expect("get should not return Err");

        assert_eq!(
            result,
            Some(Session {
                online_judge: OJKind::AtCoder,
                cookie: "my_cookie_value".to_string(),
            }),
            "expected Some(Session {{ ... }}) with correct cookie"
        );
    }

    /// get() returns None when session.toml does not exist.
    ///
    /// Uses CE_CONFIG_DIR env override pointing at an empty temp directory.
    #[test]
    #[serial]
    fn get_returns_none_when_file_missing() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let repo = SessionRepositoryImpl;
        let result = repo
            .get(&OJKind::AtCoder)
            .expect("get should not return Err");

        assert_eq!(result, None, "expected None when session.toml is absent");
    }

    /// delete() returns Ok(true) when session.toml exists, and the file is removed afterwards.
    ///
    /// Uses CE_CONFIG_DIR env override so that the file is read from a temp directory.
    #[test]
    #[serial]
    fn delete_returns_true_when_session_exists() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let config_path = tmp.path().join("session.toml");

        fs::write(&config_path, "[atcoder]\nrevel_session = \"some_cookie\"\n")
            .expect("failed to write session.toml");

        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let repo = SessionRepositoryImpl;
        let result = repo
            .delete(&OJKind::AtCoder)
            .expect("delete should not return Err");

        assert!(result, "expected Ok(true) when session existed");
        assert!(
            !config_path.exists(),
            "session.toml should have been removed after delete"
        );
    }

    /// delete() returns Ok(false) when session.toml does not exist.
    ///
    /// Uses CE_CONFIG_DIR env override pointing at an empty temp directory.
    #[test]
    #[serial]
    fn delete_returns_false_when_session_missing() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let repo = SessionRepositoryImpl;
        let result = repo
            .delete(&OJKind::AtCoder)
            .expect("delete should not return Err");

        assert!(!result, "expected Ok(false) when no session existed");
    }

    /// After save(), ~/.config/ce/session.toml must contain the expected TOML content.
    ///
    /// Uses CE_CONFIG_DIR env override so that the file is written to a temp directory
    /// instead of the real ~/.config/ce/.
    #[test]
    #[serial]
    fn save_writes_revel_session_to_toml() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let config_path = tmp.path().join("session.toml");

        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let repo = SessionRepositoryImpl;
        let session = atcoder_session("test_revel_session_value");

        repo.save(&session).expect("save should succeed");

        let contents =
            fs::read_to_string(&config_path).expect("session.toml should have been written");

        assert!(
            contents.contains("[atcoder]"),
            "expected [atcoder] section, got: {contents}"
        );
        assert!(
            contents.contains("revel_session = \"test_revel_session_value\""),
            "expected revel_session entry, got: {contents}"
        );
    }
}
