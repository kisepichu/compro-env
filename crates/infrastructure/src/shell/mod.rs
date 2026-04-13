pub mod commands;

use anyhow::Result;
use clap::Parser;
use commands::{Cli, InitCommand, LoginCommand, LogoutCommand, WhoamiCommand};
use domain::entity::OJKind;

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
                Some(c) => c.trim().to_string(),
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
        commands::Commands::Init { contest, lang } => {
            match init_with_io(&contest, lang.as_deref()) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            Ok(())
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

/// Builds a `Controller` wired with all infrastructure implementations,
/// without requiring a project root (suitable for login/whoami/logout).
fn build_controller_no_root() -> Result<Controller> {
    let service = Service::new(
        Box::new(AtCoder::new()?),
        Box::new(ContestRepositoryImpl::new(std::path::PathBuf::new())),
        Box::new(SolutionRepositoryImpl::new(std::path::PathBuf::new())),
        Box::new(SessionRepositoryImpl),
        Box::new(ConfigImpl),
    );
    Ok(Controller::new(service))
}

/// This is the testable core of the Login command. Returns an error if `cookie`
/// is empty or whitespace-only. The value is trimmed before being persisted.
pub fn login_with_io(oj: domain::entity::OJKind, cookie: &str) -> Result<()> {
    let cookie = cookie.trim();
    if cookie.is_empty() {
        anyhow::bail!("cookie must not be empty");
    }
    let input = LoginCommand {
        oj,
        cookie: cookie.to_string(),
    };
    build_controller_no_root()?.login(&input)
}

/// Returns the logged-in username for the given OJ, or `None` if no session is saved.
///
/// This is the testable core of the Whoami command.
pub fn whoami_with_io(oj: domain::entity::OJKind) -> Result<Option<String>> {
    let input = WhoamiCommand { oj };
    match build_controller_no_root()?.whoami(&input) {
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
    let input = LogoutCommand { oj };
    build_controller_no_root()?.logout(&input)
}

/// Returns true if `s` is a single safe filesystem path component (no separators, no `.`/`..`).
fn is_safe_path_component(s: &str) -> bool {
    let mut components = std::path::Path::new(s).components();
    matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    )
}

/// Parses a contest input string (contest ID or URL) into an (OJKind, contest_id) pair.
///
/// Handles:
/// - AtCoder URL: "https://atcoder.jp/contests/{id}" → (AtCoder, id)
/// - Contest ID prefix (abc/arc/agc/ahc): "abc334" → (AtCoder, "abc334")
/// - Unknown input: None
fn parse_contest_input(input: &str) -> Option<(OJKind, String)> {
    const ATCODER_URL_PREFIX: &str = "https://atcoder.jp/contests/";
    if let Some(rest) = input.strip_prefix(ATCODER_URL_PREFIX) {
        // Take only the first path segment (ignore trailing slashes or extra paths)
        let contest_id = rest.trim_end_matches('/').split('/').next()?;
        if contest_id.is_empty() {
            return None;
        }
        let contest_id = contest_id.to_lowercase();
        if !is_safe_path_component(&contest_id) {
            return None;
        }
        return Some((OJKind::AtCoder, contest_id));
    }
    if let Some(oj) = OJKind::from_contest_id_prefix(input) {
        let contest_id = input.to_lowercase();
        if !is_safe_path_component(&contest_id) {
            return None;
        }
        return Some((oj, contest_id));
    }
    None
}

/// Resolves and validates the OJ, contest_id, and language for `ce init`.
///
/// Pure function: reads config and the filesystem under `root`, but performs no I/O prompts
/// and makes no network requests. Returns `Err` for unknown languages so the shell can report
/// it without invoking the controller.
#[cfg(test)]
fn resolve_init_args(
    contest_input: &str,
    lang_override: Option<&str>,
    root: &std::path::Path,
) -> Result<(OJKind, String, domain::entity::Language)> {
    let (oj, contest_id) = parse_contest_input(contest_input).ok_or_else(|| {
        anyhow::anyhow!(
            "cannot infer OJ from \"{contest_input}\". \
             Pass a full URL or a known prefix (abc, arc, agc, ahc)."
        )
    })?;

    let language = if let Some(lang) = lang_override {
        domain::entity::Language::new(lang)
    } else {
        ConfigImpl.default_language()?
    };

    validate_language(&language, root)?;

    Ok((oj, contest_id, language))
}

/// Prompts the user for a language and validates it against templates/.
fn prompt_language(root: &std::path::Path) -> Result<domain::entity::Language> {
    use std::io::Write as _;
    print!("Language (e.g. rust, cpp): ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let s = line.trim();
    let language = s
        .parse::<domain::entity::Language>()
        .map_err(|e| anyhow::anyhow!(e))?;
    validate_language(&language, root)?;
    Ok(language)
}

/// Validates that `language` has a matching templates/ directory under `root`.
fn validate_language(
    language: &domain::entity::Language,
    root: &std::path::Path,
) -> Result<()> {
    if !is_safe_path_component(language.as_str()) {
        anyhow::bail!(
            "invalid language \"{}\": must be a single path component",
            language.as_str()
        );
    }
    let tmpl_dir = root.join("templates").join(language.as_str());
    if !tmpl_dir.is_dir() {
        let available: Vec<String> = std::fs::read_dir(root.join("templates"))
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        anyhow::bail!(
            "unknown language \"{}\". Available: {}",
            language.as_str(),
            available.join(", ")
        );
    }
    Ok(())
}

/// Initializes a contest from a contest ID or URL string.
///
/// Parses the input, detects OJ kind and contest_id, reads the default language from config
/// (or uses `lang_override` when provided), validates it against templates/, and calls the
/// controller init.
pub fn init_with_io(contest_input: &str, lang_override: Option<&str>) -> Result<()> {
    use std::io::Write as _;
    let root = find_project_root()?;

    // Step 1: Resolve OJ and contest_id; prompt for OJ if unknown.
    let (oj, contest_id) = match parse_contest_input(contest_input) {
        Some(pair) => pair,
        None => {
            print!("OJ (e.g. atcoder): ");
            std::io::stdout().flush()?;
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            let oj_str = line.trim();
            let oj_str = if oj_str.is_empty() { "atcoder" } else { oj_str };
            let oj = oj_str
                .parse::<OJKind>()
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let contest_id = contest_input.to_lowercase();
            if !is_safe_path_component(&contest_id) {
                anyhow::bail!(
                    "invalid contest ID \"{contest_input}\": must be a single path component"
                );
            }
            (oj, contest_id)
        }
    };

    // Step 2: Resolve language; prompt if not overridden and not in config.
    let language = if let Some(lang) = lang_override {
        let language = lang
            .parse::<domain::entity::Language>()
            .map_err(|e| anyhow::anyhow!(e))?;
        validate_language(&language, &root)?;
        language
    } else {
        match ConfigImpl.default_language() {
            Ok(lang) => {
                validate_language(&lang, &root)?;
                lang
            }
            Err(_) => prompt_language(&root)?,
        }
    };

    let result = build_controller()?.init(
        &InitCommand {
            contest_id: contest_id.clone(),
            oj: oj.clone(),
            language: language.clone(),
        },
        &|msg| println!("{msg}"),
    )?;

    if result.already_initialized {
        println!("Contest {contest_id} is already initialized.");
        return Ok(());
    }

    let n_problems = result.created_solutions.len();
    let problem_codes: Vec<&str> = result
        .created_solutions
        .iter()
        .map(|s| s.problem_code.as_str())
        .collect();
    let first_code = problem_codes.first().copied().unwrap_or("");
    let last_code = problem_codes.last().copied().unwrap_or("");

    // Format OJ display name (capitalize first letter)
    let oj_display = match &result.oj_kind {
        OJKind::AtCoder => "AtCoder",
    };

    println!(
        "Initialized {contest_id} ({oj_display}) — {n_problems} problems: {}",
        problem_codes.join(" ")
    );
    println!("  testcases   {} files", result.total_sample_files);
    println!(
        "  {lang}      {n_problems} solutions ({first_code}/main … {last_code}/main)",
        lang = language
    );

    Ok(())
}

fn build_controller() -> Result<Controller> {
    let root = find_project_root()?;

    let service = Service::new(
        Box::new(AtCoder::new()?),
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
    use serial_test::serial;

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

    /// login_with_io saves the session cookie to session.toml in CE_CONFIG_DIR.
    #[test]
    #[serial]
    fn login_saves_session_to_file() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

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
    #[serial]
    fn login_returns_error_on_empty_cookie() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let result = login_with_io(OJKind::AtCoder, "");
        assert!(result.is_err(), "expected Err for empty cookie, got Ok");
    }

    /// logout_with_io returns Ok(true) and removes session.toml when a session exists.
    #[test]
    #[serial]
    fn logout_returns_true_when_session_exists() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let session_toml = tmp.path().join("session.toml");

        // Write session.toml directly to avoid CE_CONFIG_DIR races with login_with_io.
        std::fs::write(&session_toml, "[atcoder]\nrevel_session = \"my_cookie\"\n")
            .expect("failed to write session.toml");

        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        let result = logout_with_io(OJKind::AtCoder).expect("logout_with_io should return Ok");
        assert!(result, "expected true when a session was removed");
        assert!(
            !session_toml.exists(),
            "session.toml should be gone after logout"
        );
    }

    /// logout_with_io returns Ok(false) when no session exists.
    #[test]
    #[serial]
    fn logout_returns_false_when_session_missing() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

        assert!(
            !tmp.path().join("session.toml").exists(),
            "session.toml must not exist for this test"
        );

        let result = logout_with_io(OJKind::AtCoder).expect("logout_with_io should return Ok");
        assert!(!result, "expected false when no session was present");
    }

    /// parse_contest_input correctly identifies an AtCoder contest ID by prefix.
    #[test]
    fn parse_contest_input_handles_atcoder_id() {
        let result = parse_contest_input("abc334");
        assert_eq!(result, Some((OJKind::AtCoder, "abc334".to_string())));
    }

    /// parse_contest_input correctly parses an AtCoder contest URL.
    #[test]
    fn parse_contest_input_handles_atcoder_url() {
        let result = parse_contest_input("https://atcoder.jp/contests/abc334");
        assert_eq!(result, Some((OJKind::AtCoder, "abc334".to_string())));
    }

    /// parse_contest_input returns None for an unknown contest ID.
    #[test]
    fn parse_contest_input_returns_none_for_unknown() {
        let result = parse_contest_input("xyz123");
        assert_eq!(result, None);
    }

    /// resolve_init_args accepts a valid lang_override and returns Ok without network I/O.
    #[test]
    fn resolve_init_args_accepts_valid_language() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");

        // Create templates/rust/ so "rust" is a recognised language.
        std::fs::create_dir_all(tmp.path().join("templates").join("rust"))
            .expect("failed to create templates/rust");

        let result = resolve_init_args("abc001", Some("rust"), tmp.path());

        assert!(result.is_ok(), "expected Ok for valid language, got: {:?}", result);
        let (oj, contest_id, lang) = result.unwrap();
        assert_eq!(oj, OJKind::AtCoder);
        assert_eq!(contest_id, "abc001");
        assert_eq!(lang.as_str(), "rust");
    }

    /// resolve_init_args rejects a lang_override that has no matching templates/ directory.
    #[test]
    fn resolve_init_args_rejects_unknown_language() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");

        // Only templates/rust/ exists — "cpp" is absent.
        std::fs::create_dir_all(tmp.path().join("templates").join("rust"))
            .expect("failed to create templates/rust");

        let result = resolve_init_args("abc001", Some("cpp"), tmp.path());

        assert!(result.is_err(), "expected Err for unknown language, got Ok");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("unknown language"),
            "expected 'unknown language' in error, got: {msg}"
        );
        assert!(
            msg.contains("cpp"),
            "expected 'cpp' mentioned in error, got: {msg}"
        );
    }

    /// whoami_with_io returns Ok(None) when no session.toml exists (not logged in).
    #[test]
    #[serial]
    fn whoami_returns_none_when_session_missing() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let _guard = EnvVarGuard::set("CE_CONFIG_DIR", tmp.path());

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
