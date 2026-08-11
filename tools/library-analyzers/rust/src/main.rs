//! `ce-rust` — Rust language analyzer for compro-env (plan 043).
//!
//! Reads a single `AnalysisRequest` JSON document from stdin, executes
//! `rustc -Vv` to record the invoking toolchain, resolves direct dependencies
//! (Task 2) using `syn`, and writes a single `AnalysisResponse` JSON document
//! to stdout. Symbol analysis is delivered by a later plan; every library
//! returns `symbol_analysis = partial` with an explicit diagnostic.

use std::io::{Read, Write};
use std::process::{Command, ExitCode};

use anyhow::{Context, bail};
use ce_library_rust_analyzer::dependencies::analyze_request;
use ce_library_rust_analyzer::module_graph::RustWorkspace;
use ce_library_rust_analyzer::request::parse_request;
use library_adapter_protocol::{
    AdapterIdentity, AnalysisRequest, AnalysisResponse, AnalysisState, Diagnostic, SCHEMA_VERSION,
    Severity, SymbolAnalysis, ToolchainIdentity,
};

/// Public adapter identity. `library-adapter-build` cross-checks this against
/// `LanguageBuildPlan::expected_adapter_name` during the handshake.
pub const ADAPTER_NAME: &str = "ce-rust";
pub const ADAPTER_VERSION: &str = "0.1.0";
/// Emitted on every library indicating that symbols are out of MVP scope.
const SYMBOLS_PENDING_CODE: &str = "rust.mvp.symbols-pending";
const SYMBOLS_PENDING_MESSAGE: &str = "Rust symbol analysis is delivered by a follow-up plan";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ce-rust: error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("read stdin")?;
    let request = parse_request(&buf)?;
    let response = build_response(&request)?;
    let json = serde_json::to_string(&response).context("serialize AnalysisResponse")?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(json.as_bytes()).context("write stdout")?;
    stdout.flush().context("flush stdout")?;
    Ok(())
}

fn build_response(request: &AnalysisRequest) -> anyhow::Result<AnalysisResponse> {
    let toolchain = detect_rustc().context("detect rustc toolchain")?;
    let adapter = AdapterIdentity {
        name: ADAPTER_NAME.into(),
        version: ADAPTER_VERSION.into(),
        toolchains: vec![toolchain],
    };
    let workspace = RustWorkspace::from_request(request).context("build workspace model")?;
    let (mut libraries, solutions) = analyze_request(request, &workspace);
    for lib in &mut libraries {
        // Symbol analysis is out of Task 2 scope; stamp a stable diagnostic
        // so downstream normalization can pass the partial state through
        // without inventing a new code path.
        lib.symbol_analysis = SymbolAnalysis {
            state: AnalysisState::Partial,
            symbols: vec![],
        };
        lib.diagnostics.insert(0, symbols_pending_diagnostic());
    }
    Ok(AnalysisResponse {
        schema_version: SCHEMA_VERSION,
        adapter,
        libraries,
        solutions,
    })
}

fn symbols_pending_diagnostic() -> Diagnostic {
    Diagnostic {
        severity: Severity::Info,
        code: SYMBOLS_PENDING_CODE.into(),
        message: SYMBOLS_PENDING_MESSAGE.into(),
        location: None,
    }
}

/// Run `rustc -Vv` and turn it into a normalized `ToolchainIdentity`.
///
/// Only the `release:` and `host:` fields are consumed. The result is
/// deterministic when `RUSTUP_TOOLCHAIN` is not set in the environment and
/// the working directory is inside the workspace that pins `rust-toolchain.toml`.
fn detect_rustc() -> anyhow::Result<ToolchainIdentity> {
    let output = Command::new("rustc")
        .arg("-Vv")
        .output()
        .context("spawn rustc")?;
    if !output.status.success() {
        bail!(
            "rustc -Vv exited with {}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
        );
    }
    let stdout = std::str::from_utf8(&output.stdout).context("rustc -Vv stdout is not UTF-8")?;
    parse_rustc_vv(stdout)
}

fn parse_rustc_vv(stdout: &str) -> anyhow::Result<ToolchainIdentity> {
    let mut release: Option<String> = None;
    let mut host: Option<String> = None;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("release: ") {
            release = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("host: ") {
            host = Some(rest.trim().to_string());
        }
    }
    let version = release.context("rustc -Vv is missing a `release:` line")?;
    Ok(ToolchainIdentity {
        name: "rustc".into(),
        version,
        target: host,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_toolchain_output() -> &'static str {
        "rustc 1.92.0 (ded5c06cf 2025-12-08)\n\
binary: rustc\n\
commit-hash: ded5c06cf21d2b93bffd5d884aa6e96934ee4234\n\
commit-date: 2025-12-08\n\
host: x86_64-unknown-linux-gnu\n\
release: 1.92.0\n\
LLVM version: 21.1.3\n"
    }

    #[test]
    fn parse_rustc_vv_extracts_release_and_host() {
        let toolchain = parse_rustc_vv(sample_toolchain_output()).unwrap();
        assert_eq!(toolchain.name, "rustc");
        assert_eq!(toolchain.version, "1.92.0");
        assert_eq!(
            toolchain.target.as_deref(),
            Some("x86_64-unknown-linux-gnu")
        );
    }

    #[test]
    fn parse_rustc_vv_fails_without_release_line() {
        let err = parse_rustc_vv("rustc 1.92.0\nhost: x86_64-unknown-linux-gnu\n").unwrap_err();
        assert!(err.to_string().contains("release:"));
    }
}
