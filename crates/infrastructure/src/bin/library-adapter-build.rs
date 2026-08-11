//! `library-adapter-build` — repository-local driver that builds, handshakes,
//! and atomically publishes the adapter build set (spec §6.9, plan 042 Task 2).
//!
//! `--check` validates the current on-disk state without touching anything.
//! Without `--check` the driver runs every registered language plan (plan 043
//! wires Rust; C++/Lean arrive in plans 044–050) through the shared build
//! orchestrator.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use domain::adapter_build::{ExpectedBuild, TargetPlatform};
use domain::adapter_prepare::ExpectedPreparedSet;
use infrastructure::library_adapter::build::{BuildRequest, build_adapters};
use infrastructure::library_adapter::build_state::{BuildStateError, inspect_build_state};
use infrastructure::library_adapter::cpp_toolchain::select_cpp_toolchain;
use infrastructure::library_adapter::inputs::{calculate_input_digest, load_build_inputs};
use infrastructure::library_adapter::language_plans::{
    cpp_build_plan, rust_build_plan, sanitized_language_env,
};
use infrastructure::library_adapter::prepare::{PREPARED_SUBDIR, prepared_dir};
use infrastructure::library_adapter::prepared::{
    expected_dependency_id, load_dependency_manifest, validate_prepared_set,
};
use infrastructure::library_adapter::process::ProcessLibraryAdapterRunner;

#[derive(Parser, Debug)]
#[command(
    name = "library-adapter-build",
    about = "Build adapters and atomically publish `bin` (spec §6.9)"
)]
struct Args {
    /// Repository root (defaults to current working directory).
    #[arg(long, value_name = "PATH")]
    repository_root: Option<PathBuf>,

    /// Analyzer root (defaults to `<repo>/target/library-analyzers`).
    #[arg(long, value_name = "PATH")]
    analyzer_root: Option<PathBuf>,

    /// Target OS (defaults to current).
    #[arg(long)]
    target_os: Option<String>,

    /// Target architecture (defaults to current).
    #[arg(long)]
    target_arch: Option<String>,

    /// Build profile recorded in the manifest.
    #[arg(long, default_value = "release")]
    build_profile: String,

    /// Validate the current on-disk build set without building or writing.
    #[arg(long)]
    check: bool,

    /// Handshake timeout in seconds.
    #[arg(long, default_value_t = 120)]
    handshake_timeout_secs: u64,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("library-adapter-build: error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> anyhow::Result<()> {
    let repository_root = match args.repository_root {
        Some(p) => p,
        None => std::env::current_dir()?,
    };
    let analyzer_root = args
        .analyzer_root
        .unwrap_or_else(|| repository_root.join("target").join("library-analyzers"));
    let target_platform = TargetPlatform {
        os: args
            .target_os
            .unwrap_or_else(|| std::env::consts::OS.into()),
        arch: args
            .target_arch
            .unwrap_or_else(|| std::env::consts::ARCH.into()),
    };
    let inputs = load_build_inputs(&repository_root)?;
    let input_digest = calculate_input_digest(&repository_root, &inputs, &target_platform)?;
    let dep_manifest = load_dependency_manifest(&repository_root)?;
    let dep_id = expected_dependency_id(&repository_root, &dep_manifest, &target_platform)?;
    let prepared_path = prepared_dir(&analyzer_root, &dep_id);

    if args.check {
        // First: prepared-set health check.
        println!("expected dependency-id: {dep_id}");
        if prepared_path.exists() {
            let expected = ExpectedPreparedSet {
                id: dep_id.clone(),
                target_platform: target_platform.clone(),
            };
            validate_prepared_set(&prepared_path, &expected)?;
            println!("prepared set at {} is valid", prepared_path.display());
        } else {
            println!(
                "prepared set not present under {}/{}; run \
                 `tools/library-analyzers/prepare` first",
                analyzer_root.join(PREPARED_SUBDIR).display(),
                dep_id
            );
        }

        // Second: build-state health.
        println!("expected input-digest: {input_digest}");
        let expected = ExpectedBuild {
            input_digest,
            target_platform,
            build_profile: args.build_profile,
            protocol_version: library_adapter_protocol::SCHEMA_VERSION,
        };
        return match inspect_build_state(&analyzer_root, &expected) {
            Ok(set) => {
                println!("current build set: {}", set.root.display());
                println!("build-id: {}", set.build_id);
                Ok(())
            }
            // "Not yet built" states are not errors under --check: this
            // command is a diagnostic, not a gate. Genuine corruption still
            // fails: the analyzer must never use a stale or tampered set.
            Err(BuildStateError::CurrentBinMissing { .. }) => {
                println!(
                    "no current build set at {}; run this driver without --check \
                     to produce one",
                    analyzer_root.display()
                );
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("current build state is not usable: {e:#}")),
        };
    }

    // Build path: refuse to start without a prepared set on disk. Downloading
    // is `prepare`'s job; the build driver must be entirely offline.
    let expected_prepared = ExpectedPreparedSet {
        id: dep_id.clone(),
        target_platform: target_platform.clone(),
    };
    let prepared_set =
        validate_prepared_set(&prepared_path, &expected_prepared).map_err(|source| {
            anyhow::anyhow!(
                "prepared set at {} is missing or corrupt: {source:#}. \
             Run `tools/library-analyzers/prepare` first.",
                prepared_path.display(),
            )
        })?;

    let git_commit_sha = read_git_commit_sha(&repository_root)?;

    let mut language_plans = vec![rust_build_plan(&repository_root)];
    // Plan 045 Task 2: attach the C++ plan when the current target is one of
    // the three officially supported LLVM 22.1.0 triples AND the prepared set
    // actually has that install unpacked. `cpp_build_plan` returns
    // `PreparedInstallMissing` in the interim between `prepare` and `build`
    // when the operator is on an unsupported host — treat both as "skip C++"
    // rather than a hard error so a Rust-only rebuild still works.
    if select_cpp_toolchain(&target_platform).is_ok() {
        match cpp_build_plan(&repository_root, &target_platform, &prepared_set) {
            Ok(plan) => language_plans.push(plan),
            Err(err) => {
                eprintln!("library-adapter-build: skipping C++ adapter: {err}");
            }
        }
    }

    let request = BuildRequest {
        repository_root: repository_root.clone(),
        analyzer_root: analyzer_root.clone(),
        target_platform: target_platform.clone(),
        build_profile: args.build_profile.clone(),
        protocol_version: library_adapter_protocol::SCHEMA_VERSION,
        input_digest,
        git_commit_sha,
        prepared_set,
        language_plans,
        handshake_timeout: Duration::from_secs(args.handshake_timeout_secs),
    };

    let runner =
        ProcessLibraryAdapterRunner::new(repository_root.clone(), sanitized_language_env());
    let set = build_adapters(&request, &runner)?;
    println!("published build set: {}", set.root.display());
    println!("build-id: {}", set.build_id);
    Ok(())
}

/// Read the current HEAD commit SHA. `library-adapter-build` records this in
/// the published manifest so operators can trace a build back to source. When
/// `git rev-parse HEAD` fails (missing `.git`) or yields a value that is not
/// a 40-character hex string, this returns a placeholder of 40 zero digits so
/// the manifest keeps the fixed-width shape; the SHA is not part of the
/// deterministic build-id derivation.
fn read_git_commit_sha(repository_root: &std::path::Path) -> anyhow::Result<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repository_root)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let sha = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
                Ok(sha)
            } else {
                Ok("0".repeat(40))
            }
        }
        _ => Ok("0".repeat(40)),
    }
}
