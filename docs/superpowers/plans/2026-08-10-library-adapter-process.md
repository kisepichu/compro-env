# Library Adapter Process Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add language-neutral build-input hashing, build manifest validation, and a strict adapter process runner without downloading or compiling adapters.

**Architecture:** Infrastructure owns filesystem and child-process mechanics; use cases consume protocol responses through ports. Build freshness is content-addressed from declared inputs, never Git timestamps or language-specific branches.

**Tech Stack:** Rust 1.92.0, serde, toml, sha2, wait-timeout, tempfile, fs2.

## Constraints

- **Branch:** `feat/040-library-adapter-process`
- **Depends on:** plan 039 merged to `main`.
- Read specification sections 6.1-6.5, 6.9, and 14.
- Do not download dependencies, compile an adapter, create a public CLI command, or invoke a shell.
- Reject symlinks, overlapping inputs, repository escapes, duplicate inputs, and non-UTF-8 JSON output.
- Child environments use an explicit allowlist and never inherit credentials or `RUSTUP_TOOLCHAIN`.

### Task 1: Model deterministic build inputs and manifests

**Files:**
- Create: `crates/domain/src/adapter_build.rs`
- Modify: `crates/domain/src/lib.rs`
- Create: `crates/infrastructure/src/library_adapter/mod.rs`
- Create: `crates/infrastructure/src/library_adapter/inputs.rs`
- Create: `crates/infrastructure/tests/adapter_inputs.rs`
- Create: `tools/library-analyzers/build-inputs.toml`

**Interfaces:**

```rust
pub struct ContentDigest(String);
pub fn load_build_inputs(root: &Path) -> Result<BuildInputs, BuildInputError>;
pub fn calculate_input_digest(
    root: &Path,
    inputs: &BuildInputs,
    platform: &TargetPlatform,
) -> Result<ContentDigest, BuildInputError>;
pub fn validate_build_manifest(
    expected: &ExpectedBuild,
    actual: &BuildManifest,
) -> Result<(), BuildManifestError>;
```

- [x] Write tests for byte-order stability, uncommitted changes, missing paths, overlapping directories,
      duplicate files, symlinks, and repository escapes.
- [x] Run `cargo test -p infrastructure --test adapter_inputs`; observe missing APIs.
- [x] Implement strict TOML parsing and SHA-256 framing that hashes relative path plus raw contents.
- [x] Re-run the focused test; expect all cases to pass.
- [x] Invoke `/commit` with `feat: define deterministic adapter build inputs`.

### Task 2: Run protocol processes through a reusable port

**Files:**
- Create: `crates/usecases/src/library_adapter.rs`
- Modify: `crates/usecases/src/lib.rs`
- Create: `crates/infrastructure/src/library_adapter/process.rs`
- Create: `crates/infrastructure/tests/adapter_process.rs`
- Create: `crates/infrastructure/tests/fixtures/adapter-process/valid.sh`
- Create: `crates/infrastructure/tests/fixtures/adapter-process/invalid-json.sh`
- Create: `crates/infrastructure/tests/fixtures/adapter-process/timeout.sh`

**Interfaces:**

```rust
pub trait LibraryAdapterRunner {
    fn analyze(
        &self,
        executable: &Path,
        request: &AnalysisRequest,
        timeout: Duration,
    ) -> Result<AnalysisResponse, AdapterRunError>;
}

pub struct ProcessLibraryAdapterRunner;
```

- [x] Write failing tests for exact argv execution, stdin closure, one JSON document, stdout limit,
      stderr tail limit, timeout, nonzero exit, schema mismatch, and secret-free environment.
- [x] Implement direct `Command` execution with piped stdin/stdout/stderr and process-group termination.
- [x] Validate protocol version and response shape before returning; never accept stdout after nonzero exit.
- [x] Run `cargo test -p infrastructure --test adapter_process`; expect all cases to pass.
- [x] Invoke `/commit` with `feat: run library adapters through strict protocol`.

### Task 3: Deliver the process boundary

- [ ] Run `cargo test -p domain -p usecases -p infrastructure adapter`.
- [ ] Run the repository-wide verification commands from the rollout plan and `git diff --check`.
- [ ] Invoke `/commit` with `docs: record adapter process completion` after checking this plan's boxes.
- [ ] Invoke `/pr --base main`; link plan 040 and state that it unblocks plan 041.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
