/**
 * Build an in-memory exact-match index from the public site-data DTOs.
 *
 * Per spec §13.1: the exact index augments Pagefind for file-name and
 * symbol-name exact matches. It stores no source, no Markdown, and only
 * public detail pages (libraries and solutions).
 */

import type {
  LibraryPageData,
  SiteData,
  SolutionPageData,
  SymbolPublic,
} from "@/lib/site-data-types.ts";
import { libraryPath, solutionPath, type UrlConfig } from "@/lib/url.ts";

export const EXACT_INDEX_SCHEMA_VERSION = 1;

export type ExactIndexPageType = "library" | "solution";

export interface ExactIndexSymbol {
  name: string;
  qualified_name?: string;
  search_names: string[];
  kind: string;
  fragment: string;
}

export interface ExactIndexFilters {
  lang: string;
  kind: string[];
  path: string[];
  verified: "true" | "false";
  status: string;
  type: ExactIndexPageType;
}

export interface ExactIndexPage {
  page_id: string;
  url: string;
  type: ExactIndexPageType;
  title: string;
  language: string;
  status: string;
  display_path: string;
  aliases: string[];
  symbols: ExactIndexSymbol[];
  filters: ExactIndexFilters;
}

export interface ExactIndex {
  schema_version: number;
  pages: ExactIndexPage[];
}

// ---- Helpers ----

function pushUnique<T>(list: T[], value: T): void {
  if (!list.includes(value)) list.push(value);
}

function splitPathSegments(raw: string): string[] {
  return raw.split("/").filter((p) => p.length > 0);
}

function basenameOfPath(rawPath: string): string {
  const parts = splitPathSegments(rawPath);
  return parts.length > 0 ? parts[parts.length - 1]! : rawPath;
}

/**
 * Strip only the last dot-suffix (`.rs`, `.cpp`). Preserves compound suffixes:
 * `foo.test.cpp` → `foo.test`, never `foo`. Hidden files starting with a dot
 * (e.g. `.gitignore`) are left intact.
 */
function stripLastExtension(name: string): string {
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return name;
  return name.slice(0, dot);
}

/**
 * Build the `path:` filter values for a page. Emits every lowercase path
 * segment followed by every cumulative prefix, deduplicated by first
 * occurrence.
 */
export function pathFilterValues(rawSegments: readonly string[]): string[] {
  const out: string[] = [];
  for (const seg of rawSegments) pushUnique(out, seg.toLowerCase());
  for (let i = 1; i <= rawSegments.length; i += 1) {
    const prefix = rawSegments.slice(0, i).join("/").toLowerCase();
    pushUnique(out, prefix);
  }
  return out;
}

/** Fragment for a symbol: `L{n}` when located, else `"symbols"`. */
function fragmentOfSymbol(sym: SymbolPublic): string {
  const line = sym.location?.start.line;
  if (typeof line === "number" && Number.isFinite(line)) return `L${line}`;
  return "symbols";
}

function aliasesFor(title: string, basename: string): string[] {
  const out: string[] = [];
  pushUnique(out, title);
  pushUnique(out, basename);
  pushUnique(out, stripLastExtension(basename));
  return out;
}

// ---- Per-page builders ----

function libraryPageEntry(
  lib: LibraryPageData,
  config: UrlConfig,
): ExactIndexPage {
  const rawSegments = splitPathSegments(lib.source_path);
  const basename = basenameOfPath(lib.source_path);
  const symbols: ExactIndexSymbol[] = lib.symbol_analysis.symbols.map((s) => {
    const entry: ExactIndexSymbol = {
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
  const kinds: string[] = [];
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

function solutionPageEntry(
  sol: SolutionPageData,
  config: UrlConfig,
): ExactIndexPage {
  // Path-filter values come from the stable solution ID, NOT the entry file
  // path — per spec §13 renaming main.rs must not change the filter values.
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

/** Build the exact-match index from public DTOs. Pure; no I/O. */
export function buildExactIndex(
  siteData: SiteData,
  config: UrlConfig,
): ExactIndex {
  const pages: ExactIndexPage[] = [];
  for (const lib of siteData.libraries) {
    pages.push(libraryPageEntry(lib, config));
  }
  for (const sol of siteData.solutions) {
    pages.push(solutionPageEntry(sol, config));
  }
  return { schema_version: EXACT_INDEX_SCHEMA_VERSION, pages };
}
