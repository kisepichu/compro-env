# Library Site Data Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generate schema-validated, privacy-safe static-site JSON from discovery, analysis, Git history, and latest verification state.

**Architecture:** A dedicated `site-schema` crate is the only public DTO authority. A separate `LibraryPlatformService` orchestrates discovery and analysis; a projection strips private data before atomically writing one build directory. No Node process runs from Rust.

**Tech Stack:** Rust 1.92.0, serde, schemars, chrono, git2, tempfile.

## Constraints

- **Branch:** `feat/051-library-site-data`
- **Depends on:** plan 056 merged to `main`.
- Read specification sections 12, 14, and 16.
- Canonical page IDs are `library:<repository-relative-library-id>` and `solution:<solution-id>`.
- A library with zero direct verifiers has aggregate status `never`, not vacuous `verified`.
- Production requires `git rev-parse --is-shallow-repository` to be `false`; missing history objects fail.
- Private paths, sources, symbols, diagnostics, and dependency counts must be absent, not redacted strings.

### Task 1: Define and generate the public schema

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `crates/site-schema/Cargo.toml`
- Create: `crates/site-schema/src/lib.rs`
- Create: `crates/site-schema/src/model.rs`
- Create: `crates/site-schema/src/schema.rs`
- Create: `crates/site-schema/tests/schema.rs`
- Create: `web/schema/site-data-v1.schema.json`

**Interfaces:**

```rust
pub const SITE_SCHEMA_VERSION: u32 = 1;
pub struct SiteData { /* build, languages, libraries, solutions */ }
pub struct LibraryPageData { /* public projection only */ }
pub struct SolutionPageData { /* public projection only */ }
pub fn site_data_schema() -> schemars::Schema;
```

- [ ] Write failing JSON/schema golden tests covering all verification and analysis states.
- [ ] Implement strict DTOs with no domain-only/private fields and generate the checked-in schema from Rust.
- [ ] Add a schema drift test and a serialized-key denylist for `private`, `token`, raw OJ data, and internal paths.
- [ ] Run `cargo test -p site-schema` and invoke `/commit` with `feat: define public library site schema`.

### Task 2: Project immutable snapshots to public pages

**Files:**
- Create: `crates/usecases/src/library_platform_service.rs`
- Create: `crates/usecases/src/site_data.rs`
- Create: `crates/usecases/tests/site_data.rs`
- Modify: `crates/usecases/src/lib.rs`

**Interfaces:**

```rust
pub struct LibraryPlatformService<D, A, V, G> { /* separate from Service */ }
pub fn project_site_data(input: PublicProjectionInput<'_>) -> Result<SiteData, SiteDataError>;
```

- [ ] Write failing three-language projection tests for public/private dependencies, cycles, locations,
      latest-only verification, `never`, rejected, unavailable, stale, and solution-entry diagnostics.
- [ ] Implement reverse/transitive projections from the immutable snapshot and remove private targets before DTO construction.
- [ ] Generate `#symbols` for locationless symbols and `#L<n>` for valid source locations.
- [ ] Run `cargo test -p usecases --test site_data`; invoke `/commit` with `feat: project public library site data`.

### Task 3: Add atomic `ce site-data generate`

**Files:**
- Create: `crates/infrastructure/src/git_history.rs`
- Create: `crates/infrastructure/src/repository_impl/site_data_repository_impl.rs`
- Modify: `crates/interfaces/src/controller.rs`
- Modify: `crates/interfaces/src/controller/input.rs`
- Modify: `crates/infrastructure/src/shell/commands.rs`
- Modify: `crates/infrastructure/src/shell/mod.rs`
- Create: `crates/infrastructure/tests/site_data_generate.rs`
- Create: `docs/commands/site-data.md`

- [ ] Write failing tests for full/shallow history, uncommitted preview, missing build set, schema failure,
      output replacement, interrupted staging, and proof that no Node/Astro/Pagefind executable starts.
- [ ] Implement `ce site-data generate --output target/ce-site-data` with production/preview modes and atomic directory swap.
- [ ] Include source SHA, schema version, observed toolchains, and generation mode in build metadata.
- [ ] Run `cargo test -p infrastructure site_data`; invoke `/commit` with `feat: generate static library site data`.

### Task 4: Deliver site data

- [ ] Generate the mixed fixture twice and compare bytes and schema validation.
- [ ] Run rollout repository verification and `git diff --check`.
- [ ] Invoke `/commit` with `docs: record site data completion`.
- [ ] Invoke `/pr --base main`; link plan 051 and state that it unblocks plan 052.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
