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

1. **`prepare`** — secretless. Checks out `main@base_sha`, then
   overlays `verification/results/**` from the `automation/verify`
   state branch so `verify-prepare` can read the current terminal
   record's attempt id and stamp it as the plan's
   `previous_attempt_id` (the CAS token consumed by
   `persist_starting`). Builds `ce`, then runs
   `tools/library-analyzers/prepare` + `tools/library-analyzers/build`
   to materialize the pinned adapter executables under
   `target/library-analyzers/bin/*-analyzer`. Only after those exist
   does it invoke `ce internal verify-prepare --plan-out plan.json
   --starting-out starting.json` — `build_analysis` fans out over every
   language declared in `config.toml`, and a missing adapter surfaces
   as `adapter executable for language ... not found`. `prepare` then
   computes SHA256 for `plan.json`, the `ce` binary, and the
   `analyzers.tar` bundle (`tar --dereference` of `bin/` plus the
   accumulated `builds/`, so downstream jobs receive plain files
   instead of the `bin/*-analyzer -> builds/<build-id>/...`
   symlinks), and uploads the `verify-plan`, `verify-ce`, and
   `verify-analyzers` artifacts. Emits `has_work`, `plan_sha`,
   `ce_sha`, `analyzers_sha`, and `base_sha` outputs consumed by the
   rest of the chain. All analyzer prepare/build steps (and the
   supporting apt install) are gated on `inputs.solution != ''`, so a
   scheduled tick with no candidate skips the entire prelude. The
   adapter caches key on `hashFiles(dependencies.toml)` (prepared
   archives) and
   `hashFiles(tools/library-analyzers/**, crates/library-adapter-protocol/**, crates/domain/src/adapter_build.rs, crates/domain/src/adapter_prepare.rs, crates/infrastructure/src/library_adapter/**, Cargo.lock, rust-toolchain.toml)`
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

### Automation PR

Every `persist_*` job appends a `ce internal verify-pr-set-state` step
after its `verify-persist` invocation. That step maintains exactly one
long-lived pull request from `automation/verify` → `main`:

- **Title:** `Automation: verification results`. **Head:**
  `automation/verify`. **Base:** `main`.
- The first `persist_starting` on a fresh state branch opens the PR as
  a **draft**; subsequent `persist_*` calls reuse it (find-or-open is
  idempotent).
- `persist_starting` and `persist_handle` keep the PR draft — the
  attempt is still mid-flight.
- `persist_terminal` inspects the persisted record and flips the PR to
  **ready-for-review + auto-merge** when the state is:
  - `Completed{Accepted | WrongAnswer | TimeLimitExceeded |
    MemoryLimitExceeded | RuntimeError | CompileError |
    OutputLimitExceeded | JudgeError}`, or
  - `Unavailable`.
- `Completed{Cancelled | Other}` and every non-terminal state
  (`Starting`, `AcceptanceUnknown`, `Submitted`, `Queued`, `Judging`,
  `InfrastructureFailure`) leave the PR draft — the outcome is either
  indeterminate or still in flight, so a human decides.

Once auto-merge fires the `main` push triggers `pages.yml` and the
site rebuilds against the new record.

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
  `source-or-config`, the dispatcher also invokes the automatic picker
  (`ce internal pick-candidate`) against the `automation/verify`
  overlay and hands the chosen `SolutionId` to the worker as
  `solution`. If the picker returns nothing eligible the dispatcher
  flips `run_worker=false` and the worker is skipped for this push.
- **Every 5 minutes (schedule)** the dispatcher wakes, overlays the
  current `automation/verify` state under `state/verification/`, and
  runs the picker. The picker walks the current publication set,
  applies the eligibility rules (see below), and prints exactly one
  `SolutionId` (or an empty line). Non-empty output feeds the worker;
  an empty line skips the tick, so retryable-failure and drifted
  `Completed` records converge one solution per tick without operator
  input.
- **Manual dispatch** via `workflow_dispatch`: supply a `solution` (e.g.
  `librarychecker-aplusb/aplusb/rust`) plus `mode: dry-run` for a
  no-OJ pass that exercises `prepare` and `persist_starting` only, or
  `mode: live` for a real Library Checker submission. An explicit
  `solution:` always wins over the picker; leave it blank to let the
  picker choose.
- **Result-only pushes** (updates under `verification/results/**`)
  are classified as `result-only`; the dispatcher skips the worker and
  only `pages.yml` republishes the site. The picker never runs on
  this path because `decide` already gave `run_worker=false`.
- **Retry backoff** target is `5 → 10 → 20 → 40 → 80` minutes, capped
  at 6 hours. Every retryable `InfrastructureFailure` is persisted with
  `next_retry_at = updated_at + retry_delay(retry_count)`
  (`crates/usecases/src/verification/backoff.rs`) and the 5-minute cron
  is dense enough to honor it. The streak lives on
  `InfrastructureFailure.retry_count`: transitioning
  `InfrastructureFailure -> InfrastructureFailure` bumps the counter,
  and any other predecessor resets it to `1` (spec §8.3 "OJ 接続成功
  または判定進行で reset する"). The `Retry-After` hint
  (`sleep_with_hint`) still overrides intra-command sleeps when the OJ
  asks for a longer wait; it does not affect the cross-workflow
  `next_retry_at`. Non-retryable failures (`HandleNotFound`,
  `AuthenticationRejected`, `CredentialsMissing`, `SchemaError`) leave
  `next_retry_at: None` so an operator has to clear them.

### Picker eligibility rules

`ce internal pick-candidate` runs in the same secretless dispatcher
job as `classify-changes` and never touches App or OJ credentials.
Given the current publication set (`config.toml` + published
`solutions/**/ce.toml`) and the `automation/verify` overlay, a
solution is eligible when its latest record is:

- absent (no verification has ever run), OR
- `InfrastructureFailure { retryable: true }` whose `next_retry_at`
  has elapsed or is `None`, OR
- `Completed` whose stored `fingerprint` disagrees with the
  freshly-recomputed fingerprint from the working tree (input drift).

Every other state is excluded. The five in-flight variants
(`Starting`, `AcceptanceUnknown`, `Submitted`, `Queued`, `Judging`)
never advance out of the picker — the worker's CAS is the sole
race guard, and the picker just avoids wasted OJ hits.
`InfrastructureFailure { retryable: false }` is excluded permanently.

### `Unavailable` is a permanent dead-letter

`VerificationState::Unavailable` records are terminal and never
re-enter the picker. `UnavailableReason` variants
(`interactive_untrackable`, `unsupported_mode`, `oj_unsupported`,
`problem_mismatch`, `language_mismatch`) are not fed into the
fingerprint, so the `Completed`-drift path that automatically
re-enables verified solutions **does not apply**. To reprocess an
`Unavailable` solution — e.g. after adding a new adapter — an
operator must clear the overlay record for that solution (usually
via a `workflow_dispatch` with an explicit `solution:` targeting the
same id, once the underlying capability changes) and let the picker
pick it up as a fresh candidate.

### Determinism and concurrency

The picker orders eligible candidates by `(retry_ready first,
next_retry_at ascending, SolutionId bytes ascending)`, so parallel
ticks that collide on the `verify-heavy` concurrency group compute
the same target. That determinism plus the worker's per-`(solution,
attempt)` CAS keeps `automation/verify` linearizable even when a
schedule tick and a push tick fire back-to-back.

### Overlay records for solutions that have been unpublished

If a solution used to live at `librarychecker-…/foo/bar` and was
later removed from the manifest, its `verification/results/<id>.json`
stays on `automation/verify` until an operator removes it. The
picker silently ignores such records — they simply do not appear in
the publication set — so unpublished solutions cannot dominate the
schedule. Clean them up in a routine sweep; leaving them in place is
harmless.

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
  reflects the observed outcome. Retryable outcomes come back
  automatically through the picker on the next scheduler tick;
  non-retryable outcomes stay put until an operator re-runs
  `workflow_dispatch` with an explicit `solution` argument, whose
  `persist_starting` CAS-replaces the failed record.
- Secret leakage in a failed job: nothing to remediate inside the
  workflow. Invalidate the affected token (App key or Library Checker
  refresh token) and follow the rotation steps below.

## Key rotation and revocation

Two secrets are in scope. Both live in per-environment secret stores; no
repository-wide secrets are involved. All rotation happens without
downtime: `VERIFY_ACTIVATED` can stay `true` throughout, and the
five-minute scheduler tolerates a single failed tick.

### Cadence

- **`VERIFY_APP_PRIVATE_KEY`** — rotate at least **every 90 days**, and
  immediately after: any suspected leak, any operator/device rotation,
  or the first successful live run following a bootstrap (the initial
  activation key is by nature a "temporary" credential and should be
  swapped once the pipeline is proven green).
- **`LIBRARYCHECKER_REFRESH_TOKEN`** — rotate at least **every 30 days**
  (Firebase refresh tokens don't have a fixed TTL but are revocable, and
  a fresh capture catches quiet server-side invalidation early), and
  immediately after: password change on the Library Checker account,
  the first successful live run following a bootstrap, or any
  `session expired and token refresh failed` from a worker job.

Record the rotation date, actor, and PEM fingerprint (App key only) in
your operator log. Do **not** commit the PEM or the refresh token.

### Rotate `VERIFY_APP_PRIVATE_KEY`

1. Open the App's settings page (Developer settings → GitHub Apps →
   the App scoped to this repo).
2. Under "Private keys", click **Generate a private key** — GitHub
   downloads a fresh `<app>.<date>.private-key.pem`.
3. Load the new PEM into the environment secret. The command reads
   the file into stdin so the PEM never appears on the shell history:

   ```bash
   gh secret set VERIFY_APP_PRIVATE_KEY \
     -R kisepichu/compro-env --env verify-state \
     < /path/to/<app>.<date>.private-key.pem
   ```

4. Verify: manually dispatch `verify` with `mode: dry-run` and any
   valid `solution`. `prepare` + `persist_starting` must complete
   green. If `persist_starting` fails with an App-auth error, the
   secret update did not take — re-run step 3.
5. Only after step 4 succeeds, revoke the old key on the App's
   settings page. GitHub keeps the previous key active until you
   delete it explicitly; leaving both keys live indefinitely defeats
   the rotation.
6. Delete the downloaded PEM from disk (`shred -u` on Linux) and
   record the rotation in your operator log.

### Rotate `LIBRARYCHECKER_REFRESH_TOKEN`

1. Locally, run `ce login librarychecker` and enter the account's
   email + password. On success it writes `session.toml` with a fresh
   `refresh_token` under `$CE_CONFIG_DIR` (if set) or `~/.config/ce/`
   otherwise — the same lookup order used by every other `ce`
   subcommand (`SessionRepositoryImpl::config_dir()` in
   `crates/infrastructure/src/repository_impl/session_repository_impl.rs`).
2. Pipe the token directly from the session file into the environment
   secret. This never writes the token to the terminal, so it cannot
   leak via scrollback, `tmux`/`screen` session logs, or screen
   recordings. The snippet honours `CE_CONFIG_DIR` so it stays in sync
   with step 1 even inside a shell that had it set for integration
   tests. `end=""` suppresses the trailing newline that `print` would
   otherwise inject into the stored secret. Requires **Python 3.11+**
   for `tomllib` — on older systems either upgrade or `pip install
   tomli` and swap the import:

   ```bash
   python3 -c 'import os, tomllib, pathlib; \
     d = os.environ.get("CE_CONFIG_DIR", "").strip() or str(pathlib.Path.home() / ".config" / "ce"); \
     print(tomllib.loads(pathlib.Path(d, "session.toml").read_text())["librarychecker"]["refresh_token"], end="")' \
     | gh secret set LIBRARYCHECKER_REFRESH_TOKEN \
         -R kisepichu/compro-env --env oj-library-checker
   ```

3. Verify: manually dispatch `verify` with `mode: live` and a cheap
   `solution` (e.g. `librarychecker-aplusb/aplusb/rust`). All six
   worker jobs must complete green with a terminal verdict on
   `automation/verify`.
4. Optional but recommended: on the Library Checker account, sign out
   of all other sessions to invalidate the previous refresh token.
5. Delete the local `session.toml` under the same directory used in
   step 1 (`$CE_CONFIG_DIR` if set, else `~/.config/ce/`) if it isn't
   otherwise needed on this machine, and record the rotation in your
   operator log.

### Emergency stop

Set `VERIFY_ACTIVATED` to `false` (or delete the repo variable). No
dispatcher or worker job will do OJ or App work until it is
re-enabled. Environments and secrets remain in place, so re-activation
is a single variable flip.
