/**
 * Search page (`/search/`) — MVP static shell per spec §12.10 and §12.11.
 *
 * The client-side search UI powered by Pagefind ships in plan 053; for
 * plan 052 this page renders the semantic shell (single `h1`, breadcrumb,
 * grammar hint, `<noscript>` recovery links) so the header's global search
 * form has a valid destination. The page is marked `noindex,nofollow`
 * and excluded from Pagefind.
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

function renderSearchMainInner(config: UrlConfig): string {
  const libraries = librariesRootPath(config);
  const solutions = solutionsRootPath(config);
  return (
    `<header class="page-header">` +
      `<h1>Search</h1>` +
      `<p class="summary">` +
        `Use the global search form in the header to look up libraries, ` +
        `solutions, and symbols across the site.` +
      `</p>` +
    `</header>` +
    `<section class="search-empty" aria-labelledby="search-empty-heading">` +
      `<h2 id="search-empty-heading">How to search</h2>` +
      `<ul>` +
        `<li>Enter a library name, file name, or symbol to see matching detail pages.</li>` +
        `<li>Search results appear here after the client-side index loads.</li>` +
      `</ul>` +
    `</section>` +
    `<noscript>` +
      `<section class="search-noscript" aria-labelledby="search-noscript-heading">` +
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
