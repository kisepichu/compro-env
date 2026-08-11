/**
 * Library browse and detail page renderers per spec §12.4, §12.5 and
 * handoff §5–§7.
 *
 * - Root, language, and directory pages are search-excluded via
 *   `data-pagefind-ignore` on `<main>`.
 * - Detail pages emit a single `<article data-pagefind-body>` with the
 *   canonical `page-{page_id}` ID for Pagefind deduplication.
 */

import type {
  DependencyAnalysisPublic,
  LanguageSummary,
  LibraryLink,
  LibraryPageData,
  RelationPublic,
  SiteData,
  SymbolAnalysisPublic,
  VerificationEvidence,
} from "../site-data-types.ts";
import {
  homePath,
  librariesRootPath,
  libraryPath,
  toInternalPath,
  type UrlConfig,
} from "../url.ts";
import { librariesForLanguage } from "./counts.ts";
import {
  formatDocumentTitle,
  homeCrumb,
  librariesCrumb,
  renderDocument,
  type BreadcrumbItem,
} from "./document.ts";
import { escapeAttribute, escapeHtml } from "./escape.ts";
import { renderStatus } from "./status.ts";

// ---- Route enumeration ----

export interface LibraryRoute {
  /** Slash-joined `path` for Astro's `[...path]` param (never leading `/`). */
  segments: string[];
  kind: "language" | "directory" | "detail";
  /** For detail routes, the underlying library. */
  library?: LibraryPageData;
  /** For language / directory routes: language id + directory path parts. */
  languageId?: string;
  directoryParts?: string[];
}

/**
 * Split a library source_path into `/`-separated parts, discarding empty
 * segments (e.g. leading `/`).
 */
export function splitSourcePath(sourcePath: string): string[] {
  return sourcePath.split("/").filter((p) => p.length > 0);
}

/**
 * Enumerate every static route below `/libraries/`:
 *   - one route per language that has at least one public library,
 *   - one route per unique directory prefix under such a language,
 *   - one route per library detail (segments include the file name).
 */
export function listLibraryRoutes(siteData: SiteData): LibraryRoute[] {
  const routes: LibraryRoute[] = [];
  const languagesWithLibs = new Set(
    siteData.libraries.map((lib) => lib.language),
  );
  for (const languageId of [...languagesWithLibs].sort()) {
    routes.push({ segments: [languageId], kind: "language", languageId });
    const dirs = new Set<string>();
    const libsInLang = siteData.libraries.filter(
      (l) => l.language === languageId,
    );
    for (const lib of libsInLang) {
      const parts = splitSourcePath(lib.source_path);
      // All directory prefixes except the file itself.
      for (let i = 1; i < parts.length; i += 1) {
        dirs.add(parts.slice(0, i).join("/"));
      }
    }
    for (const dir of [...dirs].sort()) {
      const directoryParts = dir.split("/");
      routes.push({
        segments: [languageId, ...directoryParts],
        kind: "directory",
        languageId,
        directoryParts,
      });
    }
    for (const lib of libsInLang) {
      const parts = splitSourcePath(lib.source_path);
      routes.push({
        segments: [languageId, ...parts],
        kind: "detail",
        library: lib,
      });
    }
  }
  return routes;
}

// ---- Language card (shared with Home) ----

function renderLanguageCard(
  config: UrlConfig,
  lang: LanguageSummary,
): string {
  const href = toInternalPath(config, ["libraries", lang.id]);
  return (
    `<li><article class="language-card">` +
      `<h3><a href="${escapeAttribute(href)}">${escapeHtml(lang.display_name)}</a></h3>` +
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

// ---- /libraries/ ----

function eligibleLanguages(siteData: SiteData): LanguageSummary[] {
  const copy = siteData.languages.filter((l) => l.library_count > 0);
  copy.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return copy;
}

export function renderLibrariesRootMainInner(
  config: UrlConfig,
  siteData: SiteData,
): string {
  const langs = eligibleLanguages(siteData);
  const languagesHtml =
    langs.length === 0
      ? `<p class="empty-state">No languages have public libraries yet.</p>`
      : `<ul class="language-list">${langs.map((l) => renderLanguageCard(config, l)).join("")}</ul>`;
  return (
    `<header class="page-header"><h1>Libraries</h1></header>` +
    `<section class="languages" aria-labelledby="languages-heading">` +
      `<h2 id="languages-heading">Languages</h2>` +
      languagesHtml +
    `</section>`
  );
}

export function renderLibrariesRootPage(
  config: UrlConfig,
  siteData: SiteData,
): string {
  return renderDocument({
    config,
    siteData,
    canonicalSegments: ["libraries"],
    documentTitle: formatDocumentTitle(siteData, "Libraries"),
    description: `Public libraries in ${siteData.site.title}.`,
    currentNav: "libraries",
    breadcrumb: [homeCrumb(config), { label: "Libraries" }],
    mainInnerHtml: renderLibrariesRootMainInner(config, siteData),
    mainIgnoreForPagefind: true,
  });
}

// ---- /libraries/{lang}/ and /libraries/{lang}/{dir...}/ ----

/**
 * Build the ordered list of libraries directly at a directory (no descendants).
 * `directoryParts` empty means the language root.
 */
function librariesAtDirectory(
  siteData: SiteData,
  languageId: string,
  directoryParts: readonly string[],
): LibraryPageData[] {
  const depth = directoryParts.length;
  return librariesForLanguage(siteData, languageId).filter((lib) => {
    const parts = splitSourcePath(lib.source_path);
    if (parts.length !== depth + 1) return false;
    for (let i = 0; i < depth; i += 1) {
      if (parts[i] !== directoryParts[i]) return false;
    }
    return true;
  });
}

/** Names of immediate child directories at a given directory depth. */
function childDirectoriesAt(
  siteData: SiteData,
  languageId: string,
  directoryParts: readonly string[],
): string[] {
  const depth = directoryParts.length;
  const seen = new Set<string>();
  for (const lib of librariesForLanguage(siteData, languageId)) {
    const parts = splitSourcePath(lib.source_path);
    if (parts.length <= depth + 1) continue;
    let ok = true;
    for (let i = 0; i < depth; i += 1) {
      if (parts[i] !== directoryParts[i]) {
        ok = false;
        break;
      }
    }
    if (!ok) continue;
    seen.add(parts[depth]);
  }
  return [...seen].sort();
}

/** Directory verification counts by aggregating library statuses. */
function directoryVerificationCounts(
  libraries: readonly LibraryPageData[],
): Record<string, number> {
  const counts: Record<string, number> = {
    verified: 0,
    rejected: 0,
    unavailable: 0,
    stale: 0,
    never: 0,
  };
  for (const lib of libraries) {
    counts[lib.verification.aggregate_status] += 1;
  }
  return counts;
}

/** Descendant public libraries at or below a directory. */
function descendantLibraries(
  siteData: SiteData,
  languageId: string,
  directoryParts: readonly string[],
): LibraryPageData[] {
  const depth = directoryParts.length;
  return librariesForLanguage(siteData, languageId).filter((lib) => {
    const parts = splitSourcePath(lib.source_path);
    if (parts.length <= depth) return false;
    for (let i = 0; i < depth; i += 1) {
      if (parts[i] !== directoryParts[i]) return false;
    }
    return true;
  });
}

function renderDirectoryCard(
  config: UrlConfig,
  languageId: string,
  directoryParts: readonly string[],
  childName: string,
  siteData: SiteData,
): string {
  const childDirParts = [...directoryParts, childName];
  const href = toInternalPath(config, ["libraries", languageId, ...childDirParts]);
  const descendants = descendantLibraries(siteData, languageId, childDirParts);
  const counts = directoryVerificationCounts(descendants);
  return (
    `<li><article class="directory-card">` +
      `<h3><a href="${escapeAttribute(href)}">${escapeHtml(childName)}</a></h3>` +
      `<p class="directory-path"><code>${escapeHtml(childDirParts.join("/"))}</code></p>` +
      `<dl class="directory-counts">` +
        `<dt>Public descendants</dt><dd>${descendants.length}</dd>` +
        `<dt>Verified</dt><dd>${counts.verified}</dd>` +
        `<dt>Stale</dt><dd>${counts.stale}</dd>` +
        `<dt>Rejected</dt><dd>${counts.rejected}</dd>` +
        `<dt>Unavailable</dt><dd>${counts.unavailable}</dd>` +
        `<dt>Never verified</dt><dd>${counts.never}</dd>` +
      `</dl>` +
    `</article></li>`
  );
}

function renderLibraryCard(
  config: UrlConfig,
  lib: LibraryPageData,
): string {
  const href = libraryPath(config, lib.language, lib.source_path);
  const parts = splitSourcePath(lib.source_path);
  const fileName = parts[parts.length - 1] ?? lib.source_path;
  return (
    `<li><article class="library-card">` +
      `<h3><a href="${escapeAttribute(href)}">${escapeHtml(lib.title)}</a></h3>` +
      `<p class="library-file-name"><code>${escapeHtml(fileName)}</code></p>` +
      `<p class="library-updated"><time datetime="${escapeAttribute(lib.updated_at)}">${escapeHtml(lib.updated_at)}</time></p>` +
      renderStatus("library-verification", lib.verification.aggregate_status) +
    `</article></li>`
  );
}

/** Breadcrumb chain for `/libraries/{lang}/{dir...}/` including the current page. */
function librariesBreadcrumb(
  config: UrlConfig,
  languageDisplay: string,
  languageId: string,
  directoryParts: readonly string[],
  language: LanguageSummary | undefined,
): BreadcrumbItem[] {
  const items: BreadcrumbItem[] = [homeCrumb(config), librariesCrumb(config)];
  if (directoryParts.length === 0) {
    items.push({ label: language?.display_name ?? languageDisplay });
    return items;
  }
  // Language link.
  items.push({
    label: language?.display_name ?? languageId,
    href: toInternalPath(config, ["libraries", languageId]),
  });
  for (let i = 0; i < directoryParts.length - 1; i += 1) {
    items.push({
      label: directoryParts[i],
      href: toInternalPath(config, [
        "libraries",
        languageId,
        ...directoryParts.slice(0, i + 1),
      ]),
    });
  }
  items.push({ label: directoryParts[directoryParts.length - 1] });
  return items;
}

export function renderLibraryDirectoryMainInner(
  config: UrlConfig,
  siteData: SiteData,
  languageId: string,
  directoryParts: readonly string[],
): string {
  const language = siteData.languages.find((l) => l.id === languageId);
  const displayName = language?.display_name ?? languageId;
  const heading =
    directoryParts.length === 0
      ? displayName
      : directoryParts[directoryParts.length - 1];
  const subtitleText =
    directoryParts.length === 0
      ? languageId
      : [languageId, ...directoryParts].join("/");
  const directLibraries = librariesAtDirectory(
    siteData,
    languageId,
    directoryParts,
  );
  directLibraries.sort((a, b) =>
    a.source_path < b.source_path ? -1 : a.source_path > b.source_path ? 1 : 0,
  );
  const childDirs = childDirectoriesAt(siteData, languageId, directoryParts);
  const descendants = descendantLibraries(siteData, languageId, directoryParts);
  const counts = directoryVerificationCounts(descendants);
  const libsHtml =
    directLibraries.length === 0
      ? `<p class="empty-state">No public libraries at this location.</p>`
      : `<ul class="library-list">${directLibraries.map((l) => renderLibraryCard(config, l)).join("")}</ul>`;
  const dirsHtml =
    childDirs.length === 0
      ? `<p class="empty-state">No child directories.</p>`
      : `<ul class="directory-list">${childDirs
          .map((c) =>
            renderDirectoryCard(config, languageId, directoryParts, c, siteData),
          )
          .join("")}</ul>`;
  return (
    `<header class="page-header">` +
      `<h1>${escapeHtml(heading)}</h1>` +
      `<p class="subtitle"><code>${escapeHtml(subtitleText)}</code></p>` +
      `<dl class="verification-summary">` +
        `<dt>Verified</dt><dd>${counts.verified}</dd>` +
        `<dt>Stale</dt><dd>${counts.stale}</dd>` +
        `<dt>Rejected</dt><dd>${counts.rejected}</dd>` +
        `<dt>Unavailable</dt><dd>${counts.unavailable}</dd>` +
        `<dt>Never verified</dt><dd>${counts.never}</dd>` +
      `</dl>` +
    `</header>` +
    `<section class="child-directories" aria-labelledby="child-directories-heading">` +
      `<h2 id="child-directories-heading">Child directories</h2>` +
      dirsHtml +
    `</section>` +
    `<section class="library-files" aria-labelledby="library-files-heading">` +
      `<h2 id="library-files-heading">Library files</h2>` +
      libsHtml +
    `</section>`
  );
}

export function renderLibraryDirectoryPage(
  config: UrlConfig,
  siteData: SiteData,
  languageId: string,
  directoryParts: readonly string[],
): string {
  const language = siteData.languages.find((l) => l.id === languageId);
  const displayName = language?.display_name ?? languageId;
  const heading =
    directoryParts.length === 0
      ? displayName
      : directoryParts[directoryParts.length - 1];
  const canonicalSegments = [
    "libraries",
    languageId,
    ...directoryParts,
  ];
  const description =
    directoryParts.length === 0
      ? `Public ${displayName} libraries.`
      : `Public libraries under ${[languageId, ...directoryParts].join("/")}.`;
  return renderDocument({
    config,
    siteData,
    canonicalSegments,
    documentTitle: formatDocumentTitle(siteData, heading),
    description,
    currentNav: "libraries",
    breadcrumb: librariesBreadcrumb(
      config,
      displayName,
      languageId,
      directoryParts,
      language,
    ),
    mainInnerHtml: renderLibraryDirectoryMainInner(
      config,
      siteData,
      languageId,
      directoryParts,
    ),
    mainIgnoreForPagefind: true,
  });
}

// ---- /libraries/{lang}/{source-path...}/ (detail) ----

function renderDependencyList(
  section: string,
  links: readonly LibraryLink[],
  config: UrlConfig,
  emptyMessage: string,
): string {
  if (links.length === 0) {
    return `<p class="empty-state">${escapeHtml(emptyMessage)}</p>`;
  }
  const items = links
    .map((link) => {
      const href = libraryPath(config, link.language, link.source_path);
      const manualBadge = link.manual
        ? ` <span class="manual-marker">manual</span>`
        : "";
      return (
        `<li>` +
          `<a href="${escapeAttribute(href)}">${escapeHtml(link.title)}</a>` +
          ` <span class="language">${escapeHtml(link.language)}</span>` +
          ` <code class="path">${escapeHtml(link.source_path)}</code>` +
          manualBadge +
        `</li>`
      );
    })
    .join("");
  return `<ul class="${escapeAttribute(section)}-list">${items}</ul>`;
}

function renderRelationList(
  relations: readonly RelationPublic[],
  config: UrlConfig,
): string {
  if (relations.length === 0) {
    return `<p class="empty-state">No relations declared.</p>`;
  }
  const items = relations
    .map((rel) => {
      const href = libraryPath(config, rel.target.language, rel.target.source_path);
      const manualBadge = rel.manual
        ? ` <span class="manual-marker">manual</span>`
        : "";
      return (
        `<li>` +
          `<span class="relation-kind">${escapeHtml(rel.kind)}</span> ` +
          `<a href="${escapeAttribute(href)}">${escapeHtml(rel.target.title)}</a>` +
          manualBadge +
        `</li>`
      );
    })
    .join("");
  return `<ul class="relations-list">${items}</ul>`;
}

function renderVerificationEvidenceList(
  evidence: readonly VerificationEvidence[],
): string {
  if (evidence.length === 0) {
    return `<p class="empty-state">No verification evidence recorded.</p>`;
  }
  const items = evidence
    .map((ev) => {
      const time =
        ev.judged_at !== null && ev.judged_at !== undefined
          ? `<time datetime="${escapeAttribute(ev.judged_at)}">${escapeHtml(ev.judged_at)}</time>`
          : `<span class="empty-time">not judged yet</span>`;
      const ojLink =
        ev.oj_submission_url !== null && ev.oj_submission_url !== undefined
          ? ` <a href="${escapeAttribute(ev.oj_submission_url)}" rel="noopener noreferrer">OJ submission</a>`
          : "";
      const stale =
        ev.status === "stale" && ev.stale_reason
          ? ` <p class="stale-reason">${escapeHtml(ev.stale_reason)}</p>`
          : "";
      return (
        `<li><article class="verification-evidence">` +
          `<p class="evidence-solution">` +
            `<span class="solution-id"><code>${escapeHtml(ev.solution_id)}</code></span> ` +
            `<span class="oj">${escapeHtml(ev.online_judge)}</span>` +
          `</p>` +
          renderStatus("evidence", ev.status) +
          ` <p class="evidence-judged">${time}${ojLink}</p>` +
          stale +
        `</article></li>`
      );
    })
    .join("");
  return `<ul class="evidence-list">${items}</ul>`;
}

function renderSymbolsSection(analysis: SymbolAnalysisPublic): string {
  if (analysis.symbols.length === 0) {
    if (analysis.state === "complete") {
      return `<p class="empty-state">No symbols detected.</p>`;
    }
    return `<p class="empty-state">Symbol analysis is incomplete; no symbols to show.</p>`;
  }
  const items = analysis.symbols
    .map((sym) => {
      const qualified = sym.qualified_name
        ? ` <span class="qualified">${escapeHtml(sym.qualified_name)}</span>`
        : "";
      const sig = sym.signature
        ? ` <code class="signature">${escapeHtml(sym.signature)}</code>`
        : "";
      return (
        `<li>` +
          `<span class="kind">${escapeHtml(sym.kind)}</span> ` +
          `<code class="name">${escapeHtml(sym.name)}</code>` +
          qualified +
          sig +
        `</li>`
      );
    })
    .join("");
  return `<ul class="symbols-list">${items}</ul>`;
}

function renderDiagnostics(diags: readonly { severity: string; code: string; message: string }[]): string {
  if (diags.length === 0) {
    return `<p class="empty-state">No diagnostics.</p>`;
  }
  const items = diags
    .map(
      (d) =>
        `<li>` +
          `<span class="severity">${escapeHtml(d.severity)}</span> ` +
          `<code class="diagnostic-code">${escapeHtml(d.code)}</code> ` +
          `<p class="message">${escapeHtml(d.message)}</p>` +
        `</li>`,
    )
    .join("");
  return `<ul class="diagnostics-list">${items}</ul>`;
}

function libraryBreadcrumb(
  config: UrlConfig,
  siteData: SiteData,
  lib: LibraryPageData,
): BreadcrumbItem[] {
  const language = siteData.languages.find((l) => l.id === lib.language);
  const items: BreadcrumbItem[] = [homeCrumb(config), librariesCrumb(config)];
  items.push({
    label: language?.display_name ?? lib.language,
    href: toInternalPath(config, ["libraries", lib.language]),
  });
  const parts = splitSourcePath(lib.source_path);
  // Directory prefixes (excluding the file itself).
  for (let i = 0; i < parts.length - 1; i += 1) {
    items.push({
      label: parts[i],
      href: toInternalPath(config, ["libraries", lib.language, ...parts.slice(0, i + 1)]),
    });
  }
  items.push({ label: lib.title });
  return items;
}

function renderLibraryDetailArticleInner(
  config: UrlConfig,
  lib: LibraryPageData,
  dependencyAnalysis: DependencyAnalysisPublic,
): string {
  const hasDocumentation =
    lib.description !== null &&
    lib.description !== undefined &&
    lib.description.trim().length > 0;
  const documentationBlock = hasDocumentation
    ? `<div id="documentation" class="documentation">${escapeHtml(lib.description!)}</div>`
    : "";
  const inPageNavItems: { id: string; label: string }[] = [];
  if (hasDocumentation) {
    inPageNavItems.push({ id: "documentation", label: "Documentation" });
  }
  inPageNavItems.push(
    { id: "symbols", label: "Symbols" },
    { id: "source", label: "Source" },
    { id: "dependencies", label: "Dependencies" },
    { id: "relations", label: "Relations" },
    { id: "verification", label: "Verification" },
    { id: "diagnostics", label: "Diagnostics" },
  );
  const inPageNav =
    `<nav class="in-page-navigation" aria-label="On this page" data-pagefind-ignore>` +
      `<ul>` +
        inPageNavItems
          .map(
            (item) =>
              `<li><a href="#${escapeAttribute(item.id)}">${escapeHtml(item.label)}</a></li>`,
          )
          .join("") +
      `</ul>` +
    `</nav>`;
  const privateDepNote = dependencyAnalysis.has_private_dependencies
    ? `<p class="private-dependencies-note">This library also depends on private targets.</p>`
    : "";
  const relativePath = escapeHtml(lib.source_path);
  return (
    `<header class="page-header">` +
      `<h1>${escapeHtml(lib.title)}</h1>` +
      `<p class="library-meta">` +
        `<span class="language">${escapeHtml(lib.language)}</span> ` +
        `<code class="path">${relativePath}</code> ` +
        `<time datetime="${escapeAttribute(lib.updated_at)}">${escapeHtml(lib.updated_at)}</time>` +
      `</p>` +
      renderStatus("library-verification", lib.verification.aggregate_status) +
      renderStatus("analysis", dependencyAnalysis.state) +
      renderStatus("analysis", lib.symbol_analysis.state) +
    `</header>` +
    inPageNav +
    documentationBlock +
    `<section id="symbols" aria-labelledby="symbols-heading">` +
      `<h2 id="symbols-heading">Symbols</h2>` +
      renderSymbolsSection(lib.symbol_analysis) +
    `</section>` +
    `<section id="source" aria-labelledby="source-heading">` +
      `<h2 id="source-heading">Source</h2>` +
      `<p class="pending">Source rendering pending (Task 3).</p>` +
    `</section>` +
    `<section id="dependencies" aria-labelledby="dependencies-heading">` +
      `<h2 id="dependencies-heading">Dependencies</h2>` +
      `<h3>Depends on</h3>` +
      renderDependencyList("depends-on", dependencyAnalysis.direct, config, "No direct dependencies.") +
      privateDepNote +
      `<h3>Used by</h3>` +
      renderDependencyList("used-by", lib.reverse_dependencies, config, "No public library uses this one yet.") +
    `</section>` +
    `<section id="relations" aria-labelledby="relations-heading">` +
      `<h2 id="relations-heading">Relations</h2>` +
      renderRelationList(lib.relations, config) +
    `</section>` +
    `<section id="verification" aria-labelledby="verification-heading">` +
      `<h2 id="verification-heading">Verification</h2>` +
      renderVerificationEvidenceList(lib.verification.evidence) +
    `</section>` +
    `<section id="diagnostics" aria-labelledby="diagnostics-heading">` +
      `<h2 id="diagnostics-heading">Diagnostics</h2>` +
      renderDiagnostics(lib.diagnostics) +
    `</section>`
  );
}

export function renderLibraryDetailMainInner(
  config: UrlConfig,
  siteData: SiteData,
  lib: LibraryPageData,
): string {
  const pageIdAttr = escapeAttribute(`page-${lib.page_id}`);
  return (
    `<article class="library-detail" id="${pageIdAttr}" data-pagefind-body>` +
      renderLibraryDetailArticleInner(config, lib, lib.dependency_analysis) +
    `</article>`
  );
}

export function renderLibraryDetailPage(
  config: UrlConfig,
  siteData: SiteData,
  lib: LibraryPageData,
): string {
  const canonicalSegments = [
    "libraries",
    lib.language,
    ...splitSourcePath(lib.source_path),
  ];
  const description =
    lib.description && lib.description.trim().length > 0
      ? lib.description
      : `${lib.title} — ${lib.language} library in ${siteData.site.title}.`;
  return renderDocument({
    config,
    siteData,
    canonicalSegments,
    documentTitle: formatDocumentTitle(siteData, lib.title),
    description,
    currentNav: "libraries",
    breadcrumb: libraryBreadcrumb(config, siteData, lib),
    mainInnerHtml: renderLibraryDetailMainInner(config, siteData, lib),
    mainIgnoreForPagefind: false,
    robots: "index,follow",
  });
}

// Reference `homePath` to keep the import in use across future edits.
void homePath;
