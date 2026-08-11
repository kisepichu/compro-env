# Library Static Web Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the approved semantic routes and accessible source/detail pages as base-path-safe static HTML.

**Architecture:** Astro consumes only schema-validated site-data and never discovers repository files. Components follow the semantic handoff; CSS is deliberately minimal so the user's later Claude Design output can replace presentation without changing routes, landmarks, anchors, or search attributes.

**Tech Stack:** Node 24.19.0 LTS, Astro 7.2.0, TypeScript 7.0.2, Shiki 4.4.3, Vitest 4.1.10.

## Constraints

- **Branch:** `feat/052-library-static-web`
- **Depends on:** plan 051 merged to `main`.
- Read specification sections 12.0-12.14 and the complete Web semantic handoff.
- No image handling, client-side source rendering, raw Markdown HTML, dynamic server, or root-hardcoded URLs.
- Preserve the semantic contract so visual design can be integrated later as CSS/Astro component markup only.
- Every page has one `h1`; navigation, breadcrumbs, footer, and non-detail pages are search-excluded.

### Task 1: Bootstrap a schema-validated Astro package

**Files:**
- Create: `.node-version`
- Create: `package.json`
- Create: `package-lock.json`
- Create: `web/astro.config.mjs`
- Create: `web/tsconfig.json`
- Create: `web/src/lib/site-data.ts`
- Create: `web/src/lib/url.ts`
- Create: `web/src/env.d.ts`
- Create: `web/tests/site-data.test.ts`
- Modify: `.gitignore`

- [x] Write failing tests for schema version mismatch, root/project base URLs, segment encoding, trailing slash,
      repository escape, and private-field fixture rejection.
- [x] Pin every direct package to an exact version and load `CE_SITE_ORIGIN`, `CE_SITE_BASE`, and site-data path.
- [x] Validate JSON before route generation and centralize all internal/canonical/asset URL construction.
- [x] Run `npm ci && npm test`; invoke `/commit` with `build: bootstrap static library web package`.

### Task 2: Render shared layout, browse routes, and 404

**Files:**
- Create: `web/src/layouts/BaseLayout.astro`
- Create: `web/src/components/Header.astro`
- Create: `web/src/components/Breadcrumbs.astro`
- Create: `web/src/components/Status.astro`
- Create: `web/src/components/Footer.astro`
- Create: `web/src/pages/index.astro`
- Create: `web/src/pages/libraries/index.astro`
- Create: `web/src/pages/libraries/[...path].astro`
- Create: `web/src/pages/solutions/index.astro`
- Create: `web/src/pages/solutions/[...path].astro`
- Create: `web/src/pages/404.astro`
- Create: `web/tests/semantic-pages.test.ts`

- [x] Write failing DOM tests for every route, empty lists, stable ordering, one `h1`, landmarks, breadcrumbs,
      navigation, status text, detail-only indexing, and static 404/noindex behavior.
- [x] Implement route generation from public DTOs with minimal reusable components and no design-only wrappers.
- [x] Use canonical page ID `library:*` or `solution:*` on detail articles for later result deduplication.
- [x] Run `npm test -- semantic-pages`; invoke `/commit` with `feat: render semantic library site routes`.

### Task 3: Render safe Markdown and line-addressable source

**Files:**
- Create: `web/src/lib/markdown.ts`
- Create: `web/src/lib/headings.ts`
- Create: `web/src/lib/source.ts`
- Create: `web/src/components/Documentation.astro`
- Create: `web/src/components/SourceCode.astro`
- Create: `web/tests/content-rendering.test.ts`
- Create: `web/tests/fixtures/site-data.json`

- [x] Write failing tests for heading rules/hash IDs, raw HTML stripping, GFM, unknown language fallback,
      text-only source, CRLF/Unicode, `L1` anchors, permalink base, source limits, and unsafe Markdown links.
- [x] Implement the spec's SHA-256 heading algorithm and allowlist sanitation.
- [x] Render Shiki at build time, adding stable line IDs/permalinks without trusting source as HTML.
- [x] Run `npm test -- content-rendering`; invoke `/commit` with `feat: render safe documentation and source`.

### Task 4: Deliver static Web

- [x] Run root and `/compro-env/` builds plus internal-link/HTML semantic checks.
- [x] Run `npm ci`, all Web tests, rollout Rust verification, and `git diff --check`.
- [x] Invoke `/commit` with `docs: record static web completion`.
- [x] Invoke `/pr --base main`; link plan 052 and state that it unblocks plan 053.
- [x] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
