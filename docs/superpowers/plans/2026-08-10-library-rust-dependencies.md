# Rust Library Dependency Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the Rust protocol executable and resolve direct internal/external module dependencies for library and solution files.

**Architecture:** A repository-local Rust binary consumes only protocol targets and an explicit Cargo workspace model. It parses Rust syntax with `syn`, resolves `mod`, `use`, and qualified paths against discovered files, and emits direct edges without understanding Web DTOs.

**Tech Stack:** Rust 1.92.0, syn, cargo_metadata, library-adapter-protocol.

## Constraints

- **Branch:** `feat/043-library-rust-dependencies`
- **Depends on:** plan 042 merged to `main`.
- Read specification section 6.6 and the protocol contract in 6.1-6.5.
- Pin `rust-toolchain.toml` to `1.92.0`; remove ambient `RUSTUP_TOOLCHAIN` from child environments.
- Emit direct edges only. Do not emit symbols in this plan; return `symbol_analysis = partial` with a stable diagnostic.
- Macro expansion and build-script execution are outside MVP; unresolved dynamic references must be explicit.

### Task 1: Add a handshaking Rust adapter

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `rust-toolchain.toml`
- Create: `tools/library-analyzers/rust/Cargo.toml`
- Create: `tools/library-analyzers/rust/src/main.rs`
- Create: `tools/library-analyzers/rust/src/request.rs`
- Modify: `tools/library-analyzers/build-inputs.toml`
- Modify: `tools/library-analyzers/dependencies.toml`

- [x] Write a process fixture test that sends an empty Rust request and expects protocol v1, adapter
      identity `ce-rust`, toolchain `rustc=1.92.0`, and empty target arrays.
- [x] Run `cargo test -p infrastructure rust_adapter_handshake`; observe missing executable/build input.
- [x] Implement stdin JSON parsing, strict request validation, exact `rustc -Vv` normalization, and stdout JSON.
- [x] Run `tools/library-analyzers/prepare`, then `tools/library-analyzers/build`; expect handshake success.
- [x] Invoke `/commit` with `feat: add Rust analyzer protocol executable`.

### Task 2: Resolve direct Rust dependencies

**Files:**
- Create: `tools/library-analyzers/rust/src/dependencies.rs`
- Create: `tools/library-analyzers/rust/src/module_graph.rs`
- Create: `tools/library-analyzers/rust/tests/dependencies.rs`
- Create: `tools/library-analyzers/protocol/fixtures/rust-dependencies-request.json`
- Create: `tools/library-analyzers/protocol/fixtures/rust-dependencies-response.json`

**Interfaces:**

```rust
pub fn analyze_dependencies(
    request: &AnalysisRequest,
    workspace: &RustWorkspace,
) -> Vec<TargetDependencyAnalysis>;
```

- [x] Write failing fixtures for `mod`, `#[path]`, grouped/aliased/glob `use`, `crate/self/super`,
      qualified paths, external crates, cycles, cfg-inactive items, and unresolved macro-generated paths.
- [x] Implement deterministic module ownership and same-language internal path resolution.
- [x] Mark analysis `partial` whenever an active source reference cannot be resolved uniquely.
- [x] Run `cargo test -p ce-library-rust-analyzer dependencies`; compare checked-in JSON exactly.
- [x] Invoke `/commit` with `feat: analyze direct Rust library dependencies`.

### Task 3: Deliver Rust dependencies

- [ ] Run the Rust fixture through the generic process runner and normalized snapshot tests.
- [ ] Run rollout repository verification and `git diff --check`.
- [ ] Invoke `/commit` with `docs: record Rust dependency adapter completion`.
- [ ] Invoke `/pr --base main`; link plan 043 and state that it unblocks plan 044.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
