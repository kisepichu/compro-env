//! `library-adapter-prepare` — repository-local driver that validates and
//! downloads pinned adapter dependencies (spec §6.9, plan 041 Task 2).
//!
//! - `--check`: parse `tools/library-analyzers/dependencies.toml`, compute the
//!   dependency id, and validate any already-prepared set on disk. Never
//!   downloads or writes.
//! - default: parse the manifest, download missing archives / Git tarballs
//!   into `target/library-analyzers/prepared/<dependency-id>/`, and print the
//!   dependency id on success.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use domain::adapter_build::TargetPlatform;
use domain::adapter_prepare::ExpectedPreparedSet;
use infrastructure::library_adapter::download::DownloadPolicy;
use infrastructure::library_adapter::prepare::{
    PREPARED_SUBDIR, PrepareRequest, prepare_dependencies, prepared_dir,
};
use infrastructure::library_adapter::prepared::{
    expected_dependency_id, load_dependency_manifest, validate_prepared_set,
};

#[derive(Parser, Debug)]
#[command(
    name = "library-adapter-prepare",
    about = "Prepare pinned adapter dependencies (spec §6.9)"
)]
struct Args {
    /// Repository root (defaults to current working directory).
    #[arg(long, value_name = "PATH")]
    repository_root: Option<PathBuf>,

    /// Prepared-set root (defaults to `<repo>/target/library-analyzers`).
    #[arg(long, value_name = "PATH")]
    prepared_root: Option<PathBuf>,

    /// Target OS to record in the dependency id (defaults to current).
    #[arg(long)]
    target_os: Option<String>,

    /// Target architecture to record in the dependency id (defaults to
    /// current).
    #[arg(long)]
    target_arch: Option<String>,

    /// Validate the manifest and any existing prepared set without
    /// downloading, extracting, or writing.
    #[arg(long)]
    check: bool,

    /// HTTPS proxy URL to use for downloads.
    #[arg(long)]
    https_proxy: Option<String>,

    /// Path to a PEM CA bundle for downloads.
    #[arg(long)]
    ca_bundle: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("library-adapter-prepare: error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> anyhow::Result<()> {
    let repository_root = match args.repository_root {
        Some(p) => p,
        None => std::env::current_dir()?,
    };
    let prepared_root = args
        .prepared_root
        .unwrap_or_else(|| repository_root.join("target").join("library-analyzers"));
    let target_platform = TargetPlatform {
        os: args
            .target_os
            .unwrap_or_else(|| std::env::consts::OS.into()),
        arch: args
            .target_arch
            .unwrap_or_else(|| std::env::consts::ARCH.into()),
    };
    let manifest = load_dependency_manifest(&repository_root)?;

    if args.check {
        let id = expected_dependency_id(&repository_root, &manifest, &target_platform)?;
        println!("expected dependency-id: {id}");
        let final_dir = prepared_dir(&prepared_root, &id);
        if final_dir.exists() {
            let expected = ExpectedPreparedSet {
                id: id.clone(),
                target_platform,
            };
            validate_prepared_set(&final_dir, &expected)?;
            println!("prepared set at {} is valid", final_dir.display());
        } else {
            println!(
                "prepared set at {}/{} not present; run without --check to prepare",
                prepared_root.join(PREPARED_SUBDIR).display(),
                id
            );
        }
        return Ok(());
    }

    let mut policy = DownloadPolicy::https();
    policy.https_proxy = args.https_proxy;
    if let Some(pem_path) = args.ca_bundle {
        let bytes = std::fs::read(&pem_path)?;
        policy.ca_bundle_pem = Some(bytes);
    }
    let request = PrepareRequest {
        repository_root,
        prepared_root,
        target_platform,
        download_policy: policy,
    };
    let set = prepare_dependencies(&request, &manifest)?;
    println!("prepared set: {}", set.root.display());
    println!("dependency-id: {}", set.id);
    Ok(())
}
