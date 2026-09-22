# GitHub Pages publishing (plan 061)

Operational reference for `.github/workflows/pages.yml`. The design contract
lives in `docs/superpowers/specs/2026-08-10-library-platform-design.md`
§15.5. This file explains what an operator needs to do around the workflow
and what to check when something goes wrong.

## What the workflow does

Two secretless jobs sitting behind a fixed `pages-publish` concurrency
group with `cancel-in-progress: true`:

1. **`build`** — read-only. Checkout pinned to `ref: main` with full
   history, Node pinned via `.node-version`, `npm ci`, `ce site-data
   generate`, then `npm run site:build`. Writes
   `web/dist/build-source.json` with the source commit SHA and site schema
   version, and uploads `web/dist` as a temporary Pages artifact. The
   source SHA comes from `git rev-parse HEAD` (the checked-out `main`
   tip), not from `github.sha`.
2. **`deploy`** — the only place with `pages: write` / `id-token: write`.
   Bound to the `github-pages` environment. Before invoking
   `actions/deploy-pages`, it resolves the current `main` HEAD through the
   GitHub API and refuses to publish an artifact whose source SHA does not
   match. This is what stops an old workflow rerun from rolling the site
   back — the concurrency group already blocks live races.

Triggers: `push` to `main`, `push` to `automation/verify`, and
`workflow_dispatch`. PR, schedule, and `pull_request_target` are
disallowed and enforced by `workflow_policy` tests.

### Why `automation/verify` is a trigger

Verification records live on the `automation/verify` state branch, and the
build overlays `verification/results/**` from it (see the "Overlay
verification records" step). A record push therefore changes the published
site even though no source commit landed, so the branch is a trigger and
the operator no longer runs `gh workflow run pages.yml` after a verify.

Three details make that safe:

- **`ref: main` on the checkout.** The trigger ref is not the source of
  truth; a record push must never publish `automation/verify`'s tree. The
  source always comes from `main`, records always from the overlay.
- **`git rev-parse HEAD` for the artifact SHA.** `github.sha` on a
  record-triggered run is the record commit, which can never equal `main`
  HEAD — the deploy job's stale-rerun guard would reject every
  record-triggered publish. Reading the checked-out HEAD keeps the guard
  meaningful: it still fails when the artifact was built from a `main` tip
  that has since been superseded.
- **No `paths:` filter.** A `paths:` filter applies to every branch in the
  same `push` trigger, and `main` must republish on any push. Every commit
  on `automation/verify` is a record commit anyway.

One live verify pushes three times (`persist_starting`, `persist_handle`,
`persist_terminal`). The `pages-publish` concurrency group cancels in
progress, so the two earlier builds are dropped and only the terminal one
publishes. That is intended: the terminal record is the one worth
publishing.

## One-time setup (human gate G2)

Before the first deploy, an operator must:

1. Set **Settings → Pages → Build and deployment** to `GitHub Actions`.
   Selecting the legacy `Deploy from a branch` mode bypasses the workflow
   entirely and would publish an unaudited tree.
2. Under **Settings → Environments → `github-pages`**, restrict deployment
   branches to `main` and `automation/verify`. No required reviewer is set
   — the merge gate on `main` is the approval boundary. The state branch
   has to be listed or the `deploy` job of a record-triggered run fails
   with `Branch is not allowed to deploy to github-pages due to
   environment protection rules`. It publishes `main`'s tree regardless
   (`ref: main`), and the App that writes the branch has no Workflows
   permission, so it cannot alter `pages.yml` to change that.
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
- **Every `automation/verify` push** does the same, so a verify record is
  published as soon as the worker commits it. No manual
  `gh workflow run pages.yml` is needed after a live verify.
- **Manual dispatch** starts from the current `main` HEAD, never a stored
  artifact. Use it for retrying a failed deploy or forcing a rebuild after
  an infra outage.
- **Old reruns are rejected.** The deploy job compares the artifact's
  `source_sha` output to the API's `commits/main.sha`. Mismatched runs
  fail before touching Pages, so replaying an old workflow after new
  commits landed cannot roll the site back.
- **Result-only pushes still publish.** `verification/results/**` updates
  reach the site twice: once when the worker pushes them to
  `automation/verify` (overlay) and again when the automation PR merges
  into `main`. Both paths regenerate the site, so the latest verify status
  is reflected either way.

## Debugging failures

- `Refusing to deploy artifact for <sha> — current main is <other>` in the
  deploy job means someone re-ran an older workflow after new commits
  merged. Merge or force nothing — just re-run the latest workflow.
- `Branch is not allowed to deploy to github-pages due to environment
  protection rules` on a run triggered by `automation/verify` means the
  state branch is missing from the environment's deployment branch policy
  (setup item 2). Add it with
  `gh api --method POST
  repos/<owner>/<repo>/environments/github-pages/deployment-branch-policies
  -f name='automation/verify' -f type=branch`, then re-run the workflow.
- Artifact retention is 1 day; expired artifacts require a manual
  dispatch. Do not restore from an older artifact by hand.
- `permissions:` failures usually mean the workflow ran outside the
  `github-pages` environment or the environment was renamed. Restore the
  environment name; the workflow policy tests guard against renames from
  our side.
- If Pages ever moves off the Actions-based source, the workflow rejects
  itself — `actions/configure-pages` fails when the repository is set to
  branch deploy.
