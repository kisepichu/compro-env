//! Concrete `LanguageBuildPlan` factories for adapter-build (plan 043 Task 1,
//! extended by plan 045 Task 2 for C++).
//!
//! Each function returns the plan the shared build driver needs to compile
//! and handshake one language's adapter under the sanitized environment.
//! The Rust and C++ adapters are wired here; Lean is added by plan 050.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use domain::adapter_build::TargetPlatform;
use domain::adapter_prepare::PreparedSet;

use crate::library_adapter::build::LanguageBuildPlan;
use crate::library_adapter::cpp_toolchain::{CppToolchainError, locate_prepared_llvm_root};
use crate::library_adapter::lean_toolchain::{LeanToolchainError, locate_prepared_lean_root};

/// Language ID under which the Rust adapter is registered.
pub const RUST_LANGUAGE: &str = "rust";
/// File name of the Rust adapter under `<build_dir>/bin/`.
pub const RUST_BIN_NAME: &str = "rust-analyzer";
/// Adapter identity string the ce-rust binary reports at handshake.
pub const RUST_ADAPTER_NAME: &str = "ce-rust";
/// Adapter version reported at handshake. Kept in sync with the crate's
/// declared version in `tools/library-analyzers/rust/Cargo.toml`.
pub const RUST_ADAPTER_VERSION: &str = "0.1.0";

/// Language ID under which the C++ adapter is registered.
pub const CPP_LANGUAGE: &str = "cpp";
/// File name of the C++ adapter under `<build_dir>/bin/`.
pub const CPP_BIN_NAME: &str = "cpp-analyzer";
/// Adapter identity string the cpp-analyzer binary reports at handshake.
pub const CPP_ADAPTER_NAME: &str = "ce-cpp";
/// Adapter version reported at handshake. Kept in sync with
/// `tools/library-analyzers/cpp/include/protocol.hpp`.
pub const CPP_ADAPTER_VERSION: &str = "0.1.0";

/// Language ID under which the Lean adapter is registered.
pub const LEAN_LANGUAGE: &str = "lean";
/// File name of the Lean adapter under `<build_dir>/bin/`.
pub const LEAN_BIN_NAME: &str = "lean-analyzer";
/// Adapter identity string the lean-analyzer binary reports at handshake.
pub const LEAN_ADAPTER_NAME: &str = "ce-lean";
/// Adapter version reported at handshake. Kept in sync with
/// `tools/library-analyzers/lean/Analyzer/Protocol.lean` (`adapterVersion`).
pub const LEAN_ADAPTER_VERSION: &str = "0.1.0";

/// Environment allowlist forwarded to language builds and handshakes.
///
/// `env_clear()` is called first by the driver and the runner, so anything
/// not in the returned map does not reach the child. `RUSTUP_TOOLCHAIN` is
/// deliberately excluded so ambient host settings cannot override the
/// pinned `rust-toolchain.toml`.
///
/// `LIBRARY_PATH`, `LD_LIBRARY_PATH`, and `C_INCLUDE_PATH` are forwarded so
/// the C++ adapter build (plan 046) can locate host-provided libraries the
/// pinned LLVM tarball still links against — notably `libz`, which
/// `libclang-cpp.so` needs at both link and run time. Nix-based hosts
/// already publish those variables; systems where they are unset simply
/// fall through to the compiler/linker defaults.
pub fn sanitized_language_env() -> BTreeMap<String, String> {
    const FORWARD: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "LIBRARY_PATH",
        "LD_LIBRARY_PATH",
        "C_INCLUDE_PATH",
        "CPLUS_INCLUDE_PATH",
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

/// Build plan for the C++ adapter (spec §§6.7, 6.9; plan 045 Task 2).
///
/// The plan invokes `tools/library-analyzers/cpp/build.sh` under `bash`. That
/// script runs the two-step CMake configure + build against the caller-picked
/// staging directory (`CE_ADAPTER_STAGE_DIR`, injected by the build driver)
/// and points CMake at the prepared LLVM install via `CE_LLVM_DIR`.
///
/// The resulting executable lands at `<CE_ADAPTER_STAGE_DIR>/cpp/cpp-analyzer`,
/// so `output_relative_path` is set relative to the plan's working directory
/// — left unset here so the build driver defaults it to the staging directory.
///
/// The handshake runs the freshly built executable against the empty
/// `AnalysisRequest`. Plan 046 links `libclang-cpp.so` into the binary, so
/// the handshake env forwards `LIBRARY_PATH`/`LD_LIBRARY_PATH` (see
/// `sanitized_language_env`) alongside the RPATH that `CMakeLists.txt`
/// bakes in.
pub fn cpp_build_plan(
    repository_root: &Path,
    platform: &TargetPlatform,
    prepared_set: &PreparedSet,
) -> Result<LanguageBuildPlan, CppToolchainError> {
    let llvm_root = locate_prepared_llvm_root(prepared_set, platform)?;
    let script = repository_root
        .join("tools/library-analyzers/cpp/build.sh")
        .to_string_lossy()
        .into_owned();

    let mut env = sanitized_language_env();
    // The build driver clears any `RUSTUP_*` flavor keys via `env_clear`.
    // Fold the LLVM install root into the plan so build.sh can point CMake at
    // exactly the pinned toolchain (spec §6.7).
    env.insert(
        "CE_LLVM_DIR".into(),
        llvm_root.to_string_lossy().into_owned(),
    );

    let handshake_env = sanitized_language_env();

    Ok(LanguageBuildPlan {
        language: CPP_LANGUAGE.into(),
        file_name: CPP_BIN_NAME.into(),
        expected_adapter_name: CPP_ADAPTER_NAME.into(),
        expected_adapter_version: CPP_ADAPTER_VERSION.into(),
        argv: vec!["bash".into(), script],
        environment: env,
        // Working directory unset → driver uses the staging directory. The
        // cmake build tree lives at `<staging>/cpp/cpp-analyzer`, which is
        // where the driver reads from before copying into `<staging>/bin/`.
        working_directory: None,
        output_relative_path: "cpp/cpp-analyzer".into(),
        handshake_environment: handshake_env,
    })
}

/// Build plan for the Lean adapter (spec §§6.8, 6.9; plan 048 Task 2).
///
/// The plan invokes `tools/library-analyzers/lean/build.sh` under `bash`.
/// That script runs `lake build ce-lean` against the caller-picked staging
/// directory (`CE_ADAPTER_STAGE_DIR`, injected by the build driver) with
/// `PATH` pinned to the prepared `bin/`. `CE_LEAN_ROOT` points build.sh at
/// the prepared Lean install.
///
/// The resulting executable lands at `<CE_ADAPTER_STAGE_DIR>/lean/ce-lean`,
/// so `output_relative_path` is set relative to the plan's working directory
/// — left unset here so the build driver defaults it to the staging
/// directory. The handshake runs the freshly built executable against the
/// empty `AnalysisRequest`; both environments prepend `CE_LEAN_ROOT/lib` to
/// `LD_LIBRARY_PATH` so the dynamic linker resolves `libLean_shared.so`
/// (and its siblings shipped in the tarball) at run time.
pub fn lean_build_plan(
    repository_root: &Path,
    platform: &TargetPlatform,
    prepared_set: &PreparedSet,
) -> Result<LanguageBuildPlan, LeanToolchainError> {
    let lean_root = locate_prepared_lean_root(prepared_set, platform)?;
    let script = repository_root
        .join("tools/library-analyzers/lean/build.sh")
        .to_string_lossy()
        .into_owned();

    let build_env = build_lean_env(&lean_root, sanitized_language_env());
    let handshake_env = build_lean_env(&lean_root, sanitized_language_env());

    Ok(LanguageBuildPlan {
        language: LEAN_LANGUAGE.into(),
        file_name: LEAN_BIN_NAME.into(),
        expected_adapter_name: LEAN_ADAPTER_NAME.into(),
        expected_adapter_version: LEAN_ADAPTER_VERSION.into(),
        argv: vec!["bash".into(), script],
        environment: build_env,
        // Working directory unset → driver uses the staging directory. The
        // lake build tree lives at `<staging>/lean/ce-lean`, which is where
        // the driver reads from before copying into `<staging>/bin/`.
        working_directory: None,
        output_relative_path: "lean/ce-lean".into(),
        handshake_environment: handshake_env,
    })
}

/// Inject `CE_LEAN_ROOT` plus `<lean_root>/lib` on `LD_LIBRARY_PATH` into a
/// sanitized env map. Any existing `LD_LIBRARY_PATH` is preserved by
/// appending after the prepared `lib/` — the pinned Lean shared objects
/// resolve first, and host-provided libraries the loader still needs
/// (libc, libstdc++, …) continue to resolve through their original
/// entries.
fn build_lean_env(lean_root: &Path, mut env: BTreeMap<String, String>) -> BTreeMap<String, String> {
    let root_str = lean_root.to_string_lossy().into_owned();
    let lib_dir = lean_root.join("lib").to_string_lossy().into_owned();
    env.insert("CE_LEAN_ROOT".into(), root_str);
    let entry = env.entry("LD_LIBRARY_PATH".into()).or_default();
    if entry.is_empty() {
        *entry = lib_dir;
    } else {
        *entry = format!("{lib_dir}:{entry}");
    }
    env
}
