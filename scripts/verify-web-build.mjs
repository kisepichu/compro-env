#!/usr/bin/env node
/**
 * Build the static site twice — root base `/` and project base `/compro-env/` —
 * then walk every generated HTML file and enforce the semantic contract:
 *   - exactly one `<h1>` per page
 *   - every internal `href`, `action`, and `src` sits under the configured base
 *   - landmarks and skip link exist in expected order
 *   - detail pages carry `data-pagefind-body` and `page-{page_id}`; browse pages
 *     carry `data-pagefind-ignore` on `<main>`
 *   - `/search/` and `404.html` set robots noindex
 *
 * Exit code 1 on any violation, 0 on success.
 */

import {
  existsSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
} from "node:fs";
import { join, relative, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import { JSDOM } from "jsdom";

const repoRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));

const FIXTURE = join(repoRoot, "web/tests/fixtures/site-data.json");
const ORIGIN = "https://example.test";
const BUILD_TARGETS = [
  { base: "/", label: "root", outDir: join(repoRoot, "web/dist-root") },
  {
    base: "/compro-env/",
    label: "compro-env",
    outDir: join(repoRoot, "web/dist-compro-env"),
  },
];

let violations = 0;

function fail(target, file, message) {
  violations += 1;
  console.error(`[${target}] ${file}: ${message}`);
}

function runBuild({ base, label, outDir }) {
  rmSync(outDir, { recursive: true, force: true });
  const result = spawnSync(
    "npx",
    [
      "astro",
      "build",
      "--root",
      "web",
      "--outDir",
      relative(join(repoRoot, "web"), outDir),
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        CE_SITE_DATA_PATH: FIXTURE,
        CE_SITE_ORIGIN: ORIGIN,
        CE_SITE_BASE: base,
      },
      encoding: "utf8",
      stdio: "inherit",
    },
  );
  if (result.status !== 0) {
    console.error(`astro build failed for base ${base}`);
    process.exit(1);
  }
  if (!existsSync(join(outDir, "assets", "site.css"))) {
    fail(label, "assets/site.css", "shared stylesheet was not copied");
  }
  console.log(`✓ built ${label} into ${relative(repoRoot, outDir)}`);
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
  if (!header) fail(target, relPath, "missing <header class=\"site-header\">");
  const main = doc.querySelector("main#main-content");
  if (!main) fail(target, relPath, "missing <main id=\"main-content\">");
  const footer = doc.querySelector("footer.site-footer");
  if (!footer) fail(target, relPath, "missing <footer class=\"site-footer\">");

  const stylesheets = doc.querySelectorAll('link[rel="stylesheet"]');
  const expectedStylesheet = `${base}assets/site.css`;
  if (stylesheets.length !== 1) {
    fail(
      target,
      relPath,
      `expected exactly one stylesheet link, found ${stylesheets.length}`,
    );
  } else if (stylesheets[0].getAttribute("href") !== expectedStylesheet) {
    fail(
      target,
      relPath,
      `stylesheet href must be "${expectedStylesheet}"`,
    );
  }

  const is404 = relPath === "404.html";
  const article = doc.querySelector("article[data-pagefind-body]");
  const isDetail = article !== null;

  if (isDetail && article) {
    if (
      !article.id.startsWith("page-library:") &&
      !article.id.startsWith("page-solution:")
    ) {
      fail(target, relPath, `detail article id must start with page-library:/page-solution: (got "${article.id}")`);
    }
  }

  if (is404) {
    const robots = doc.querySelector("meta[name='robots']");
    if (!robots || !/noindex/i.test(robots.getAttribute("content") ?? "")) {
      fail(target, relPath, "404 page must set robots noindex");
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

  const attrsToCheck = ["href", "action", "src"];
  for (const attr of attrsToCheck) {
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
      if (!raw.startsWith("/")) continue; // relative — safe under directory-style routes
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

function main() {
  for (const target of BUILD_TARGETS) {
    runBuild(target);
    const files = walkHtml(target.outDir);
    for (const file of files) checkPage(target.label, target.base, target.outDir, file);
    console.log(`  ${files.length} HTML files checked`);
  }
  if (violations > 0) {
    console.error(`\n${violations} violation(s) found.`);
    process.exit(1);
  }
  console.log("\n✓ all internal-link / HTML semantic checks passed");
}

main();
