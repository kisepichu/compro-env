pub mod commands;

use anyhow::Result;
use clap::Parser;
use commands::Cli;

use crate::{
    config_impl::ConfigImpl,
    online_judge_impl::atcoder::AtCoder,
    repository_impl::{
        contest_repository_impl::ContestRepositoryImpl,
        session_repository_impl::SessionRepositoryImpl,
        solution_repository_impl::SolutionRepositoryImpl,
    },
};
use interfaces::controller::Controller;
use usecases::service::Service;

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        // These commands only touch ~/.config/ce/... and do not need a project root.
        commands::Commands::Login { oj: _ } => {
            todo!()
        }
        commands::Commands::Whoami { oj: _ } => {
            todo!()
        }
        // These commands require a project root.
        commands::Commands::Init { contest: _ } => {
            let _controller = build_controller()?;
            todo!()
        }
        commands::Commands::New {
            contest: _,
            problem: _,
            solution: _,
            lang: _,
        } => {
            let _controller = build_controller()?;
            todo!()
        }
        commands::Commands::Test {
            contest: _,
            problem: _,
            solution: _,
            lang: _,
        } => {
            let _controller = build_controller()?;
            todo!()
        }
        commands::Commands::Submit {
            contest: _,
            problem: _,
            solution: _,
            lang: _,
        } => {
            let _controller = build_controller()?;
            todo!()
        }
    }
}

fn build_controller() -> Result<Controller> {
    let root = find_project_root()?;

    let service = Service::new(
        Box::new(AtCoder),
        Box::new(ContestRepositoryImpl::new(root.clone())),
        Box::new(SolutionRepositoryImpl::new(root.clone())),
        Box::new(SessionRepositoryImpl),
        Box::new(ConfigImpl),
    );

    Ok(Controller::new(service))
}

/// Locates the project root by searching upward for the `templates/` directory.
///
/// `Cargo.toml` is not used as a sentinel because every Rust contest workspace and
/// solution package under `solutions/{contest_id}/rust/...` also contains a `Cargo.toml`,
/// which would resolve to the wrong root when running from a contest subdirectory.
fn find_project_root() -> Result<std::path::PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("templates").is_dir() {
            return Ok(dir);
        }
        if !dir.pop() {
            anyhow::bail!("could not find project root (no templates/ directory found)");
        }
    }
}
