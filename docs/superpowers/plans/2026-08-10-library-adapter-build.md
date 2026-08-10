# Library Adapter Build Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build all declared adapters offline, validate them with the normal protocol, and atomically publish one complete build set.

**Architecture:** A generic Rust build driver consumes prepared caches and declarative language commands. It stages every executable, performs empty-request handshakes through the shared runner, writes a hashed manifest, then switches `target/library-analyzers/bin` atomically.

**Tech Stack:** Rust 1.92.0, fs2, sha2, tempfile, library-adapter-protocol.

## Constraints

- **Branch:** `feat/042-library-adapter-build`
- **Depends on:** plan 041 merged to `main`.
- Read specification sections 6.9 and 14.
- Build and handshake are offline and secretless; no fallback to global Cargo, Lean, Lake, or Clang caches.
- Do not delete old build/prepared sets and do not add a force option.
- A marker blocks analysis even when an older build exists; failures never switch `bin`.

### Task 1: Validate build state and freshness

**Files:**
- Create: `crates/infrastructure/src/library_adapter/build_state.rs`
- Create: `crates/infrastructure/tests/adapter_build_state.rs`

**Interfaces:**

```rust
pub fn inspect_build_state(root: &Path, expected: &ExpectedBuild)
    -> Result<UsableBuildSet, BuildStateError>;
pub fn derive_build_id(manifest: &UnsignedBuildManifest) -> Result<BuildId, BuildStateError>;
```

- [ ] Write failing tests for missing executable, wrong hash, stale input digest, marker with held/free lock,
      bad symlink target, duplicate adapter identity, and deterministic build ID.
- [ ] Implement manifest/hash/symlink validation and distinct `BuildRunning` versus `PreviousBuildFailed` errors.
- [ ] Run `cargo test -p infrastructure --test adapter_build_state`; expect all cases to pass.
- [ ] Invoke `/commit` with `feat: validate adapter build state`.

### Task 2: Build, handshake, and publish atomically

**Files:**
- Create: `crates/infrastructure/src/library_adapter/build.rs`
- Create: `crates/infrastructure/src/bin/library-adapter-build.rs`
- Create: `crates/infrastructure/tests/adapter_build.rs`
- Create: `tools/library-analyzers/build`
- Modify: `tools/library-analyzers/build-inputs.toml`

**Interfaces:**

```rust
pub fn build_adapters(request: &BuildRequest) -> Result<UsableBuildSet, BuildError>;
pub fn handshake_adapter(
    runner: &dyn LibraryAdapterRunner,
    executable: &Path,
    language: &LanguageId,
) -> Result<AdapterIdentity, BuildError>;
```

- [ ] Write fake-adapter tests for stable language order, missing prepared set, sanitized environment,
      nonzero build, handshake mismatch, crash recovery, concurrent lock, and successful atomic switch.
- [ ] Implement fail-fast `build.lock`, persistent `build-in-progress`, unique staging, offline environment,
      executable hashing, manifest fsync, rename, and atomic relative symlink replacement.
- [ ] Use the normal empty `AnalysisRequest`; do not introduce a second handshake schema.
- [ ] Run `cargo test -p infrastructure --test adapter_build`; expect failure paths to retain marker and old set.
- [ ] Invoke `/commit` with `feat: publish complete offline adapter builds`.

### Task 3: Deliver adapter build infrastructure

- [ ] Run `cargo test -p infrastructure adapter_build` and `tools/library-analyzers/build --check`.
- [ ] Run rollout repository verification and `git diff --check`.
- [ ] Invoke `/commit` with `docs: record adapter build completion` after checking plan progress.
- [ ] Invoke `/pr --base main`; link plan 042 and state that it unblocks plan 043.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
