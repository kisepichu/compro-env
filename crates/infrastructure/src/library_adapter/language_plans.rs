//! Concrete `LanguageBuildPlan` factories for adapter-build (plan 043 Task 1).
//!
//! Each function returns the plan the shared build driver needs to compile
//! and handshake one language's adapter under the sanitized environment.
//! Only the Rust adapter is wired in this plan; C++ and Lean are added by
//! plans 044–050.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::library_adapter::build::LanguageBuildPlan;

/// Language ID under which the Rust adapter is registered.
pub const RUST_LANGUAGE: &str = "rust";
/// File name of the Rust adapter under `<build_dir>/bin/`.
pub const RUST_BIN_NAME: &str = "rust-analyzer";
/// Adapter identity string the ce-rust binary reports at handshake.
pub const RUST_ADAPTER_NAME: &str = "ce-rust";
/// Adapter version reported at handshake. Kept in sync with the crate's
/// declared version in `tools/library-analyzers/rust/Cargo.toml`.
pub const RUST_ADAPTER_VERSION: &str = "0.1.0";

/// Environment allowlist forwarded to language builds and handshakes.
///
/// `env_clear()` is called first by the driver and the runner, so anything
/// not in the returned map does not reach the child. `RUSTUP_TOOLCHAIN` is
/// deliberately excluded so ambient host settings cannot override the
/// pinned `rust-toolchain.toml`.
pub fn sanitized_language_env() -> BTreeMap<String, String> {
    const FORWARD: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "CARGO_HOME",
        "RUSTUP_HOME",
    ];
    let mut env = BTreeMap::new();
    for key in FORWARD {
        if let Ok(v) = std::env::var(key) {
            env.insert((*key).into(), v);
        }
    }
    // `PATH` must always be present; fall back to a POSIX default so hosts
    // that clear the launcher's PATH still find `cargo`/`rustc` shims.
    env.entry("PATH".into())
        .or_insert_with(|| "/usr/bin:/bin".into());
    env
}

/// Build plan for the Rust adapter.
///
/// * The working directory is set to `repository_root` so `rust-toolchain.toml`
///   is picked up and the workspace `Cargo.lock` applies.
/// * `--offline --locked` prevents any network fetch during the build; Rust
///   dependencies land in the prepared set (added by follow-up plans).
/// * The compiled binary is expected at `target/release/rust-analyzer` in the
///   workspace; the driver copies it into `<staging>/bin/rust-analyzer`.
pub fn rust_build_plan(repository_root: &Path) -> LanguageBuildPlan {
    let cwd: PathBuf = repository_root.to_path_buf();
    let env = sanitized_language_env();
    let handshake_env = env.clone();
    LanguageBuildPlan {
        language: RUST_LANGUAGE.into(),
        file_name: RUST_BIN_NAME.into(),
        expected_adapter_name: RUST_ADAPTER_NAME.into(),
        expected_adapter_version: RUST_ADAPTER_VERSION.into(),
        argv: vec![
            "cargo".into(),
            "build".into(),
            "--quiet".into(),
            "--locked".into(),
            "--offline".into(),
            "--release".into(),
            "--package".into(),
            "ce-library-rust-analyzer".into(),
        ],
        environment: env,
        working_directory: Some(cwd),
        output_relative_path: "target/release/rust-analyzer".into(),
        handshake_environment: handshake_env,
    }
}
