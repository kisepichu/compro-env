# Library Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add strict project-local library configuration, source/solution discovery, adapter protocol v1, and immutable normalized analysis snapshots without exposing a user-facing command.

**Architecture:** Keep value objects and normalized analysis state in focused `domain` modules, define the process-neutral JSON contract in a dependency-light workspace crate, and put TOML/filesystem parsing in `infrastructure`. A use-case pipeline validates protocol responses against discovery manifests and produces one immutable snapshot consumed by later site and verification plans.

**Tech Stack:** Rust 1.92.0, serde, serde_json, schemars, toml, globset, walkdir, chrono, sha2, tempfile.

## Global Constraints

- **Branch:** `feat/039-library-foundation`
- **Depends on:** PR #41 merged into `main`.
- Read specification sections 4, 5, 6.1-6.5, 16, and 17 before editing.
- Protocol `schema_version` is the integer `1`; request and response must match exactly.
- Rust, C++, and Lean are fixture values, never a closed Rust enum.
- Project `config.toml` is not merged with the existing user-global config.
- Discovery rejects symlinks and repository-escaping paths and sorts by UTF-8 path bytes.
- Private sources remain analysis inputs; publication is a projection performed later.
- Keep `domain::entity::Solution`, `usecases::Service`, `ConfigImpl`, and `SolutionRepository`
  unchanged; the library platform has separate models, orchestration, strict config, and discovery.
- Do not add CLI commands, spawn adapter processes, render HTML, or contact an OJ in this plan.

---

### Task 1: Create the canonical adapter protocol crate

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `crates/library-adapter-protocol/Cargo.toml`
- Create: `crates/library-adapter-protocol/src/lib.rs`
- Create: `crates/library-adapter-protocol/src/schema.rs`
- Create: `crates/library-adapter-protocol/tests/protocol.rs`
- Create: `tools/library-analyzers/protocol/analysis-v1.schema.json`
- Create: `tools/library-analyzers/protocol/fixtures/empty-request.json`
- Create: `tools/library-analyzers/protocol/fixtures/empty-response.json`
- Create: `tools/library-analyzers/protocol/fixtures/invalid-version-response.json`

**Interfaces:**
- Produces: `library_adapter_protocol::SCHEMA_VERSION: u32 = 1`.
- Produces: serde/schemars types `AnalysisRequest`, `AnalysisResponse`, `LibraryTarget`,
  `SolutionTarget`, `LibraryAnalysis`, `SolutionAnalysis`, `DependencyAnalysis`, `SymbolAnalysis`,
  `Dependency`, `Symbol`, `Diagnostic`, `Location`, `Position`, `AdapterIdentity`, and
  `ToolchainIdentity`.
- Produces: `schema::analysis_schema() -> schemars::Schema` and `schema::write_analysis_schema(&Path) -> anyhow::Result<()>`.

- [ ] **Step 1: Register the crate and write a failing round-trip test**

Add the workspace member and shared dependencies `schemars = "1"` and `sha2 = "0.10"`. Write this test shape in `tests/protocol.rs`:

```rust
#[test]
fn empty_protocol_fixture_round_trips() {
    let request: AnalysisRequest = serde_json::from_str(include_str!(
        "../../../tools/library-analyzers/protocol/fixtures/empty-request.json"
    ))
    .unwrap();
    assert_eq!(request.schema_version, SCHEMA_VERSION);
    assert!(request.libraries.is_empty());

    let response: AnalysisResponse = serde_json::from_str(include_str!(
        "../../../tools/library-analyzers/protocol/fixtures/empty-response.json"
    ))
    .unwrap();
    assert_eq!(response.schema_version, request.schema_version);
    assert!(response.solutions.is_empty());
}
```

- [ ] **Step 2: Run the focused test and observe the missing API**

Run: `cargo test -p library-adapter-protocol --test protocol empty_protocol_fixture_round_trips`

Expected: compilation fails because the protocol types and constant do not exist.

- [ ] **Step 3: Define strict protocol v1 types**

Use `#[serde(deny_unknown_fields)]` on structs and snake-case tagged enums. The central signatures are:

```rust
pub const SCHEMA_VERSION: u32 = 1;

pub struct AnalysisRequest {
    pub schema_version: u32,
    pub repository_root: String,
    pub language: String,
    pub libraries: Vec<LibraryTarget>,
    pub solutions: Vec<SolutionTarget>,
}

pub struct AnalysisResponse {
    pub schema_version: u32,
    pub adapter: AdapterIdentity,
    pub libraries: Vec<LibraryAnalysis>,
    pub solutions: Vec<SolutionAnalysis>,
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Dependency {
    Internal { path: String, location: Option<Location> },
    External { name: String, location: Option<Location> },
    Unresolved { key: String, display: String, location: Option<Location> },
}
```

Represent analysis state as `Complete`, `Partial`, or `Failed`; diagnostic severity as `Info`, `Warning`, or `Error`. Keep symbol `kind` as a validated string in the core rather than a protocol enum.

- [ ] **Step 4: Add strict invalid-version and unknown-field tests**

Deserialize `invalid-version-response.json`, then validate through a helper:

```rust
pub fn validate_version(actual: u32) -> Result<(), ProtocolVersionError> {
    if actual == SCHEMA_VERSION { Ok(()) } else { Err(ProtocolVersionError { actual }) }
}
```

Assert version `2` is rejected and an added unknown JSON key fails serde deserialization.

- [ ] **Step 5: Generate and compare the checked-in schema**

Add a test that writes `analysis_schema()` to a temporary file and compares normalized bytes with `analysis-v1.schema.json`. Regenerate the checked-in file through a small test-only example or `write_analysis_schema` call; do not hand-maintain a second schema.

Run: `cargo test -p library-adapter-protocol`

Expected: all protocol, fixture, version, and schema-drift tests pass.

- [ ] **Step 6: Commit the protocol boundary**

Invoke `/commit` with explicit protocol/workspace paths and message:

```text
feat: define strict library adapter protocol
```

### Task 2: Add focused domain models and validated IDs

**Files:**
- Modify: `crates/domain/src/lib.rs`
- Create: `crates/domain/src/library.rs`
- Create: `crates/domain/src/analysis.rs`
- Create: `crates/domain/src/solution.rs`

**Interfaces:**
- Produces: `LanguageId::parse(&str) -> Result<LanguageId, IdError>`.
- Produces: `LibraryId::parse(&str) -> Result<LibraryId, IdError>`.
- Produces: `SolutionId::parse(&str) -> Result<SolutionId, IdError>`.
- Produces: `LibraryProjectConfig`, `LanguageConfig`, `AnalyzerConfig`, `SiteConfig`, and `ExpectedToolchain`.
- Produces: `LibraryFile`, `PublishedSolution`, `DiscoveryManifest`, `AnalysisSnapshot`, and `TargetAnalysisState`.

- [ ] **Step 1: Write failing ID and publication tests**

Add unit tests proving:

```rust
assert_eq!(LanguageId::parse("cpp-23").unwrap().as_str(), "cpp-23");
assert!(LanguageId::parse("C++").is_err());
assert_eq!(LibraryId::parse("libraries/rust/a.rs").unwrap().as_str(), "libraries/rust/a.rs");
assert!(LibraryId::parse("../private.rs").is_err());
```

Also construct a private `LibraryFile` and assert `managed == true` while `published == false`.

- [ ] **Step 2: Run the domain tests and observe missing modules**

Run these focused filters separately:

```bash
cargo test -p domain library::tests
cargo test -p domain analysis::tests
cargo test -p domain solution::tests
```

Expected: compilation fails because the modules and types are absent.

- [ ] **Step 3: Implement IDs and model boundaries**

Use newtypes with private strings. Validate language IDs against `[a-z][a-z0-9-]*`; validate repository paths component-by-component without Unicode normalization or case folding. Define:

```rust
pub struct DiscoveryManifest {
    pub languages: BTreeMap<LanguageId, DiscoveredLanguage>,
    pub libraries: Vec<LibraryFile>,
    pub solutions: Vec<PublishedSolution>,
}

pub struct AnalysisSnapshot {
    pub schema_version: u32,
    pub repository_revision: String,
    pub discovery_hash: String,
    pub source_hashes: BTreeMap<String, String>,
    pub languages: BTreeMap<LanguageId, NormalizedLanguageAnalysis>,
    pub snapshot_hash: String,
}
```

Keep `domain::entity::Solution` unchanged for current CLI commands; the new `PublishedSolution` represents discovery metadata and avoids a broad refactor.

- [ ] **Step 4: Add stable-order and state-separation tests**

Prove libraries sort by ID bytes, dependency and symbol states remain independent, and cyclic direct edges are representable without graph traversal.

Run: `cargo test -p domain`

Expected: all old and new domain tests pass.

- [ ] **Step 5: Commit the domain boundary**

Invoke `/commit` with message:

```text
feat: model library discovery and analysis state
```

### Task 3: Parse strict project-local library configuration

**Files:**
- Modify: `crates/infrastructure/Cargo.toml`
- Modify: `crates/infrastructure/src/lib.rs`
- Create: `crates/infrastructure/src/library_project/mod.rs`
- Create: `crates/infrastructure/src/library_project/config.rs`
- Create: `crates/infrastructure/tests/fixtures/library-project/config-valid.toml`
- Create: `crates/infrastructure/tests/fixtures/library-project/config-unknown-key.toml`

**Interfaces:**
- Produces: `ProjectLibraryConfigLoader::load(repository_root: &Path) -> anyhow::Result<LibraryProjectConfig>`.
- Consumes: domain configuration types from Task 2.

- [ ] **Step 1: Write failing strict-config tests**

Cover a valid three-language config and assertions for missing root/include/analyzer command, empty command arrays, unknown keys, invalid language IDs, invalid timeouts, duplicate toolchains, and missing production site metadata.

```rust
let config = ProjectLibraryConfigLoader::load(fixture_root()).unwrap();
assert_eq!(config.languages.keys().map(LanguageId::as_str).collect::<Vec<_>>(),
           vec!["cpp", "lean", "rust"]);
assert_eq!(config.languages[&LanguageId::parse("rust").unwrap()].analyzer.timeout_seconds, 600);
```

- [ ] **Step 2: Run the focused tests and observe the missing loader**

Run: `cargo test -p infrastructure library_project::config`

Expected: compilation fails because `ProjectLibraryConfigLoader` does not exist.

- [ ] **Step 3: Implement serde-backed strict parsing**

Parse only `<repository_root>/config.toml`; do not call `ConfigImpl` or inspect `CE_CONFIG_DIR`. Use private raw `Deserialize` structs with `deny_unknown_fields`, then validate into domain types. Root/analyzer relative paths stay repository-relative and commands remain argv arrays.

- [ ] **Step 4: Prove global config is ignored**

Set `CE_CONFIG_DIR` to a fixture containing a conflicting `[library]` section and assert the repository config wins unchanged.

Run: `cargo test -p infrastructure library_project::config`

Expected: all strict config tests pass without reading the user-global file.

- [ ] **Step 5: Commit project configuration**

Invoke `/commit` with message:

```text
feat: parse project-local library configuration
```

### Task 4: Discover managed libraries and metadata

**Files:**
- Modify: `crates/infrastructure/src/library_project/mod.rs`
- Create: `crates/infrastructure/src/library_project/discovery.rs`
- Create: `crates/infrastructure/src/library_project/metadata.rs`
- Create: `crates/infrastructure/tests/library_discovery.rs`
- Create: `crates/infrastructure/tests/fixtures/library-project/libraries/rust/public.rs`
- Create: `crates/infrastructure/tests/fixtures/library-project/libraries/rust/public.rs.md`
- Create: `crates/infrastructure/tests/fixtures/library-project/libraries/rust/private.rs`
- Create: `crates/infrastructure/tests/fixtures/library-project/libraries/rust/private.rs.md`
- Create: `crates/infrastructure/tests/fixtures/library-project/libraries/cpp/monoid.hpp`
- Create: `crates/infrastructure/tests/fixtures/library-project/libraries/lean/Monoid.lean`

**Interfaces:**
- Produces: `LibraryDiscovery::discover(&Path, &LibraryProjectConfig) -> anyhow::Result<DiscoveryManifest>`.
- Produces: `parse_library_sidecar(&Path) -> anyhow::Result<LibraryMetadata>` and `parse_directory_index(&Path) -> anyhow::Result<DirectoryMetadata>`.

- [ ] **Step 1: Write failing mixed-language discovery tests**

Assert the fixture returns four libraries ordered by repository path, retains `private.rs` as managed, marks only that file unpublished, and associates `source.ext.md` without treating it as source.

- [ ] **Step 2: Run the discovery test and observe the missing implementation**

Run: `cargo test -p infrastructure --test library_discovery`

Expected: compilation fails because `LibraryDiscovery` is absent.

- [ ] **Step 3: Implement include/exclude enumeration**

Use `walkdir` without following links and `globset` against `/`-separated paths relative to each language root. Reject missing roots and symlink candidates, warn through a returned diagnostics collection for zero matches, and sort final IDs by raw UTF-8 bytes.

- [ ] **Step 4: Implement strict TOML frontmatter parsing**

Support sidecar keys `title`, `publish`, `relations`, and `dependency_overrides`; support only `title` in `_index.md`. Require `+++` TOML fences when frontmatter exists and reject orphan sidecars, malformed TOML, empty titles, and unknown keys.

- [ ] **Step 5: Add failure fixtures**

Create temporary tests for excluded files, orphan sidecars, malformed frontmatter, symlinked files/directories, root escape attempts, duplicate relations, and `_index.md` unknown keys.

Run: `cargo test -p infrastructure --test library_discovery`

Expected: success and stable results across reversed fixture creation order.

- [ ] **Step 6: Commit library discovery**

Invoke `/commit` with message:

```text
feat: discover managed library sources
```

### Task 5: Discover publishable solutions and verification metadata

**Files:**
- Modify: `crates/infrastructure/src/library_project/discovery.rs`
- Create: `crates/infrastructure/src/library_project/solution_metadata.rs`
- Create: `crates/infrastructure/tests/solution_discovery.rs`
- Create: `crates/infrastructure/tests/fixtures/library-project/solutions/librarychecker-aplusb/aplusb/main/ce.toml`
- Create: `crates/infrastructure/tests/fixtures/library-project/solutions/librarychecker-aplusb/.ce.toml`
- Create: `crates/infrastructure/tests/fixtures/library-project/solutions/abc999/a/private/ce.toml`

**Interfaces:**
- Produces: strict parsing of `publish`, RFC 3339 `solved_at`, `test_command`, `test_timeout_seconds`, and optional `[verify]` into `PublishedSolution`.
- Produces: solution ID `{contest_id}/{problem_code}/{solution_name}` and entry path from language configuration.

- [ ] **Step 1: Write failing opt-in publication tests**

Assert the LibraryChecker fixture is public with an explicit `solved_at`, while the private fixture is omitted from adapter solution targets. Assert `[verify]` on a private solution is rejected.

- [ ] **Step 2: Run the focused tests and observe missing solution parsing**

Run: `cargo test -p infrastructure --test solution_discovery`

Expected: tests fail because solution discovery is not wired.

- [ ] **Step 3: Parse solution and contest metadata**

Read each `solutions/<contest>/<problem>/<solution>/ce.toml` and the contest `.ce.toml`. Preserve current CLI keys while rejecting unknown library-publication keys. Require timezone-bearing RFC 3339 `solved_at` for public or verified solutions; do not infer time from Git or filesystems.

- [ ] **Step 4: Validate verify references and orphan results structurally**

Require non-empty unique `[verify].libraries`, explicit or project-mapped OJ language ID, and public
direct targets. Define the path expected under `verification/results/<solution-id>.json`; actual result
parsing belongs to plan 055.

Run: `cargo test -p infrastructure --test solution_discovery`

Expected: public/private, invalid time, missing test command for verify, and unknown-language cases all pass.

- [ ] **Step 5: Commit solution discovery**

Invoke `/commit` with message:

```text
feat: discover public and verified solutions
```

### Task 6: Normalize fixture responses into an immutable snapshot

**Files:**
- Modify: `crates/usecases/Cargo.toml`
- Modify: `crates/usecases/src/lib.rs`
- Create: `crates/usecases/src/library_analysis.rs`
- Create: `crates/usecases/tests/library_analysis.rs`
- Create: `crates/usecases/tests/fixtures/mixed-analysis-response.json`

**Interfaces:**
- Produces: `normalize_analysis(manifest: &DiscoveryManifest, responses: BTreeMap<LanguageId, AnalysisResponse>, revision: &str, source_bytes: &BTreeMap<String, Vec<u8>>) -> anyhow::Result<AnalysisSnapshot>`.
- Produces: direct internal edges only; reverse edges and transitive closure are pure methods on `AnalysisSnapshot`.
- Consumes: protocol v1 responses and Task 2 domain models.

- [ ] **Step 1: Write a failing three-language normalization test**

Load one response for each language, normalize them, and assert source hashes, direct edges, reverse edges, a cyclic closure, independent dependency/symbol state, observed toolchains, and a stable snapshot hash.

- [ ] **Step 2: Run the focused test and observe the missing pipeline**

Run: `cargo test -p usecases --test library_analysis`

Expected: compilation fails because `normalize_analysis` is absent.

- [ ] **Step 3: Validate response completeness before normalization**

Reject schema mismatch, wrong adapter language target sets, missing/extra/duplicate libraries or solutions, unsafe paths, internal dependencies outside the same language manifest, duplicate toolchain names, invalid locations, and transitive fixture edges marked as direct-contract violations.

- [ ] **Step 4: Normalize and hash deterministic state**

Sort all maps/arrays by the specification, SHA-256 source bytes and canonical JSON, derive reverse edges and cycle-safe closures, and keep adapter diagnostics separate from public projection. Do not add a cross-run cache.

- [ ] **Step 5: Add negative and stability tests**

Prove shuffled inputs have the same snapshot hash and test every rejection from Step 3. Prove changing only adapter/toolchain identity leaves source closure hashes unchanged while observed identity remains recorded.

Run: `cargo test -p usecases --test library_analysis`

Expected: all mixed-language, malformed-response, and deterministic-hash tests pass.

- [ ] **Step 6: Commit snapshot normalization**

Invoke `/commit` with message:

```text
feat: normalize immutable library analysis snapshots
```

### Task 7: Verify and deliver the foundation PR

**Files:**
- Modify: `docs/superpowers/plans/2026-08-10-library-foundation.md`

- [ ] **Step 1: Run the plan integration suite**

```bash
cargo test -p library-adapter-protocol
cargo test -p domain
cargo test -p infrastructure library_project
cargo test -p infrastructure --test library_discovery --test solution_discovery
cargo test -p usecases --test library_analysis
```

Expected: all commands exit 0.

- [ ] **Step 2: Run repository-wide verification**

```bash
cargo test --all
cargo clippy --all --all-features -- -D warnings
cargo fmt --all --check
git diff --check origin/main...HEAD
```

Expected: all commands exit 0; no CLI behavior changes.

- [ ] **Step 3: Commit checked plan progress**

Invoke `/commit` with message:

```text
docs: record library foundation completion
```

- [ ] **Step 4: Open and review the PR**

Invoke `/pr --base main`, then `/pr-review` until Copilot reports no comments or no new
comments. Merge only after CI succeeds. This PR unblocks plans 040 and 054.
