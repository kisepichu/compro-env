//! Handshake fixture test for the ce-rust adapter (plan 043 Task 1).
//!
//! Builds the `rust-analyzer` binary in this workspace, then runs it through
//! `ProcessLibraryAdapterRunner` with the empty `AnalysisRequest` that
//! `library-adapter-build` uses as its handshake smoke test. Asserts the
//! adapter reports protocol v1, identity `ce-rust`, toolchain `rustc=1.92.0`,
//! and no libraries/solutions.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use infrastructure::library_adapter::process::ProcessLibraryAdapterRunner;
use library_adapter_protocol::{AnalysisRequest, SCHEMA_VERSION};
use serial_test::serial;
use usecases::library_adapter::LibraryAdapterRunner;

/// Locate the workspace root by walking up from this crate's manifest.
fn workspace_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let mut current = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
        loop {
            if current.join("Cargo.toml").is_file()
                && current.join("tools/library-analyzers").is_dir()
            {
                return current;
            }
            if !current.pop() {
                panic!(
                    "workspace root not found from {}",
                    env!("CARGO_MANIFEST_DIR")
                );
            }
        }
    })
}

/// Environment allowlist that mirrors `sanitized_language_env`: enough to run
/// rustup shims and rustc, nothing more.
fn sanitized_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for key in [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "CARGO_HOME",
        "RUSTUP_HOME",
    ] {
        if let Ok(v) = std::env::var(key) {
            env.insert(key.into(), v);
        }
    }
    env.entry("PATH".into())
        .or_insert_with(|| "/usr/bin:/bin".into());
    env
}

/// Build `ce-library-rust-analyzer` (release) once per test process and return
/// the path to the produced executable.
fn built_rust_analyzer() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let root = workspace_root();
        let status = Command::new("cargo")
            .args([
                "build",
                "--quiet",
                "--release",
                "--locked",
                "--package",
                "ce-library-rust-analyzer",
            ])
            .current_dir(root)
            .status()
            .expect("spawn cargo build");
        assert!(
            status.success(),
            "cargo build ce-library-rust-analyzer failed with {status}",
        );
        let bin = root.join("target").join("release").join("rust-analyzer");
        assert!(bin.is_file(), "expected binary at {}", bin.display());
        bin
    })
    .as_path()
}

fn empty_rust_request() -> AnalysisRequest {
    AnalysisRequest {
        schema_version: SCHEMA_VERSION,
        repository_root: workspace_root().display().to_string(),
        language: "rust".into(),
        libraries: vec![],
        solutions: vec![],
    }
}

#[test]
#[serial]
fn rust_adapter_handshake_returns_ce_rust_identity() {
    let bin = built_rust_analyzer();
    let runner = ProcessLibraryAdapterRunner::new(workspace_root().to_path_buf(), sanitized_env());
    let response = runner
        .analyze(bin, &empty_rust_request(), Duration::from_secs(30))
        .expect("handshake succeeds");

    assert_eq!(response.schema_version, SCHEMA_VERSION);
    assert_eq!(response.adapter.name, "ce-rust");
    assert_eq!(response.adapter.version, "0.1.0");

    let rustc = response
        .adapter
        .toolchains
        .iter()
        .find(|t| t.name == "rustc")
        .expect("rustc toolchain reported");
    assert_eq!(rustc.version, "1.92.0");
    assert!(
        rustc.target.is_some(),
        "rustc host triple should be reported"
    );

    assert!(
        response.libraries.is_empty(),
        "empty request must yield empty library analyses"
    );
    assert!(
        response.solutions.is_empty(),
        "empty request must yield empty solution analyses"
    );
}
