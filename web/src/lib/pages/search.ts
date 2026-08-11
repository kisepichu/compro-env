/**
 * Search page (`/search/`) — full semantic shell per spec §13.2.
 *
 * The client script (`web/src/search/client.ts`) hydrates the elements
 * below: it reads the URL, parses the query, runs both Pagefind and the
 * exact-match index, and rewrites `#search-status`, `#search-summary`,
 * `#search-results`, and `#search-pagination`. The page still ships a
 * meaningful static skeleton so screen readers and no-JS users get a
 * useful landmark structure. The `<noscript>` panel exposes browse
 * links.
 */

import type { SiteData } from "../site-data-types.ts";
import {
  homePath,
  librariesRootPath,
  solutionsRootPath,
  type UrlConfig,
} from "../url.ts";
import {
  formatDocumentTitle,
  homeCrumb,
  renderDocument,
} from "./document.ts";
import { escapeAttribute, escapeHtml } from "./escape.ts";

/**
 * Emit the static `<main>` skeleton for the search page.
 *
 * The client script populates every element that starts hidden. Element
 * IDs are the load-bearing part of this contract — tests rely on them.
 */
export function renderSearchMainInner(config: UrlConfig): string {
  const libraries = librariesRootPath(config);
  const solutions = solutionsRootPath(config);
  const base = config.base;
  return (
    `<section id="search-app" class="search-app" data-base="${escapeAttribute(base)}">` +
      `<header class="page-header">` +
        `<h1>Search</h1>` +
        `<p class="summary">` +
          `Use the global search form in the header to look up libraries, ` +
          `solutions, and symbols across the site.` +
        `</p>` +
      `</header>` +
      // Parsed-filter chip UI populated by the client when filters are present.
      `<section id="parsed-filters" class="parsed-filters" ` +
        `aria-label="Active filters" hidden></section>` +
      // Loading indicator. role="status" per spec §13.2.
      `<div id="search-status" class="search-status" role="status" ` +
        `aria-live="polite" hidden></div>` +
      // Query error / Pagefind load failure. role="alert" per spec §13.2.
      `<div id="search-alert" class="search-alert" role="alert" hidden></div>` +
      // Result summary ("Showing 1–20 of 45"). aria-live="polite" per spec §13.2.
      `<p id="search-summary" class="search-summary" aria-live="polite" hidden></p>` +
      // Ordered result list — client rewrites this in place.
      `<ol id="search-results" class="search-results"></ol>` +
      // Pagination controls (Previous / current / Next).
      `<nav id="search-pagination" class="pagination" ` +
        `aria-label="Search pagination" hidden></nav>` +
      // Empty / grammar-hint state shown for missing q or empty result.
      `<section id="search-empty" class="search-empty" ` +
        `aria-labelledby="search-empty-heading" hidden>` +
        `<h2 id="search-empty-heading">How to search</h2>` +
        `<p>` +
          `Type a term or symbol name, or combine with filters:` +
        `</p>` +
        `<ul class="search-grammar">` +
          `<li><code>monoid</code> — full-text search</li>` +
          `<li><code>monoid lang:cpp</code> — combine terms and filters</li>` +
          `<li><code>path:algebra verified:true</code> — filter only</li>` +
          `<li><code>path:"data structures/fenwick tree.cpp"</code> — quoted values</li>` +
          `<li>Filter keys: <code>lang</code>, <code>kind</code>, <code>path</code>, ` +
            `<code>verified</code>, <code>status</code>, <code>type</code></li>` +
        `</ul>` +
      `</section>` +
    `</section>` +
    `<noscript>` +
      `<section class="search-noscript" ` +
        `aria-labelledby="search-noscript-heading">` +
        `<h2 id="search-noscript-heading">Search requires JavaScript</h2>` +
        `<p>` +
          `The static index needs JavaScript to run. Meanwhile you can ` +
          `browse the site directly.` +
        `</p>` +
        `<ul>` +
          `<li><a href="${escapeAttribute(libraries)}">${escapeHtml("Libraries")}</a></li>` +
          `<li><a href="${escapeAttribute(solutions)}">${escapeHtml("Solutions")}</a></li>` +
        `</ul>` +
      `</section>` +
    `</noscript>`
  );
}

export function renderSearchPage(
  config: UrlConfig,
  siteData: SiteData,
): string {
  void homePath;
  return renderDocument({
    config,
    siteData,
    canonicalSegments: ["search"],
    documentTitle: formatDocumentTitle(siteData, "Search"),
    description: "Search libraries, solutions, and symbols on compro-env.",
    robots: "noindex,nofollow",
    currentNav: "search",
    breadcrumb: [homeCrumb(config), { label: "Search" }],
    mainInnerHtml: renderSearchMainInner(config),
    mainIgnoreForPagefind: true,
  });
}
