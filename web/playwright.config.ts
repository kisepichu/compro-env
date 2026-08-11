/**
 * Playwright configuration for the search-page browser tests.
 *
 * The tests in `web/e2e/` require both `@playwright/test` and a built
 * static site with Pagefind. Neither is installed by default: run
 *
 *   npm install --save-dev --save-exact @playwright/test@1.62.1
 *   npx playwright install --with-deps chromium
 *
 * and then execute the suite against a preview build:
 *
 *   npm run site:build
 *   npx pagefind --site web/dist-root --output-subdir pagefind
 *   npx playwright test --config web/playwright.config.ts
 *
 * We keep this file untracked from CI intentionally — plan 060 wires
 * the browser suite into `site:build`.
 */
import { defineConfig } from "@playwright/test";

const port = Number(process.env.PLAYWRIGHT_PORT ?? "4321");
const previewCommand =
  process.env.PLAYWRIGHT_PREVIEW_COMMAND ??
  `npx http-server web/dist-root -p ${port} -c-1 --silent`;
const baseURL = process.env.PLAYWRIGHT_BASE_URL ?? `http://127.0.0.1:${port}`;

export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  fullyParallel: false,
  reporter: [["list"]],
  use: {
    baseURL,
    headless: true,
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: process.env.PLAYWRIGHT_SKIP_WEB_SERVER
    ? undefined
    : {
        command: previewCommand,
        url: baseURL,
        reuseExistingServer: true,
        timeout: 60_000,
      },
});
