# Library Verify Automation Activation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Activate credential-separated verification workflows after repository environments and GitHub App permissions are confirmed by the user.

**Architecture:** A lightweight dispatcher classifies main changes outside heavy concurrency. A reusable worker prepares immutable artifacts without secrets, then alternates OJ-only start/poll jobs and App-only persistence jobs. Result-only pushes skip OJ work but still publish Pages.

**Tech Stack:** GitHub Actions, GitHub App installation tokens, LibraryChecker Firebase refresh token, hidden `ce internal` commands.

## Constraints

- **Branch:** `feat/062-library-verify-activation`
- **Depends on:** plan 061 merged to `main` and the human setup gate below.
- Read specification sections 15.1-15.4.
- Never expose credentials to PR/feature-branch code or put OJ and App credentials in one job.
- Verdict rejection/unavailable is a successful workflow completion but a failed verification state.
- Draft remains for pending, acceptance-unknown, or infrastructure failure; complete terminal results become ready.

### Task 1: Finish activation workflow and policy tests without secrets

**Files:**
- Create: `.github/workflows/verify.yml`
- Modify: `.github/workflows/verify-worker.yml`
- Modify: `.github/workflows/verify-result-integrity.yml`
- Modify: `crates/infrastructure/tests/workflow_policy.rs`
- Create: `docs/operations/verify-automation.md`

- [ ] Write failing tests for push/schedule/manual classification, result-only skip, `verify-heavy` concurrency
      with no cancellation, prepare/start/persist/poll boundaries, retry schedule, permissions, and environments.
- [ ] Implement dispatcher and worker artifact hashes; secret jobs download only reviewed pinned artifacts.
- [ ] Set workflow retry intervals 5/10/20/40/80 minutes capped at six hours; schedule/manual always resume.
- [ ] Keep workflow uncallable until the environment variables/secrets below exist and are confirmed.
- [ ] Run workflow policy tests and invoke `/commit` with `ci: prepare verify automation activation`.

### Task 2: Human credential and repository-policy gate

- [ ] Invoke `/pr --base main`, then `/pr-review`; reach no new comments and green secretless checks first.
- [ ] **HUMAN GATE:** Ask the user to create/install a repository-only GitHub App with Contents read/write,
      Pull requests read/write, Metadata read; create `verify-state` main-only environment; add variable
      `VERIFY_APP_ID` and secret `VERIFY_APP_PRIVATE_KEY`.
- [ ] Ask the user to create `oj-library-checker` main-only environment and add only durable secret
      `LIBRARYCHECKER_REFRESH_TOKEN`; neither environment has required reviewers.
- [ ] Ask the user to enable branch protection, required CI/result-integrity checks, and repository auto-merge.
- [ ] Record confirmation and key rotation/revocation procedure in the operations doc; do not print secret values.

### Task 3: Merge reviewed activation and perform a no-submission dry run

- [ ] After user confirmation, merge the reviewed green PR to `main` so protected main code is the only code
      allowed to access both environments.
- [ ] Run a manual dry-run mode from `main` that mints tokens, creates/updates only the draft
      `automation/verify` PR, validates CAS and permissions, and never calls `POST /submit`.
- [ ] Verify App token is absent from Git credentials/logs/artifacts and OJ secret never enters App/prepare jobs.
- [ ] Verify result integrity rejects source/workflow/symlink changes and accepts a sole valid result JSON.
- [ ] If the dry run fails, create a new fix branch from current `main`, pass `/pr-review`, and merge before retry.

### Task 4: Activate, observe one real attempt, and deliver

- [ ] Enable live start only after presenting the exact solution/plan hash to the user and receiving confirmation.
- [ ] Observe Starting remote persistence before POST, immediate handle persistence, polling, terminal update,
      ready/auto-merge, result-only OJ skip, and Pages rebuild from the merged latest result.
- [ ] Run rollout Rust/Web/workflow verification and `git diff --check`.
- [ ] If live acceptance exposes a defect, use the same new-branch `/commit`/`/pr`/`/pr-review` fix flow.
- [ ] Revoke temporary test credentials if any; report the durable environments and rotation instructions.
