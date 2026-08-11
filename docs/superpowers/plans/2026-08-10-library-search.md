# Library Static Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Pagefind full-text search, exact file/symbol lookup, filters such as `lang:cpp`, and an accessible static result page.

**Architecture:** Astro emits Pagefind metadata and a minimal exact index from public DTOs. A small client parses queries, runs exact and Pagefind searches, merges by canonical page ID, sanitizes excerpts/URLs, and paginates the union without a backend.

**Tech Stack:** Pagefind 1.5.2, Astro 7.2.0, TypeScript 7.0.2, Vitest 4.1.10, Playwright 1.62.1.

## Constraints

- **Branch:** `feat/053-library-search`
- **Depends on:** plan 052 merged to `main`.
- Read specification sections 12.15 and 13.
- Do not snapshot or reinterpret Pagefind's undocumented numeric relevance score.
- Exact index excludes Markdown, source, diagnostics, dependencies, private data, and non-detail pages.
- Search result URLs must resolve under configured base to a generated detail URL and optional valid fragment.

### Task 1: Parse the complete query grammar

**Files:**
- Create: `web/src/search/query.ts`
- Create: `web/src/search/types.ts`
- Create: `web/tests/search-query.test.ts`

**Interfaces:**

```ts
export function parseSearchQuery(input: string): ParsedQuery | QueryError;
export function canonicalPage(input: string | null): number;
```

- [x] Write table tests for bare terms, phrases, quoted filters, escapes, repeated-key OR, cross-key AND,
      unknown keys as text, all invalid forms, `verified` aliasing, and page canonicalization.
- [x] Implement one-pass parsing without percent decoding; lowercase only keys/comparison values.
- [x] Run `npm test -- search-query`; invoke `/commit` with `feat: parse static search queries`.

### Task 2: Emit exact and Pagefind metadata

**Files:**
- Create: `web/src/search/exact-index.ts`
- Create: `web/src/search/build-index.ts`
- Modify: `web/src/pages/libraries/[...path].astro`
- Modify: `web/src/pages/solutions/[...path].astro`
- Create: `web/tests/search-index.test.ts`

- [x] Write failing tests for page IDs, aliases, symbol fragments, path prefixes, filters, duplicate names,
      symbol-only punctuation, private exclusion, and exact/public page-set equality.
- [x] Generate `exact-search-index.json`; emit Pagefind body/weights/filter/metadata only on detail articles.
- [x] Use `#symbols` for locationless symbols and validate every `doc-*`/`L*` fragment against generated HTML.
- [x] Run `npm test -- search-index`; invoke `/commit` with `feat: generate static search indexes`.

### Task 3: Implement and test the search page

**Files:**
- Create: `web/src/pages/search/index.astro`
- Create: `web/src/search/client.ts`
- Create: `web/src/search/merge.ts`
- Create: `web/src/components/SearchResults.astro`
- Create: `web/tests/search-merge.test.ts`
- Create: `web/e2e/search.spec.ts`
- Create: `web/playwright.config.ts`

- [x] Write failing merge tests for exact-first order, page-ID deduplication, all duplicate exact pages,
      sub-result limit/order, 20-card pagination, filter-only queries, and Pagefind failure.
- [x] Implement the single header form, URL state, loading/error/empty states, text-only metadata, safe `mark`
      excerpts, noscript browse links, and Previous/Next links preserving `q`.
- [x] Run browser tests for root/project base, reload/history/share, WASM worker CSP, keyboard labels,
      `monoid lang:cpp`, `kind:trait`, punctuation symbols, and invalid query.
- [x] Invoke `/commit` with `feat: add static library search experience`.

### Task 4: Define the single Web build boundary and deliver

**Files:**
- Modify: `package.json`
- Create: `web/scripts/site-build.mjs`
- Create: `web/scripts/site-verify.mjs`
- Create: `web/tests/site-build.test.ts`

- [x] Test exact order: offline adapter build, `ce check`, site-data, Astro, Pagefind, final verification.
- [x] Implement `npm run site:build`; do not call prepare. Make `site:dev` reject a stale Pagefind index.
- [x] Run root/project-base builds, all unit/browser/link/CSP/index-size checks, and rollout Rust verification.
- [x] Invoke `/commit` with `docs: record static search completion`.
- [ ] Invoke `/pr --base main`; link plan 053 and state that it satisfies the Web prerequisite of plan 060.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
