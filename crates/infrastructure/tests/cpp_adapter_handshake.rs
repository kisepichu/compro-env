//! Handshake fixture test for the ce-cpp adapter (spec §§6.7, 6.9; plan 045
//! Task 2).
//!
//! Unlike the Rust handshake test, this one deliberately does **not** try to
//! run `tools/library-analyzers/prepare` or compile the analyzer itself: the
//! prepared LLVM tarballs are ~300 MB and CMake needs Clang's CMake package
//! plus a network hop to build offline. Instead the test is gated behind
//! `CE_RUN_CPP_HANDSHAKE=1` and probes for the already-published
//! `<repo>/target/library-analyzers/bin/cpp-analyzer` symlink. Without the
//! gate the test prints a hint and returns success, so `cargo test --all`
//! stays green on fresh clones.
//!
//! When the gate is set, the test uses the same `ProcessLibraryAdapterRunner`
//! the build driver uses, sends the empty handshake request, and asserts the
//! response matches spec §6.7: protocol v1, adapter `ce-cpp` v `0.1.0`,
//! toolchain `clang = 22.1.0`, no libraries or solutions.

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

/// Environment for the C++ analyzer's handshake. This delegates to
/// `analyze_language_env` so the test exercises the exact env
/// `build_analyze_envs` will supply at `ce site-data generate` time —
/// same allowlist, same LD_LIBRARY_PATH forwarding, no ad-hoc extras.
fn sanitized_env() -> BTreeMap<String, String> {
    use domain::adapter_build::TargetPlatform;
    use infrastructure::library_adapter::language_plans::analyze_language_env;

    // C++ does not consult the prepared set at analyze time; the
    // workspace root is a safe stand-in — `analyze_language_env` only
    // reads the prepared set for Lean.
    let platform = TargetPlatform {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
    };
    analyze_language_env(workspace_root(), &platform, "cpp").expect("cpp analyze env")
}

fn empty_cpp_request() -> AnalysisRequest {
    AnalysisRequest {
        schema_version: SCHEMA_VERSION,
        repository_root: workspace_root().display().to_string(),
        language: "cpp".into(),
        libraries: vec![],
        solutions: vec![],
    }
}

#[test]
#[serial]
fn cpp_adapter_handshake_returns_ce_cpp_identity() {
    if std::env::var_os("CE_RUN_CPP_HANDSHAKE").is_none() {
        eprintln!(
            "cpp_adapter_handshake: skipping — set CE_RUN_CPP_HANDSHAKE=1 and run \
             `tools/library-analyzers/prepare && tools/library-analyzers/build` first"
        );
        return;
    }

    let bin = workspace_root()
        .join("target")
        .join("library-analyzers")
        .join("bin")
        .join("cpp-analyzer");
    assert!(
        bin.exists(),
        "expected the C++ adapter at {}. Run \
         `tools/library-analyzers/prepare && tools/library-analyzers/build` first.",
        bin.display()
    );

    let runner = ProcessLibraryAdapterRunner::new(workspace_root().to_path_buf());
    let env = sanitized_env();
    let response = runner
        .analyze(&bin, &empty_cpp_request(), Duration::from_secs(30), &env)
        .expect("handshake succeeds");

    assert_eq!(response.schema_version, SCHEMA_VERSION);
    assert_eq!(response.adapter.name, "ce-cpp");
    assert_eq!(response.adapter.version, "0.1.0");

    let clang = response
        .adapter
        .toolchains
        .iter()
        .find(|t| t.name == "clang")
        .expect("clang toolchain reported");
    assert_eq!(
        clang.version, "22.1.0",
        "spec §6.7 pins LLVM/Clang to 22.1.0 exactly"
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
