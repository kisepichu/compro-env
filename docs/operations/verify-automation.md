# Verify automation (plan 062)

Operational reference for `.github/workflows/verify.yml` and
`.github/workflows/verify-worker.yml`. The design contract lives in
`docs/superpowers/specs/2026-08-10-library-platform-design.md`
§15.1–§15.4. This file explains what an operator needs to know to run
and monitor the two-workflow verify pipeline.

## What the workflows do

The pipeline is split into a lightweight dispatcher and a heavy worker
so that classification runs outside the `verify-heavy` concurrency
group (§15.3):

**Dispatcher — `verify.yml`**

- `dispatch` — secretless. Runs on `push` to `main`, on a
  `*/5 * * * *` schedule, and on `workflow_dispatch` (`mode: dry-run |
  live`, default `dry-run`). Every job is gated on the repository
  variable `VERIFY_ACTIVATED == 'true'`; when the variable is unset or
  false, the workflow is fully dormant. The job checks out, builds
  `ce`, runs `ce internal classify-changes --before --after`, and
  decides whether to invoke the worker. `push` events classified as
  `result-only` or `empty` skip the worker; `schedule` and
  `workflow_dispatch` always call it so pending records can resume.
  The worker is called via `uses: ./.github/workflows/verify-worker.yml`
  with `secrets: inherit`.

**Worker — `verify-worker.yml`**

`workflow_call`-only, with inputs `after` (immutable plan base SHA),
`mode` (default `dry-run`; only an explicit `workflow_dispatch` with
`mode: live` picks the OJ path), and `solution` (empty means no work
this run). All jobs live in the `verify-heavy` concurrency group with
`cancel-in-progress: false`. The six jobs form a strict `needs:`
chain:

1. **`prepare`** — secretless. Checks out, builds `ce`, then runs
   `tools/library-analyzers/prepare` + `tools/library-analyzers/build`
   to materialize the pinned adapter executables under
   `target/library-analyzers/bin/*-analyzer`. Only after those exist
   does it invoke `ce internal verify-prepare --plan-out plan.json
   --starting-out starting.json` — `build_analysis` fans out over every
   language declared in `config.toml`, and a missing adapter surfaces
   as `adapter executable for language ... not found`. `prepare` then
   computes SHA256 for `plan.json`, the `ce` binary, and the
   `analyzers.tar` bundle (`bin/` + `builds/`), and uploads the
   `verify-plan`, `verify-ce`, and `verify-analyzers` artifacts. Emits
   `has_work`, `plan_sha`, `ce_sha`, `analyzers_sha`, and `base_sha`
   outputs consumed by the rest of the chain. The adapter caches key on
   `hashFiles(dependencies.toml)` (prepared archives) and
   `hashFiles(tools/library-analyzers/**, crates/library-adapter-protocol/**, Cargo.lock, rust-toolchain.toml)`
   (built binaries), so a cold run downloading LLVM 22.1 (~700MB) and
   Lean 4.30 (~500MB) only happens when those inputs change.
2. **`persist_starting`** — `verify-state` environment,
   `permissions: contents: read`. Downloads both artifacts,
   re-validates the SHAs, mints an App installation token via
   `actions/create-github-app-token@<sha>` (`app-id:
   ${{ vars.VERIFY_APP_ID }}`, `private-key:
   ${{ secrets.VERIFY_APP_PRIVATE_KEY }}`), and runs
   `./ce internal verify-persist --plan-hash-in plan.sha256
   --candidate-in starting.json --repository ${{ github.repository }}
   --base-sha $BASE_SHA --token-env GH_APP_TOKEN`. The token is passed
   through the environment only, never on the command line.
3. **`submit`** — `oj-library-checker` environment,
   `permissions: contents: read`. Skipped unless `inputs.mode ==
   'live'`. Downloads the `verify-plan`, `verify-ce`, and
   `verify-analyzers` artifacts, re-validates all three SHAs, unpacks
   `analyzers.tar` into `target/library-analyzers/` on top of the
   `automation/verify` checkout, writes `~/.config/ce/session.toml`
   from `secrets.LIBRARYCHECKER_REFRESH_TOKEN`, runs
   `./ce internal verify-start --plan-in plan.json`, and uploads the
   emitted handle record as the `verify-handle` artifact.
4. **`persist_handle`** — `verify-state` environment. Downloads the
   handle record, mints a fresh App token, and persists it through
   `verify-persist`.
5. **`poll`** — `oj-library-checker` environment. Downloads the
   handle plus the same `verify-analyzers` bundle (so
   `verify-poll` can boot `build_analysis` without a toolchain), polls
   once (looping if the terminal verdict has not yet landed), and
   uploads the terminal record artifact.
6. **`persist_terminal`** — `verify-state` environment. Persists the
   terminal record and releases the automation PR to ready-for-review
   when the verdict is terminal.

Triggers allowed: `push` to `main`, `schedule` on the dispatcher, and
`workflow_dispatch` on the dispatcher. The worker accepts only
`workflow_call`. `pull_request`, `pull_request_target`, and any other
event are disallowed and enforced by `workflow_policy` tests.

## One-time setup — HUMAN GATE G2

Complete every step below before flipping `VERIFY_ACTIVATED` to
`true`. The plan calls this out as a hard gate.

1. Create a GitHub App scoped to this repository only. Grant
   **Contents: read and write**, **Pull requests: read and write**,
   **Metadata: read**. Do not grant Actions, Checks, Workflows,
   Administration, Pages, Deployments, or any organization-level
   permission.
2. Install the App on this repository only. Do not authorize it for
   the whole account.
3. Note the App ID (public) and generate a private-key PEM (secret).
   Store the PEM in a password manager; do not commit it.
4. Create GitHub Environment **`verify-state`** and configure it as
   follows:
   - Deployment branch policy: "Selected branches and tags" — allow
     only `main`.
   - Required reviewers: **none**. The workflow runs unattended per
     §15.4.
   - Environment variable: `VERIFY_APP_ID` = the App ID.
   - Environment secret: `VERIFY_APP_PRIVATE_KEY` = the PEM contents.
5. Create GitHub Environment **`oj-library-checker`** and configure
   it as follows:
   - Deployment branch policy: only `main`.
   - Required reviewers: **none**.
   - Environment secret: `LIBRARYCHECKER_REFRESH_TOKEN` = the Firebase
     refresh token captured by running `ce login` against Library
     Checker manually.
6. Bootstrap the `automation/verify` state branch from the current
   `main` tip: `git push origin main:automation/verify`. Every
   `persist_*` job's CAS assumes this ref already exists; without it
   the first `persist_starting` fails opaquely with
   `PATCH refs/heads/automation/verify → 404`.
7. Under **Settings → Actions → Variables** (repository scope), add
   `VERIFY_ACTIVATED = true`. This is the master activation switch;
   setting it back to `false` disables the workflow without needing
   to delete the environments or rotate secrets.
8. Enable branch protection on `main` with these required status
   checks: `CI / Cargo test + clippy + fmt`, `CI / Web build`, and any
   `verify-result-integrity` check that surfaces on the automation
   PRs.
9. Enable **Settings → General → Allow auto-merge** so the bot's
   terminal-verdict PRs can auto-merge once all required checks pass.
10. Record the completion date, the App ID, and the PEM fingerprint in
    your operator log. Never commit the PEM itself.

Do not enable `VERIFY_ACTIVATED` before every item is confirmed.

## Operating the workflow

- **Every `main` push** runs the dispatcher. If classification returns
  `source-or-config`, the worker is called — but `prepare` still needs
  a `solution` input to do anything (see below), so today's push
  effectively short-circuits to `has_work=false` until automatic
  candidate selection lands.
- **Every 5 minutes (schedule)** the dispatcher wakes and calls the
  worker with `solution: ''`. Until automatic candidate selection is
  implemented, the worker exits at `prepare` with `has_work=false` and
  every downstream job is skipped. **Scheduled ticks are currently
  no-ops.** Operators watching the cron will see a green
  five-minute-interval workflow run doing nothing until either a
  `workflow_dispatch` supplies a `solution` or the follow-up plan wires
  in the candidate picker.
- **Manual dispatch** via `workflow_dispatch`: supply a `solution` (e.g.
  `librarychecker-aplusb/aplusb/rust`) plus `mode: dry-run` for a
  no-OJ pass that exercises `prepare` and `persist_starting` only, or
  `mode: live` for a real Library Checker submission. A blank
  `solution` is a no-op (same as schedule).
- **Result-only pushes** (updates under `verification/results/**`)
  are classified as `result-only`; the dispatcher skips the worker and
  only `pages.yml` republishes the site.
- **Retry backoff** target is `5 → 10 → 20 → 40 → 80` minutes, capped
  at 6 hours. The record's `next_retry_at` encodes the schedule and
  the 5-minute cron is dense enough to honor it — but the actual
  retry consumption path is gated on the same follow-up as automatic
  candidate selection.

## Debugging failures

- `environment 'verify-state' not found` (or the same for
  `oj-library-checker`) means G2 was not completed or an environment
  was renamed. Restore the environment names verbatim.
- `plan artifact digest mismatch` in `persist_starting`,
  `persist_handle`, or `persist_terminal` indicates tampering or a
  corrupted artifact download. Re-run the workflow; do not attempt to
  patch the artifact by hand.
- `analyzers digest mismatch` in `submit` or `poll` follows the same
  root cause as `plan artifact digest mismatch` but points at
  `analyzers.tar`. Re-run the workflow; the bundle is regenerated
  deterministically in `prepare` from the pinned toolchains.
- `adapter executable for language ... not found` from the `prepare`
  job means one of `tools/library-analyzers/prepare` or
  `tools/library-analyzers/build` failed silently, or the analyzer
  cache key is stale. Check the two build-step logs; the caches key on
  `dependencies.toml` and `tools/library-analyzers/**` so a corrupted
  cache is invalidated by touching the tree.
- `PATCH refs/heads/automation/verify remained non-fast-forward`
  after retries is a CAS conflict on the state ref. The worker will
  reattempt on the next 5-minute tick — no operator action needed.
- `no verification record stored for <id>` from `poll` means the
  `automation/verify` state branch does not have the record
  `persist_handle` was supposed to commit for this attempt. Either the
  `persist_handle` job failed silently, or the `poll` job's Checkout
  targeted the wrong ref (the workflow pins it to `automation/verify`
  precisely to see `persist_handle`'s commit). Check `persist_handle`'s
  run log first; if it succeeded, re-run `poll` against the same run.
- `::warning::verify-poll ended in a non-terminal state` from the
  `poll` job is emitted by design when the OJ returned
  `BudgetExhausted` / `HandleLost` / `InfrastructureError`.
  `persist_terminal` still runs on this path via `!cancelled()` so the
  emitted record lands on `automation/verify`; the workflow just
  surfaces a warning so operators know a follow-up tick is expected.
- Non-`Trackable` `verify-start` outcomes (`Unavailable` /
  `AcceptanceUnknown` / `ConfirmedNotAccepted` / `InfrastructureError`)
  are still captured: `submit` emits their `VerificationRecord` to
  `handle.json`, `persist_handle` commits it to `automation/verify`,
  and the downstream `poll` bails on the non-handle state so
  `persist_terminal` skips. The state on `automation/verify` accurately
  reflects the observed outcome — no infinite retry occurs because the
  scheduler is a no-op until automatic candidate selection lands. To
  resume, re-run `workflow_dispatch` with a `solution` argument; the
  new attempt's `persist_starting` CAS-replaces the failed record.
- Secret leakage in a failed job: nothing to remediate inside the
  workflow. Invalidate the affected token (App key or Library Checker
  refresh token) and follow the rotation steps below.

## Key rotation and revocation

- **`VERIFY_APP_PRIVATE_KEY`**: on the App's settings page, generate a
  new private key. Update the `verify-state` environment secret with
  the new PEM. Once a `persist_*` job succeeds against the new key,
  delete the old key on the App page.
- **`LIBRARYCHECKER_REFRESH_TOKEN`**: run `ce login` locally against
  Library Checker, copy the resulting refresh token, and update the
  `oj-library-checker` environment secret. No cascading changes are
  required.
- **Emergency stop**: set `VERIFY_ACTIVATED` to `false` (or delete the
  variable). No dispatcher or worker job will do OJ or App work until
  it is re-enabled. Environments and secrets remain in place, so
  re-activation is a single variable flip.
