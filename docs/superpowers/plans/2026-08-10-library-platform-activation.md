# Library Platform and Pages Activation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the repository's real three-language library configuration and fixtures, make normal CI build the full site reproducibly, and publish current `main` through GitHub Pages.

**Architecture:** One mixed Rust/C++/Lean fixture proves the generic core and public pages. Normal CI prepares/builds adapters secretlessly and invokes the single Web build command with full Git history. Pages uploads a temporary artifact and deploys only a build whose source SHA is still current main.

**Tech Stack:** Rust 1.92.0, LLVM/Clang 22.1.0, Lean 4.30.0, Node 24.19.0, Astro/Pagefind, GitHub Pages Actions.

## Constraints

- **Branch:** `feat/061-library-platform-activation`
- **Depends on:** plan 060 merged to `main`.
- Read specification sections 12, 14, 15, 16.1, and 17.
- Do not create `gh-pages`, artifact, develop, library, solution, or epic integration branches.
- Generated site-data, adapter builds, Pagefind index, and Pages artifact remain ignored/uncommitted.
- Pages deploy requires the human gate below; complete implementation/review before asking for it.

### Task 1: Add production project config and mixed-language libraries

**Files:**
- Create: `config.toml`
- Create: `libraries/rust/algebra/monoid.rs`
- Create: `libraries/rust/algebra/monoid.rs.md`
- Create: `libraries/cpp/algebra/monoid.hpp`
- Create: `libraries/cpp/algebra/monoid.hpp.md`
- Create: `libraries/lean/Algebra/Monoid.lean`
- Create: `libraries/lean/Algebra/Monoid.lean.md`
- Create: `solutions/librarychecker-aplusb/aplusb/rust/ce.toml`
- Create: `solutions/librarychecker-aplusb/aplusb/rust/src/main.rs`
- Create: `crates/infrastructure/tests/fixtures/library-platform/verification/accepted.json`

- [ ] Write an acceptance fixture first for one page/source file per language, cross-file cycles, searchable
      `Monoid` kinds, one public solution, accepted evidence, private dependency, and all public DTO states.
- [ ] Configure exact roots, include/exclude, direct argv checks, analyzer symlink commands, syntax highlight,
      expected toolchains, OJ language mapping, site origin/base, and source size boundaries.
- [ ] Keep representative accepted/rejected/unavailable records under test fixtures only; production
      `verification/results/` remains empty until a real verify run records evidence.
- [ ] Run discovery, all adapters, check, site-data, schema, and search fixture tests.
- [ ] Invoke `/commit` with `feat: activate three-language library content`.

### Task 2: Make normal CI reproducible and complete

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `crates/infrastructure/tests/workflow_policy.rs`
- Modify: `.gitignore`

- [ ] Write failing policy tests for `fetch-depth: 0`, exact Rust/Node, full Action SHAs, no `dev` PR base,
      secretless prepare/build, `npm ci`, one `npm run site:build`, cache validation, and no deployment on PR.
- [ ] Pin every Action to a reviewed 40-character SHA and set minimal job permissions.
- [ ] Build both base `/` and `/compro-env/`; run schema/link/search/CSP/source-size checks and publish size/toolchain summary.
- [ ] Run workflow policy plus local equivalents; invoke `/commit` with `ci: build the complete library site`.

### Task 3: Add current-main-only Pages deployment

**Files:**
- Create: `.github/workflows/pages.yml`
- Modify: `crates/infrastructure/tests/workflow_policy.rs`
- Create: `docs/operations/pages.md`

- [ ] Write failing policy tests for main push/manual only, fixed `pages-publish` concurrency, build permissions,
      deploy artifact-only job, pinned Pages Actions, source-SHA metadata, and old-rerun rejection.
- [ ] Implement full-history build with `npm run site:build`, temporary Pages artifact, and deploy job with only
      `pages: write`/`id-token: write` plus `github-pages` environment.
- [ ] Immediately before deploy, query current main and require artifact source SHA equality.
- [ ] Run policy tests; invoke `/commit` with `ci: publish current main through GitHub Pages`.

### Task 4: Human Pages gate, merge, and deployed acceptance

- [ ] Invoke `/pr --base main`, then `/pr-review`; reach no new comments and green non-deploy checks first.
- [ ] **HUMAN GATE:** Ask the user to select Pages source `GitHub Actions`, restrict `github-pages` environment
      to `main`, confirm public origin/base, branch protection, and latest CI green. Do not deploy before confirmation.
- [ ] Run rollout Rust/Web verification and `git diff --check`.
- [ ] After confirmation, merge the reviewed green PR to `main`, then run the manual Pages workflow from
      `main`; verify deployed metadata SHA, links, 404, three language pages, search CSP, and result-only push.
- [ ] If deployed acceptance fails, create a new fix branch from current `main`; do not amend the merged branch.
- [ ] State that plan 061 unblocks plan 062.
