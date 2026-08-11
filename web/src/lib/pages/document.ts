/**
 * Renders the shared HTML shell around a page-specific `<main>` body.
 *
 * The shell wires the semantic contract from spec §12.2 and the
 * `Library Web semantic structure handoff` §3: skip link, header with
 * primary navigation and global search, main with breadcrumb, and
 * footer with repository link plus short build SHA.
 */

import { sanitizeExternalUrl } from "../safe-url.ts";
import type { SiteData } from "../site-data-types.ts";
import {
  homePath,
  librariesRootPath,
  searchPath,
  solutionsRootPath,
  toCanonicalUrl,
  type UrlConfig,
} from "../url.ts";
import { escapeAttribute, escapeHtml } from "./escape.ts";

export type NavKey = "libraries" | "solutions" | "search";

export interface BreadcrumbItem {
  label: string;
  /** When omitted, the item is the current page (rendered as `<li>` without `<a>`). */
  href?: string;
}

export interface DocumentOptions {
  config: UrlConfig;
  siteData: SiteData;
  /** The path segments used to build the canonical URL (base is applied by helpers). */
  canonicalSegments: readonly string[];
  /** Full contents of `<title>` — page code composes `{page} | {site}`; Home passes `site.title`. */
  documentTitle: string;
  /** Meta description (defaults to `site.description` when the caller passes it explicitly). */
  description: string;
  /** Optional robots meta content. Defaults to `index,follow`. */
  robots?: string;
  /** Which entry in the primary navigation is current, if any. */
  currentNav?: NavKey | null;
  /** Breadcrumb items. Empty array => no breadcrumb (Home only). */
  breadcrumb: readonly BreadcrumbItem[];
  /** Page-specific inner HTML for `<main>`. */
  mainInnerHtml: string;
  /** When true, add `data-pagefind-ignore` to `<main>` (all non-detail pages). */
  mainIgnoreForPagefind?: boolean;
}

function renderHead(opts: DocumentOptions): string {
  const canonical = toCanonicalUrl(opts.config, opts.canonicalSegments);
  const robots = opts.robots ?? "index,follow";
  const siteTitle = opts.siteData.site.title;
  return [
    `<meta charset="utf-8">`,
    `<meta name="viewport" content="width=device-width,initial-scale=1">`,
    `<title>${escapeHtml(opts.documentTitle)}</title>`,
    `<meta name="description" content="${escapeAttribute(opts.description)}">`,
    `<meta name="robots" content="${escapeAttribute(robots)}">`,
    `<link rel="canonical" href="${escapeAttribute(canonical)}">`,
    `<meta property="og:type" content="website">`,
    `<meta property="og:site_name" content="${escapeAttribute(siteTitle)}">`,
    `<meta property="og:title" content="${escapeAttribute(opts.documentTitle)}">`,
    `<meta property="og:description" content="${escapeAttribute(opts.description)}">`,
    `<meta property="og:url" content="${escapeAttribute(canonical)}">`,
  ].join("");
}

export interface HeaderOptions {
  config: UrlConfig;
  siteData: SiteData;
  currentNav?: NavKey | null;
}

export function renderHeader(opts: HeaderOptions): string {
  const home = homePath(opts.config);
  const libraries = librariesRootPath(opts.config);
  const solutions = solutionsRootPath(opts.config);
  const search = searchPath(opts.config);
  const current = opts.currentNav ?? null;
  function navLink(key: NavKey, href: string, label: string): string {
    const currentAttr =
      current === key ? ` aria-current="page"` : "";
    return (
      `<li><a href="${escapeAttribute(href)}"${currentAttr}>${escapeHtml(label)}</a></li>`
    );
  }
  return (
    `<header class="site-header" data-pagefind-ignore>` +
      `<a class="site-title" href="${escapeAttribute(home)}">${escapeHtml(opts.siteData.site.title)}</a>` +
      `<nav class="primary-navigation" aria-label="Primary" data-pagefind-ignore>` +
        `<ul>` +
          navLink("libraries", libraries, "Libraries") +
          navLink("solutions", solutions, "Solutions") +
          navLink("search", search, "Search") +
        `</ul>` +
      `</nav>` +
      `<form class="global-search" role="search" method="get" action="${escapeAttribute(search)}">` +
        `<label for="global-search-query">Search</label>` +
        `<input id="global-search-query" name="q" type="search" autocomplete="off">` +
        `<button type="submit">Search</button>` +
      `</form>` +
    `</header>`
  );
}

export function renderBreadcrumb(items: readonly BreadcrumbItem[]): string {
  if (items.length === 0) return "";
  const lis = items
    .map((item) => {
      if (item.href === undefined) {
        return `<li aria-current="page">${escapeHtml(item.label)}</li>`;
      }
      return `<li><a href="${escapeAttribute(item.href)}">${escapeHtml(item.label)}</a></li>`;
    })
    .join("");
  return (
    `<nav class="breadcrumb" aria-label="Breadcrumb" data-pagefind-ignore>` +
      `<ol>${lis}</ol>` +
    `</nav>`
  );
}

export interface FooterOptions {
  siteData: SiteData;
}

export function renderFooter(opts: FooterOptions): string {
  const safeRepoUrl = sanitizeExternalUrl(opts.siteData.site.repository_url);
  const shortSha = opts.siteData.build.source_commit_short_sha;
  const repoLink =
    safeRepoUrl !== null
      ? `<a class="repository-link" href="${escapeAttribute(safeRepoUrl)}" rel="noopener noreferrer">Repository</a>`
      : `<span class="repository-link">Repository</span>`;
  return (
    `<footer class="site-footer" data-pagefind-ignore>` +
      repoLink +
      ` <span class="build-source-commit">Build ` +
      `<code class="build-source-commit-sha">${escapeHtml(shortSha)}</code></span>` +
    `</footer>`
  );
}

export function renderDocument(opts: DocumentOptions): string {
  const htmlLang = escapeAttribute(opts.siteData.site.language || "en");
  const head = renderHead(opts);
  const header = renderHeader(opts);
  const breadcrumb = renderBreadcrumb(opts.breadcrumb);
  const footer = renderFooter(opts);
  const mainAttr = opts.mainIgnoreForPagefind ? ` data-pagefind-ignore` : "";
  return (
    `<!DOCTYPE html>` +
    `<html lang="${htmlLang}">` +
      `<head>${head}</head>` +
      `<body>` +
        `<a class="skip-link" href="#main-content">Skip to main content</a>` +
        header +
        `<main id="main-content"${mainAttr}>` +
          breadcrumb +
          opts.mainInnerHtml +
        `</main>` +
        footer +
      `</body>` +
    `</html>`
  );
}

/** Format a full document title. Home uses `site.title`; others `{page} | {site}`. */
export function formatDocumentTitle(
  siteData: SiteData,
  pageTitle: string | null,
): string {
  if (pageTitle === null) return siteData.site.title;
  return `${pageTitle} | ${siteData.site.title}`;
}

/** Reusable breadcrumb helpers for common landmarks. */
export function homeCrumb(config: UrlConfig): BreadcrumbItem {
  return { label: "Home", href: homePath(config) };
}
export function librariesCrumb(config: UrlConfig): BreadcrumbItem {
  return { label: "Libraries", href: librariesRootPath(config) };
}
export function solutionsCrumb(config: UrlConfig): BreadcrumbItem {
  return { label: "Solutions", href: solutionsRootPath(config) };
}
