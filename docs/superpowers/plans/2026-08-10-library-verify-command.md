# Library Verify Command Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add resumable local `ce verify` orchestration that checks, tests, freezes, submits, polls, and atomically records only never/stale targets.

**Architecture:** A common submission lifecycle resumes saved attempts before planning new work. The public command composes pure planning, command runner, preprocess, start/poll/recovery ports, repository, clock, and backoff policy; hidden start/poll helpers remain reusable by later CI.

**Tech Stack:** Rust 1.92.0, existing layered CLI, verification/submission ports.

## Constraints

- **Branch:** `feat/059-library-verify-command`
- **Depends on:** plan 058 merged to `main`.
- Read specification sections 7.2, 8, and 10.
- Resume `Starting`, `AcceptanceUnknown`, and handles before discovering new candidates.
- Run each relevant language check once; if any check fails, start no new preprocess/plan/submission.
- One in-flight submission per OJ; a pending timeout stops later submissions. No force option.

### Task 1: Implement reusable resume/start/poll lifecycle

**Files:**
- Create: `crates/usecases/src/submission_lifecycle.rs`
- Create: `crates/usecases/tests/submission_lifecycle.rs`

**Interfaces:**

```rust
pub fn resume_pending(
    repositories: &VerificationRepositories,
    ports: &SubmissionPorts,
    selection: &VerifySelection,
) -> Result<ResumeSummary>;
pub fn start_plan(
    plan: &SubmissionPlan,
    starting: &VerificationRecord,
    starter: &dyn SubmissionStarter,
) -> Result<StartEvent>;
pub fn poll_handle(
    record: &VerificationRecord,
    poller: &dyn SubmissionPoller,
    policy: &PollingPolicy,
) -> Result<PollEvent>;
```

- [ ] Write fake-port tests for every resume state, recovery exact/inconclusive, persisted Starting before
      connection, immediate handle persistence, 2-to-15-second polling, retry-after, 30-second error backoff,
      15-minute budget, terminal/error transitions, and no duplicated start.
- [ ] Implement injectable clock/sleeper and transition persistence after every durable state.
- [ ] Run `cargo test -p usecases --test submission_lifecycle`.
- [ ] Invoke `/commit` with `feat: orchestrate resumable submission lifecycle`.

### Task 2: Add the public command and hidden CI boundary

**Files:**
- Create: `crates/usecases/src/service/verify.rs`
- Modify: `crates/usecases/src/service.rs`
- Modify: `crates/interfaces/src/controller.rs`
- Modify: `crates/interfaces/src/controller/input.rs`
- Modify: `crates/infrastructure/src/shell/commands.rs`
- Modify: `crates/infrastructure/src/shell/mod.rs`
- Create: `crates/infrastructure/tests/verify_command.rs`
- Create: `docs/commands/verify.md`

- [ ] Write failing CLI tests for all/one selection, stable solution order, resume-first, never/stale-only,
      one check per language, solution tests, check/test failure barrier, unavailable, rejected, and exit codes.
- [ ] Implement `ce verify [solution-id]` with strict project config and explicit OJ language mapping.
- [ ] Add hidden `internal verify-prepare/start/poll` argument schemas; keep them undocumented and constrained.
- [ ] Ensure AtCoder verify writes unavailable and exits 1; credentials/network failures remain infrastructure errors.
- [ ] Run `cargo test -p infrastructure --test verify_command`.
- [ ] Invoke `/commit` with `feat: add resumable library verification command`.

### Task 3: Deliver `ce verify`

- [ ] Run a fake mixed-OJ end-to-end fixture, including a timeout followed by resume to accepted.
- [ ] Run rollout repository verification and `git diff --check`.
- [ ] Invoke `/commit` with `docs: record verify command completion`.
- [ ] Invoke `/pr --base main`; link plan 059 and state that it satisfies the verify prerequisite of plan 060.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
