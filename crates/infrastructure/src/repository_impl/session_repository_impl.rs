use anyhow::Result;
use domain::entity::{OJKind, Session};
use std::path::{Path, PathBuf};
use toml;
use usecases::repository::session_repository::SessionRepository;

pub struct SessionRepositoryImpl;

impl SessionRepositoryImpl {
    /// Returns the config directory path.
    /// Uses the `CE_CONFIG_DIR` environment variable if set; otherwise `~/.config/ce/`.
    fn config_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("CE_CONFIG_DIR") {
            PathBuf::from(dir)
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/root"));
            PathBuf::from(home).join(".config").join("ce")
        }
    }

    fn session_toml_path() -> PathBuf {
        Self::config_dir().join("session.toml")
    }
}

impl SessionRepository for SessionRepositoryImpl {
    fn get(&self, oj: &OJKind) -> Result<Option<Session>> {
        let path = Self::session_toml_path();
        get_session_from_path(oj, &path)
    }

    fn save(&self, session: &Session) -> Result<()> {
        let path = Self::session_toml_path();
        save_session_to_path(session, &path)
    }
}

/// Serializes a `Session` to TOML and writes it to `path`.
/// Parent directories are created if they don't exist.
fn save_session_to_path(session: &Session, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let toml_content = match &session.online_judge {
        OJKind::AtCoder => {
            format!("[atcoder]\nrevel_session = {:?}\n", session.cookie)
        }
    };

    std::fs::write(path, toml_content)?;
    Ok(())
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
    use std::fs;

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
    fn get_returns_session_when_file_exists() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let config_path = tmp.path().join("session.toml");

        // Write a valid session.toml manually.
        fs::write(
            &config_path,
            "[atcoder]\nrevel_session = \"my_cookie_value\"\n",
        )
        .expect("failed to write session.toml");

        std::env::set_var("CE_CONFIG_DIR", tmp.path());

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
    fn get_returns_none_when_file_missing() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");

        std::env::set_var("CE_CONFIG_DIR", tmp.path());

        let repo = SessionRepositoryImpl;
        let result = repo
            .get(&OJKind::AtCoder)
            .expect("get should not return Err");

        assert_eq!(result, None, "expected None when session.toml is absent");
    }

    /// After save(), ~/.config/ce/session.toml must contain the expected TOML content.
    ///
    /// Uses CE_CONFIG_DIR env override so that the file is written to a temp directory
    /// instead of the real ~/.config/ce/.
    #[test]
    fn save_writes_revel_session_to_toml() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let config_path = tmp.path().join("session.toml");

        // Point the impl at the temp directory via environment variable.
        std::env::set_var("CE_CONFIG_DIR", tmp.path());

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
