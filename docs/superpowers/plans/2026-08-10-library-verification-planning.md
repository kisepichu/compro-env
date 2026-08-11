# Library Verification Planning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compute dependency-aware fingerprints/statuses and immutable submission plans, and validate every lifecycle transition before any OJ integration.

**Architecture:** Pure use-case modules consume the immutable analysis snapshot and saved latest record. Canonical hashing creates explainable fingerprint inputs; plan creation freezes source bytes and attempt identity so later start/poll jobs never reread the working tree.

**Tech Stack:** Rust 1.92.0, sha2, canonical serde_json, uuid, chrono.

## Constraints

- **Branch:** `feat/056-library-verification-planning`
- **Depends on:** plan 055 merged to `main`.
- Read specification sections 8.1, 10, and 11.
- New submissions are scheduled only for `never` or `stale`; no force option.
- Dependency analysis partial/failed blocks fingerprinting; symbol failure does not.
- Current rejected/unavailable results are terminal and are not automatically resubmitted.

### Task 1: Compute closure, fingerprints, and statuses

**Files:**
- Create: `crates/usecases/src/verification/mod.rs`
- Create: `crates/usecases/src/verification/fingerprint.rs`
- Create: `crates/usecases/src/verification/status.rs`
- Modify: `crates/usecases/src/lib.rs`

**Interfaces:**

```rust
pub fn verification_closure(
    explicit: &BTreeSet<LibraryId>,
    solution_dependencies: &DependencyAnalysis,
    graph: &DependencyGraph,
) -> Result<BTreeSet<LibraryId>, FingerprintError>;
pub fn calculate_fingerprint(material: &FingerprintMaterial) -> Result<VerifyFingerprint>;
pub fn classify_solution_status(
    verify_spec: Option<&VerifySpec>,
    current: Result<&VerifyFingerprint, FingerprintError>,
    saved: Option<&VerificationRecord>,
) -> VerificationStatus;
```

- [x] Write failing tests for cycles, private closure, stable byte order, source bytes without newline
      normalization, rename/content/mapping/capability changes, symbol-only failure, and status precedence.
- [x] Implement field-framed canonical hashing with per-input hashes for stale reasons.
- [x] Aggregate libraries from direct verifiers only; zero direct verifiers is `never`.
- [x] Run `cargo test -p usecases verification::fingerprint` and
      `cargo test -p usecases verification::status`.
- [x] Invoke `/commit` with `feat: derive verification fingerprints and statuses`.

### Task 2: Build immutable plans and validate transitions

**Files:**
- Create: `crates/usecases/src/verification/plan.rs`
- Create: `crates/usecases/src/verification/transition.rs`
- Create: `crates/usecases/src/clock.rs`
- Create: `crates/usecases/src/id_generator.rs`

**Interfaces:**

```rust
pub fn build_submission_plan(
    input: PrepareVerificationInput<'_>,
    clock: &dyn Clock,
    ids: &dyn AttemptIdGenerator,
) -> Result<SubmissionPlan>;
pub fn apply_transition(
    current: &VerificationRecord,
    event: VerificationEvent,
) -> Result<VerificationRecord, InvalidTransition>;
```

- [x] Write failing tests for stable plan JSON/hash, frozen submitted source, `replaces_attempt_id`, every
      valid transition, stale events, handle preservation, and every forbidden backward/attempt transition.
- [x] Implement immutable `SubmissionPlanBody` plus plan hash and exhaustive transition matching.
- [x] Add hidden in-process prepare serialization helpers, but no public CLI command.
- [x] Run `cargo test -p usecases verification::plan` and
      `cargo test -p usecases verification::transition`.
- [x] Invoke `/commit` with `feat: build immutable verification plans`.

### Task 3: Deliver verification planning

- [x] Run all fingerprint/status/plan/transition fixtures twice and compare canonical bytes.
- [x] Run rollout repository verification and `git diff --check`.
- [x] Invoke `/commit` with `docs: record verification planning completion`.
- [ ] Invoke `/pr --base main`; link plan 056 and state that it unblocks plan 057.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
