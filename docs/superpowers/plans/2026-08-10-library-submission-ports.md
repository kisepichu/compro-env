# Library Submission Ports Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Separate submission start, result polling, and recovery into reusable ports while preserving existing `ce submit` behavior.

**Architecture:** `OnlineJudge` retains login/problem metadata. Three capability-specific ports model start/poll/recovery and registries expose only supported behavior. The future verify command and `ce submit --watch` can share them without coupling polling to submission.

**Tech Stack:** Rust 1.92.0, serde, existing reqwest infrastructure.

## Constraints

- **Branch:** `feat/057-library-submission-ports`
- **Depends on:** plan 056 merged to `main`.
- Read specification sections 8 and 9.
- Keep start, poll, and recovery as separate traits and registries.
- `ce submit` continues to use global configuration; verify will use explicit project OJ language mapping.
- A transport failure after a request may have been transmitted is `AcceptanceUnknown`, never a safe retry.

### Task 1: Define submission ports and outcomes

**Files:**
- Create: `crates/usecases/src/submission.rs`
- Modify: `crates/usecases/src/lib.rs`
- Create: `crates/usecases/tests/submission.rs`

**Interfaces:**

```rust
pub trait SubmissionStarter {
    fn descriptor(&self) -> SubmissionAdapterDescriptor;
    fn start_submission(
        &self, request: &SubmissionRequest, session: Option<&Session>,
    ) -> Result<SubmissionStart, StartSubmissionError>;
}
pub trait SubmissionPoller {
    fn poll_submission(
        &self, handle: &SubmissionHandle, session: Option<&Session>,
    ) -> Result<PollObservation, PollSubmissionError>;
}
pub trait SubmissionRecovery {
    fn recover_submission(
        &self, request: &RecoveryRequest, session: Option<&Session>,
    ) -> Result<RecoveryOutcome, RecoverSubmissionError>;
}
```

- [x] Write failing tests for serializable handles, all start/poll/recovery outcomes, sanitized errors,
      capability consistency, and confirmed-not-accepted versus acceptance-unknown boundaries.
- [x] Implement strict request/outcome models and separate registries for all three ports.
- [x] Run `cargo test -p usecases --test submission`.
- [x] Invoke `/commit` with `feat: define reusable submission lifecycle ports`.

### Task 2: Migrate existing submit implementations

**Files:**
- Modify: `crates/usecases/src/online_judge.rs`
- Modify: `crates/usecases/src/service/submit.rs`
- Modify: `crates/infrastructure/src/online_judge_impl/atcoder.rs`
- Modify: `crates/infrastructure/src/online_judge_impl/librarychecker.rs`
- Modify: `crates/infrastructure/src/online_judge_impl/registry.rs`
- Modify: `crates/interfaces/src/controller.rs`
- Modify: `crates/infrastructure/src/shell/mod.rs`

- [x] Write characterization tests for current AtCoder browser opening and LibraryChecker submitted URL output.
- [x] Move submission responsibility from `OnlineJudge` to `SubmissionStarter` after all call sites migrate.
- [x] Declare AtCoder `InteractiveUntrackable/OverallOnly/None`; return `UserActionRequired`.
- [x] Declare LibraryChecker `UnattendedTrackable/TestcaseDetails/BestEffort`; return a trackable handle.
- [x] Run all login/whoami/submit tests; invoke `/commit` with `refactor: migrate submit to lifecycle ports`.

### Task 3: Deliver submission ports

- [x] Prove `ce submit` output and global-config resolution remain unchanged with characterization tests.
- [x] Run rollout repository verification and `git diff --check`.
- [x] Invoke `/commit` with `docs: record submission port completion`.
- [ ] Invoke `/pr --base main`; link plan 057 and state that it unblocks plan 058.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
