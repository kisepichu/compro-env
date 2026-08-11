//! `library-adapter-build` — repository-local driver that builds, handshakes,
//! and atomically publishes the adapter build set (spec §6.9, plan 042 Task 2).
//!
//! `--check` validates the current on-disk state without touching anything.
//! Without `--check` the driver refuses to run because MVP language plans are
//! introduced by plans 043–050. Adding a language later wires it up here
//! rather than duplicating build orchestration per adapter.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use domain::adapter_build::{ExpectedBuild, TargetPlatform};
use domain::adapter_prepare::ExpectedPreparedSet;
use infrastructure::library_adapter::build_state::{BuildStateError, inspect_build_state};
use infrastructure::library_adapter::inputs::{calculate_input_digest, load_build_inputs};
use infrastructure::library_adapter::prepare::{PREPARED_SUBDIR, prepared_dir};
use infrastructure::library_adapter::prepared::{
    expected_dependency_id, load_dependency_manifest, validate_prepared_set,
};

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

    if args.check {
        // First: prepared-set health check.
        let dep_manifest = load_dependency_manifest(&repository_root)?;
        let dep_id = expected_dependency_id(&repository_root, &dep_manifest, &target_platform)?;
        let prepared_path = prepared_dir(&analyzer_root, &dep_id);
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
        match inspect_build_state(&analyzer_root, &expected) {
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
                     once language plans are wired up",
                    analyzer_root.display()
                );
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("current build state is not usable: {e:#}")),
        }
    } else {
        anyhow::bail!(
            "language plans are introduced by plans 043–050; \
             `library-adapter-build` currently only supports `--check` \
             (input digest computed: {input_digest})"
        )
    }
}
