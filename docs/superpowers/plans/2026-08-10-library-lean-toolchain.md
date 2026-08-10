# Lean Library Toolchain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pin Lean 4.30.0/Lake, build the repository-local Lean adapter offline, and pass the normal protocol handshake.

**Architecture:** Preparation installs a verified official Lean release into the content-addressed set. Lake uses explicit repository-local package/build directories and a committed manifest. The first Lean executable handles empty requests and reports exact toolchain identity.

**Tech Stack:** Lean 4.30.0, Lake, batteries, Std, JSON protocol.

## Constraints

- **Branch:** `feat/048-library-lean-toolchain`
- **Depends on:** plan 047 merged to `main`.
- Read specification sections 6.8 and 6.9.
- Pin `lean-toolchain` to `leanprover/lean4:v4.30.0` and commit `lake-manifest.json`.
- Prepared official archives and SHA-256 are:
  - macOS aarch64 `lean-4.30.0-darwin_aarch64.tar.zst`:
    `072dca4a38fbc0d3cedb96fea886cc243b424f2bd16247596200b9a9ab93f0f5`.
  - Linux x86_64 `lean-4.30.0-linux.tar.zst`:
    `4dad74141c2c119ca1aa626656be83b8e14238afba97271fd7bf1eb3f081b319`.
  - Linux aarch64 `lean-4.30.0-linux_aarch64.tar.zst`:
    `c99c6f0edd446956d4758c59d4383e8e6411ff6cc71a01f9caabe5eba454121d`.
- Never use ambient Elan, `.lake`, user package caches, or network during build/analysis.

### Task 1: Prepare exact Lean and Lake inputs

**Files:**
- Create: `tools/library-analyzers/lean/lean-toolchain`
- Create: `tools/library-analyzers/lean/lakefile.toml`
- Create: `tools/library-analyzers/lean/lake-manifest.json`
- Modify: `tools/library-analyzers/dependencies.toml`
- Modify: `tools/library-analyzers/build-inputs.toml`
- Create: `crates/infrastructure/tests/lean_toolchain.rs`

- [ ] Write failing selection tests for the three supported triples, unsupported triples, wrong digest,
      exact `lean --version`, exact `lake --version`, and missing locked package content.
- [ ] Add official HTTPS artifacts/digests and validate extracted executable/package layout.
- [ ] Configure Lake `packagesDir` and `buildDir` under the prepared/build staging directories.
- [ ] Run `cargo test -p infrastructure --test lean_toolchain`.
- [ ] Invoke `/commit` with `build: pin Lean 4.30.0 for library analysis`.

### Task 2: Build a handshaking Lean adapter

**Files:**
- Create: `tools/library-analyzers/lean/Analyzer/Main.lean`
- Create: `tools/library-analyzers/lean/Analyzer/Protocol.lean`
- Create: `tools/library-analyzers/lean/Analyzer/Diagnostics.lean`
- Create: `tools/library-analyzers/lean/Tests/Handshake.lean`

- [ ] Write a failing empty-request test expecting adapter `ce-lean`, protocol v1, toolchain
      `lean=4.30.0`, and empty target arrays.
- [ ] Implement one UTF-8 JSON document from stdin to stdout with strict version/field validation.
- [ ] Compile only through prepared `lake build --no-build`/offline resolution as encoded by the build driver.
- [ ] Run `tools/library-analyzers/prepare && tools/library-analyzers/build`; expect all handshakes to pass.
- [ ] Invoke `/commit` with `feat: add Lean analyzer protocol executable`.

### Task 3: Deliver the Lean toolchain boundary

- [ ] Run Lean handshake tests and the generic adapter build/process suites.
- [ ] Run rollout repository verification and `git diff --check`.
- [ ] Invoke `/commit` with `docs: record Lean toolchain completion`.
- [ ] Invoke `/pr --base main`; link plan 048 and state that it unblocks plan 049.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
