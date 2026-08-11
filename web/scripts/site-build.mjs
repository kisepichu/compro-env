#!/usr/bin/env node
/**
 * `npm run site:build` — the single Web-build boundary (spec §12.14).
 *
 * Prerequisites — must have run upstream before invoking this script:
 *
 *   - The Rust adapter build has produced up-to-date offline adapters.
 *   - `ce check` has passed.
 *   - Production site-data has been generated (or, for smoke tests, the
 *     fixture at `web/tests/fixtures/site-data.json` is being used).
 *
 * This script does NOT invoke `prepare`, the Rust adapter build, or
 * `ce check`. Those live upstream so a search-only rebuild can iterate
 * without re-running the Rust pipeline.
 *
 * Steps, in exact order:
 *
 *   1. Load site-data JSON and validate its schema.
 *   2. Build the exact-match index in memory from the site-data DTOs.
 *   3. Run `astro build` — emits HTML into `<outDir>`.
 *   4. Write the exact-match index to `<outDir>/exact-search-index.json`
 *      (Astro would have cleared the directory in step 3, so we defer
 *      the write until after Astro finishes).
 *   5. Run Pagefind on `<outDir>` (skipped with `--skip-pagefind`).
 *   6. Run the site-verify checks against `<outDir>`.
 *
 * CLI flags:
 *
 *   --base=<path>       default `/`
 *   --origin=<url>      default `https://example.test`
 *   --out=<dir>         default `web/dist`
 *   --fixture=<path>    default `web/tests/fixtures/site-data.json`
 *   --skip-pagefind     skip Pagefind indexing (useful when Pagefind is
 *                       not installed locally; the verify step still runs)
 */

import { existsSync, readFileSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { relative, resolve } from "node:path";

import { runPipeline } from "./site-build-core.mjs";
import { buildExactIndex, writeExactIndex } from "./emit-exact-index.mjs";

const scriptDir = fileURLToPath(new URL(".", import.meta.url));
const repoRoot = resolve(scriptDir, "..", "..");

function parseArgs(argv) {
  const args = {
    base: "/",
    origin: "https://example.test",
    out: "web/dist",
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
    if (key === "--base") args.base = value;
    else if (key === "--origin") args.origin = value;
    else if (key === "--out") args.out = value;
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

const SUPPORTED_SCHEMA_VERSION = 1;

function loadSiteDataJson(fixturePath) {
  const raw = readFileSync(fixturePath, "utf8");
  const parsed = JSON.parse(raw);
  // Guard against silently loading a mismatched fixture: writeExactIndex
  // and Astro both assume the v1 shape. `web/src/lib/site-data.ts` runs a
  // full JSON-Schema check inside Astro; this fast check catches obviously
  // stale inputs before Astro spins up.
  if (parsed?.schema_version !== SUPPORTED_SCHEMA_VERSION) {
    throw new Error(
      `site-build: unsupported schema_version at ${fixturePath}: ` +
        `expected ${SUPPORTED_SCHEMA_VERSION}, got ${parsed?.schema_version}`,
    );
  }
  return parsed;
}

// ---- Real runners (composed with runPipeline) ----

function makeRunners({ outDirAbs }) {
  return {
    async emitExactIndex({ siteData, config }) {
      const index = buildExactIndex(siteData, config);
      return { index, status: 0 };
    },

    async astroBuild({ outDir, base, origin, fixture }) {
      // Clean the target so `astro build` starts fresh.
      rmSync(outDirAbs, { recursive: true, force: true });
      const outRelToWeb = relative(resolve(repoRoot, "web"), outDirAbs);
      const result = spawnSync(
        "npx",
        ["astro", "build", "--root", "web", "--outDir", outRelToWeb],
        {
          cwd: repoRoot,
          env: {
            ...process.env,
            CE_SITE_DATA_PATH: resolve(repoRoot, fixture),
            CE_SITE_ORIGIN: origin,
            CE_SITE_BASE: base,
          },
          encoding: "utf8",
          stdio: "inherit",
        },
      );
      return { status: result.status ?? 1 };
    },

    async writeExactIndex({ index, outDir }) {
      await writeExactIndex(index, outDirAbs);
      return { status: 0 };
    },

    async pagefind({ outDir }) {
      const result = spawnSync(
        "npx",
        ["pagefind", "--site", outDirAbs],
        { cwd: repoRoot, encoding: "utf8", stdio: "inherit" },
      );
      return { status: result.status ?? 1 };
    },

    async verify({ outDir, base, fixture, expectPagefind }) {
      const args = [
        resolve(scriptDir, "site-verify.mjs"),
        `--out=${outDirAbs}`,
        `--base=${base}`,
        `--fixture=${resolve(repoRoot, fixture)}`,
      ];
      if (!expectPagefind) args.push("--skip-pagefind");
      const result = spawnSync("node", args, {
        cwd: repoRoot,
        encoding: "utf8",
        stdio: "inherit",
      });
      return { status: result.status ?? 1 };
    },
  };
}

async function main() {
  const args = parseArgs(process.argv);
  const base = normalizeBase(args.base);
  const outDirAbs = resolve(repoRoot, args.out);
  const fixturePath = resolve(repoRoot, args.fixture);
  if (!existsSync(fixturePath)) {
    console.error(`site-build: fixture not found at ${fixturePath}`);
    process.exit(1);
  }
  const siteData = loadSiteDataJson(fixturePath);
  const runners = makeRunners({ outDirAbs });
  const result = await runPipeline(runners, {
    siteData,
    config: { origin: args.origin, base },
    outDir: outDirAbs,
    fixture: args.fixture,
    skipPagefind: args.skipPagefind,
  });
  if (result.status !== 0) {
    console.error(
      `site-build failed at stage ${result.ran[result.ran.length - 1]} (exit ${result.status})`,
    );
    process.exit(result.status);
  }
  console.log(`\nsite-build: OK (${result.ran.join(" -> ")})`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
