#!/usr/bin/env node
/**
 * `site:dev` preflight — refuses to launch Astro dev when the Pagefind
 * bundle in `web/dist` is missing or stale (spec §12.14: "dev で current
 * Pagefind index がない場合は古い index を使わず、検索 unavailable を明示する").
 *
 * The dev server must not fall back to a stale index. When this preflight
 * fails, the user is told to run `npm run site:build` first (which is the
 * single Web-build boundary that regenerates the Pagefind bundle, the
 * exact index, and the HTML in one pass).
 */

import { existsSync, statSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = fileURLToPath(new URL(".", import.meta.url));
const repoRoot = resolve(scriptDir, "..", "..");

const distDir = resolve(repoRoot, "web", "dist");
const pagefindJs = join(distDir, "pagefind", "pagefind.js");
const searchSources = [
  join(repoRoot, "web", "src", "search"),
  join(repoRoot, "web", "src", "pages", "search"),
  join(repoRoot, "web", "src", "lib", "site-data-types.ts"),
  join(repoRoot, "web", "src", "lib", "url.ts"),
];

function fail(message) {
  console.error(`site-dev-guard: ${message}`);
  console.error(`  → run \`npm run site:build\` to regenerate the bundle`);
  process.exit(1);
}

function newestMtimeUnder(path) {
  if (!existsSync(path)) return 0;
  const stats = statSync(path);
  if (stats.isFile()) return stats.mtimeMs;
  let newest = stats.mtimeMs;
  for (const entry of readdirSync(path)) {
    const sub = newestMtimeUnder(join(path, entry));
    if (sub > newest) newest = sub;
  }
  return newest;
}

if (!existsSync(distDir)) {
  fail("web/dist is missing — no built site to serve as dev fallback");
}
if (!existsSync(pagefindJs)) {
  fail(
    "web/dist/pagefind/pagefind.js is missing — search would silently degrade",
  );
}

const bundleMtime = statSync(pagefindJs).mtimeMs;
let sourceMtime = 0;
for (const src of searchSources) {
  const m = newestMtimeUnder(src);
  if (m > sourceMtime) sourceMtime = m;
}
if (sourceMtime > bundleMtime) {
  fail(
    `pagefind bundle (${new Date(bundleMtime).toISOString()}) is older than ` +
      `search sources (${new Date(sourceMtime).toISOString()})`,
  );
}

console.log("site-dev-guard: pagefind bundle is present and fresh.");
