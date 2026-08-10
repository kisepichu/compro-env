//! `ce check`: run each language's `check_command` and aggregate the outcome.
//!
//! Semantics come from spec §7.1:
//!
//! * Iterate languages in ascending `LanguageId` byte order
//!   (`BTreeMap` already yields that order).
//! * Missing `check_command` is `Skipped`, not a failure.
//! * Continue after failure or timeout; return the aggregated summary.
//! * `check_timeout_seconds` (default 600) is enforced by the injected
//!   [`CommandRunner`]; a timeout is reported as `TimedOut` and counts as a
//!   check failure (`aggregate_success` returns `false`).
//! * Each command runs via `sh -c` from the language root, with a sanitized
//!   environment that exports `CE_REPOSITORY_ROOT`, `CE_LIBRARY_ROOT`, and
//!   `CE_LANGUAGE` and forwards the shell essentials (`PATH`, `HOME`, `TERM`).
//! * `run_checks` never touches solution `test_command` — that stays with
//!   `Service::test`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use anyhow::{Result, anyhow};
use domain::library::{LanguageId, LibraryProjectConfig};

use crate::command_runner::{CommandRequest, CommandRunner};

/// Which languages `run_checks` should exercise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckSelection {
    /// Every language declared under `[library.languages]`.
    All,
    /// Only the given language id (used by `ce check --language <id>`).
    Language(LanguageId),
}

/// Terminal status of a single language's check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageCheckStatus {
    Passed,
    Failed { exit_code: i32 },
    TimedOut,
    Skipped,
}

/// Outcome for one language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageCheckResult {
    pub language: LanguageId,
    pub status: LanguageCheckStatus,
}

/// Aggregate result across all languages in a run.
///
/// Results are stored in the order they ran (ascending `LanguageId` byte order,
/// matching spec §7.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckSummary {
    pub results: Vec<LanguageCheckResult>,
}

impl CheckSummary {
    /// `true` when every language passed or was skipped. Timeout and non-zero
    /// exits both count as failures per spec §7.1.
    pub fn aggregate_success(&self) -> bool {
        self.results.iter().all(|r| {
            matches!(
                r.status,
                LanguageCheckStatus::Passed | LanguageCheckStatus::Skipped
            )
        })
    }
}

/// Run the selected languages' `check_command` and return the aggregated
/// [`CheckSummary`]. Does not bail early on failure or timeout.
///
/// * `config` — validated project-local library configuration.
/// * `selection` — `All` or a single language id. An unknown id in
///   `Language(_)` is a caller error.
/// * `runner` — where the actual shell invocation happens (see
///   [`CommandRunner`]).
/// * `repository_root` — used to resolve each language's relative `root` into
///   the child's working directory and to populate `CE_REPOSITORY_ROOT`.
///
/// Per-language status is streamed to stderr as each language finishes so long
/// runs give operators feedback even if the aggregate summary is printed later.
pub fn run_checks(
    config: &LibraryProjectConfig,
    selection: &CheckSelection,
    runner: &dyn CommandRunner,
    repository_root: &Path,
) -> Result<CheckSummary> {
    if let CheckSelection::Language(id) = selection
        && !config.languages.contains_key(id)
    {
        return Err(anyhow!(
            "language `{id}` is not configured under [library.languages]"
        ));
    }

    let mut results = Vec::new();

    for (language_id, language) in &config.languages {
        if let CheckSelection::Language(id) = selection
            && language_id != id
        {
            continue;
        }

        let status = match &language.check_command {
            None => {
                eprintln!("[{language_id}] skipped (no check_command configured)");
                LanguageCheckStatus::Skipped
            }
            Some(command) => {
                let working_dir = repository_root.join(&language.root);
                let environment =
                    build_environment(repository_root, &working_dir, language_id.as_str());

                let request = CommandRequest {
                    program: OsString::from("sh"),
                    arguments: vec![OsString::from("-c"), OsString::from(command)],
                    current_dir: working_dir,
                    environment,
                    timeout: Duration::from_secs(u64::from(language.check_timeout_seconds)),
                };

                let outcome = runner.run_streaming(&request)?;
                if outcome.timed_out {
                    eprintln!(
                        "[{language_id}] timed out after {}s",
                        language.check_timeout_seconds
                    );
                    LanguageCheckStatus::TimedOut
                } else {
                    match outcome.exit_code {
                        Some(0) => {
                            eprintln!("[{language_id}] passed");
                            LanguageCheckStatus::Passed
                        }
                        // The runner reports a killed-by-signal child as
                        // `exit_code = None`. Treat that as a failure so
                        // aggregate_success reflects reality.
                        Some(code) => {
                            eprintln!("[{language_id}] failed (exit {code})");
                            LanguageCheckStatus::Failed { exit_code: code }
                        }
                        None => {
                            eprintln!("[{language_id}] failed (killed by signal)");
                            LanguageCheckStatus::Failed { exit_code: -1 }
                        }
                    }
                }
            }
        };

        results.push(LanguageCheckResult {
            language: language_id.clone(),
            status,
        });
    }

    Ok(CheckSummary { results })
}

/// Build the sanitized environment given to a check command. `PATH`, `HOME`,
/// and `TERM` are forwarded from the parent so `sh` and standard utilities can
/// still be resolved; nothing else leaks in.
fn build_environment(
    repository_root: &Path,
    library_root: &Path,
    language_id: &str,
) -> BTreeMap<OsString, OsString> {
    let mut env = BTreeMap::new();
    env.insert(
        OsString::from("CE_REPOSITORY_ROOT"),
        OsString::from(repository_root.as_os_str()),
    );
    env.insert(
        OsString::from("CE_LIBRARY_ROOT"),
        OsString::from(library_root.as_os_str()),
    );
    env.insert(OsString::from("CE_LANGUAGE"), OsString::from(language_id));
    for key in ["PATH", "HOME", "TERM"] {
        if let Some(value) = std::env::var_os(key) {
            env.insert(OsString::from(key), value);
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_runner::CommandOutcome;
    use domain::library::{AnalyzerConfig, DEFAULT_TIMEOUT_SECONDS, LanguageConfig, SiteConfig};
    use std::cell::RefCell;

    struct RecordingRunner {
        calls: RefCell<Vec<CommandRequest>>,
        outcome: CommandOutcome,
    }

    impl RecordingRunner {
        fn passed() -> Self {
            Self {
                calls: RefCell::new(vec![]),
                outcome: CommandOutcome {
                    exit_code: Some(0),
                    timed_out: false,
                },
            }
        }
    }

    impl CommandRunner for RecordingRunner {
        fn run_streaming(&self, request: &CommandRequest) -> Result<CommandOutcome> {
            self.calls.borrow_mut().push(request.clone());
            Ok(self.outcome.clone())
        }
    }

    fn language(id: &str, root: &str, check: Option<&str>) -> (LanguageId, LanguageConfig) {
        let lid = LanguageId::parse(id).unwrap();
        let cfg = LanguageConfig {
            id: lid.clone(),
            display_name: None,
            root: root.to_string(),
            include: vec!["**/*".into()],
            exclude: vec![],
            check_command: check.map(|s| s.to_string()),
            check_timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            syntax_highlight: None,
            analyzer: AnalyzerConfig {
                command: vec!["./adapter".into()],
                timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            },
            expected_toolchains: vec![],
            online_judges: BTreeMap::new(),
            entry_file: "src/main.rs".into(),
        };
        (lid, cfg)
    }

    fn base_config(langs: Vec<(LanguageId, LanguageConfig)>) -> LibraryProjectConfig {
        LibraryProjectConfig {
            languages: langs.into_iter().collect(),
            site: Some(SiteConfig {
                title: "t".into(),
                description: "d".into(),
                language: "en".into(),
                repository_url: "https://example.com".into(),
            }),
        }
    }

    /// `Language(id)` with an unknown id fails fast before running anything.
    #[test]
    fn unknown_language_id_fails_before_running() {
        let config = base_config(vec![language("rust", "libraries/rust", Some("true"))]);
        let runner = RecordingRunner::passed();
        let err = run_checks(
            &config,
            &CheckSelection::Language(LanguageId::parse("cpp").unwrap()),
            &runner,
            Path::new("/tmp"),
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("cpp"),
            "unexpected error: {err}"
        );
        assert!(runner.calls.borrow().is_empty(), "no command should run");
    }
}
