/**
 * Home page renderer per spec §12.3 and handoff §4.
 *
 * Sections: page-header, status-overview, languages, recent-libraries,
 * recent-solutions, attention-required. Empty sections still render an
 * `h2` and a short empty-state paragraph. No Home-specific search form.
 * The entire `<main>` is search-excluded via `data-pagefind-ignore`.
 */

import type {
  LanguageSummary,
  LibraryPageData,
  SiteData,
  SolutionPageData,
} from "../site-data-types.ts";
import {
  libraryPath,
  solutionPath,
  type UrlConfig,
} from "../url.ts";
import {
  attentionLibraries,
  recentLibraries,
  recentSolutions,
  sumLanguageVerification,
  sumSolutionVerification,
} from "./counts.ts";
import {
  formatDocumentTitle,
  renderDocument,
} from "./document.ts";
import { escapeAttribute, escapeHtml } from "./escape.ts";
import { renderStatus } from "./status.ts";
import { formatCompactTimestamp } from "./time.ts";

function renderLanguageCard(lang: LanguageSummary): string {
  return (
    `<li><article class="language-card">` +
      `<h3>${escapeHtml(lang.display_name)}</h3>` +
      `<p class="language-id">${escapeHtml(lang.id)}</p>` +
      `<dl class="language-counts">` +
        `<dt>Public libraries</dt><dd>${lang.library_count}</dd>` +
        `<dt>Verified</dt><dd>${lang.verification_summary.verified}</dd>` +
        `<dt>Stale</dt><dd>${lang.verification_summary.stale}</dd>` +
        `<dt>Rejected</dt><dd>${lang.verification_summary.rejected}</dd>` +
        `<dt>Unavailable</dt><dd>${lang.verification_summary.unavailable}</dd>` +
        `<dt>Never verified</dt><dd>${lang.verification_summary.never}</dd>` +
      `</dl>` +
    `</article></li>`
  );
}

function renderRecentLibraryItem(
  config: UrlConfig,
  lib: LibraryPageData,
): string {
  const href = libraryPath(config, lib.language, lib.source_path);
  return (
    `<li><article class="library-card">` +
      `<h3><a href="${escapeAttribute(href)}">${escapeHtml(lib.title)}</a></h3>` +
      `<p class="library-language">${escapeHtml(lib.language)}</p>` +
      `<p class="library-path"><code>${escapeHtml(lib.source_path)}</code></p>` +
      `<p class="library-updated"><time datetime="${escapeAttribute(lib.updated_at)}">${escapeHtml(formatCompactTimestamp(lib.updated_at))}</time></p>` +
      renderStatus("library-verification", lib.verification.aggregate_status) +
    `</article></li>`
  );
}

function renderRecentSolutionItem(
  config: UrlConfig,
  sol: SolutionPageData,
): string {
  const href = solutionPath(config, sol.contest_id, sol.problem_code, sol.solution_name);
  return (
    `<li><article class="solution-card">` +
      `<h3><a href="${escapeAttribute(href)}">${escapeHtml(sol.solution_name)}</a></h3>` +
      `<p class="solution-contest">${escapeHtml(sol.contest_id)} / ${escapeHtml(sol.problem_code)}</p>` +
      `<p class="solution-language">${escapeHtml(sol.language)}</p>` +
      `<p class="solution-solved"><time datetime="${escapeAttribute(sol.solved_at)}">${escapeHtml(formatCompactTimestamp(sol.solved_at))}</time></p>` +
      renderStatus("solution-verification", sol.verification.status) +
    `</article></li>`
  );
}

function renderAttentionItem(
  config: UrlConfig,
  lib: LibraryPageData,
): string {
  const href = libraryPath(config, lib.language, lib.source_path);
  return (
    `<li><article class="attention-card">` +
      `<h3><a href="${escapeAttribute(href)}">${escapeHtml(lib.title)}</a></h3>` +
      `<p class="attention-language">${escapeHtml(lib.language)}</p>` +
      renderStatus("library-verification", lib.verification.aggregate_status) +
      (lib.symbol_analysis.state !== "complete"
        ? renderStatus("analysis", lib.symbol_analysis.state)
        : "") +
      (lib.dependency_analysis.state !== "complete"
        ? renderStatus("analysis", lib.dependency_analysis.state)
        : "") +
    `</article></li>`
  );
}

export function renderHomeMainInner(
  config: UrlConfig,
  siteData: SiteData,
): string {
  const languages = [...siteData.languages].sort((a, b) =>
    a.id < b.id ? -1 : a.id > b.id ? 1 : 0,
  );
  const libTotals = sumLanguageVerification(languages);
  const solTotals = sumSolutionVerification(siteData.solutions);
  const recent = recentLibraries(siteData.libraries, 10);
  const recentSol = recentSolutions(siteData.solutions, 10);
  const attention = attentionLibraries(siteData.libraries).slice(0, 10);

  const languagesHtml =
    languages.length === 0
      ? `<p class="empty-state">No languages are configured yet.</p>`
      : `<ul class="language-list">${languages.map(renderLanguageCard).join("")}</ul>`;

  const recentLibrariesHtml =
    recent.length === 0
      ? `<p class="empty-state">No public libraries yet.</p>`
      : `<ul class="library-list">${recent.map((l) => renderRecentLibraryItem(config, l)).join("")}</ul>`;

  const recentSolutionsHtml =
    recentSol.length === 0
      ? `<p class="empty-state">No public solutions yet.</p>`
      : `<ul class="solution-list">${recentSol.map((s) => renderRecentSolutionItem(config, s)).join("")}</ul>`;

  const attentionHtml =
    attention.length === 0
      ? `<p class="empty-state">Nothing needs attention right now.</p>`
      : `<ul class="attention-list">${attention.map((l) => renderAttentionItem(config, l)).join("")}</ul>`;

  return (
    `<header class="page-header">` +
      `<h1>${escapeHtml(siteData.site.title)}</h1>` +
      `<p class="summary">${escapeHtml(siteData.site.description)}</p>` +
    `</header>` +
    `<section class="status-overview" aria-labelledby="status-overview-heading">` +
      `<h2 id="status-overview-heading">Repository status</h2>` +
      `<dl class="status-counts">` +
        `<dt>Public libraries</dt><dd>${libTotals.total}</dd>` +
        `<dt>Public solutions</dt><dd>${solTotals.total}</dd>` +
        `<dt>Verified libraries</dt><dd>${libTotals.verified}</dd>` +
        `<dt>Stale libraries</dt><dd>${libTotals.stale}</dd>` +
        `<dt>Rejected libraries</dt><dd>${libTotals.rejected}</dd>` +
        `<dt>Unavailable libraries</dt><dd>${libTotals.unavailable}</dd>` +
        `<dt>Never verified libraries</dt><dd>${libTotals.never}</dd>` +
      `</dl>` +
    `</section>` +
    `<section class="languages" aria-labelledby="languages-heading">` +
      `<h2 id="languages-heading">Languages</h2>` +
      languagesHtml +
    `</section>` +
    `<section class="recent-libraries" aria-labelledby="recent-libraries-heading">` +
      `<h2 id="recent-libraries-heading">Recently updated libraries</h2>` +
      recentLibrariesHtml +
    `</section>` +
    `<section class="recent-solutions" aria-labelledby="recent-solutions-heading">` +
      `<h2 id="recent-solutions-heading">Recently solved solutions</h2>` +
      recentSolutionsHtml +
    `</section>` +
    `<section class="attention-required" aria-labelledby="attention-required-heading">` +
      `<h2 id="attention-required-heading">Attention required</h2>` +
      attentionHtml +
    `</section>`
  );
}

export function renderHomePage(config: UrlConfig, siteData: SiteData): string {
  const mainInner = renderHomeMainInner(config, siteData);
  return renderDocument({
    config,
    siteData,
    canonicalSegments: [],
    documentTitle: formatDocumentTitle(siteData, null),
    description: siteData.site.description,
    currentNav: null,
    breadcrumb: [],
    mainInnerHtml: mainInner,
    mainIgnoreForPagefind: true,
  });
}
