//! Handshake fixture test for the ce-lean adapter (spec §§6.8, 6.9; plan
//! 048 Task 2).
//!
//! Like the C++ handshake test, this one deliberately does **not** try to
//! run `tools/library-analyzers/prepare` or compile the analyzer itself:
//! the prepared Lean tarball is ~150 MB, and the compiled `ce-lean`
//! binary still needs `libLean_shared.so` from that install on
//! `LD_LIBRARY_PATH` at run time (the committed `lake-manifest.json`
//! declares no external packages, so `lake build` itself is offline).
//! Instead the test is gated behind `CE_RUN_LEAN_HANDSHAKE=1` and probes
//! for the already-published
//! `<repo>/target/library-analyzers/bin/lean-analyzer` symlink. Without
//! the gate the test prints a hint and returns success, so
//! `cargo test --all` stays green on fresh clones.
//!
//! When the gate is set, the test uses the same `ProcessLibraryAdapterRunner`
//! the build driver uses, sends the empty handshake request, and asserts
//! the response matches spec §6.8: protocol v1, adapter `ce-lean` v `0.1.0`,
//! toolchain `lean = 4.30.0`, no libraries or solutions.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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

/// Environment for the Lean analyzer's handshake. Discovers the single
/// prepared set under `<workspace>/target/library-analyzers/prepared/`
/// and delegates to `analyze_language_env` so the test exercises the
/// exact env `build_analyze_envs` supplies at run time — including
/// `CE_LEAN_ROOT`, `<lean_root>/bin` on `PATH`, and `<lean_root>/lib`
/// on `LD_LIBRARY_PATH`.
fn sanitized_env() -> BTreeMap<String, String> {
    use domain::adapter_build::TargetPlatform;
    use infrastructure::library_adapter::language_plans::analyze_language_env;

    let analyzer_root = workspace_root().join("target").join("library-analyzers");
    let prepared_root = discover_prepared_root(&analyzer_root);
    let platform = TargetPlatform {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
    };
    analyze_language_env(&prepared_root, &platform, "lean").expect("lean analyze env")
}

/// Locate the single non-staging prepared-set directory. Mirrors
/// `shell::discover_prepared_root` so the gated handshake tests do not
/// grow their own layout assumptions.
fn discover_prepared_root(analyzer_root: &Path) -> PathBuf {
    let prepared_dir = analyzer_root.join("prepared");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&prepared_dir)
        .unwrap_or_else(|e| {
            panic!("failed to read {}: {e}", prepared_dir.display());
        })
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            !name.to_string_lossy().starts_with("staging-")
        })
        .map(|entry| entry.path())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one prepared set under {}, found {}",
        prepared_dir.display(),
        entries.len()
    );
    entries.pop().unwrap()
}

fn empty_lean_request() -> AnalysisRequest {
    AnalysisRequest {
        schema_version: SCHEMA_VERSION,
        repository_root: workspace_root().display().to_string(),
        language: "lean".into(),
        libraries: vec![],
        solutions: vec![],
    }
}

#[test]
#[serial]
fn lean_adapter_handshake_returns_ce_lean_identity() {
    if std::env::var_os("CE_RUN_LEAN_HANDSHAKE").is_none() {
        eprintln!(
            "lean_adapter_handshake: skipping — set CE_RUN_LEAN_HANDSHAKE=1 and run \
             `tools/library-analyzers/prepare && tools/library-analyzers/build` first"
        );
        return;
    }

    let bin = workspace_root()
        .join("target")
        .join("library-analyzers")
        .join("bin")
        .join("lean-analyzer");
    assert!(
        bin.exists(),
        "expected the Lean adapter at {}. Run \
         `tools/library-analyzers/prepare && tools/library-analyzers/build` first.",
        bin.display()
    );

    let runner = ProcessLibraryAdapterRunner::new(workspace_root().to_path_buf());
    let env = sanitized_env();
    let response = runner
        .analyze(&bin, &empty_lean_request(), Duration::from_secs(30), &env)
        .expect("handshake succeeds");

    assert_eq!(response.schema_version, SCHEMA_VERSION);
    assert_eq!(response.adapter.name, "ce-lean");
    assert_eq!(response.adapter.version, "0.1.0");

    let lean = response
        .adapter
        .toolchains
        .iter()
        .find(|t| t.name == "lean")
        .expect("lean toolchain reported");
    assert_eq!(
        lean.version, "4.30.0",
        "spec §6.8 pins Lean to 4.30.0 exactly"
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
