/**
 * Static 404 renderer per spec §12.1 tail and handoff §11.
 *
 * The page uses the same header, footer, and global search as any other
 * page but sets `<meta name="robots" content="noindex,nofollow">` and adds
 * `data-pagefind-ignore` to the `<main>` so it is not indexed by Google or
 * Pagefind. No JavaScript is used to reveal the requested path.
 */

import type { SiteData } from "../site-data-types.ts";
import {
  homePath,
  librariesRootPath,
  searchPath,
  solutionsRootPath,
  type UrlConfig,
} from "../url.ts";
import {
  formatDocumentTitle,
  homeCrumb,
  renderDocument,
} from "./document.ts";
import { escapeAttribute, escapeHtml } from "./escape.ts";

export function renderNotFoundMainInner(config: UrlConfig): string {
  const home = homePath(config);
  const libraries = librariesRootPath(config);
  const solutions = solutionsRootPath(config);
  const search = searchPath(config);
  return (
    `<header class="page-header">` +
      `<h1>Page not found</h1>` +
      `<p class="summary">The page you were looking for is not available at this URL.</p>` +
    `</header>` +
    `<nav class="recovery-navigation" aria-label="Recovery" data-pagefind-ignore>` +
      `<ul>` +
        `<li><a href="${escapeAttribute(home)}">${escapeHtml("Home")}</a></li>` +
        `<li><a href="${escapeAttribute(libraries)}">${escapeHtml("Libraries")}</a></li>` +
        `<li><a href="${escapeAttribute(solutions)}">${escapeHtml("Solutions")}</a></li>` +
        `<li><a href="${escapeAttribute(search)}">${escapeHtml("Search")}</a></li>` +
      `</ul>` +
    `</nav>`
  );
}

export function renderNotFoundPage(
  config: UrlConfig,
  siteData: SiteData,
): string {
  return renderDocument({
    config,
    siteData,
    canonicalSegments: [],
    documentTitle: formatDocumentTitle(siteData, "Page not found"),
    description: "The requested page could not be found.",
    robots: "noindex,nofollow",
    currentNav: null,
    breadcrumb: [homeCrumb(config), { label: "Page not found" }],
    mainInnerHtml: renderNotFoundMainInner(config),
    mainIgnoreForPagefind: true,
  });
}
