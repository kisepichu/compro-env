pub mod commands;

use anyhow::Result;
use clap::Parser;
use commands::{Cli, LoginCommand, LogoutCommand, WhoamiCommand};

use crate::{
    config_impl::ConfigImpl,
    online_judge_impl::atcoder::AtCoder,
    repository_impl::{
        contest_repository_impl::ContestRepositoryImpl,
        session_repository_impl::SessionRepositoryImpl,
        solution_repository_impl::SolutionRepositoryImpl,
    },
};
use interfaces::controller::Controller;
use usecases::config::Config as _;
use usecases::service::Service;

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        // These commands only touch ~/.config/ce/... and do not need a project root.
        commands::Commands::Login { oj, cookie } => {
            let oj_kind = match oj {
                Some(s) => s
                    .parse::<domain::entity::OJKind>()
                    .map_err(|e| anyhow::anyhow!(e))?,
                None => ConfigImpl.default_online_judge(),
            };

            let cookie = match cookie {
                Some(c) => c,
                None => {
                    println!("1. Open https://atcoder.jp in your browser and log in.");
                    println!("2. Open DevTools -> Application -> Cookies -> https://atcoder.jp");
                    println!("3. Copy the value of REVEL_SESSION.");
                    print!("REVEL_SESSION: ");
                    use std::io::Write as _;
                    std::io::stdout().flush()?;

                    let mut line = String::new();
                    std::io::stdin().read_line(&mut line)?;
                    line.trim().to_string()
                }
            };

            match login_with_io(oj_kind, &cookie) {
                Ok(()) => println!("Saved. Run `ce whoami` to verify."),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            Ok(())
        }
        commands::Commands::Whoami { oj } => {
            let oj_kind = match oj {
                Some(s) => s
                    .parse::<domain::entity::OJKind>()
                    .map_err(|e| anyhow::anyhow!(e))?,
                None => ConfigImpl.default_online_judge(),
            };

            match whoami_with_io(oj_kind) {
                Ok(Some(username)) => println!("{username}"),
                Ok(None) => {
                    println!("(not logged in)");
                    println!("Run `ce login` to save your session.");
                    return Ok(());
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            Ok(())
        }
        commands::Commands::Logout { oj } => {
            let oj_kind = match oj {
                Some(s) => s
                    .parse::<domain::entity::OJKind>()
                    .map_err(|e| anyhow::anyhow!(e))?,
                None => ConfigImpl.default_online_judge(),
            };

            match logout_with_io(oj_kind.clone()) {
                Ok(true) => println!("Logged out from {oj_kind}."),
                Ok(false) => println!("Already logged out."),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            Ok(())
        }
        // These commands require a project root.
        commands::Commands::Init { contest: _ } => {
            let _controller = build_controller()?;
            todo!()
        }
        commands::Commands::New {
            contest: _,
            problem: _,
            solution: _,
            lang: _,
        } => {
            let _controller = build_controller()?;
            todo!()
        }
        commands::Commands::Test {
            contest: _,
            problem: _,
            solution: _,
            lang: _,
        } => {
            let _controller = build_controller()?;
            todo!()
        }
        commands::Commands::Submit {
            contest: _,
            problem: _,
            solution: _,
            lang: _,
        } => {
            let _controller = build_controller()?;
            todo!()
        }
    }
}

/// Saves the login session for the given OJ using the provided cookie string.
///
/// This is the testable core of the Login command: the caller is responsible
/// for reading the cookie from stdin and validating that it is non-empty.
pub fn login_with_io(oj: domain::entity::OJKind, cookie: &str) -> Result<()> {
    let service = Service::new(
        Box::new(AtCoder),
        Box::new(ContestRepositoryImpl::new(std::path::PathBuf::new())),
        Box::new(SolutionRepositoryImpl::new(std::path::PathBuf::new())),
        Box::new(SessionRepositoryImpl),
        Box::new(ConfigImpl),
    );
    let controller = Controller::new(service);
    let input = LoginCommand {
        oj,
        cookie: cookie.to_string(),
    };
    controller.login(&input)
}

/// Returns the logged-in username for the given OJ, or `None` if no session is saved.
///
/// This is the testable core of the Whoami command.
pub fn whoami_with_io(oj: domain::entity::OJKind) -> Result<Option<String>> {
    let service = Service::new(
        Box::new(AtCoder),
        Box::new(ContestRepositoryImpl::new(std::path::PathBuf::new())),
        Box::new(SolutionRepositoryImpl::new(std::path::PathBuf::new())),
        Box::new(SessionRepositoryImpl),
        Box::new(ConfigImpl),
    );
    let controller = Controller::new(service);
    let input = WhoamiCommand { oj };
    match controller.whoami(&input) {
        Ok(username) => Ok(Some(username)),
        Err(e) => {
            if e.downcast_ref::<domain::error::CeError>()
                .map(|ce| matches!(ce, domain::error::CeError::SessionNotFound { .. }))
                .unwrap_or(false)
            {
                Ok(None)
            } else {
                Err(e)
            }
        }
    }
}

/// Removes the saved session for the given OJ. Returns `true` if a session was
/// removed, `false` if no session was found.
///
/// This is the testable core of the Logout command.
pub fn logout_with_io(oj: domain::entity::OJKind) -> Result<bool> {
    let service = Service::new(
        Box::new(AtCoder),
        Box::new(ContestRepositoryImpl::new(std::path::PathBuf::new())),
        Box::new(SolutionRepositoryImpl::new(std::path::PathBuf::new())),
        Box::new(SessionRepositoryImpl),
        Box::new(ConfigImpl),
    );
    let controller = Controller::new(service);
    let input = LogoutCommand { oj };
    controller.logout(&input)
}

fn build_controller() -> Result<Controller> {
    let root = find_project_root()?;

    let service = Service::new(
        Box::new(AtCoder),
        Box::new(ContestRepositoryImpl::new(root.clone())),
        Box::new(SolutionRepositoryImpl::new(root.clone())),
        Box::new(SessionRepositoryImpl),
        Box::new(ConfigImpl),
    );

    Ok(Controller::new(service))
}

/// Locates the project root by searching upward for the `templates/` directory.
///
/// `Cargo.toml` is not used as a sentinel because every Rust contest workspace and
/// solution package under `solutions/{contest_id}/rust/...` also contains a `Cargo.toml`,
/// which would resolve to the wrong root when running from a contest subdirectory.
fn find_project_root() -> Result<std::path::PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("templates").is_dir() {
            return Ok(dir);
        }
        if !dir.pop() {
            anyhow::bail!("could not find project root (no templates/ directory found)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::entity::OJKind;

    /// login_with_io saves the session cookie to session.toml in CE_CONFIG_DIR.
    #[test]
    fn login_saves_session_to_file() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        std::env::set_var("CE_CONFIG_DIR", tmp.path());

        login_with_io(OJKind::AtCoder, "my_cookie").expect("login_with_io should succeed");

        let session_toml = tmp.path().join("session.toml");
        assert!(
            session_toml.exists(),
            "session.toml should have been created"
        );

        let contents = std::fs::read_to_string(&session_toml).expect("failed to read session.toml");
        assert!(
            contents.contains("[atcoder]"),
            "expected [atcoder] section, got: {contents}"
        );
        assert!(
            contents.contains("revel_session = \"my_cookie\""),
            "expected revel_session entry, got: {contents}"
        );
    }

    /// login_with_io returns Err when the cookie is empty.
    #[test]
    fn login_returns_error_on_empty_cookie() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        std::env::set_var("CE_CONFIG_DIR", tmp.path());

        let result = login_with_io(OJKind::AtCoder, "");
        assert!(result.is_err(), "expected Err for empty cookie, got Ok");
    }

    /// logout_with_io returns Ok(true) and removes session.toml when a session exists.
    #[test]
    fn logout_returns_true_when_session_exists() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let session_toml = tmp.path().join("session.toml");

        // Write session.toml directly to avoid CE_CONFIG_DIR races with login_with_io.
        std::fs::write(&session_toml, "[atcoder]\nrevel_session = \"my_cookie\"\n")
            .expect("failed to write session.toml");

        std::env::set_var("CE_CONFIG_DIR", tmp.path());

        let result = logout_with_io(OJKind::AtCoder).expect("logout_with_io should return Ok");
        assert!(result, "expected true when a session was removed");
        assert!(!session_toml.exists(), "session.toml should be gone after logout");
    }

    /// logout_with_io returns Ok(false) when no session exists.
    #[test]
    fn logout_returns_false_when_session_missing() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        std::env::set_var("CE_CONFIG_DIR", tmp.path());

        assert!(
            !tmp.path().join("session.toml").exists(),
            "session.toml must not exist for this test"
        );

        let result = logout_with_io(OJKind::AtCoder).expect("logout_with_io should return Ok");
        assert!(!result, "expected false when no session was present");
    }

    /// whoami_with_io returns Ok(None) when no session.toml exists (not logged in).
    #[test]
    fn whoami_returns_none_when_session_missing() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        std::env::set_var("CE_CONFIG_DIR", tmp.path());

        // Confirm there is no session file in the temp dir.
        assert!(
            !tmp.path().join("session.toml").exists(),
            "session.toml must not exist for this test"
        );

        let result = whoami_with_io(OJKind::AtCoder);
        assert_eq!(
            result.expect("whoami_with_io should return Ok, not Err"),
            None,
            "expected None when no session is saved"
        );
    }
}
