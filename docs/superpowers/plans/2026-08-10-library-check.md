# Library Check Command Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a reusable timeout-safe command runner and `ce check` that executes configured language checks locally or in CI without persisting/publicizing results.

**Architecture:** Use cases describe commands and aggregate language outcomes; infrastructure owns Unix process groups and streaming. Existing solution tests migrate to the same runner, while `ce check` remains a separate project-local library operation.

**Tech Stack:** Rust 1.92.0, std::process, nix signals/process, wait-timeout.

## Constraints

- **Branch:** `feat/054-library-check`
- **Depends on:** plan 039 merged to `main`.
- Read specification section 7.1.
- Check results stay local/CI, are never saved under verification results, and never enter site-data.
- Run language IDs in UTF-8 byte order and continue after failure/timeout.
- Missing check command is `skipped`; aggregate success requires every non-skipped command to pass.

### Task 1: Add a timeout-safe streaming runner

**Files:**
- Create: `crates/usecases/src/command_runner.rs`
- Modify: `crates/usecases/src/lib.rs`
- Create: `crates/infrastructure/src/command_runner_impl.rs`
- Modify: `crates/infrastructure/src/lib.rs`
- Create: `crates/infrastructure/tests/command_runner.rs`

**Interfaces:**

```rust
pub struct CommandRequest {
    pub program: OsString,
    pub arguments: Vec<OsString>,
    pub current_dir: PathBuf,
    pub environment: BTreeMap<OsString, OsString>,
    pub timeout: Duration,
}
pub struct CommandOutcome { pub exit_code: Option<i32>, pub timed_out: bool }
pub trait CommandRunner {
    fn run_streaming(&self, request: &CommandRequest) -> Result<CommandOutcome>;
}
```

- [x] Write failing tests for stdout/stderr streaming, exit code, environment replacement, timeout,
      SIGTERM then five-second SIGKILL, child process-group death, and subsequent command execution.
- [x] Implement `UnixCommandRunner`; never use `sh -c` inside the runner.
- [x] Run `cargo test -p infrastructure --test command_runner`.
- [x] Invoke `/commit` with `feat: run commands with streaming timeouts`.

### Task 2: Implement `ce check` and migrate solution tests

**Files:**
- Create: `crates/usecases/src/check.rs`
- Modify: `crates/usecases/src/service.rs`
- Modify: `crates/usecases/src/service/test.rs`
- Modify: `crates/interfaces/src/controller.rs`
- Modify: `crates/interfaces/src/controller/input.rs`
- Modify: `crates/infrastructure/src/shell/commands.rs`
- Modify: `crates/infrastructure/src/shell/mod.rs`
- Create: `crates/infrastructure/tests/check_command.rs`
- Create: `docs/commands/check.md`
- Modify: `docs/commands/test.md`

**Interfaces:**

```rust
pub enum CheckSelection { All, Language(LanguageId) }
pub enum LanguageCheckStatus { Passed, Failed { exit_code: i32 }, TimedOut, Skipped }
pub fn run_checks(
    config: &LibraryProjectConfig,
    selection: &CheckSelection,
    runner: &dyn CommandRunner,
) -> Result<CheckSummary>;
```

- [x] Write failing tests for stable order, filter, skip, aggregate failure, configured/default timeout,
      continued execution, exported `CE_*` paths/language, and no solution `test_command` execution.
- [x] Implement `ce check [--language <id>]`; use project-local config and direct argv commands.
- [x] Migrate `Service::test` to the runner with default `test_timeout_seconds = 600` without behavior drift.
- [x] Run `cargo test -p usecases check`, `cargo test -p usecases service::test`, and
      `cargo test -p infrastructure check_command`.
- [x] Invoke `/commit` with `feat: add project library checks`.

### Task 3: Deliver check

- [x] Run a mixed fixture where one language fails and prove all selected languages ran.
      Covered by `crates/infrastructure/tests/check_command.rs::aggregate_failure_records_all_results`
      and `::continues_after_middle_failure` (both green).
- [x] Run rollout repository verification and `git diff --check`.
- [x] Invoke `/commit` with `docs: record library check completion`.
- [ ] Invoke `/pr --base main`; link plan 054 and state that it unblocks plan 055.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
