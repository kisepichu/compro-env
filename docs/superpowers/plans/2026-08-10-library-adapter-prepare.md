# Library Adapter Prepare Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prepare pinned public adapter dependencies and toolchains into a verified content-addressed cache.

**Architecture:** A repository-local launcher calls a Rust driver. The driver resolves no versions: it validates checked-in pins, downloads public HTTPS artifacts to staging, verifies digests and archive safety, then atomically publishes a prepared set.

**Tech Stack:** Rust 1.92.0, reqwest rustls, sha2, tar, zip, flate2, tempfile, fs2.

## Constraints

- **Branch:** `feat/041-library-adapter-prepare`
- **Depends on:** plan 040 merged to `main`.
- Read specification section 6.9.
- Network is permitted only in `prepare`; all sources are public HTTPS with immutable revision/checksum.
- Never run downloaded installer scripts or persist proxy credentials, headers, tokens, or cookies.
- Reject absolute paths, `..`, symlinks, hard links, devices, sockets, repository-external local paths,
  mutable Git refs, HTTP, SSH, SCP-style URLs, and URL userinfo.

### Task 1: Parse and validate preparation inputs

**Files:**
- Create: `crates/infrastructure/src/library_adapter/prepared.rs`
- Create: `crates/infrastructure/src/library_adapter/archive.rs`
- Create: `crates/infrastructure/tests/adapter_prepared.rs`
- Create: `tools/library-analyzers/dependencies.toml`

**Interfaces:**

```rust
pub fn load_dependency_manifest(root: &Path) -> Result<DependencyManifest, PrepareError>;
pub fn expected_dependency_id(
    manifest: &DependencyManifest,
    platform: &TargetPlatform,
) -> Result<DependencyId, PrepareError>;
pub fn validate_prepared_set(path: &Path, expected: &ExpectedPreparedSet)
    -> Result<PreparedSet, PrepareError>;
```

- [x] Write failing tests for URL policy, full Git SHA, digest syntax, stable dependency ID, local input
      hashing, platform mismatch, incomplete directory, and manifest mismatch.
- [x] Run `cargo test -p infrastructure --test adapter_prepared`; observe missing APIs.
- [x] Implement strict parsing and content-addressed identity without performing network I/O.
- [x] Re-run the focused tests and invoke `/commit` with `feat: validate adapter preparation inputs`.

### Task 2: Download and atomically publish prepared sets

**Files:**
- Create: `crates/infrastructure/src/bin/library-adapter-prepare.rs`
- Create: `crates/infrastructure/src/library_adapter/download.rs`
- Create: `crates/infrastructure/tests/adapter_prepare.rs`
- Create: `tools/library-analyzers/prepare`

**Interfaces:**

```rust
pub fn prepare_dependencies(request: &PrepareRequest) -> Result<PreparedSet, PrepareError>;
```

- [x] Start a local HTTP fixture server and write failing tests for checksum mismatch, truncated download,
      unsafe tar/zip members, redirect to non-HTTPS policy, concurrent lock failure, and atomic success.
- [x] Implement staging under `target/library-analyzers/prepared`, fail-fast `prepare.lock`, bounded download,
      digest verification, safe extraction, manifest fsync, and rename-on-success.
- [x] Permit only documented CA/proxy variables in the fetch environment and redact their values from errors.
- [x] Run `cargo test -p infrastructure --test adapter_prepare`; assert failure leaves no cache hit.
- [x] Invoke `/commit` with `feat: prepare pinned adapter dependencies`.

### Task 3: Deliver preparation

- [x] Run `tools/library-analyzers/prepare --check` against checked-in inputs; expect validation success without mutation.
- [x] Run rollout repository verification and `git diff --check`.
- [x] Invoke `/commit` with `docs: record adapter preparation completion` after checking plan progress.
- [ ] Invoke `/pr --base main`; link plan 041 and state that it unblocks plan 042.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
