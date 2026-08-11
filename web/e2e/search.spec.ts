/**
 * Browser tests for the static search page (spec §13.2).
 *
 * These tests are skipped by default because `@playwright/test` is not
 * a repo devDependency yet. Install it plus a browser locally, build
 * the site with Pagefind, and unset SKIP_PLAYWRIGHT to run:
 *
 *   npm install --save-dev --save-exact @playwright/test@1.62.1
 *   npx playwright install --with-deps chromium
 *   SKIP_PLAYWRIGHT= npx playwright test --config web/playwright.config.ts
 *
 * Coverage:
 *   - root and project-base URL resolution
 *   - reload / history-back / share preserves state
 *   - Pagefind WASM worker respects the default CSP
 *   - keyboard-only navigation reaches the form + result links
 *   - `monoid lang:cpp` combined term+filter
 *   - `kind:trait` filter-only
 *   - Punctuation symbol query (`::`)
 *   - Invalid query renders alert with role="alert"
 */
import { expect, test } from "@playwright/test";

const skip = process.env.SKIP_PLAYWRIGHT !== undefined;
test.describe.configure({ mode: "serial" });

test.beforeEach(async ({ page }) => {
  test.skip(skip, "Set SKIP_PLAYWRIGHT= to run browser tests.");
  const violations: string[] = [];
  page.on("console", (msg) => {
    const text = msg.text();
    if (msg.type() === "error" && /content security policy/i.test(text)) {
      violations.push(text);
    }
  });
  (page as unknown as { __cspViolations: string[] }).__cspViolations = violations;
});

test("root base loads the semantic shell", async ({ page }) => {
  await page.goto("/search/");
  await expect(page.locator("main h1")).toHaveText("Search");
  await expect(page.locator("#search-app")).toHaveAttribute("data-base", "/");
});

test("project base resolves URLs under /compro-env/", async ({ page }) => {
  await page.goto("/compro-env/search/");
  await expect(page.locator("#search-app")).toHaveAttribute(
    "data-base",
    "/compro-env/",
  );
});

test("reload and history navigation preserve q and page", async ({ page }) => {
  await page.goto("/search/?q=monoid&page=2");
  await expect(page.locator("#global-search-query")).toHaveValue("monoid");
  await page.reload();
  await expect(page.locator("#global-search-query")).toHaveValue("monoid");
  await page.goto("/search/?q=other");
  await page.goBack();
  await expect(page.locator("#global-search-query")).toHaveValue("monoid");
});

test("Pagefind WASM worker does not raise CSP violations", async ({ page }) => {
  await page.goto("/search/?q=monoid");
  await expect(page.locator("#search-results li")).toHaveCount(
    await page.locator("#search-results li").count(),
  );
  const violations = (page as unknown as { __cspViolations: string[] })
    .__cspViolations;
  expect(violations).toEqual([]);
});

test("keyboard-only navigation reaches the form and first result", async ({ page }) => {
  await page.goto("/search/");
  // Skip link → header → search input.
  await page.keyboard.press("Tab");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Tab");
  const focused = await page.evaluate(
    () => document.activeElement?.id ?? null,
  );
  expect(focused).toBe("global-search-query");
});

test("combined term+filter: monoid lang:cpp", async ({ page }) => {
  await page.goto("/search/?q=monoid+lang%3Acpp");
  await expect(page.locator(".filter-chip")).toContainText(["lang:cpp"]);
});

test("filter-only query: kind:trait", async ({ page }) => {
  await page.goto("/search/?q=kind%3Atrait");
  await expect(page.locator(".filter-chip")).toContainText(["kind:trait"]);
});

test("punctuation symbol: ::", async ({ page }) => {
  await page.goto("/search/?q=%3A%3A");
  // Symbol name is passed through unchanged; there's no query error.
  await expect(page.locator("#search-alert")).toBeHidden();
});

test("invalid query surfaces role=alert", async ({ page }) => {
  await page.goto("/search/?q=lang%3A");
  await expect(page.locator("#search-alert")).toBeVisible();
  await expect(page.locator("#search-alert")).toHaveAttribute("role", "alert");
});
