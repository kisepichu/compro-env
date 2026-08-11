//! Handshake fixture test for the ce-lean adapter (spec §§6.8, 6.9; plan
//! 048 Task 2).
//!
//! Like the C++ handshake test, this one deliberately does **not** try to
//! run `tools/library-analyzers/prepare` or compile the analyzer itself:
//! the prepared Lean tarball is ~150 MB and `lake build` needs both that
//! archive and a network hop to fetch Lean's built-in modules on a fresh
//! host. Instead the test is gated behind `CE_RUN_LEAN_HANDSHAKE=1` and
//! probes for the already-published
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

/// Minimal env allowlist for running the analyzer binary. Lean's runtime
/// resolves `libLean_shared.so` and friends through `LD_LIBRARY_PATH`, so
/// the caller must set that pointing at the prepared Lean install before
/// running the gated test.
fn sanitized_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for key in ["PATH", "HOME", "USER", "LOGNAME", "LD_LIBRARY_PATH"] {
        if let Ok(v) = std::env::var(key) {
            env.insert(key.into(), v);
        }
    }
    env.entry("PATH".into())
        .or_insert_with(|| "/usr/bin:/bin".into());
    env
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

    let runner = ProcessLibraryAdapterRunner::new(workspace_root().to_path_buf(), sanitized_env());
    let response = runner
        .analyze(&bin, &empty_lean_request(), Duration::from_secs(30))
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
