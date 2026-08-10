//! Integration tests for `ce check` (spec §7.1).
//!
//! Each test builds a `LibraryProjectConfig` in a temp repository root and
//! invokes `run_checks` against the real `UnixCommandRunner`. Because the
//! runner inherits the parent's stdio, tests observe behaviour through files
//! the check scripts write, not by capturing stdout/stderr.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use domain::library::{
    AnalyzerConfig, DEFAULT_TIMEOUT_SECONDS, LanguageConfig, LanguageId, LibraryProjectConfig,
    SiteConfig,
};
use infrastructure::command_runner_impl::UnixCommandRunner;
use infrastructure::library_project::config::ProjectLibraryConfigLoader;
use usecases::check::{CheckSelection, LanguageCheckStatus, run_checks};

/// Build a `LanguageConfig` with the supplied fields; other fields get the
/// same defaults the strict loader would assign.
fn language(
    id: &str,
    root: &str,
    check_command: Option<&str>,
    check_timeout_seconds: Option<u32>,
) -> (LanguageId, LanguageConfig) {
    let lid = LanguageId::parse(id).unwrap();
    let cfg = LanguageConfig {
        id: lid.clone(),
        display_name: None,
        root: root.to_string(),
        include: vec!["**/*".into()],
        exclude: vec![],
        check_command: check_command.map(str::to_string),
        check_timeout_seconds: check_timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
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

fn config_with(langs: Vec<(LanguageId, LanguageConfig)>) -> LibraryProjectConfig {
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

/// Ensure the language root exists inside the repo root before the runner
/// changes into it — otherwise the child's `chdir` fails, which is a test
/// setup problem, not a `run_checks` problem.
fn make_language_root(repo_root: &Path, relative: &str) -> PathBuf {
    let root = repo_root.join(relative);
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_marker(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

/// 1. Languages run in ascending `LanguageId` byte order.
#[test]
fn languages_run_in_byte_order() {
    let repo = tempfile::tempdir().unwrap();
    for lang in ["cpp", "lean", "rust"] {
        make_language_root(repo.path(), &format!("libraries/{lang}"));
    }
    let log = write_marker(&repo, "order.log");

    let langs = vec![
        language(
            "rust",
            "libraries/rust",
            Some(&format!("echo rust >> '{}'", log.display())),
            None,
        ),
        language(
            "cpp",
            "libraries/cpp",
            Some(&format!("echo cpp >> '{}'", log.display())),
            None,
        ),
        language(
            "lean",
            "libraries/lean",
            Some(&format!("echo lean >> '{}'", log.display())),
            None,
        ),
    ];
    let config = config_with(langs);

    let summary = run_checks(
        &config,
        &CheckSelection::All,
        &UnixCommandRunner,
        repo.path(),
    )
    .expect("run_checks should succeed");

    assert!(summary.aggregate_success());
    let recorded = fs::read_to_string(&log).unwrap();
    assert_eq!(recorded, "cpp\nlean\nrust\n");

    let statuses: Vec<_> = summary
        .results
        .iter()
        .map(|r| (r.language.as_str().to_string(), r.status.clone()))
        .collect();
    assert_eq!(
        statuses,
        vec![
            ("cpp".to_string(), LanguageCheckStatus::Passed),
            ("lean".to_string(), LanguageCheckStatus::Passed),
            ("rust".to_string(), LanguageCheckStatus::Passed),
        ]
    );
}

/// 2. `--language cpp` runs only cpp.
#[test]
fn language_filter_runs_only_target() {
    let repo = tempfile::tempdir().unwrap();
    for lang in ["cpp", "rust"] {
        make_language_root(repo.path(), &format!("libraries/{lang}"));
    }
    let log = write_marker(&repo, "filter.log");

    let langs = vec![
        language(
            "rust",
            "libraries/rust",
            Some(&format!("echo rust >> '{}'", log.display())),
            None,
        ),
        language(
            "cpp",
            "libraries/cpp",
            Some(&format!("echo cpp >> '{}'", log.display())),
            None,
        ),
    ];
    let config = config_with(langs);

    let summary = run_checks(
        &config,
        &CheckSelection::Language(LanguageId::parse("cpp").unwrap()),
        &UnixCommandRunner,
        repo.path(),
    )
    .unwrap();

    assert_eq!(summary.results.len(), 1);
    assert_eq!(summary.results[0].language.as_str(), "cpp");
    assert_eq!(summary.results[0].status, LanguageCheckStatus::Passed);
    let recorded = fs::read_to_string(&log).unwrap();
    assert_eq!(recorded, "cpp\n");
}

/// 3. A language without `check_command` reports `Skipped` and writes nothing.
#[test]
fn missing_check_command_is_skipped() {
    let repo = tempfile::tempdir().unwrap();
    make_language_root(repo.path(), "libraries/rust");

    let langs = vec![language("rust", "libraries/rust", None, None)];
    let config = config_with(langs);

    let summary = run_checks(
        &config,
        &CheckSelection::All,
        &UnixCommandRunner,
        repo.path(),
    )
    .unwrap();

    assert!(summary.aggregate_success());
    assert_eq!(summary.results.len(), 1);
    assert_eq!(summary.results[0].status, LanguageCheckStatus::Skipped);
}

/// 4. Aggregate failure: one passes (exit 0), the next fails (exit 3); both
///    ran in byte order and `aggregate_success` is false.
#[test]
fn aggregate_failure_records_all_results() {
    let repo = tempfile::tempdir().unwrap();
    for lang in ["cpp", "rust"] {
        make_language_root(repo.path(), &format!("libraries/{lang}"));
    }

    let langs = vec![
        language("cpp", "libraries/cpp", Some("exit 0"), None),
        language("rust", "libraries/rust", Some("exit 3"), None),
    ];
    let config = config_with(langs);

    let summary = run_checks(
        &config,
        &CheckSelection::All,
        &UnixCommandRunner,
        repo.path(),
    )
    .unwrap();

    assert!(!summary.aggregate_success());
    assert_eq!(summary.results.len(), 2);
    assert_eq!(summary.results[0].language.as_str(), "cpp");
    assert_eq!(summary.results[0].status, LanguageCheckStatus::Passed);
    assert_eq!(summary.results[1].language.as_str(), "rust");
    assert_eq!(
        summary.results[1].status,
        LanguageCheckStatus::Failed { exit_code: 3 }
    );
}

/// 5. Continued execution: three languages where the middle one fails; assert
///    all three ran (each writes to its own marker file).
#[test]
fn continues_after_middle_failure() {
    let repo = tempfile::tempdir().unwrap();
    for lang in ["a", "b", "c"] {
        make_language_root(repo.path(), &format!("libraries/{lang}"));
    }
    let marker_a = write_marker(&repo, "a.mark");
    let marker_b = write_marker(&repo, "b.mark");
    let marker_c = write_marker(&repo, "c.mark");

    let langs = vec![
        language(
            "a",
            "libraries/a",
            Some(&format!("touch '{}'; exit 0", marker_a.display())),
            None,
        ),
        language(
            "b",
            "libraries/b",
            Some(&format!("touch '{}'; exit 5", marker_b.display())),
            None,
        ),
        language(
            "c",
            "libraries/c",
            Some(&format!("touch '{}'; exit 0", marker_c.display())),
            None,
        ),
    ];
    let config = config_with(langs);

    let summary = run_checks(
        &config,
        &CheckSelection::All,
        &UnixCommandRunner,
        repo.path(),
    )
    .unwrap();

    assert!(marker_a.exists(), "language a must have run");
    assert!(marker_b.exists(), "language b must have run");
    assert!(marker_c.exists(), "language c must have run");
    assert_eq!(summary.results.len(), 3);
    assert_eq!(
        summary.results[1].status,
        LanguageCheckStatus::Failed { exit_code: 5 }
    );
    assert!(!summary.aggregate_success());
}

/// 6. `check_timeout_seconds = 1` on `sleep 5` maps to `TimedOut`; elapsed
///    stays under three seconds because SIGTERM kills `sleep` immediately.
#[test]
fn configured_timeout_produces_timed_out_status() {
    let repo = tempfile::tempdir().unwrap();
    make_language_root(repo.path(), "libraries/rust");

    let langs = vec![language("rust", "libraries/rust", Some("sleep 5"), Some(1))];
    let config = config_with(langs);

    let start = Instant::now();
    let summary = run_checks(
        &config,
        &CheckSelection::All,
        &UnixCommandRunner,
        repo.path(),
    )
    .unwrap();
    let elapsed = start.elapsed();

    assert!(!summary.aggregate_success());
    assert_eq!(summary.results.len(), 1);
    assert_eq!(summary.results[0].status, LanguageCheckStatus::TimedOut);
    assert!(
        elapsed < Duration::from_secs(3),
        "expected quick SIGTERM cleanup, took {elapsed:?}"
    );
}

/// 7. Default `check_timeout_seconds` is 600 when the config omits it.
///
/// The loader's own unit tests exercise this against a fixture; this test
/// pins it against the domain `LanguageConfig` type via the loader, which is
/// what `run_checks` ultimately reads.
#[test]
fn default_check_timeout_seconds_is_600_via_loader() {
    let repo = tempfile::tempdir().unwrap();
    let config_toml = r#"
[library.site]
title = "t"
description = "d"
language = "en"
repository_url = "https://example.com"

[library.languages.rust]
root = "libraries/rust"
include = ["**/*.rs"]
check_command = "true"

[library.languages.rust.analyzer]
command = ["./bin/rust"]
"#;
    fs::write(repo.path().join("config.toml"), config_toml).unwrap();
    let config = ProjectLibraryConfigLoader::load(repo.path()).unwrap();
    let rust = &config.languages[&LanguageId::parse("rust").unwrap()];
    assert_eq!(rust.check_timeout_seconds, 600);
}

/// 8. Exported CE_* env vars appear in the check environment with the
///    expected values.
#[test]
fn ce_environment_variables_are_exported() {
    let repo = tempfile::tempdir().unwrap();
    let language_root = make_language_root(repo.path(), "libraries/rust");
    let out = write_marker(&repo, "env.log");

    let script = format!(
        "printf '%s|%s|%s' \"$CE_REPOSITORY_ROOT\" \"$CE_LIBRARY_ROOT\" \"$CE_LANGUAGE\" > '{}'",
        out.display()
    );
    let langs = vec![language("rust", "libraries/rust", Some(&script), None)];
    let config = config_with(langs);

    let summary = run_checks(
        &config,
        &CheckSelection::All,
        &UnixCommandRunner,
        repo.path(),
    )
    .unwrap();
    assert!(summary.aggregate_success());

    let recorded = fs::read_to_string(&out).unwrap();
    let expected = format!("{}|{}|rust", repo.path().display(), language_root.display());
    assert_eq!(recorded, expected);
}

/// 9. `run_checks` never triggers a solution's `test_command`. Place a
///    solution under `solutions/<contest>/<problem>/main/` whose `ce.toml`
///    would write to a marker if executed, then confirm the marker is absent.
#[test]
fn does_not_run_solution_test_command() {
    let repo = tempfile::tempdir().unwrap();
    make_language_root(repo.path(), "libraries/rust");

    let solution_dir = repo.path().join("solutions/abc001/a/main");
    fs::create_dir_all(&solution_dir).unwrap();
    let solution_marker = repo.path().join("solution.mark");
    fs::write(
        solution_dir.join("ce.toml"),
        format!(
            "language = \"rust\"\ntest_command = \"touch '{}'\"\n",
            solution_marker.display()
        ),
    )
    .unwrap();

    let check_marker = write_marker(&repo, "check.mark");
    let langs = vec![language(
        "rust",
        "libraries/rust",
        Some(&format!("touch '{}'", check_marker.display())),
        None,
    )];
    let config = config_with(langs);

    run_checks(
        &config,
        &CheckSelection::All,
        &UnixCommandRunner,
        repo.path(),
    )
    .unwrap();

    assert!(check_marker.exists(), "language check should have run");
    assert!(
        !solution_marker.exists(),
        "run_checks must not execute solution test_command"
    );
}

/// 10. `All` selection continues after both a skip and a failure.
#[test]
fn all_continues_past_skip_and_failure() {
    let repo = tempfile::tempdir().unwrap();
    for lang in ["a", "b", "c"] {
        make_language_root(repo.path(), &format!("libraries/{lang}"));
    }

    let langs = vec![
        language("a", "libraries/a", None, None), // Skipped
        language("b", "libraries/b", Some("exit 7"), None),
        language("c", "libraries/c", Some("exit 0"), None),
    ];
    let config = config_with(langs);

    let summary = run_checks(
        &config,
        &CheckSelection::All,
        &UnixCommandRunner,
        repo.path(),
    )
    .unwrap();

    assert_eq!(summary.results.len(), 3);
    assert_eq!(summary.results[0].status, LanguageCheckStatus::Skipped);
    assert_eq!(
        summary.results[1].status,
        LanguageCheckStatus::Failed { exit_code: 7 }
    );
    assert_eq!(summary.results[2].status, LanguageCheckStatus::Passed);
    assert!(!summary.aggregate_success());
}
