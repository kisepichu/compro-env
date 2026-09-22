# GitHub Pages publishing (plan 061)

Operational reference for `.github/workflows/pages.yml`. The design contract
lives in `docs/superpowers/specs/2026-08-10-library-platform-design.md`
§15.5. This file explains what an operator needs to do around the workflow
and what to check when something goes wrong.

Adding a library or a verify solution (author-facing procedure, Japanese):
`docs/operations/contributing-content.md`.

## What the workflow does

Three secretless jobs sitting behind a fixed `pages-publish` concurrency
group with `cancel-in-progress: true`:

1. **`gate`** — read-only (`contents: read`, `actions: read`), no build
   tooling. Decides whether this event is worth a rebuild. `push` to `main`
   and `workflow_dispatch` always pass. A `workflow_run` event passes only
   when the triggering `verify` run succeeded **and** its
   `Persist terminal result (App-only)` job succeeded, i.e. a terminal
   record actually landed on `automation/verify`.
2. **`build`** — read-only, gated on `needs.gate.outputs.should_build`.
   Checkout pinned to `ref: main` with full history, Node pinned via
   `.node-version`, `npm ci`, `ce site-data generate`, then `npm run
   site:build`. Writes `web/dist/build-source.json` with the source commit
   SHA and site schema version, and uploads `web/dist` as a temporary Pages
   artifact. The source SHA comes from `git rev-parse HEAD` (the
   checked-out `main` tip), not from `github.sha`.
3. **`deploy`** — the only place with `pages: write` / `id-token: write`.
   Bound to the `github-pages` environment. Before invoking
   `actions/deploy-pages`, it resolves the current `main` HEAD through the
   GitHub API and refuses to publish an artifact whose source SHA does not
   match. This is what stops an old workflow rerun from rolling the site
   back — the concurrency group already blocks live races.

Triggers: `push` to `main`, `workflow_run` on the `verify` workflow
(`types: [completed]`), and `workflow_dispatch`. PR, schedule, and
`pull_request_target` are disallowed and enforced by `workflow_policy`
tests.

### Why verify records republish the site

Verification records live on the `automation/verify` state branch, and the
build overlays `verification/results/**` from it (see the "Overlay
verification records" step). A record therefore changes the published site
even though no source commit landed, so the site has to follow the verify
pipeline — an operator no longer runs `gh workflow run pages.yml` after a
live verify.

**The trigger is `workflow_run`, not a `push` on `automation/verify`.** A
push-triggered run executes the workflow file *as it exists on the pushed
ref*, and the `github-pages` environment would have to list that branch in
its deployment branch policy for the deploy job to run. Together those two
facts are a privilege escalation: anything able to write the state branch
(the verify App installation token) could replace `pages.yml` and publish
arbitrary content to the live site. With the policy restricted to `main`,
leaking that token gets an attacker records on a side branch and nothing
else — reaching the site still requires passing `main`'s branch protection
and required checks.

A `workflow_run` run has no such exposure: both `github.ref` and the
workflow file come from the default branch, so `github-pages` keeps its
`main`-only policy and this file is always the reviewed one.

Three further details:

- **The `gate` job.** `verify` runs on a `*/5 * * * *` cron and most ticks
  publish nothing. Rebuilding on every completion would start a
  multi-minute site build every five minutes, with `cancel-in-progress`
  making them kill each other. `gate` reads the triggering run's job list
  (`GET /repos/{repo}/actions/runs/{id}/jobs`) and only proceeds when the
  `Persist terminal result` job concluded `success`. The job name carries
  the caller's prefix (`Invoke verify worker / Persist terminal result
  (App-only)`) because the worker is a reusable workflow, so the check
  matches on a substring.
- **`ref: main` on the checkout.** `workflow_run` already resolves to the
  default branch, but the pin makes that a property of this file rather
  than of the event. The source always comes from `main`, records always
  from the overlay.
- **`git rev-parse HEAD` for the artifact SHA.** `github.sha` tracks the
  event rather than the built tree — on `workflow_run` it is the
  default-branch tip recorded when `verify` finished, which can drift from
  what `ref: main` resolves minutes later. Reading the real HEAD keeps the
  deploy job's stale-rerun guard meaningful: it still fails when the
  artifact was built from a `main` tip that has since been superseded.

One live verify pushes three record commits (`persist_starting`,
`persist_handle`, `persist_terminal`) but produces a single `verify` run,
and only its terminal-record completion passes the gate — so a verify
attempt causes at most one site build.

## One-time setup (human gate G2)

Before the first deploy, an operator must:

1. Set **Settings → Pages → Build and deployment** to `GitHub Actions`.
   Selecting the legacy `Deploy from a branch` mode bypasses the workflow
   entirely and would publish an unaudited tree.
2. Under **Settings → Environments → `github-pages`**, restrict deployment
   branches to **`main` only**. No required reviewer is set — the merge
   gate on `main` is the approval boundary. Do not add
   `automation/verify` or any other bot-writable branch: a push-triggered
   run would execute that branch's copy of `pages.yml`, so listing it
   would let whoever can write the branch publish arbitrary content. The
   verify pipeline reaches the site through `workflow_run`, which runs
   from the default branch and needs no extra policy entry.
3. Confirm the site origin and base path referenced by `CE_SITE_ORIGIN` /
   `CE_SITE_BASE`. For GitHub Project Pages this is
   `/<repository-name>/` (`/compro-env/` here); the workflow reads the
   base path from `actions/configure-pages` outputs so it matches the
   Pages configuration.
4. Verify `main` branch protection has the CI status check required and
   force-push disabled — otherwise a rewritten history could pass the
   SHA-equality check with tampered source.

Do not run the workflow before every item is confirmed. The plan calls out
G2 as a hard gate.

## Operating the workflow

- **Every `main` push** kicks a fresh build and deploy. Merge → wait for
  the workflow → check the deployment summary.
- **Every `verify` run that persists a terminal record** republishes the
  site through the `workflow_run` trigger, so a verify result appears
  without waiting for the automation PR to merge and without a manual
  `gh workflow run pages.yml`. Runs that published nothing (the usual
  5-minute tick, dry-run ticks, failed runs) stop at the `gate` job, which
  shows up as a skipped `build` / `deploy`. That is the healthy state, not
  a failure.
- **Manual dispatch** starts from the current `main` HEAD, never a stored
  artifact. Use it for retrying a failed deploy or forcing a rebuild after
  an infra outage.
- **Old reruns are rejected.** The deploy job compares the artifact's
  `source_sha` output to the API's `commits/main.sha`. Mismatched runs
  fail before touching Pages, so replaying an old workflow after new
  commits landed cannot roll the site back.
- **Result-only pushes still publish.** `verification/results/**` updates
  reach the site twice: once when the `verify` run that persisted them
  completes (overlay from `automation/verify`) and again when the
  automation PR merges into `main`. Both paths regenerate the site, so the
  latest verify status is reflected either way.

## Debugging failures

- `Refusing to deploy artifact for <sha> — current main is <other>` in the
  deploy job means someone re-ran an older workflow after new commits
  merged. Merge or force nothing — just re-run the latest workflow.
- A `workflow_run`-triggered run whose `build` / `deploy` are skipped is
  normal: the `gate` job found no successful `Persist terminal result` job
  in the triggering `verify` run. Check the gate log — it prints which
  run id it inspected and why it declined. If a terminal record *did*
  land and the gate still declined, the worker job name changed; update
  the substring the gate matches on.
- `Branch is not allowed to deploy to github-pages due to environment
  protection rules` means the run's ref is not `main`. It should be
  impossible via the shipped triggers (`push` is `main`-only and
  `workflow_run` executes on the default branch). Do not "fix" it by
  widening the environment's deployment branch policy — find out which
  ref started the run first.
- Artifact retention is 1 day; expired artifacts require a manual
  dispatch. Do not restore from an older artifact by hand.
- `permissions:` failures usually mean the workflow ran outside the
  `github-pages` environment or the environment was renamed. Restore the
  environment name; the workflow policy tests guard against renames from
  our side.
- If Pages ever moves off the Actions-based source, the workflow rejects
  itself — `actions/configure-pages` fails when the repository is set to
  branch deploy.
