//! `ce-rust` — Rust language analyzer for compro-env (plan 043 Task 1).
//!
//! Reads a single `AnalysisRequest` JSON document from stdin, executes
//! `rustc -Vv` to record the invoking toolchain, and writes a single
//! `AnalysisResponse` JSON document to stdout. Dependency and symbol analysis
//! are delivered by follow-up tasks; every non-empty target is echoed back
//! with a `partial` state so the pipeline behaves predictably before Task 2
//! wires in the real resolver.

mod request;

use std::io::{Read, Write};
use std::process::{Command, ExitCode};

use anyhow::{Context, bail};
use library_adapter_protocol::{
    AdapterIdentity, AnalysisRequest, AnalysisResponse, AnalysisState, DependencyAnalysis,
    Diagnostic, LibraryAnalysis, SCHEMA_VERSION, Severity, SolutionAnalysis, SymbolAnalysis,
    ToolchainIdentity,
};

use request::parse_request;

/// Public adapter identity. `library-adapter-build` cross-checks this against
/// `LanguageBuildPlan::expected_adapter_name` during the handshake.
pub const ADAPTER_NAME: &str = "ce-rust";
pub const ADAPTER_VERSION: &str = "0.1.0";
/// Emitted on library/solution entries when Task 2 has not yet delivered.
const PENDING_DIAGNOSTIC_CODE: &str = "rust.mvp.dependencies-pending";
const PENDING_DIAGNOSTIC_MESSAGE: &str =
    "Rust dependency analysis is delivered by plan 043 Task 2; targets emitted as partial";

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
    let libraries = request
        .libraries
        .iter()
        .map(|t| LibraryAnalysis {
            path: t.path.clone(),
            dependency_analysis: partial_dependencies(),
            symbol_analysis: partial_symbols(),
            diagnostics: vec![pending_diagnostic()],
        })
        .collect();
    let solutions = request
        .solutions
        .iter()
        .map(|t| SolutionAnalysis {
            id: t.id.clone(),
            dependency_analysis: partial_dependencies(),
            diagnostics: vec![pending_diagnostic()],
        })
        .collect();
    Ok(AnalysisResponse {
        schema_version: SCHEMA_VERSION,
        adapter,
        libraries,
        solutions,
    })
}

fn partial_dependencies() -> DependencyAnalysis {
    DependencyAnalysis {
        state: AnalysisState::Partial,
        dependencies: vec![],
    }
}

fn partial_symbols() -> SymbolAnalysis {
    SymbolAnalysis {
        state: AnalysisState::Partial,
        symbols: vec![],
    }
}

fn pending_diagnostic() -> Diagnostic {
    Diagnostic {
        severity: Severity::Warning,
        code: PENDING_DIAGNOSTIC_CODE.into(),
        message: PENDING_DIAGNOSTIC_MESSAGE.into(),
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
    use library_adapter_protocol::{LibraryTarget, SolutionTarget};

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

    #[test]
    fn build_response_echoes_targets_as_partial() {
        // Bypass `detect_rustc` by asserting only the deterministic parts of
        // the response.
        let request = AnalysisRequest {
            schema_version: SCHEMA_VERSION,
            repository_root: ".".into(),
            language: "rust".into(),
            libraries: vec![LibraryTarget {
                path: "library/rust/a.rs".into(),
            }],
            solutions: vec![SolutionTarget {
                id: "abc/A/main".into(),
                root: "solutions/abc/A/main".into(),
                entry: "src/main.rs".into(),
            }],
        };
        // We can only run this if rustc is on PATH — matches the runtime
        // requirement of the binary.
        if Command::new("rustc").arg("-Vv").output().is_err() {
            eprintln!("skipping: rustc not available");
            return;
        }
        let response = build_response(&request).unwrap();
        assert_eq!(response.schema_version, SCHEMA_VERSION);
        assert_eq!(response.adapter.name, ADAPTER_NAME);
        assert_eq!(response.libraries.len(), 1);
        assert_eq!(
            response.libraries[0].dependency_analysis.state,
            AnalysisState::Partial
        );
        assert_eq!(
            response.libraries[0].symbol_analysis.state,
            AnalysisState::Partial
        );
        assert_eq!(response.solutions.len(), 1);
    }
}
