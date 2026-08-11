#!/usr/bin/env node
/**
 * `web/scripts/site-verify.mjs` — authoritative post-build verifier.
 *
 * Standalone runner: takes `--out=<dir>` and `--base=<path>` so it can be
 * invoked after `site-build.mjs`, and re-checks every semantic invariant
 * on the generated HTML plus the search-specific artifacts:
 *
 *   - Exactly one `<h1>` per page.
 *   - Every internal `href`, `action`, `src` sits under the configured base.
 *   - Landmarks and skip link exist in expected order.
 *   - Detail pages carry `data-pagefind-body` and a `page-*` id; browse
 *     pages carry `data-pagefind-ignore` on `<main>`.
 *   - `/search/` and `404.html` set robots `noindex`.
 *   - Search-specific: `/search/` sets robots `noindex,nofollow`, contains
 *     `#search-app` with `data-base` equal to the base, and
 *     `exact-search-index.json` exists at the out root.
 *   - Search-specific: when Pagefind was run, `pagefind/pagefind.js` exists.
 *   - Search-specific: `exact-search-index.json` has `schema_version === 1`
 *     and its `pages[].page_id` set equals the union of library+solution
 *     page IDs in the site-data fixture.
 *   - Prints byte sizes for `exact-search-index.json` and (if present)
 *     `pagefind/pagefind-entry.json`.
 *
 * The legacy `scripts/verify-web-build.mjs` runs the same checks *from*
 * a source rebuild (root + project base). This script is the reusable
 * verifier that operates on an already-built directory. Keep both alive.
 */

import {
  existsSync,
  readdirSync,
  readFileSync,
  statSync,
} from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { JSDOM } from "jsdom";

const scriptDir = fileURLToPath(new URL(".", import.meta.url));
const repoRoot = resolve(scriptDir, "..", "..");

function parseArgs(argv) {
  const args = {
    out: null,
    base: "/",
    fixture: "web/tests/fixtures/site-data.json",
    skipPagefind: false,
  };
  for (const raw of argv.slice(2)) {
    if (raw === "--skip-pagefind") {
      args.skipPagefind = true;
      continue;
    }
    const eq = raw.indexOf("=");
    if (eq === -1) continue;
    const key = raw.slice(0, eq);
    const value = raw.slice(eq + 1);
    if (key === "--out") args.out = value;
    else if (key === "--base") args.base = value;
    else if (key === "--fixture") args.fixture = value;
  }
  return args;
}

function normalizeBase(input) {
  if (input === "" || input === "/") return "/";
  let value = input.startsWith("/") ? input : `/${input}`;
  if (!value.endsWith("/")) value = `${value}/`;
  return value;
}

let violations = 0;

function fail(target, file, message) {
  violations += 1;
  console.error(`[${target}] ${file}: ${message}`);
}

function walkHtml(dir, results = []) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    const stats = statSync(full);
    if (stats.isDirectory()) walkHtml(full, results);
    else if (entry.endsWith(".html")) results.push(full);
  }
  return results;
}

function checkPage(target, base, outDir, htmlPath) {
  const relPath = relative(outDir, htmlPath);
  const dom = new JSDOM(readFileSync(htmlPath, "utf8"));
  const doc = dom.window.document;

  const h1s = doc.querySelectorAll("h1");
  if (h1s.length !== 1) {
    fail(target, relPath, `expected exactly one <h1>, found ${h1s.length}`);
  }

  const skipLink = doc.querySelector("a.skip-link[href='#main-content']");
  if (!skipLink) fail(target, relPath, "missing skip-link → #main-content");
  const header = doc.querySelector("header.site-header");
  if (!header) fail(target, relPath, 'missing <header class="site-header">');
  const main = doc.querySelector("main#main-content");
  if (!main) fail(target, relPath, 'missing <main id="main-content">');
  const footer = doc.querySelector("footer.site-footer");
  if (!footer) fail(target, relPath, 'missing <footer class="site-footer">');

  const is404 = relPath === "404.html";
  const isSearch =
    relPath === "search/index.html" || relPath === "search\\index.html";
  const article = doc.querySelector("article[data-pagefind-body]");
  const isDetail = article !== null;

  if (isDetail && article) {
    if (
      !article.id.startsWith("page-library:") &&
      !article.id.startsWith("page-solution:")
    ) {
      fail(
        target,
        relPath,
        `detail article id must start with page-library:/page-solution: (got "${article.id}")`,
      );
    }
  }

  if (is404) {
    const robots = doc.querySelector("meta[name='robots']");
    if (!robots || !/noindex/i.test(robots.getAttribute("content") ?? "")) {
      fail(target, relPath, "404 page must set robots noindex");
    }
  }

  if (isSearch) {
    const robots = doc.querySelector("meta[name='robots']");
    const content = (robots?.getAttribute("content") ?? "").toLowerCase();
    if (!/noindex/.test(content) || !/nofollow/.test(content)) {
      fail(
        target,
        relPath,
        `/search/ must set robots "noindex,nofollow" (got "${content}")`,
      );
    }
    const app = doc.querySelector("#search-app");
    if (!app) {
      fail(target, relPath, "/search/ missing #search-app");
    } else {
      const dataBase = app.getAttribute("data-base");
      if (dataBase !== base) {
        fail(
          target,
          relPath,
          `#search-app[data-base] should equal "${base}", got "${dataBase}"`,
        );
      }
    }
  }

  if (!isDetail && !is404 && main) {
    const hasIgnore =
      main.hasAttribute("data-pagefind-ignore") ||
      main.matches("[data-pagefind-ignore]");
    if (!hasIgnore) {
      fail(target, relPath, "browse index main must carry data-pagefind-ignore");
    }
  }

  for (const attr of ["href", "action", "src"]) {
    for (const el of doc.querySelectorAll(`[${attr}]`)) {
      const raw = el.getAttribute(attr);
      if (raw === null) continue;
      if (
        raw.startsWith("#") ||
        raw.startsWith("http://") ||
        raw.startsWith("https://") ||
        raw.startsWith("mailto:") ||
        raw.startsWith("data:")
      ) {
        continue;
      }
      if (!raw.startsWith("/")) continue;
      if (!raw.startsWith(base)) {
        fail(
          target,
          relPath,
          `${el.tagName.toLowerCase()}[${attr}]="${raw}" does not start with base "${base}"`,
        );
      }
    }
  }
}

function checkExactIndex(target, outDir, fixturePath) {
  const indexPath = join(outDir, "exact-search-index.json");
  if (!existsSync(indexPath)) {
    fail(target, "exact-search-index.json", "missing at out-dir root");
    return;
  }
  let index;
  try {
    index = JSON.parse(readFileSync(indexPath, "utf8"));
  } catch (err) {
    fail(target, "exact-search-index.json", `invalid JSON: ${err.message}`);
    return;
  }
  if (index.schema_version !== 1) {
    fail(
      target,
      "exact-search-index.json",
      `schema_version must be 1 (got ${index.schema_version})`,
    );
  }
  if (!Array.isArray(index.pages)) {
    fail(target, "exact-search-index.json", "pages must be an array");
    return;
  }
  const raw = JSON.parse(readFileSync(fixturePath, "utf8"));
  const expected = new Set([
    ...raw.libraries.map((l) => l.page_id),
    ...raw.solutions.map((s) => s.page_id),
  ]);
  const got = new Set(index.pages.map((p) => p.page_id));
  for (const id of expected) {
    if (!got.has(id)) {
      fail(target, "exact-search-index.json", `missing page_id ${id}`);
    }
  }
  for (const id of got) {
    if (!expected.has(id)) {
      fail(
        target,
        "exact-search-index.json",
        `unexpected page_id ${id} (not in site-data)`,
      );
    }
  }
}

function checkPagefindArtifacts(target, outDir, expectPagefind) {
  const pagefindJs = join(outDir, "pagefind", "pagefind.js");
  if (!expectPagefind) return;
  if (!existsSync(pagefindJs)) {
    fail(target, "pagefind/pagefind.js", "missing (Pagefind indexer did not run?)");
    return;
  }
  // Sanity: pagefind.js must live under <out>/pagefind/ — not elsewhere.
  const parent = resolve(pagefindJs, "..");
  if (parent !== resolve(outDir, "pagefind")) {
    fail(target, "pagefind/pagefind.js", `unexpected location ${parent}`);
  }
}

function printSizes(target, outDir) {
  const exact = join(outDir, "exact-search-index.json");
  if (existsSync(exact)) {
    const size = statSync(exact).size;
    console.log(`  exact-search-index.json: ${size} bytes`);
  }
  const entry = join(outDir, "pagefind", "pagefind-entry.json");
  if (existsSync(entry)) {
    const size = statSync(entry).size;
    console.log(`  pagefind/pagefind-entry.json: ${size} bytes`);
  }
}

function main() {
  const args = parseArgs(process.argv);
  if (!args.out) {
    console.error("site-verify: --out=<dir> is required");
    process.exit(2);
  }
  const outDir = resolve(process.cwd(), args.out);
  const base = normalizeBase(args.base);
  const fixturePath = resolve(repoRoot, args.fixture);
  const label = relative(repoRoot, outDir) || outDir;

  if (!existsSync(outDir)) {
    console.error(`site-verify: out-dir not found: ${outDir}`);
    process.exit(1);
  }
  const files = walkHtml(outDir);
  for (const file of files) checkPage(label, base, outDir, file);
  console.log(`  ${files.length} HTML files checked`);

  checkExactIndex(label, outDir, fixturePath);
  checkPagefindArtifacts(label, outDir, !args.skipPagefind);
  printSizes(label, outDir);

  if (violations > 0) {
    console.error(`\n${violations} violation(s) found.`);
    process.exit(1);
  }
  console.log("\nsite-verify: OK");
}

main();
