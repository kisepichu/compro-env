# GitHub Pages publishing (plan 061)

Operational reference for `.github/workflows/pages.yml`. The design contract
lives in `docs/superpowers/specs/2026-08-10-library-platform-design.md`
§15.5. This file explains what an operator needs to do around the workflow
and what to check when something goes wrong.

## What the workflow does

Two secretless jobs sitting behind a fixed `pages-publish` concurrency
group with `cancel-in-progress: true`:

1. **`build`** — read-only. Full-history checkout, Node pinned via
   `.node-version`, `npm ci`, then a single `npm run site:build`. Writes
   `web/dist/build-source.json` with the source commit SHA and site schema
   version, and uploads `web/dist` as a temporary Pages artifact.
2. **`deploy`** — the only place with `pages: write` / `id-token: write`.
   Bound to the `github-pages` environment. Before invoking
   `actions/deploy-pages`, it resolves the current `main` HEAD through the
   GitHub API and refuses to publish an artifact whose source SHA does not
   match. This is what stops an old workflow rerun from rolling the site
   back — the concurrency group already blocks live races.

`main` push and `workflow_dispatch` are the only triggers. PR, schedule,
and `pull_request_target` are disallowed and enforced by `workflow_policy`
tests.

## One-time setup (human gate G2)

Before the first deploy, an operator must:

1. Set **Settings → Pages → Build and deployment** to `GitHub Actions`.
   Selecting the legacy `Deploy from a branch` mode bypasses the workflow
   entirely and would publish an unaudited tree.
2. Under **Settings → Environments → `github-pages`**, restrict deployment
   branches to `main` only. No required reviewer is set — the merge gate
   on `main` is the approval boundary.
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
  the workflow → check the deployment summary. Only `main` push and
  manual dispatch trigger it.
- **Manual dispatch** starts from the current `main` HEAD, never a stored
  artifact. Use it for retrying a failed deploy or forcing a rebuild after
  an infra outage.
- **Old reruns are rejected.** The deploy job compares the artifact's
  `source_sha` output to the API's `commits/main.sha`. Mismatched runs
  fail before touching Pages, so replaying an old workflow after new
  commits landed cannot roll the site back.
- **Result-only pushes still publish.** `verification/results/**` updates
  are ordinary `main` pushes; the site regenerates so the latest verify
  status is reflected.

## Debugging failures

- `Refusing to deploy artifact for <sha> — current main is <other>` in the
  deploy job means someone re-ran an older workflow after new commits
  merged. Merge or force nothing — just re-run the latest workflow.
- Artifact retention is 1 day; expired artifacts require a manual
  dispatch. Do not restore from an older artifact by hand.
- `permissions:` failures usually mean the workflow ran outside the
  `github-pages` environment or the environment was renamed. Restore the
  environment name; the workflow policy tests guard against renames from
  our side.
- If Pages ever moves off the Actions-based source, the workflow rejects
  itself — `actions/configure-pages` fails when the repository is set to
  branch deploy.
