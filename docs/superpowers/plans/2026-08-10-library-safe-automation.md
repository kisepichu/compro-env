# Library Safe Automation Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add constrained GitHub state writing, complete change-policy tests, and non-triggering verify workflow definitions before any repository or OJ credential is configured.

**Architecture:** Secretless prepare produces immutable hashed artifacts. Tiny internal commands consume only those artifacts. A GitHub App client performs attempt-level CAS through GitHub APIs and can change only verification result JSON plus one bot PR; checked-in workflow policy tests enforce credential/job separation.

**Tech Stack:** Rust 1.92.0, reqwest, secrecy, serde_yaml, GitHub Git Data/Pulls APIs, GitHub Actions.

## Constraints

- **Branch:** `feat/060-library-safe-automation`
- **Depends on:** plans 050, 053, and 059 merged to `main`.
- Read specification sections 15.1-15.4.
- This plan must not create an OJ-triggering dispatcher or use live secrets.
- Third-party Actions use full 40-character commit SHAs; no `pull_request_target`.
- Secret jobs never checkout, build, analyze, preprocess, run repository scripts, or hold both OJ/App credentials.

### Task 1: Classify Git changes independently of event path lists

**Files:**
- Create: `crates/infrastructure/src/git_change_classifier.rs`
- Create: `crates/infrastructure/tests/git_change_classifier.rs`
- Modify: `crates/infrastructure/src/lib.rs`

**Interfaces:**

```rust
pub enum ChangeClass { ResultOnly, SourceOrConfig, Empty }
pub fn classify_changes(root: &Path, before: &str, after: &str)
    -> Result<ChangeClass, ChangeClassificationError>;
```

- [x] Write failing repository-fixture tests for empty, one/many results, mixed source, rename/delete,
      symlink, invalid SHA, NUL-safe paths, and more than 300 changed files.
- [x] Implement `git diff --name-only -z` classification; accept only normal
      `verification/results/**/*.json` files as result-only.
- [x] Run `cargo test -p infrastructure --test git_change_classifier`.
- [x] Invoke `/commit` with `feat: classify verification-only Git changes`.

### Task 2: Write verification state through constrained GitHub APIs

**Files:**
- Create: `crates/infrastructure/src/github/mod.rs`
- Create: `crates/infrastructure/src/github/verification_state_writer.rs`
- Create: `crates/infrastructure/tests/github_state_writer.rs`

**Interfaces:**

```rust
pub struct PersistStateRequest {
    pub repository: String,
    pub base_sha: String,
    pub branch: String,
    pub candidate: VerificationRecord,
}
impl GitHubVerificationStateWriter {
    pub fn persist(&self, request: &PersistStateRequest) -> Result<PersistedState>;
    pub fn set_pull_request_state(&self, state: BotPullRequestState) -> Result<()>;
}
```

- [x] Write local-server tests for base SHA, plan hash, schema, attempt CAS, sole-path allowlist, branch exactly
      `automation/verify`, draft/ready, auto-merge, conflict retry, and sanitized request failures.
- [x] Use Git Data/Pulls APIs; hold token in `SecretString` and never install it in Git credentials.
- [x] Validate base/plan/CAS/path immediately before every mutating API call.
- [x] Run `cargo test -p infrastructure --test github_state_writer`.
- [x] Invoke `/commit` with `feat: persist verification state through GitHub API`.

### Task 3: Add hidden artifact commands and policy-checked dormant workflows

**Files:**
- Modify: `crates/infrastructure/src/shell/commands.rs`
- Modify: `crates/infrastructure/src/shell/mod.rs`
- Create: `crates/infrastructure/tests/workflow_policy.rs`
- Create: `.github/workflows/verify-worker.yml`
- Create: `.github/workflows/verify-result-integrity.yml`

- [ ] Add hidden `verify-persist`, `verify-validate-result-pr`, and `classify-changes` commands with strict files.
- [ ] Write policy tests for no target trigger, main-only secret use, action SHA pins, permissions, environment
      names, no checkout/build in secret jobs, OJ/App separation, and result path restriction.
- [ ] Define `verify-worker.yml` with `workflow_call` only and no caller; define secretless result-integrity PR checks.
- [ ] Run workflow-policy tests and invoke `/commit` with `ci: add dormant safe verification automation`.

### Task 4: Deliver the safe automation foundation

- [ ] Prove no workflow path can currently call an OJ or mint an App token.
- [ ] Run rollout Rust/Web verification and `git diff --check`.
- [ ] Invoke `/commit` with `docs: record safe automation completion`.
- [ ] Invoke `/pr --base main`; link plan 060 and state that it unblocks plan 061.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
