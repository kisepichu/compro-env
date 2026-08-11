/**
 * Browser-side hydration for the static search page.
 *
 * This module runs after the semantic shell from `renderSearchMainInner`
 * has been served. It reads `?q=...&page=...`, parses the query, runs
 * both the exact-match index and Pagefind, merges by canonical page ID
 * (see `merge.ts`), and rewrites the pre-existing DOM elements in-place
 * per spec §13.2.
 */

import type { ExactIndex, ExactIndexPage } from "./exact-index.ts";
import {
  mergeResults,
  paginate,
  sortSubResults,
  type MergePage,
  type MergedPage,
  type SubResult,
} from "./merge.ts";
import { canonicalPage, parseSearchQuery } from "./query.ts";
import type { ParsedQuery } from "./types.ts";

/* ---------------- Minimal Pagefind types --------------------------- */

interface PagefindFilters {
  [key: string]: string[] | { any?: string[] };
}

interface PagefindFragmentAnchor {
  element: string;
  id: string;
  text?: string;
  location?: number;
}

interface PagefindSubResult {
  title: string;
  url: string;
  anchor?: PagefindFragmentAnchor;
  excerpt?: string;
}

interface PagefindResultFragment {
  url: string;
  raw_url?: string;
  excerpt?: string;
  meta?: Record<string, string>;
  anchors?: PagefindFragmentAnchor[];
  sub_results?: PagefindSubResult[];
  filters?: Record<string, string[]>;
}

interface PagefindResultRef {
  id: string;
  data(): Promise<PagefindResultFragment>;
}

interface PagefindSearchResponse {
  results: PagefindResultRef[];
}

interface PagefindApi {
  init?: () => Promise<void>;
  options?: (opts: Record<string, unknown>) => Promise<void>;
  search: (
    term: string | null,
    opts?: { filters?: PagefindFilters },
  ) => Promise<PagefindSearchResponse>;
}

/* ---------------- Small utilities ---------------------------------- */

function esc(input: string): string {
  return input
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/**
 * Sanitize a Pagefind excerpt: allow only text nodes and `<mark>` tags
 * (no attributes). Anything else is stripped. Pagefind excerpts are the
 * only untrusted HTML we render.
 */
function sanitizeExcerpt(html: string): string {
  const doc = new DOMParser().parseFromString(
    `<div>${html}</div>`,
    "text/html",
  );
  const container = doc.body.firstElementChild;
  if (container === null) return "";
  const out: string[] = [];
  const walk = (node: Node): void => {
    if (node.nodeType === Node.TEXT_NODE) {
      out.push(esc(node.textContent ?? ""));
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    const el = node as Element;
    if (el.tagName.toLowerCase() === "mark") {
      out.push("<mark>");
      for (const child of el.childNodes) walk(child);
      out.push("</mark>");
      return;
    }
    for (const child of el.childNodes) walk(child);
  };
  for (const child of container.childNodes) walk(child);
  return out.join("");
}

function buildQueryString(q: string, page: number): string {
  const p = new URLSearchParams();
  if (q !== "") p.set("q", q);
  if (page > 1) p.set("page", String(page));
  const s = p.toString();
  return s === "" ? "" : `?${s}`;
}

/* ---------------- Base URL resolution ------------------------------ */

function resolveBase(app: HTMLElement): string {
  const attr = app.dataset.base;
  if (typeof attr === "string" && attr.length > 0) return attr;
  return "/";
}

function joinBase(base: string, rest: string): string {
  const b = base.endsWith("/") ? base : `${base}/`;
  const r = rest.startsWith("/") ? rest.slice(1) : rest;
  return `${b}${r}`;
}

/* ---------------- Filter matching against exact index -------------- */

function pageMatchesFilters(
  page: ExactIndexPage,
  q: ParsedQuery,
): boolean {
  const f = q.filters;
  if (f.lang.length > 0 && !f.lang.includes(page.filters.lang)) return false;
  if (f.kind.length > 0) {
    const any = page.filters.kind.some((k) => f.kind.includes(k));
    if (!any) return false;
  }
  if (f.path.length > 0) {
    const any = page.filters.path.some((p) => f.path.includes(p));
    if (!any) return false;
  }
  if (f.status.length > 0 && !f.status.includes(page.filters.status as never)) {
    return false;
  }
  if (f.type.length > 0 && !f.type.includes(page.filters.type)) return false;
  if (f.verified_true && page.filters.verified !== "true") return false;
  if (f.verified_false && page.filters.verified !== "false") return false;
  return true;
}

/* ---------------- Exact alias lookup ------------------------------- */

function lowercase(s: string): string {
  return s.toLocaleLowerCase("en-US");
}

/**
 * Look up an alias exactly, per spec §13.1: only when full-text is a
 * single bare token or a single phrase. Compares Unicode-lowercase
 * equality with no normalization or tokenization. Returns the exact
 * matches with match reasons + sub-results filled in.
 */
function exactLookup(
  index: ExactIndex,
  parsed: ParsedQuery,
): MergePage[] {
  // Only run when exactly one full-text piece (bare or phrase) is present.
  if (parsed.fullTextTokens.length !== 1) return [];
  const alias = lowercase(parsed.fullTextTokens[0]!);
  const out: MergePage[] = [];
  for (const page of index.pages) {
    if (!pageMatchesFilters(page, parsed)) continue;
    const reasons = new Set<MergePage["matchReasons"][number]>();
    const matchedSymbols: (typeof page.symbols)[number][] = [];
    // Title / basename aliases.
    for (const a of page.aliases) {
      if (lowercase(a) === alias) {
        if (a === page.title) reasons.add("Title match");
        else reasons.add("File match");
      }
    }
    for (const sym of page.symbols) {
      if (lowercase(sym.name) === alias) {
        matchedSymbols.push(sym);
        reasons.add("Symbol match");
        continue;
      }
      const found = sym.search_names.some((n) => lowercase(n) === alias);
      if (found) {
        matchedSymbols.push(sym);
        reasons.add("Symbol match");
      }
    }
    if (reasons.size === 0) continue;
    const subResults: SubResult[] = matchedSymbols.map((sym) => ({
      label: sym.name,
      fragment: sym.fragment,
      url: `${page.url}#${sym.fragment}`,
      isExactSymbol: true,
      kind: sym.kind.toLowerCase(),
      name: sym.name,
      location: /^L\d+$/.test(sym.fragment)
        ? { startLine: parseInt(sym.fragment.slice(1), 10) }
        : undefined,
    }));
    out.push({
      page_id: page.page_id,
      url: page.url,
      title: page.title,
      type: page.type,
      language: page.language,
      status: page.status,
      display_path: page.display_path,
      matchReasons: [...reasons],
      subResults,
    });
  }
  return out;
}

/* ---------------- Pagefind result → MergePage ---------------------- */

function pagefindToMergePage(
  frag: PagefindResultFragment,
): MergePage | null {
  const meta = frag.meta ?? {};
  const pageId = meta.page_id;
  if (typeof pageId !== "string" || pageId === "") return null;
  const url = typeof meta.url === "string" && meta.url.length > 0 ? meta.url : frag.url;
  const type = meta.type === "solution" ? "solution" : "library";
  const title = meta.title ?? url;
  const language = meta.language ?? "";
  const status = meta.status ?? "";
  const displayPath = meta.display_path ?? "";
  const subResults: SubResult[] = [];
  for (const sub of frag.sub_results ?? []) {
    const frag2 = sub.anchor?.id ?? "";
    const subUrl = sub.url ?? (frag2 === "" ? url : `${url}#${frag2}`);
    subResults.push({
      label: sub.title,
      fragment: frag2,
      url: subUrl,
      isExactSymbol: false,
    });
  }
  return {
    page_id: pageId,
    url,
    title,
    type,
    language,
    status,
    display_path: displayPath,
    matchReasons: [],
    subResults,
    excerpt: frag.excerpt !== undefined ? sanitizeExcerpt(frag.excerpt) : undefined,
  };
}

/* ---------------- Rendering --------------------------------------- */

function setHidden(el: HTMLElement, hidden: boolean): void {
  if (hidden) el.setAttribute("hidden", "");
  else el.removeAttribute("hidden");
}

function renderStatus(el: HTMLElement, msg: string | null): void {
  if (msg === null) {
    el.textContent = "";
    setHidden(el, true);
    return;
  }
  el.textContent = msg;
  setHidden(el, false);
}

function renderAlert(el: HTMLElement, msg: string | null): void {
  if (msg === null) {
    el.textContent = "";
    setHidden(el, true);
    return;
  }
  el.textContent = msg;
  setHidden(el, false);
}

function renderSummary(
  el: HTMLElement,
  total: number,
  start: number,
  end: number,
): void {
  if (total === 0) {
    setHidden(el, true);
    el.textContent = "";
    return;
  }
  el.textContent =
    total === 1
      ? "Showing 1 result."
      : `Showing ${start}–${end} of ${total} results.`;
  setHidden(el, false);
}

function renderFilters(el: HTMLElement, parsed: ParsedQuery): void {
  const chips: string[] = [];
  const push = (label: string): void => {
    chips.push(
      `<span class="filter-chip">${esc(label)}</span>`,
    );
  };
  for (const v of parsed.filters.lang) push(`lang:${v}`);
  for (const v of parsed.filters.kind) push(`kind:${v}`);
  for (const v of parsed.filters.path) push(`path:${v}`);
  for (const v of parsed.filters.status) push(`status:${v}`);
  for (const v of parsed.filters.type) push(`type:${v}`);
  if (parsed.filters.verified_true) push(`verified:true`);
  if (parsed.filters.verified_false) push(`verified:false`);
  if (chips.length === 0) {
    setHidden(el, true);
    el.innerHTML = "";
    return;
  }
  el.innerHTML =
    `<h2 class="parsed-filters-heading">Active filters</h2>` +
    `<div class="parsed-filters-chips">${chips.join("")}</div>`;
  setHidden(el, false);
}

function renderResults(
  listEl: HTMLElement,
  pages: MergedPage[],
): void {
  if (pages.length === 0) {
    listEl.innerHTML = "";
    return;
  }
  const items = pages.map(renderCard);
  listEl.innerHTML = items.join("");
}

function renderCard(page: MergedPage): string {
  const meta: string[] = [];
  meta.push(`<span class="meta-type">${esc(page.type)}</span>`);
  if (page.language !== "") {
    meta.push(`<span class="meta-language">${esc(page.language)}</span>`);
  }
  if (page.status !== "") {
    meta.push(`<span class="meta-status">${esc(page.status)}</span>`);
  }
  if (page.display_path !== "") {
    meta.push(`<span class="meta-path">${esc(page.display_path)}</span>`);
  }
  const reasonsHtml =
    page.matchReasons.length === 0
      ? ""
      : `<ul class="match-reasons" aria-label="Match reasons">${page.matchReasons
          .map((r) => `<li>${esc(r)}</li>`)
          .join("")}</ul>`;
  const excerptHtml =
    page.excerpt !== undefined && page.excerpt !== ""
      ? `<p class="card-excerpt">${page.excerpt}</p>`
      : "";
  const { items: subs, remainderCount } = sortSubResults(page.subResults);
  const subsHtml =
    subs.length === 0
      ? ""
      : `<ul class="card-subresults">` +
        subs
          .map((s) => {
            const suffix =
              s.location !== undefined
                ? ` <span class="subresult-line">line ${s.location.startLine}</span>`
                : "";
            return `<li><a href="${esc(s.url)}">${esc(s.label)}</a>${suffix}</li>`;
          })
          .join("") +
        (remainderCount > 0
          ? `<li class="subresult-more"><a href="${esc(page.url)}">and ${remainderCount} more</a></li>`
          : "") +
        `</ul>`;
  return (
    `<li>` +
      `<article class="search-card" data-page-id="${esc(page.page_id)}">` +
        `<h2 class="card-title"><a href="${esc(page.url)}">${esc(page.title)}</a></h2>` +
        `<p class="card-meta">${meta.join(" ")}</p>` +
        reasonsHtml +
        excerptHtml +
        subsHtml +
      `</article>` +
    `</li>`
  );
}

function renderPagination(
  el: HTMLElement,
  q: string,
  page: number,
  totalPages: number,
): void {
  if (totalPages <= 1) {
    el.innerHTML = "";
    setHidden(el, true);
    return;
  }
  const parts: string[] = [];
  if (page > 1) {
    parts.push(
      `<a class="pagination-prev" rel="prev" href="${esc(buildQueryString(q, page - 1))}">Previous</a>`,
    );
  } else {
    parts.push(`<span class="pagination-prev" aria-disabled="true">Previous</span>`);
  }
  parts.push(
    `<span class="pagination-current" aria-current="page">Page ${page} of ${totalPages}</span>`,
  );
  if (page < totalPages) {
    parts.push(
      `<a class="pagination-next" rel="next" href="${esc(buildQueryString(q, page + 1))}">Next</a>`,
    );
  } else {
    parts.push(`<span class="pagination-next" aria-disabled="true">Next</span>`);
  }
  el.innerHTML = parts.join("");
  setHidden(el, false);
}

/* ---------------- Main hydration entry ----------------------------- */

interface Elements {
  app: HTMLElement;
  status: HTMLElement;
  alert: HTMLElement;
  summary: HTMLElement;
  filters: HTMLElement;
  results: HTMLElement;
  pagination: HTMLElement;
  empty: HTMLElement;
  input: HTMLInputElement | null;
}

function getElements(): Elements | null {
  const app = document.getElementById("search-app");
  if (app === null) return null;
  const status = document.getElementById("search-status");
  const alert = document.getElementById("search-alert");
  const summary = document.getElementById("search-summary");
  const filters = document.getElementById("parsed-filters");
  const results = document.getElementById("search-results");
  const pagination = document.getElementById("search-pagination");
  const empty = document.getElementById("search-empty");
  const input = document.getElementById(
    "global-search-query",
  ) as HTMLInputElement | null;
  if (
    status === null ||
    alert === null ||
    summary === null ||
    filters === null ||
    results === null ||
    pagination === null ||
    empty === null
  ) {
    return null;
  }
  return {
    app: app as HTMLElement,
    status: status as HTMLElement,
    alert: alert as HTMLElement,
    summary: summary as HTMLElement,
    filters: filters as HTMLElement,
    results: results as HTMLElement,
    pagination: pagination as HTMLElement,
    empty: empty as HTMLElement,
    input,
  };
}

function showEmpty(el: Elements): void {
  setHidden(el.empty, false);
  setHidden(el.pagination, true);
  el.results.innerHTML = "";
  setHidden(el.summary, true);
}

function hideEmpty(el: Elements): void {
  setHidden(el.empty, true);
}

async function fetchExactIndex(base: string): Promise<ExactIndex | null> {
  try {
    const url = joinBase(base, "exact-search-index.json");
    const resp = await fetch(url, { credentials: "same-origin" });
    if (!resp.ok) return null;
    return (await resp.json()) as ExactIndex;
  } catch {
    return null;
  }
}

async function loadPagefind(base: string): Promise<PagefindApi | null> {
  try {
    const url = new URL(joinBase(base, "pagefind/pagefind.js"), window.location.origin);
    const mod = (await import(/* @vite-ignore */ url.href)) as { default?: PagefindApi } & PagefindApi;
    const api = (mod.default ?? mod) as PagefindApi;
    if (api.init !== undefined) await api.init();
    return api;
  } catch {
    return null;
  }
}

function buildPagefindFilters(parsed: ParsedQuery): PagefindFilters {
  const out: PagefindFilters = {};
  const setAny = (key: string, values: string[]): void => {
    if (values.length === 0) return;
    out[key] = values.length === 1 ? [values[0]!] : { any: values };
  };
  setAny("lang", parsed.filters.lang);
  setAny("kind", parsed.filters.kind);
  setAny("path", parsed.filters.path);
  setAny("status", parsed.filters.status);
  setAny("type", parsed.filters.type);
  // `verified:true verified:false` covers the whole domain — skip the
  // filter entirely so both branches remain reachable. A single flag on
  // its own still narrows the Pagefind result set.
  const wantTrue = parsed.filters.verified_true;
  const wantFalse = parsed.filters.verified_false;
  if (wantTrue && !wantFalse) out["verified"] = ["true"];
  else if (wantFalse && !wantTrue) out["verified"] = ["false"];
  return out;
}

async function runSearch(el: Elements): Promise<void> {
  const params = new URLSearchParams(window.location.search);
  const qRaw = params.get("q");
  const pageInput = params.get("page");
  const page = canonicalPage(pageInput);
  const q = qRaw ?? "";

  if (el.input !== null) el.input.value = q;

  // Empty query — show grammar hint.
  if (q.trim() === "") {
    renderFilters(el.filters, {
      ok: true,
      raw: "",
      fullText: "",
      fullTextTokens: [],
      filters: {
        lang: [],
        kind: [],
        path: [],
        status: [],
        type: [],
        verified_true: false,
        verified_false: false,
      },
    });
    renderStatus(el.status, null);
    renderAlert(el.alert, null);
    showEmpty(el);
    return;
  }

  const parsed = parseSearchQuery(q);
  if (!parsed.ok) {
    hideEmpty(el);
    renderStatus(el.status, null);
    renderAlert(el.alert, parsed.message);
    el.results.innerHTML = "";
    setHidden(el.pagination, true);
    setHidden(el.summary, true);
    renderFilters(el.filters, {
      ok: true,
      raw: q,
      fullText: "",
      fullTextTokens: [],
      filters: {
        lang: [],
        kind: [],
        path: [],
        status: [],
        type: [],
        verified_true: false,
        verified_false: false,
      },
    });
    return;
  }

  renderAlert(el.alert, null);
  renderFilters(el.filters, parsed);

  const base = resolveBase(el.app);
  renderStatus(el.status, "Loading search…");

  const [exactIndex, pagefind] = await Promise.all([
    fetchExactIndex(base),
    loadPagefind(base),
  ]);

  hideEmpty(el);

  // Compute exact matches.
  const exact =
    exactIndex !== null ? exactLookup(exactIndex, parsed) : [];

  // Query Pagefind.
  let pagefindPages: MergePage[] = [];
  let pagefindFailed = false;
  if (pagefind !== null) {
    try {
      const term = parsed.fullText.trim() === "" ? null : parsed.fullText;
      const response = await pagefind.search(term, {
        filters: buildPagefindFilters(parsed),
      });
      const fragments = await Promise.all(
        response.results.map((r) => r.data()),
      );
      for (const frag of fragments) {
        const mp = pagefindToMergePage(frag);
        if (mp !== null) pagefindPages.push(mp);
      }
    } catch {
      pagefindFailed = true;
      pagefindPages = [];
    }
  } else {
    pagefindFailed = true;
  }

  const merged = mergeResults(exact, pagefindPages);
  const pageOfResults = paginate(merged, page, 20);

  renderStatus(el.status, null);
  renderResults(el.results, pageOfResults.pageItems);
  renderPagination(el.pagination, q, pageOfResults.page, pageOfResults.totalPages);
  const start = merged.length === 0 ? 0 : (pageOfResults.page - 1) * 20 + 1;
  const end = start === 0 ? 0 : start + pageOfResults.pageItems.length - 1;
  renderSummary(el.summary, merged.length, start, end);

  if (merged.length === 0) {
    if (pagefindFailed && exactIndex === null) {
      renderAlert(
        el.alert,
        "Search index failed to load. Try again after the site build completes.",
      );
    } else if (pagefindFailed) {
      renderAlert(
        el.alert,
        "Full-text search is unavailable; exact matches only.",
      );
    } else {
      renderAlert(el.alert, `No results match “${q}”.`);
    }
  }
}

function bootstrap(): void {
  const el = getElements();
  if (el === null) return;
  // Make the shell state consistent while the async pipeline runs.
  setHidden(el.status, true);
  setHidden(el.alert, true);
  setHidden(el.summary, true);
  setHidden(el.pagination, true);
  void runSearch(el).catch((err: unknown) => {
    const msg = err instanceof Error ? err.message : String(err);
    renderStatus(el.status, null);
    renderAlert(el.alert, `Search failed: ${msg}`);
  });
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", bootstrap);
} else {
  bootstrap();
}
