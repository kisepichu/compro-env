# Library Visual Structure Refinement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate the approved pastel visual layer and refine browse/detail markup so information order, compact timestamps, links, and headings match the approved Human gate G1 design.

**Architecture:** Keep the existing Astro route entries and pure HTML renderers. Add one pure UTC timestamp-formatting module, update renderer DOM order without changing routes or public DTOs, and adapt the shared stylesheet to the new semantic order; root/project-base build verifiers remain the integration boundary.

**Tech Stack:** Astro 7.2.0, TypeScript 7.0.2, Vitest 4.1.10, JSDOM 30.0.1, plain CSS, Node 24.19.0, Rust/Cargo repository gates.

## Global Constraints

- Approved refinement: `docs/superpowers/specs/2026-08-12-library-visual-structure-refinement-design.md`.
- Normative structure: `docs/superpowers/specs/2026-08-10-library-web-structure-handoff.md` and `docs/superpowers/specs/2026-08-10-library-platform-design.md`.
- Preserve route URLs, canonical URLs, breadcrumb links, Pagefind attributes, `L*` source anchors, source permalink behavior, landmark order, and status text labels.
- Keep `VerificationEvidence.solution_page_id`, `SolutionPageData.verifies`, and `SolutionPageData.direct_dependencies` unchanged in the DTO; only the public rendering changes.
- Every `<time datetime>` retains the original RFC 3339 string. Compact text is `YYYY-MM-DD`; detailed text is `YYYY-MM-DD HH:mm UTC`.
- Use only the same-origin `assets/site.css`; do not add external fonts, stylesheets, scripts, images, `@import`, `@font-face`, or CSS `url(...)` references.
- The site must build under both `/` and `/compro-env/` bases.
- Use TDD for every new behavior: run the focused test and observe the expected failure before modifying production code.
- Preserve unrelated `pnpm-lock.yaml` and `pnpm-workspace.yaml`; never stage them for this work.
- Before commits, attempt `prek run --all-files`; if the repository has no prek/pre-commit configuration, record that and continue with the repository gates.
- Before delivery run `npm test`, `npm run verify:builds`, `npm run site:build`, `cargo test --all`, `cargo clippy --all --all-features -- -D warnings`, and `cargo fmt --all --check`.

---

### Task 1: Checkpoint the approved visual baseline

**Files:**
- Create: `web/public/assets/site.css`
- Modify: `web/src/lib/pages/document.ts`
- Modify: `web/src/lib/pages/status.ts`
- Modify: `web/src/pages/search/index.astro`
- Modify: `web/tests/semantic-pages.test.ts`
- Modify: `scripts/verify-web-build.mjs`
- Modify: `web/scripts/site-verify.mjs`
- Include: `docs/superpowers/plans/2026-08-12-library-visual-structure-refinement.md`

**Interfaces:**
- Consumes: `toAssetUrl(config, ["assets", "site.css"]): string` and `renderStatus(variant, value): string`.
- Produces: exactly one base-aware stylesheet link per page, a copied `assets/site.css`, and non-empty decorative status SVGs.

This task checkpoints the already completed external-design integration after its red/green cycle. Do not rewrite the returned stylesheet before recording this boundary.

- [ ] **Step 1: Confirm the baseline diff and unrelated files**

Run:

```bash
git status --short
git diff --check
git diff --stat
```

Expected: the visual files listed above are modified/untracked; the two `pnpm-*` files remain untracked and unstaged; `git diff --check` exits 0.

- [ ] **Step 2: Verify the visual baseline behavior**

Run:

```bash
npm test
npm run verify:builds
npm run site:build
```

Expected: 173 Vitest tests pass, root and `/compro-env/` each build 13 HTML pages, and `site-verify: OK` is printed.

- [ ] **Step 3: Verify stylesheet safety**

Run:

```bash
node - <<'NODE'
const fs = require('node:fs');
const source = fs.readFileSync('web/public/assets/site.css', 'utf8');
const css = source.replace(/\/\*[\s\S]*?\*\//g, '');
for (const [name, pattern] of [
  ['@import', /@import\b/i],
  ['@font-face', /@font-face\b/i],
  ['url()', /url\s*\(/i],
  ['external URL', /https?:\/\//i],
]) {
  if (pattern.test(css)) throw new Error(`${name} found`);
}
NODE
```

Expected: exit 0 with no output.

- [ ] **Step 4: Commit only the visual baseline and this plan**

```bash
git add docs/superpowers/plans/2026-08-12-library-visual-structure-refinement.md \
  scripts/verify-web-build.mjs \
  web/public/assets/site.css \
  web/scripts/site-verify.mjs \
  web/src/lib/pages/document.ts \
  web/src/lib/pages/status.ts \
  web/src/pages/search/index.astro \
  web/tests/semantic-pages.test.ts
git commit --no-gpg-sign -m "feat: apply library site visual design"
```

Expected: `pnpm-lock.yaml` and `pnpm-workspace.yaml` are not in the commit.

---

### Task 2: Add one UTC timestamp presentation boundary

**Files:**
- Create: `web/src/lib/pages/time.ts`
- Create: `web/tests/time-format.test.ts`
- Modify: `web/src/lib/pages/home.ts`
- Modify: `web/src/lib/pages/libraries.ts`
- Modify: `web/src/lib/pages/solutions.ts`

**Interfaces:**
- Consumes: validated RFC 3339 strings from `LibraryPageData.updated_at`, `SolutionPageData.solved_at`, `VerificationEvidence.judged_at`, and `VerificationResultPublic.judged_at`.
- Produces: `formatCompactTimestamp(value: string): string` and `formatDetailedTimestamp(value: string): string`.

- [ ] **Step 1: Write failing unit tests for UTC formatting and fallback**

Create `web/tests/time-format.test.ts`:

```ts
import { describe, expect, it } from "vitest";

import {
  formatCompactTimestamp,
  formatDetailedTimestamp,
} from "@/lib/pages/time.ts";

describe("timestamp presentation", () => {
  it("formats compact timestamps as a UTC calendar date", () => {
    expect(formatCompactTimestamp("2026-08-10T00:30:00+09:00")).toBe(
      "2026-08-09",
    );
  });

  it("formats detailed timestamps to UTC minute precision", () => {
    expect(formatDetailedTimestamp("2026-08-10T00:30:45+09:00")).toBe(
      "2026-08-09 15:30 UTC",
    );
  });

  it("returns invalid input unchanged", () => {
    expect(formatCompactTimestamp("not-a-timestamp")).toBe("not-a-timestamp");
    expect(formatDetailedTimestamp("not-a-timestamp")).toBe("not-a-timestamp");
  });
});
```

- [ ] **Step 2: Run the focused test and observe RED**

Run:

```bash
npm test -- tests/time-format.test.ts
```

Expected: FAIL because `@/lib/pages/time.ts` does not exist.

- [ ] **Step 3: Implement the pure formatting module**

Create `web/src/lib/pages/time.ts`:

```ts
function normalizedIso(value: string): string | null {
  const epoch = Date.parse(value);
  return Number.isFinite(epoch) ? new Date(epoch).toISOString() : null;
}

export function formatCompactTimestamp(value: string): string {
  const iso = normalizedIso(value);
  return iso === null ? value : iso.slice(0, 10);
}

export function formatDetailedTimestamp(value: string): string {
  const iso = normalizedIso(value);
  return iso === null ? value : `${iso.slice(0, 10)} ${iso.slice(11, 16)} UTC`;
}
```

- [ ] **Step 4: Run the focused unit test and observe GREEN**

Run:

```bash
npm test -- tests/time-format.test.ts
```

Expected: 3 tests pass.

- [ ] **Step 5: Add failing renderer assertions for text versus datetime**

In `web/tests/semantic-pages.test.ts`, assert these representative contracts:

```ts
const compact = parse(renderHomePage(rootConfig, buildFixtureSiteData()))
  .querySelector(".recent-solutions time")!;
expect(compact.getAttribute("datetime")).toBe("2026-08-10T13:00:00Z");
expect(compact.textContent).toBe("2026-08-10");

const detailData = buildFixtureSiteData();
const detail = parse(
  await renderSolutionDetailPage(
    rootConfig,
    detailData,
    detailData.solutions[0],
  ),
).querySelector(".solution-language-time time")!;
expect(detail.getAttribute("datetime")).toBe("2026-08-10T13:00:00Z");
expect(detail.textContent).toBe("2026-08-10 13:00 UTC");
```

Also assert a contest card, problem card, library list item, library detail header, library evidence row, and solution verification `Judged` value use the correct compact/detailed formatter.

- [ ] **Step 6: Run the renderer test and observe RED**

Run:

```bash
npm test -- tests/semantic-pages.test.ts
```

Expected: FAIL because renderer text still contains raw RFC 3339 values.

- [ ] **Step 7: Apply the formatters without changing datetime or sort keys**

Import `formatCompactTimestamp` in `home.ts`, and both functions where needed in `libraries.ts` and `solutions.ts`.

Use compact formatting for:

```text
Home recent library updated_at
Home recent solution solved_at
Library directory file card updated_at
Solutions root contest latestSolvedAt
Contest problem latestSolvedAt
Problem solution card solved_at
```

Use detailed formatting for:

```text
Library detail updated_at
Library verification evidence judged_at
Solution detail solved_at
Solution verification result judged_at
```

Every template keeps `datetime="${escapeAttribute(originalValue)}"` and escapes only the formatter result for visible text.

- [ ] **Step 8: Run time and semantic tests**

Run:

```bash
npm test -- tests/time-format.test.ts tests/semantic-pages.test.ts
```

Expected: all focused tests pass.

- [ ] **Step 9: Commit the timestamp boundary**

```bash
git add web/src/lib/pages/time.ts \
  web/src/lib/pages/home.ts \
  web/src/lib/pages/libraries.ts \
  web/src/lib/pages/solutions.ts \
  web/tests/time-format.test.ts \
  web/tests/semantic-pages.test.ts
git commit --no-gpg-sign -m "feat: format public timestamps for display"
```

---

### Task 3: Refine Home and browse-page structure

**Files:**
- Modify: `web/src/lib/pages/home.ts`
- Modify: `web/src/lib/pages/libraries.ts`
- Modify: `web/src/lib/pages/solutions.ts`
- Modify: `web/public/assets/site.css`
- Modify: `web/tests/semantic-pages.test.ts`

**Interfaces:**
- Consumes: `formatCompactTimestamp(value)` from Task 2 and existing base-aware path helpers.
- Produces: root lists without redundant sections, subtitle-free browse headers, and stable row DOM for Home/problem solutions.

- [ ] **Step 1: Write failing semantic tests for browse structure**

Add assertions to `web/tests/semantic-pages.test.ts`:

```ts
const recent = parse(renderHomePage(rootConfig, buildFixtureSiteData()))
  .querySelector(".recent-solutions .solution-card")!;
expect([...recent.children].map((el) =>
  el.matches("h3") ? "title" :
  el.classList.contains("solution-language") ? "language" :
  el.classList.contains("solution-contest") ? "contest" :
  el.classList.contains("solution-solved") ? "time" :
  el.classList.contains("status-badge") ? "status" : "unexpected",
)).toEqual(["title", "language", "contest", "time", "status"]);

const librariesRoot = parse(
  renderLibrariesRootPage(rootConfig, buildFixtureSiteData()),
);
expect(librariesRoot.querySelector("main > .page-header + .language-list"))
  .toBeTruthy();
expect(librariesRoot.querySelector("main > section.languages")).toBeNull();
expect(librariesRoot.querySelector("main h2")).toBeNull();

const solutionsRoot = parse(
  renderSolutionsRootPage(rootConfig, buildFixtureSiteData()),
);
expect(solutionsRoot.querySelector("main > .page-header + .contest-list"))
  .toBeTruthy();
expect(solutionsRoot.querySelector("main > section.contests")).toBeNull();
expect(solutionsRoot.querySelector("main h2")).toBeNull();
```

For empty roots, assert `.page-header + .empty-state`. For library category, contest, and problem pages, assert `.page-header .subtitle` is absent. For the first problem-page solution row, map direct children and expect:

```ts
["title", "language", "time", "dependencies", "status"]
```

- [ ] **Step 2: Run the semantic test and observe RED**

Run:

```bash
npm test -- tests/semantic-pages.test.ts
```

Expected: FAIL on the old Home order, redundant root sections/headings, browse subtitles, and status-before-dependency problem card order.

- [ ] **Step 3: Update renderer DOM with no CSS ordering**

Make these minimal renderer changes:

```text
home.ts: move solution-language before solution-contest.
libraries.ts: return page-header + languagesHtml directly; remove subtitleText and category subtitle.
solutions.ts: return page-header + contestsHtml directly; remove contest/problem subtitle nodes.
solutions.ts renderSolutionCard: emit title, language, solved time, dependency count, then status.
```

Do not use CSS `order`; DOM and visual order must agree.

- [ ] **Step 4: Run semantic tests and observe renderer GREEN**

Run:

```bash
npm test -- tests/semantic-pages.test.ts
```

Expected: all semantic tests pass before CSS adjustments.

- [ ] **Step 5: Convert problem solutions to the shared row-list visual language**

In `web/public/assets/site.css`:

- Remove `.solutions .solution-list` and `.solutions .solution-card` from the auto-fill card-grid selectors.
- Add `.solutions .solution-list` to the bordered row-list container and separator rules.
- Add `.solutions .solution-card` to the flex row rule with title `flex: 1 1 16rem`.
- At `min-width: 720px` with subgrid, use columns:

```css
grid-template-columns:
  minmax(0, 1fr) 4.5rem max-content max-content max-content;
```

- Update the Home recent solution columns to `title | language | contest/problem | date | status`.
- Make `.solutions .solution-card .status-badge` the final right-aligned column.
- In the mobile block, let problem solution titles take the full row and preserve source order.

- [ ] **Step 6: Run focused tests and both base builds**

Run:

```bash
npm test -- tests/semantic-pages.test.ts
npm run verify:builds
```

Expected: semantic tests pass; root and `/compro-env/` builds pass all stylesheet, link, landmark, and heading checks.

- [ ] **Step 7: Commit the browse refinement**

```bash
git add web/public/assets/site.css \
  web/src/lib/pages/home.ts \
  web/src/lib/pages/libraries.ts \
  web/src/lib/pages/solutions.ts \
  web/tests/semantic-pages.test.ts
git commit --no-gpg-sign -m "feat: refine library browse structure"
```

---

### Task 4: Link evidence and simplify Solution detail

**Files:**
- Modify: `web/src/lib/pages/libraries.ts`
- Modify: `web/src/lib/pages/solutions.ts`
- Modify: `web/public/assets/site.css`
- Modify: `web/tests/semantic-pages.test.ts`

**Interfaces:**
- Consumes: `VerificationEvidence.solution_id`, `VerificationEvidence.solution_page_id`, `SiteData.solutions`, `solutionPath(config, contestId, problemCode, solutionName)`, and `formatDetailedTimestamp(value)`.
- Produces: strict base-aware evidence links, `solution-header-meta`, and a visible Depends on-only section that retains `id="libraries"`.

- [ ] **Step 1: Write failing evidence-link and order tests**

In the Library detail tests, add:

```ts
const siteData = buildFixtureSiteData();
const doc = parse(
  await renderLibraryDetailPage(projectConfig, siteData, siteData.libraries[0]),
);
const evidence = doc.querySelector(".verification-evidence")!;
const solutionLink = evidence.querySelector(".evidence-solution a")!;
expect(solutionLink.textContent).toBe("abc300/a/dijkstra_solve");
expect(solutionLink.getAttribute("href")).toBe(
  "/compro-env/solutions/abc300/a/dijkstra_solve/",
);
expect([...evidence.children].map((el) =>
  el.classList.contains("evidence-solution") ? "solution" :
  el.classList.contains("evidence-judged") ? "judged" :
  el.classList.contains("status-badge") ? "status" :
  el.classList.contains("stale-reason") ? "stale" : "unexpected",
).slice(0, 3)).toEqual(["solution", "judged", "status"]);
```

Add the strict invariant test:

```ts
const broken = buildFixtureSiteData();
broken.libraries[0].verification.evidence[0].solution_page_id =
  "solution:missing/a/answer";
await expect(
  renderLibraryDetailPage(rootConfig, broken, broken.libraries[0]),
).rejects.toThrow(/verification evidence.*public solution/i);
```

- [ ] **Step 2: Write failing Solution detail structure tests**

Add:

```ts
const siteData = buildFixtureSiteData();
const sol = siteData.solutions[0];
const doc = parse(await renderSolutionDetailPage(rootConfig, siteData, sol));
const metadata = doc.querySelector(".solution-header-meta")!;
expect([...metadata.children].map((el) =>
  el.classList.contains("language") ? "language" :
  el.classList.contains("oj") ? "oj" :
  el.matches("time") ? "time" :
  el.classList.contains("status-badge") ? "status" : "unexpected",
)).toEqual(["language", "oj", "time", "status"]);
expect(doc.querySelector(".solution-meta")).toBeNull();
expect(doc.querySelector(".solution-language-time")).toBeNull();

const navLink = doc.querySelector('.in-page-navigation a[href="#libraries"]')!;
expect(navLink.textContent).toBe("Depends on");
const section = doc.getElementById("libraries")!;
expect(section.querySelector("h2")!.textContent).toBe("Depends on");
expect(section.querySelector("h3")).toBeNull();
expect(section.querySelector(".verifies-list")).toBeNull();
expect(section.querySelector(".depends-on-list")).toBeTruthy();
expect(sol.verifies.length).toBeGreaterThan(0);
```

- [ ] **Step 3: Run the semantic test and observe RED**

Run:

```bash
npm test -- tests/semantic-pages.test.ts
```

Expected: FAIL because evidence identity is plain text, status precedes judged time, missing evidence is not rejected, the detail header has two metadata rows, and Verifies is visible.

- [ ] **Step 4: Implement strict evidence resolution and link rendering**

In `libraries.ts`, import `solutionPath` and change the renderer signature to:

```ts
function renderVerificationEvidenceList(
  config: UrlConfig,
  siteData: SiteData,
  evidence: readonly VerificationEvidence[],
): string
```

Resolve each item with both identities:

```ts
const solution = siteData.solutions.find(
  (candidate) =>
    candidate.page_id === ev.solution_page_id &&
    candidate.solution_id === ev.solution_id,
);
if (solution === undefined) {
  throw new Error(
    `Verification evidence ${ev.solution_id} does not resolve to a public solution`,
  );
}
const href = solutionPath(
  config,
  solution.contest_id,
  solution.problem_code,
  solution.solution_name,
);
```

Render `.evidence-solution` with an `<a href="..."><code>solution_id</code></a>`, then `.evidence-judged`, then `renderStatus(...)`, then optional `.stale-reason`. Pass `config` and `siteData` from the Library detail renderer.

- [ ] **Step 5: Implement the single-row Solution header and Depends on view**

In `solutions.ts`:

- Remove `contestLink`, `problemLink`, `.solution-meta`, and `.solution-language-time`.
- Render one `.solution-header-meta` with language, OJ, detailed solved time, and status in that DOM order.
- Change the in-page item label for `id: "libraries"` to `Depends on`.
- Keep `<section id="libraries">`, set its `h2` to `Depends on`, remove both `h3` nodes and the `sol.verifies` renderer, and render only `sol.direct_dependencies` plus the private dependency note.
- Do not mutate or delete `sol.verifies` from the DTO.

- [ ] **Step 6: Run semantic tests and observe GREEN**

Run:

```bash
npm test -- tests/semantic-pages.test.ts
```

Expected: all semantic tests pass, including the strict missing-evidence rejection.

- [ ] **Step 7: Align CSS with the new detail DOM order**

In `site.css`:

- Replace `.solution-meta` / `.solution-language-time` selectors with `.solution-header-meta`.
- Style language and OJ chips within that row.
- Reset the nested status margin and place the final status at the row end on desktop; allow source-order wrapping on mobile.
- Update the verification evidence comment and flex fallback for `solution → judged/OJ → status`.
- Keep the subgrid at `minmax(0, 1fr) max-content max-content`; natural DOM placement now puts status in the third/rightmost column.
- Keep `.stale-reason` spanning all columns.

- [ ] **Step 8: Verify detail semantics and both bases**

Run:

```bash
npm test -- tests/semantic-pages.test.ts tests/search-index.test.ts
npm run verify:builds
```

Expected: all tests and both base builds pass; Pagefind metadata/path filters remain unchanged.

- [ ] **Step 9: Commit the detail refinement**

```bash
git add web/public/assets/site.css \
  web/src/lib/pages/libraries.ts \
  web/src/lib/pages/solutions.ts \
  web/tests/semantic-pages.test.ts
git commit --no-gpg-sign -m "feat: refine solution detail relationships"
```

---

### Task 5: Align normative structure docs and run the Human gate

**Files:**
- Modify: `docs/superpowers/specs/2026-08-10-library-web-structure-handoff.md`
- Modify: `docs/superpowers/specs/2026-08-10-library-platform-design.md`

**Interfaces:**
- Consumes: the implemented renderer contracts from Tasks 2–4.
- Produces: normative documentation matching the generated HTML and the final G1 verification record.

- [ ] **Step 1: Update the semantic handoff**

Update sections 4–9, 12, and 14 of `2026-08-10-library-web-structure-handoff.md` so they explicitly state:

```text
Home recent solution order is title / language / contest-problem / date / status.
Libraries and Solutions roots have h1 followed directly by list or empty state.
Browse/card time text is YYYY-MM-DD; detail/evidence time text is YYYY-MM-DD HH:mm UTC; datetime keeps RFC 3339.
Library category, contest, and problem headers do not repeat breadcrumb paths/subtitles.
Problem solutions are full-width rows ending with status.
Verification evidence links to the public solution and ends with status.
Solution detail header is language / OJ / time / status.
Solution detail keeps section#libraries but labels it Depends on and shows direct dependencies only.
```

- [ ] **Step 2: Update the platform design**

Update sections 12.3–12.7 and the verification evidence description in `2026-08-10-library-platform-design.md`. Preserve the domain rule that `depends_on` and `verifies` are separate and add that `verifies` remains in site data but is surfaced from the Library verification view rather than Solution detail.

- [ ] **Step 3: Scan docs for obsolete contracts**

Run:

```bash
rg -n "section\.languages|section\.contests|subtitle.*Contest|verifies と direct dependencies は別 list|contest / problem / OJ|language / solved time" \
  docs/superpowers/specs/2026-08-10-library-web-structure-handoff.md \
  docs/superpowers/specs/2026-08-10-library-platform-design.md
git diff --check
```

Expected: no obsolete statement remains; any match is either the newly clarified internal-data distinction or requires correction before continuing.

- [ ] **Step 4: Run fresh Web verification**

Run:

```bash
npm test
npm run verify:builds
npm run site:build
```

Expected: every Vitest test passes; both base builds pass; the full site builds 13 HTML pages and `site-verify: OK` is printed.

- [ ] **Step 5: Run fresh repository gates**

On NixOS where OpenSSL is not in the normal shell, run:

```bash
nix-shell -p openssl pkg-config --run 'LD_LIBRARY_PATH="$(pkg-config --variable=libdir openssl)" cargo test --all && cargo clippy --all --all-features -- -D warnings && cargo fmt --all --check'
```

Expected: all Rust tests pass, Clippy emits no warnings under `-D warnings`, and rustfmt exits 0.

- [ ] **Step 6: Verify final scope and stylesheet safety**

Run:

```bash
git diff --check origin/main...HEAD
git status --short
git log --oneline origin/main..HEAD
```

Run the comment-stripping stylesheet scan from Task 1 again. Expected: no external references; only the design/spec/plan, Web renderer/test/verifier, and `site.css` files are in branch commits; unrelated `pnpm-*` files remain untracked.

- [ ] **Step 7: Commit normative documentation**

```bash
git add docs/superpowers/specs/2026-08-10-library-platform-design.md \
  docs/superpowers/specs/2026-08-10-library-web-structure-handoff.md
git commit --no-gpg-sign -m "docs: align web structure with visual refinements"
```

- [ ] **Step 8: HUMAN GATE — inspect the final site**

Ask the user to run:

```bash
npm run site:preview
```

The user inspects Home, Libraries root/category/detail, Solutions root/contest/problem/detail, and Search at desktop and narrow widths. Specifically confirm compact/detailed time text, evidence links, right-aligned statuses, keyboard focus, sticky in-page navigation, and that removed duplicate metadata is absent.

Do not create the PR until the user approves this visual gate or all feedback is incorporated and the verification steps are rerun.

---

### Task 6: Incorporate the final G1 Home alignment feedback

**Files:**
- Modify: `web/public/assets/site.css`
- Modify: `docs/superpowers/plans/2026-08-12-library-visual-structure-refinement.md`

**Interfaces:**
- Consumes: the existing desktop subgrid selectors for Home recent library and solution lists.
- Produces: matching `0.8fr : 1.6fr` title-to-path track ratios without changing DOM or mobile layout.

- [ ] **Step 1: Apply the minimal desktop CSS change**

Change only the two Home recent list declarations to:

```css
grid-template-columns:
  minmax(0, 0.8fr) 4.5rem minmax(0, 1.6fr) max-content max-content;
```

- [ ] **Step 2: Run full Web verification**

Run:

```bash
npm test
npm run verify:builds
npm run site:build
```

Expected: all tests pass, both base builds produce 13 HTML pages, and `site-verify: OK` is printed.

- [ ] **Step 3: Verify the generated artifact and stylesheet safety**

Confirm `web/dist/assets/site.css` contains both updated Home grid declarations after `site:build`. Run the
comment-stripping stylesheet safety scan from Task 1 and `git diff --check`.

Do not add an automated assertion for the literal ratio: it would test CSS source text rather than rendered
behavior and would fail only on an intentional design change.

- [ ] **Step 4: Commit the feedback adjustment**

Commit the stylesheet and this plan correction:

```bash
git add web/public/assets/site.css \
  docs/superpowers/plans/2026-08-12-library-visual-structure-refinement.md
git commit -m "style: align home recent metadata columns"
```

- [ ] **Step 5: Record G1 approval and continue the rollout**

The user's approval covers all earlier G1 checks plus this requested spacing adjustment. After fresh verification,
deliver the branch through the rollout's `/pr --base main` workflow; do not reopen the visual gate unless the
implementation diverges from this exact ratio-only change.
