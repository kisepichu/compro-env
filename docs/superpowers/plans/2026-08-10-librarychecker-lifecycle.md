# LibraryChecker Submission Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Normalize LibraryChecker pending/final results and recover ambiguous submissions conservatively from its official submission APIs.

**Architecture:** Split the current implementation by concern and parse checked-in sanitized OpenAPI-shaped fixtures. Polling uses public submission detail; best-effort recovery lists narrowly filtered candidates then hashes each exact source, accepting only one match in the attempt window.

**Tech Stack:** Rust 1.92.0, reqwest, serde, sha2, local HTTP mock.

## Constraints

- **Branch:** `feat/058-librarychecker-lifecycle`
- **Depends on:** plan 057 merged to `main`.
- Read specification sections 8-9 and `docs/online_judges/librarychecker.md`.
- Official APIs are `GET /submissions` and `GET /submissions/{id}` from upstream OpenAPI commit
  `9a9ee40f4b284e56615f123fa69f06943d0b710c`.
- Recovery filters problem/language/current user, then matches exact source hash and bounded submission time.
- Zero/multiple candidates are `Inconclusive`; zero is never proof that the POST was not accepted.

### Task 1: Split transport and capture sanitized response fixtures

**Files:**
- Replace: `crates/infrastructure/src/online_judge_impl/librarychecker.rs`
- Create: `crates/infrastructure/src/online_judge_impl/librarychecker/mod.rs`
- Create: `crates/infrastructure/src/online_judge_impl/librarychecker/auth.rs`
- Create: `crates/infrastructure/src/online_judge_impl/librarychecker/problem.rs`
- Create: `crates/infrastructure/src/online_judge_impl/librarychecker/submission.rs`
- Create: `crates/infrastructure/src/online_judge_impl/librarychecker/schema.rs`
- Create: `crates/infrastructure/tests/fixtures/librarychecker/submission-list.json`
- Create: `crates/infrastructure/tests/fixtures/librarychecker/submission-pending.json`
- Create: `crates/infrastructure/tests/fixtures/librarychecker/submission-accepted.json`
- Modify: `docs/online_judges/librarychecker.md`

- [x] Record the pinned OpenAPI commit in docs and write parsing tests for list/detail/current user.
- [x] Move existing auth/problem/start logic without semantic changes and retain all old tests.
- [x] Strip raw source, compile output, case stderr/checker output from domain errors/logs.
- [x] Run `cargo test -p infrastructure online_judge_impl::librarychecker`.
- [x] Invoke `/commit` with `refactor: split LibraryChecker API concerns`.

### Task 2: Poll and normalize results

**Files:**
- Modify: `crates/infrastructure/src/online_judge_impl/librarychecker/submission.rs`
- Create: `crates/infrastructure/tests/librarychecker_submission.rs`
- Create: `crates/infrastructure/tests/fixtures/librarychecker/submission-rejected.json`
- Create: `crates/infrastructure/tests/fixtures/librarychecker/submission-unknown-verdict.json`

- [x] Write local-server tests for queued/judging, every known verdict, unknown raw verdict, metrics,
      null/empty case details, retry-after, 401 refresh, 429/5xx, malformed response, and rounding units.
- [x] Implement `GET /submissions/{id}` mapping and retain exact submission ID/URL in the handle.
- [x] Apply the public extra allowlist and sanitize every error summary.
- [x] Run `cargo test -p infrastructure --test librarychecker_submission`.
- [x] Invoke `/commit` with `feat: poll LibraryChecker submission results`.

### Task 3: Recover only a unique exact candidate

**Files:**
- Modify: `crates/infrastructure/src/online_judge_impl/librarychecker/submission.rs`
- Modify: `crates/infrastructure/tests/librarychecker_submission.rs`

- [x] Write failing tests for one/zero/multiple candidates, wrong problem/language/user/source/time,
      pagination limit, list/detail failure, duplicate IDs, and ambiguous POST disconnect.
- [x] Query list newest-first with problem/language/user, bound pages to the attempt window, fetch details,
      compare raw source SHA-256 and timestamp, and recover only one exact match.
- [x] Return `Inconclusive` for every inability to prove uniqueness; never auto-resubmit.
- [x] Run the focused suite and invoke `/commit` with `feat: recover LibraryChecker submissions conservatively`.

### Task 4: Deliver LibraryChecker lifecycle

- [x] Run all old/new LibraryChecker tests with a configurable local base URL only.
- [x] Run rollout repository verification and `git diff --check`.
- [x] Invoke `/commit` with `docs: record LibraryChecker lifecycle completion`.
- [ ] Invoke `/pr --base main`; link plan 058 and state that it unblocks plan 059.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
