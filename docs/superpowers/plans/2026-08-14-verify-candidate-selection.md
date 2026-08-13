# Verify Candidate Selection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `verify.yml`'s scheduled and eligible-push runs pick one publishable solution per tick automatically, so the `automation/verify → main → pages` verification loop advances without operator dispatch. Retryable failures whose `next_retry_at` has elapsed become candidates too.

**Architecture:** Add a secretless `ce internal pick-candidate` subcommand that reads the project's published+verify-marked solutions (existing `LibraryProjectConfig` + discovery) and overlays the current `automation/verify` state (`verification/results/**`) exactly the way `verify-prepare` does. A pure `select_next_candidate` in `usecases::verification` returns at most one `SolutionId` per invocation using deterministic ordering. The `verify.yml` dispatcher forwards the picker's output as the worker's `solution` input. The worker's six-job chain is not modified — the picker only replaces the currently-hard-coded empty string on schedule/push events.

**Tech Stack:** Rust 1.92.0 / Cargo, existing workspace (`domain`, `usecases`, `infrastructure`, `interfaces`), `clap`, `chrono`, GitHub Actions.

## Global Constraints

- **Branch:** `feat/063-verify-candidate-selection`
- **Depends on:** plan 062 activation cycle complete on `main` (last merged 062-tagged PR).
- Secretless: the picker runs in the same dispatcher job as `classify-changes`. It must not read App or OJ credentials and must not open new environments or secrets.
- Do not touch the six-job worker chain in `verify-worker.yml`; only extend the dispatcher's `solution` computation.
- `workflow_dispatch` inputs still win: an explicit `solution:` from a manual dispatch skips the picker.
- Deterministic ordering: parallel ticks colliding on the concurrency group must pick the same target given the same inputs.
- Never pick a solution whose latest record is in an in-flight state (`Submitted` with an unresolved handle). The worker's CAS already protects against races; skipping avoids wasted OJ hits and log noise.
- Never pick a solution whose latest completed record's fingerprint matches the current source/library closure. That solution is already verified and stable.
- The picker returns an empty output when no candidate is eligible; the dispatcher must translate that into `run_worker=false` so `prepare` does not spin up.
- Retry consumption is in scope: `InfrastructureFailure` records with `retryable = true` and `next_retry_at <= now` are eligible.

---

### Task 1: Domain rule for candidate selection

**Files:**
- Modify: `crates/usecases/src/verification/mod.rs`
- Create: `crates/usecases/src/verification/candidate.rs`

**Interfaces:**
- Produces: `pub fn select_next_candidate(now: DateTime<FixedOffset>, published: &[PublishedSolution], records: &BTreeMap<SolutionId, VerificationRecord>, fingerprints: &BTreeMap<SolutionId, VerifyFingerprint>) -> Option<SolutionId>`.
- Consumes: existing `domain::solution::PublishedSolution`, `domain::verification::{VerificationRecord, VerificationState, VerifyFingerprint, InfrastructureFailure, CompletedState}`.

- [ ] **Step 1: Write failing selection-order tests**

Cover in `candidate.rs::tests`:

- Empty state: three published solutions with no records → returns the smallest `SolutionId` by UTF-8 bytes.
- Retry ready: one solution has a `Failed{ retryable: true, next_retry_at: t0 }` record; another has no record; `now = t0` → returns the retry candidate (retry deadline sorts before fresh candidates).
- Retry not ready: `next_retry_at > now` → the retry candidate is filtered out; unrecorded solutions still selectable.
- In-flight skip: latest record is a `Submitted`/`HandleHeld` state → that solution is excluded even if no other candidate exists (returns `None`, not the in-flight one).
- Stable-fingerprint skip: `CompletedState` whose `input_hashes` reduce to a `VerifyFingerprint` equal to the fingerprint the caller supplies for the current tree → excluded.
- Fingerprint drift: `CompletedState` whose fingerprint differs → eligible; ordered after retry-ready candidates.
- Non-retryable failure: `Failed { retryable: false }` → excluded permanently.

- [ ] **Step 2: Run the focused tests and observe the missing module**

Run: `cargo test -p usecases verification::candidate`

Expected: compilation fails; `candidate` module and `select_next_candidate` do not exist.

- [ ] **Step 3: Implement selection**

Rules, in this order:

1. Build a working set of every `PublishedSolution` whose latest record permits selection: no record OR (retryable `InfrastructureFailure` with `next_retry_at <= now`) OR (`CompletedState` with `fingerprint(record) != fingerprints[id]`).
2. Reject solutions whose latest state is `Submitted` (any handle) or any other in-flight variant introduced by later plans; treat unknown non-terminal states as in-flight (fail closed).
3. Sort candidates by `(next_retry_at.unwrap_or(DateTime::<FixedOffset>::MAX), solution_id.as_str().as_bytes())`.
4. Return `Some(first)` or `None`.

Keep the module free of I/O; it is a pure function over the caller-provided data.

- [ ] **Step 4: Add stability + tie-break tests**

- Two retry candidates with the same `next_retry_at` → smaller `SolutionId` wins.
- Retry candidate with `next_retry_at = now` is selected (boundary case).
- Reordering the input `published` slice does not change the output.

Run: `cargo test -p usecases verification::candidate`

Expected: all cases pass.

- [ ] **Step 5: Commit the domain rule**

Invoke `/commit` with message:

```text
feat: select next verify candidate deterministically
```

### Task 2: `ce internal pick-candidate` subcommand

**Files:**
- Modify: `crates/infrastructure/src/shell/commands.rs`
- Modify: `crates/interfaces/src/verify.rs`
- Create: `crates/infrastructure/src/verify_pick_candidate.rs`
- Create: `crates/infrastructure/tests/pick_candidate.rs`

**Interfaces:**
- Produces: `ce internal pick-candidate --root <repo> --state <automation-verify-worktree> [--now <rfc3339>]`.
- Prints the picked `SolutionId` (or empty string) on stdout followed by a single newline. Exits `0` on both hit and miss; non-zero only on configuration or I/O errors.
- Consumes: existing `ProjectLibraryConfigLoader`, `LibraryDiscovery`, and Task 1's `select_next_candidate`.

- [ ] **Step 1: Write failing integration tests**

Under `crates/infrastructure/tests/pick_candidate.rs`, mount a temp repo containing:

- `config.toml` with one language and one solution.
- `solutions/<id>/ce.toml` marking the solution as published + verify.
- A parallel `state/` directory representing the overlay from `automation/verify`, containing one `verification/results/<id>.json` for a mocked `Failed { retryable, next_retry_at }` record.

Assert:

- `--now` before `next_retry_at` → prints empty.
- `--now` at or after `next_retry_at` → prints the solution id.
- No `state/` overlay → prints the solution id (no record means eligible).
- `state/` overlay for a non-existent solution id → picker ignores it and still selects the configured solution.

- [ ] **Step 2: Run the focused tests and observe the missing subcommand**

Run: `cargo test -p infrastructure --test pick_candidate`

Expected: compilation fails.

- [ ] **Step 3: Wire the subcommand**

Add `PickCandidate { root: PathBuf, state: PathBuf, now: Option<String> }` to `InternalSubcommand`. In the handler:

1. Load `LibraryProjectConfig` from `--root`.
2. Discover published+verify solutions via existing `LibraryDiscovery`.
3. Overlay `<state>/verification/results/**` onto the working set (read every JSON, deserialize as `VerificationRecord`).
4. Compute the current `VerifyFingerprint` for each candidate (reuse the fingerprint logic already exercised by `verify-prepare`; do not duplicate it — refactor the shared code into a common module in this task if inline).
5. Call `select_next_candidate`.
6. Print the selected id, or an empty line.

Reject a `--state` that is not a directory. Skip records whose `SolutionId` is not in the current publication set (they belong to solutions that have since been unpublished; leaving them alone is documented in the runbook).

- [ ] **Step 4: Add error-path tests**

- Missing `config.toml` → non-zero exit, error mentions the file.
- Malformed record JSON in `state/verification/results/` → non-zero exit, error names the file.
- Symlinked `state/` target → rejected (spec §6.1 discovery rules).

Run: `cargo test -p infrastructure --test pick_candidate`

Expected: pass.

- [ ] **Step 5: Commit the subcommand**

Invoke `/commit` with message:

```text
feat: expose ce internal pick-candidate
```

### Task 3: Wire the dispatcher to auto-pick

**Files:**
- Modify: `.github/workflows/verify.yml`

**Interfaces:**
- Consumes: `ce internal pick-candidate` from Task 2 and the existing `automation/verify` overlay pattern used by `verify-prepare`.
- Produces: the worker's `solution` input on `schedule` and `push` events without operator involvement.

- [ ] **Step 1: Draft the dispatcher change and expected log lines**

Sketch the intended diff in the PR description: add a `Pick verify candidate` step after `Classify main` that runs only when `run_worker=true` AND `inputs.solution` is empty (so `workflow_dispatch` still short-circuits the picker). Overlay `automation/verify` into a `state/` scratch dir with `git fetch origin automation/verify` + a `git --work-tree=state` checkout of the results path only.

- [ ] **Step 2: Add a workflow lint that asserts the picker branch**

Add a matcher in `verify.yml`'s existing self-check step (or a new `if:` guard test) that fails the job when the picker output would otherwise be silently ignored on `schedule` or `push`. Prefer a small `shell: bash` sanity block over a new action.

- [ ] **Step 3: Implement the dispatcher change**

Update `verify.yml`:

1. After the classify step decides `run_worker=true` and `inputs.solution == ''`, fetch `automation/verify` into `state/` (secretless: only public refs).
2. Run `./target/release/ce internal pick-candidate --root . --state state --now "$(date -u +%FT%TZ)"` and capture stdout.
3. If empty → set `run_worker=false` and skip the worker invocation. If non-empty → export the picked id as `dispatch.outputs.picked_solution`.
4. Change the worker `with:` block to `solution: ${{ inputs.solution || needs.dispatch.outputs.picked_solution || '' }}`.
5. Keep every existing gate (`VERIFY_ACTIVATED`, `github.ref == 'refs/heads/main'`) unchanged.

Do not introduce new secrets, environments, or reusable-workflow calls.

- [ ] **Step 4: Add a workflow dry-run test**

Add a `workflow_dispatch` scratch case (documented in the PR body, not committed) that sets `mode: dry-run` and leaves `solution:` blank and observes:

- Scheduler picks a candidate.
- Worker's `prepare` step exits with `has_work=true`.
- `persist_starting` succeeds against `automation/verify`.
- Live `submit` / `poll` / `persist_terminal` remain skipped because `mode: dry-run`.

- [ ] **Step 5: Commit the workflow change**

Invoke `/commit` with message:

```text
feat: pick verify candidates in the dispatcher
```

### Task 4: Runbook and rollout catch-up

**Files:**
- Modify: `docs/operations/verify-automation.md`
- Modify: `docs/superpowers/plans/2026-08-10-library-platform-rollout.md`

- [ ] **Step 1: Rewrite the "no-op" language in the runbook**

Replace the paragraphs at lines ~180–205 that currently say "Scheduled ticks are currently no-ops" with a description of the picker's rules, its determinism, and the in-flight/stable-fingerprint skips. Keep the manual-dispatch section — an operator still needs it for one-off `mode: live` runs against a specific solution.

Remove the sentence "the actual retry consumption path is gated on the same follow-up as automatic candidate selection." after this plan lands. Do not add follow-up hedging.

- [ ] **Step 2: Add row 063 to the rollout plan**

Under `## Plan Index`, insert `| 063 | feat/063-verify-candidate-selection | 2026-08-14-verify-candidate-selection.md | 062 | Scheduler picks one candidate per tick; retry consumption |`.

Under `## PR Dependency Graph`, extend the last line to `062 verify activation -- 063 verify candidate selection`.

Under the ready sets bullet list, add `- After 062: 063.` and `- After 063: rollout complete.`

- [ ] **Step 3: Commit the docs sync**

Invoke `/commit` with message:

```text
docs: record verify candidate selection activation
```

### Task 5: Verify and deliver the PR

- [ ] **Step 1: Run the plan integration suite**

```bash
cargo test -p usecases verification::candidate
cargo test -p infrastructure --test pick_candidate
cargo test --all
cargo clippy --all --all-features -- -D warnings
cargo fmt --all --check
```

Expected: all commands exit 0.

- [ ] **Step 2: End-to-end smoke on a scratch dispatch**

From the PR branch, dispatch `verify.yml` with `mode: dry-run` and blank `solution:`. Confirm the picker log line names one candidate and `persist_starting` succeeds against `automation/verify`. Do not run `mode: live` from the PR branch — activation happens once the PR is merged to `main`.

- [ ] **Step 3: Open and drive the PR**

Invoke `/pr --base main`. PR body links this plan, the runbook diff, and the scratch dispatch run URL. State that this closes out the 039–062 rollout.

Invoke `/pr-review` until Copilot returns no new comments. Merge only after CI is green.
