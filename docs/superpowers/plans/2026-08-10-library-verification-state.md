# Library Verification State Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define OJ capabilities and the complete versioned verification record lifecycle, then persist latest records atomically with attempt-level compare-and-swap.

**Architecture:** Domain modules own normalized verdicts/capabilities/states. A narrow repository port stores one latest JSON record per discovered solution. Infrastructure rejects unsafe/orphan paths and atomically replaces only the expected attempt.

**Tech Stack:** Rust 1.92.0, serde, chrono, uuid, tempfile, fsync.

## Constraints

- **Branch:** `feat/055-library-verification-state`
- **Depends on:** plan 054 merged to `main`.
- Read specification sections 8-11.
- `Unavailable` is a terminal non-success result: `ce verify` exits failed while Web displays unavailable.
- Persist latest only; do not create history pages or retain result history in the public model.
- Never serialize sessions, tokens, headers, cookies, raw responses, or unrestricted OJ fields.
- Reuse plan 039's `LibraryId`, `SolutionId`, and `LanguageId`; define only verification-specific
  `AttemptId`, `ContentHash`, and `VerifyFingerprint` newtypes here.

### Task 1: Define capabilities, verdicts, and states

**Files:**
- Create: `crates/domain/src/online_judge.rs`
- Create: `crates/domain/src/verification.rs`
- Modify: `crates/domain/src/lib.rs`
- Create: `crates/domain/tests/verification_golden.rs`
- Create: `crates/domain/tests/fixtures/verification/accepted.json`
- Create: `crates/domain/tests/fixtures/verification/rejected.json`
- Create: `crates/domain/tests/fixtures/verification/unavailable.json`
- Create: `crates/domain/tests/fixtures/verification/pending.json`
- Create: `crates/domain/tests/fixtures/verification/infrastructure-failure.json`

**Interfaces:**

```rust
pub struct SubmissionCapabilities {
    pub submission_mode: SubmissionMode,
    pub result_detail: ResultDetail,
    pub recovery_mode: RecoveryMode,
}
pub struct VerificationRecord {
    pub schema_version: u32,
    pub solution_id: SolutionId,
    pub attempt_id: AttemptId,
    pub replaces_attempt_id: Option<AttemptId>,
    pub fingerprint: VerifyFingerprint,
    pub state: VerificationState,
}
pub enum VerificationState {
    Starting(StartingState), AcceptanceUnknown(AcceptanceUnknownState),
    Submitted(SubmittedState), Queued(PendingState), Judging(PendingState),
    InfrastructureFailure(InfrastructureFailure), Completed(CompletedState),
    Unavailable(UnavailableState),
}
```

- [x] Write failing golden tests for every state/verdict, optional metrics, null versus empty test cases,
      capability combinations, unknown raw verdict preservation, and public `extra` allowlist.
- [x] Implement validated ID/hash newtypes and strict versioned serde models.
- [x] Run `cargo test -p domain verification`; invoke `/commit` with `feat: model verification state lifecycle`.

### Task 2: Persist records with atomic CAS

**Files:**
- Create: `crates/usecases/src/repository/verification_repository.rs`
- Modify: `crates/usecases/src/repository/mod.rs`
- Create: `crates/infrastructure/src/repository_impl/verification_repository_impl.rs`
- Modify: `crates/infrastructure/src/repository_impl/mod.rs`
- Create: `crates/infrastructure/tests/verification_repository.rs`
- Create: `crates/infrastructure/tests/fixtures/verification/accepted.json`
- Create: `crates/infrastructure/tests/fixtures/verification/stale-attempt.json`

**Interfaces:**

```rust
pub trait VerificationRepository {
    fn load(&self, id: &SolutionId) -> Result<Option<VerificationRecord>>;
    fn load_all(&self, discovered: &BTreeSet<SolutionId>)
        -> Result<BTreeMap<SolutionId, VerificationRecord>>;
    fn compare_and_swap(
        &self, id: &SolutionId, expected: Option<&AttemptId>, next: &VerificationRecord,
    ) -> Result<()>;
    fn remove_if_attempt(&self, id: &SolutionId, expected: &AttemptId) -> Result<()>;
}
```

- [x] Write failing tests for canonical `verification/results/{contest}/{problem}/{solution}.json`,
      traversal, symlink, orphan/non-JSON rejection, initial/replace/conflict CAS, fsync/rename, and removal.
- [x] Implement same-directory temporary writes and compare current attempt to `replaces_attempt_id`.
- [x] Run `cargo test -p infrastructure --test verification_repository`.
- [x] Invoke `/commit` with `feat: persist latest verification records atomically`.

### Task 3: Deliver verification state

- [x] Re-run golden/repository tests with reversed input creation order.
- [x] Run rollout repository verification and `git diff --check`.
- [x] Invoke `/commit` with `docs: record verification state completion`.
- [ ] Invoke `/pr --base main`; link plan 055 and state that it unblocks plan 056.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
