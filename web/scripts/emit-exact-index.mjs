/**
 * JavaScript reimplementation of `web/src/search/exact-index.ts` +
 * `web/src/search/build-index.ts`, kept in sync by a parity unit test.
 *
 * We duplicate the small (~150 line) TS logic rather than shell out to
 * `node --experimental-strip-types` on the .ts sources. The reasons:
 *
 *   - `exact-index.ts` uses the `@/` alias to import `@/lib/url.ts` at
 *     runtime; Node's strip-types loader does not resolve that alias
 *     without a custom hook and rewriting the source to use relative
 *     paths breaks the codebase convention.
 *   - `--experimental-strip-types` prints a warning on every run and is
 *     not stable across Node minor lines.
 *   - The logic is small, pure, and covered by a parity vitest that
 *     compares this JS output against the TS `buildExactIndex` output
 *     on the same fixture.
 */

import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

export const EXACT_INDEX_SCHEMA_VERSION = 1;
export const EXACT_INDEX_FILENAME = "exact-search-index.json";

// ---- URL helpers (mirrors web/src/lib/url.ts, minimal surface) ----

function normalizeConfig(config) {
  const origin = config.origin.endsWith("/")
    ? config.origin.slice(0, -1)
    : config.origin;
  let base = config.base;
  if (base === "" || base === "/") {
    base = "/";
  } else {
    if (!base.startsWith("/")) base = `/${base}`;
    if (!base.endsWith("/")) base = `${base}/`;
  }
  return { origin, base };
}

function encodeSegment(segment) {
  if (segment === "" || segment === "." || segment === "..") {
    throw new Error(
      `Path segment escapes the site root: ${JSON.stringify(segment)}`,
    );
  }
  if (segment.includes("/") || segment.includes("\\")) {
    throw new Error(
      `Path segment must not contain a separator: ${JSON.stringify(segment)}`,
    );
  }
  return encodeURIComponent(segment);
}

function toInternalPath(config, segments) {
  const { base } = normalizeConfig(config);
  if (segments.length === 0) return base;
  const encoded = segments.map(encodeSegment).join("/");
  return `${base}${encoded}/`;
}

function libraryPath(config, language, sourceRelativePath) {
  const parts = sourceRelativePath.split("/").filter((p) => p.length > 0);
  return toInternalPath(config, ["libraries", language, ...parts]);
}

function solutionPath(config, contestId, problemCode, solutionName) {
  return toInternalPath(config, [
    "solutions",
    contestId,
    problemCode,
    solutionName,
  ]);
}

// ---- Exact-index helpers (mirrors web/src/search/exact-index.ts) ----

function pushUnique(list, value) {
  if (!list.includes(value)) list.push(value);
}

function splitPathSegments(raw) {
  return raw.split("/").filter((p) => p.length > 0);
}

function basenameOfPath(rawPath) {
  const parts = splitPathSegments(rawPath);
  return parts.length > 0 ? parts[parts.length - 1] : rawPath;
}

function stripLastExtension(name) {
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return name;
  return name.slice(0, dot);
}

export function pathFilterValues(rawSegments) {
  const out = [];
  for (const seg of rawSegments) pushUnique(out, seg.toLowerCase());
  for (let i = 1; i <= rawSegments.length; i += 1) {
    const prefix = rawSegments.slice(0, i).join("/").toLowerCase();
    pushUnique(out, prefix);
  }
  return out;
}

function fragmentOfSymbol(sym) {
  const line = sym.location?.start?.line;
  if (typeof line === "number" && Number.isFinite(line)) return `L${line}`;
  return "symbols";
}

function aliasesFor(title, basename) {
  const out = [];
  pushUnique(out, title);
  pushUnique(out, basename);
  pushUnique(out, stripLastExtension(basename));
  return out;
}

function libraryPageEntry(lib, config) {
  const rawSegments = splitPathSegments(lib.source_path);
  const basename = basenameOfPath(lib.source_path);
  const symbols = lib.symbol_analysis.symbols.map((s) => {
    const entry = {
      name: s.name,
      search_names: [...s.search_names],
      kind: s.kind,
      fragment: fragmentOfSymbol(s),
    };
    if (s.qualified_name !== null && s.qualified_name !== undefined) {
      entry.qualified_name = s.qualified_name;
    }
    return entry;
  });
  const kinds = [];
  for (const s of lib.symbol_analysis.symbols) {
    pushUnique(kinds, s.kind.toLowerCase());
  }
  const status = lib.verification.aggregate_status;
  return {
    page_id: lib.page_id,
    url: libraryPath(config, lib.language, lib.source_path),
    type: "library",
    title: lib.title,
    language: lib.language,
    status,
    display_path: lib.source_path,
    aliases: aliasesFor(lib.title, basename),
    symbols,
    filters: {
      lang: lib.language.toLowerCase(),
      kind: kinds,
      path: pathFilterValues(rawSegments),
      verified: status === "verified" ? "true" : "false",
      status: status.toLowerCase(),
      type: "library",
    },
  };
}

function solutionPageEntry(sol, config) {
  const rawSegments = splitPathSegments(sol.solution_id);
  const basename = sol.solution_name;
  const status = sol.verification.status;
  return {
    page_id: sol.page_id,
    url: solutionPath(
      config,
      sol.contest_id,
      sol.problem_code,
      sol.solution_name,
    ),
    type: "solution",
    title: sol.solution_name,
    language: sol.language,
    status,
    display_path: sol.solution_id,
    aliases: aliasesFor(sol.solution_name, basename),
    symbols: [],
    filters: {
      lang: sol.language.toLowerCase(),
      kind: [],
      path: pathFilterValues(rawSegments),
      verified: status === "verified" ? "true" : "false",
      status: status.toLowerCase(),
      type: "solution",
    },
  };
}

/**
 * Build the exact-match index in memory from a site-data DTO and a
 * `{ origin, base }` URL configuration. Pure; no I/O.
 */
export function buildExactIndex(siteData, config) {
  const pages = [];
  for (const lib of siteData.libraries) {
    pages.push(libraryPageEntry(lib, config));
  }
  for (const sol of siteData.solutions) {
    pages.push(solutionPageEntry(sol, config));
  }
  return { schema_version: EXACT_INDEX_SCHEMA_VERSION, pages };
}

/**
 * Persist an exact-match index to `<outDir>/exact-search-index.json`.
 * Creates the directory if it does not already exist.
 */
export async function writeExactIndex(index, outDir) {
  await mkdir(outDir, { recursive: true });
  const target = join(outDir, EXACT_INDEX_FILENAME);
  await writeFile(target, JSON.stringify(index), "utf8");
}
