/**
 * Solution browse and detail page renderers per spec §12.6, §12.7 and
 * handoff §8–§9.
 *
 * - Root, contest, and problem pages are search-excluded via
 *   `data-pagefind-ignore` on `<main>`.
 * - Detail pages emit a single `<article data-pagefind-body>` with the
 *   canonical `page-{page_id}` ID.
 */

import type {
  LibraryLink,
  SiteData,
  SolutionPageData,
} from "../site-data-types.ts";
import {
  libraryPath,
  solutionsRootPath,
  solutionPath,
  toInternalPath,
  type UrlConfig,
} from "../url.ts";
import {
  formatDocumentTitle,
  homeCrumb,
  renderDocument,
  solutionsCrumb,
  type BreadcrumbItem,
} from "./document.ts";
import { escapeAttribute, escapeHtml } from "./escape.ts";
import { renderStatus } from "./status.ts";

// ---- Route enumeration ----

export interface SolutionRoute {
  segments: string[];
  kind: "contest" | "problem" | "detail";
  contestId?: string;
  problemCode?: string;
  solution?: SolutionPageData;
}

export function listSolutionRoutes(siteData: SiteData): SolutionRoute[] {
  const routes: SolutionRoute[] = [];
  const byContest = new Map<string, SolutionPageData[]>();
  for (const sol of siteData.solutions) {
    const arr = byContest.get(sol.contest_id) ?? [];
    arr.push(sol);
    byContest.set(sol.contest_id, arr);
  }
  const contestIds = [...byContest.keys()].sort();
  for (const contestId of contestIds) {
    routes.push({ segments: [contestId], kind: "contest", contestId });
    const problems = new Set<string>();
    for (const sol of byContest.get(contestId)!) {
      problems.add(sol.problem_code);
    }
    for (const problemCode of [...problems].sort()) {
      routes.push({
        segments: [contestId, problemCode],
        kind: "problem",
        contestId,
        problemCode,
      });
    }
    for (const sol of byContest.get(contestId)!) {
      routes.push({
        segments: [contestId, sol.problem_code, sol.solution_name],
        kind: "detail",
        solution: sol,
      });
    }
  }
  return routes;
}

// ---- /solutions/ ----

interface ContestGroup {
  contestId: string;
  latestSolvedAt: string;
  problems: Set<string>;
  count: number;
}

function contestGroups(siteData: SiteData): ContestGroup[] {
  const groups = new Map<string, ContestGroup>();
  for (const sol of siteData.solutions) {
    const g = groups.get(sol.contest_id) ?? {
      contestId: sol.contest_id,
      latestSolvedAt: sol.solved_at,
      problems: new Set<string>(),
      count: 0,
    };
    if (sol.solved_at > g.latestSolvedAt) g.latestSolvedAt = sol.solved_at;
    g.problems.add(sol.problem_code);
    g.count += 1;
    groups.set(sol.contest_id, g);
  }
  const list = [...groups.values()];
  list.sort((a, b) => {
    if (a.latestSolvedAt === b.latestSolvedAt) {
      return a.contestId < b.contestId ? -1 : a.contestId > b.contestId ? 1 : 0;
    }
    return a.latestSolvedAt < b.latestSolvedAt ? 1 : -1;
  });
  return list;
}

function renderContestCard(config: UrlConfig, group: ContestGroup): string {
  const href = toInternalPath(config, ["solutions", group.contestId]);
  return (
    `<li><article class="contest-card">` +
      `<h3><a href="${escapeAttribute(href)}">${escapeHtml(group.contestId)}</a></h3>` +
      `<dl class="contest-counts">` +
        `<dt>Public problems</dt><dd>${group.problems.size}</dd>` +
        `<dt>Public solutions</dt><dd>${group.count}</dd>` +
        `<dt>Latest solved</dt><dd><time datetime="${escapeAttribute(group.latestSolvedAt)}">${escapeHtml(group.latestSolvedAt)}</time></dd>` +
      `</dl>` +
    `</article></li>`
  );
}

export function renderSolutionsRootMainInner(
  config: UrlConfig,
  siteData: SiteData,
): string {
  const groups = contestGroups(siteData);
  const contestsHtml =
    groups.length === 0
      ? `<p class="empty-state">No public solutions yet.</p>`
      : `<ul class="contest-list">${groups.map((g) => renderContestCard(config, g)).join("")}</ul>`;
  return (
    `<header class="page-header"><h1>Solutions</h1></header>` +
    `<section class="contests" aria-labelledby="contests-heading">` +
      `<h2 id="contests-heading">Contests</h2>` +
      contestsHtml +
    `</section>`
  );
}

export function renderSolutionsRootPage(
  config: UrlConfig,
  siteData: SiteData,
): string {
  return renderDocument({
    config,
    siteData,
    canonicalSegments: ["solutions"],
    documentTitle: formatDocumentTitle(siteData, "Solutions"),
    description: `Public solutions in ${siteData.site.title}.`,
    currentNav: "solutions",
    breadcrumb: [homeCrumb(config), { label: "Solutions" }],
    mainInnerHtml: renderSolutionsRootMainInner(config, siteData),
    mainIgnoreForPagefind: true,
  });
}

// ---- /solutions/{contest}/ ----

interface ProblemGroup {
  problemCode: string;
  latestSolvedAt: string;
  count: number;
}

function problemGroups(
  siteData: SiteData,
  contestId: string,
): ProblemGroup[] {
  const map = new Map<string, ProblemGroup>();
  for (const sol of siteData.solutions) {
    if (sol.contest_id !== contestId) continue;
    const g = map.get(sol.problem_code) ?? {
      problemCode: sol.problem_code,
      latestSolvedAt: sol.solved_at,
      count: 0,
    };
    if (sol.solved_at > g.latestSolvedAt) g.latestSolvedAt = sol.solved_at;
    g.count += 1;
    map.set(sol.problem_code, g);
  }
  const list = [...map.values()];
  list.sort((a, b) =>
    a.problemCode < b.problemCode ? -1 : a.problemCode > b.problemCode ? 1 : 0,
  );
  return list;
}

function renderProblemCard(
  config: UrlConfig,
  contestId: string,
  problem: ProblemGroup,
): string {
  const href = toInternalPath(config, ["solutions", contestId, problem.problemCode]);
  return (
    `<li><article class="problem-card">` +
      `<h3><a href="${escapeAttribute(href)}">${escapeHtml(problem.problemCode)}</a></h3>` +
      `<dl class="problem-counts">` +
        `<dt>Public solutions</dt><dd>${problem.count}</dd>` +
        `<dt>Latest solved</dt><dd><time datetime="${escapeAttribute(problem.latestSolvedAt)}">${escapeHtml(problem.latestSolvedAt)}</time></dd>` +
      `</dl>` +
    `</article></li>`
  );
}

export function renderContestMainInner(
  config: UrlConfig,
  siteData: SiteData,
  contestId: string,
): string {
  const problems = problemGroups(siteData, contestId);
  const html =
    problems.length === 0
      ? `<p class="empty-state">No public solutions in this contest.</p>`
      : `<ul class="problem-list">${problems.map((p) => renderProblemCard(config, contestId, p)).join("")}</ul>`;
  return (
    `<header class="page-header">` +
      `<h1>${escapeHtml(contestId)}</h1>` +
      `<p class="subtitle">Contest</p>` +
    `</header>` +
    `<section class="problems" aria-labelledby="problems-heading">` +
      `<h2 id="problems-heading">Problems</h2>` +
      html +
    `</section>`
  );
}

export function renderContestPage(
  config: UrlConfig,
  siteData: SiteData,
  contestId: string,
): string {
  return renderDocument({
    config,
    siteData,
    canonicalSegments: ["solutions", contestId],
    documentTitle: formatDocumentTitle(siteData, contestId),
    description: `Public solutions for contest ${contestId}.`,
    currentNav: "solutions",
    breadcrumb: [
      homeCrumb(config),
      solutionsCrumb(config),
      { label: contestId },
    ],
    mainInnerHtml: renderContestMainInner(config, siteData, contestId),
    mainIgnoreForPagefind: true,
  });
}

// ---- /solutions/{contest}/{problem}/ ----

function solutionsForProblem(
  siteData: SiteData,
  contestId: string,
  problemCode: string,
): SolutionPageData[] {
  const sols = siteData.solutions.filter(
    (s) => s.contest_id === contestId && s.problem_code === problemCode,
  );
  sols.sort((a, b) => {
    if (a.solved_at === b.solved_at) {
      return a.solution_id < b.solution_id ? -1 : a.solution_id > b.solution_id ? 1 : 0;
    }
    return a.solved_at < b.solved_at ? 1 : -1;
  });
  return sols;
}

function renderSolutionCard(
  config: UrlConfig,
  sol: SolutionPageData,
): string {
  const href = solutionPath(config, sol.contest_id, sol.problem_code, sol.solution_name);
  return (
    `<li><article class="solution-card">` +
      `<h3><a href="${escapeAttribute(href)}">${escapeHtml(sol.solution_name)}</a></h3>` +
      `<p class="solution-language">${escapeHtml(sol.language)}</p>` +
      `<p class="solution-solved"><time datetime="${escapeAttribute(sol.solved_at)}">${escapeHtml(sol.solved_at)}</time></p>` +
      renderStatus("solution-verification", sol.verification.status) +
      `<p class="solution-dep-count">Direct dependencies: ${sol.direct_dependencies.length}</p>` +
    `</article></li>`
  );
}

export function renderProblemMainInner(
  config: UrlConfig,
  siteData: SiteData,
  contestId: string,
  problemCode: string,
): string {
  const sols = solutionsForProblem(siteData, contestId, problemCode);
  const html =
    sols.length === 0
      ? `<p class="empty-state">No public solutions for this problem.</p>`
      : `<ul class="solution-list">${sols.map((s) => renderSolutionCard(config, s)).join("")}</ul>`;
  return (
    `<header class="page-header">` +
      `<h1>${escapeHtml(problemCode)}</h1>` +
      `<p class="subtitle">${escapeHtml(contestId)} / ${escapeHtml(problemCode)}</p>` +
    `</header>` +
    `<section class="solutions" aria-labelledby="solutions-heading">` +
      `<h2 id="solutions-heading">Solutions</h2>` +
      html +
    `</section>`
  );
}

export function renderProblemPage(
  config: UrlConfig,
  siteData: SiteData,
  contestId: string,
  problemCode: string,
): string {
  return renderDocument({
    config,
    siteData,
    canonicalSegments: ["solutions", contestId, problemCode],
    documentTitle: formatDocumentTitle(siteData, `${contestId} / ${problemCode}`),
    description: `Public solutions for ${contestId} / ${problemCode}.`,
    currentNav: "solutions",
    breadcrumb: [
      homeCrumb(config),
      solutionsCrumb(config),
      {
        label: contestId,
        href: toInternalPath(config, ["solutions", contestId]),
      },
      { label: problemCode },
    ],
    mainInnerHtml: renderProblemMainInner(
      config,
      siteData,
      contestId,
      problemCode,
    ),
    mainIgnoreForPagefind: true,
  });
}

// ---- /solutions/{contest}/{problem}/{name}/ (detail) ----

function renderLibraryLinkList(
  className: string,
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
      return (
        `<li>` +
          `<a href="${escapeAttribute(href)}">${escapeHtml(link.title)}</a> ` +
          `<span class="language">${escapeHtml(link.language)}</span> ` +
          `<code class="path">${escapeHtml(link.source_path)}</code>` +
        `</li>`
      );
    })
    .join("");
  return `<ul class="${escapeAttribute(className)}">${items}</ul>`;
}

function renderSolutionDetailArticleInner(
  config: UrlConfig,
  sol: SolutionPageData,
): string {
  const status = sol.verification.status;
  const contestLink = toInternalPath(config, ["solutions", sol.contest_id]);
  const problemLink = toInternalPath(config, [
    "solutions",
    sol.contest_id,
    sol.problem_code,
  ]);
  const inPageItems: { id: string; label: string }[] = [
    { id: "source", label: "Source" },
    { id: "libraries", label: "Libraries" },
  ];
  const showVerification = status !== "not_configured";
  if (showVerification) inPageItems.push({ id: "verification", label: "Verification" });
  inPageItems.push({ id: "diagnostics", label: "Diagnostics" });
  const inPageNav =
    `<nav class="in-page-navigation" aria-label="On this page" data-pagefind-ignore>` +
      `<ul>` +
        inPageItems
          .map(
            (item) =>
              `<li><a href="#${escapeAttribute(item.id)}">${escapeHtml(item.label)}</a></li>`,
          )
          .join("") +
      `</ul>` +
    `</nav>`;
  const preprocessNote = sol.has_preprocess
    ? `<p class="preprocess-note">Displayed source is the repository entry file; the actual submission is preprocessed.</p>`
    : "";
  const privateDepNote = sol.has_private_dependencies
    ? `<p class="private-dependencies-note">This solution also depends on private libraries.</p>`
    : "";
  const verificationSection = renderSolutionVerificationSection(sol);
  const diagnostics =
    sol.diagnostics.length === 0
      ? `<p class="empty-state">No diagnostics.</p>`
      : `<ul class="diagnostics-list">${sol.diagnostics
          .map((d) => {
            const noticeOrLocation =
              d.location_notice !== null && d.location_notice !== undefined
                ? ` <p class="location-notice">${escapeHtml(d.location_notice)}</p>`
                : "";
            return (
              `<li>` +
                `<span class="severity">${escapeHtml(d.severity)}</span> ` +
                `<code class="diagnostic-code">${escapeHtml(d.code)}</code> ` +
                `<p class="message">${escapeHtml(d.message)}</p>` +
                noticeOrLocation +
              `</li>`
            );
          })
          .join("")}</ul>`;
  return (
    `<header class="page-header">` +
      `<h1>${escapeHtml(sol.solution_name)}</h1>` +
      `<p class="solution-meta">` +
        `<a href="${escapeAttribute(contestLink)}">${escapeHtml(sol.contest_id)}</a> / ` +
        `<a href="${escapeAttribute(problemLink)}">${escapeHtml(sol.problem_code)}</a> ` +
        `<span class="oj">${escapeHtml(sol.online_judge)}</span>` +
      `</p>` +
      `<p class="solution-language-time">` +
        `<span class="language">${escapeHtml(sol.language)}</span> ` +
        `<time datetime="${escapeAttribute(sol.solved_at)}">${escapeHtml(sol.solved_at)}</time>` +
      `</p>` +
      renderStatus("solution-verification", status) +
    `</header>` +
    inPageNav +
    `<section id="source" aria-labelledby="source-heading">` +
      `<h2 id="source-heading">Source</h2>` +
      preprocessNote +
      `<p class="pending">Source rendering pending (Task 3).</p>` +
    `</section>` +
    `<section id="libraries" aria-labelledby="libraries-heading">` +
      `<h2 id="libraries-heading">Libraries</h2>` +
      `<h3>Verifies</h3>` +
      renderLibraryLinkList(
        "verifies-list",
        sol.verifies,
        config,
        "This solution does not verify any library.",
      ) +
      `<h3>Depends on</h3>` +
      renderLibraryLinkList(
        "depends-on-list",
        sol.direct_dependencies,
        config,
        "No direct library dependencies.",
      ) +
      privateDepNote +
    `</section>` +
    verificationSection +
    `<section id="diagnostics" aria-labelledby="diagnostics-heading">` +
      `<h2 id="diagnostics-heading">Diagnostics</h2>` +
      diagnostics +
    `</section>`
  );
}

function renderSolutionVerificationSection(sol: SolutionPageData): string {
  const status = sol.verification.status;
  if (status === "not_configured") {
    // Spec §12.7: not_configured => omit verification section.
    return "";
  }
  if (status === "never") {
    return (
      `<section id="verification" aria-labelledby="verification-heading">` +
        `<h2 id="verification-heading">Verification</h2>` +
        `<p class="empty-state">This solution has never been submitted for verification.</p>` +
      `</section>`
    );
  }
  const result = sol.verification.result;
  if (result === null || result === undefined) {
    return (
      `<section id="verification" aria-labelledby="verification-heading">` +
        `<h2 id="verification-heading">Verification</h2>` +
        `<p class="empty-state">No verification result recorded yet.</p>` +
      `</section>`
    );
  }
  const verdict = result.verdict ?? "unknown";
  const judged =
    result.judged_at !== null && result.judged_at !== undefined
      ? `<dt>Judged</dt><dd><time datetime="${escapeAttribute(result.judged_at)}">${escapeHtml(result.judged_at)}</time></dd>`
      : "";
  const time =
    result.execution_time_ms !== null && result.execution_time_ms !== undefined
      ? `<dt>Execution</dt><dd>${result.execution_time_ms} ms</dd>`
      : "";
  const mem =
    result.memory_kib !== null && result.memory_kib !== undefined
      ? `<dt>Memory</dt><dd>${result.memory_kib} KiB</dd>`
      : "";
  const oj =
    result.oj_submission_url !== null && result.oj_submission_url !== undefined
      ? `<dt>OJ submission</dt><dd><a href="${escapeAttribute(result.oj_submission_url)}" rel="noopener noreferrer">Open</a></dd>`
      : "";
  const stale =
    result.stale_reason !== null && result.stale_reason !== undefined
      ? `<p class="stale-reason">${escapeHtml(result.stale_reason)}</p>`
      : "";
  const testcaseTable =
    result.testcases.length === 0
      ? ""
      : `<table class="testcases">` +
          `<caption>Testcase verdicts</caption>` +
          `<thead><tr><th scope="col">Name</th><th scope="col">Verdict</th><th scope="col">Time (ms)</th><th scope="col">Memory (KiB)</th></tr></thead>` +
          `<tbody>` +
            result.testcases
              .map(
                (t) =>
                  `<tr>` +
                    `<td><code>${escapeHtml(t.name)}</code></td>` +
                    `<td>${escapeHtml(t.verdict)}</td>` +
                    `<td>${t.execution_time_ms ?? ""}</td>` +
                    `<td>${t.memory_kib ?? ""}</td>` +
                  `</tr>`,
              )
              .join("") +
          `</tbody>` +
        `</table>`;
  return (
    `<section id="verification" aria-labelledby="verification-heading">` +
      `<h2 id="verification-heading">Verification</h2>` +
      `<dl class="verification-summary">` +
        `<dt>Verdict</dt><dd>${escapeHtml(verdict)}</dd>` +
        judged +
        time +
        mem +
        oj +
      `</dl>` +
      stale +
      testcaseTable +
    `</section>`
  );
}

export function renderSolutionDetailMainInner(
  config: UrlConfig,
  sol: SolutionPageData,
): string {
  const pageIdAttr = escapeAttribute(`page-${sol.page_id}`);
  return (
    `<article class="solution-detail" id="${pageIdAttr}" data-pagefind-body>` +
      renderSolutionDetailArticleInner(config, sol) +
    `</article>`
  );
}

export function renderSolutionDetailPage(
  config: UrlConfig,
  siteData: SiteData,
  sol: SolutionPageData,
): string {
  const canonicalSegments = [
    "solutions",
    sol.contest_id,
    sol.problem_code,
    sol.solution_name,
  ];
  const description = `${sol.solution_name} — ${sol.contest_id} / ${sol.problem_code} (${sol.language}).`;
  return renderDocument({
    config,
    siteData,
    canonicalSegments,
    documentTitle: formatDocumentTitle(
      siteData,
      `${sol.contest_id} / ${sol.problem_code} / ${sol.solution_name}`,
    ),
    description,
    currentNav: "solutions",
    breadcrumb: [
      homeCrumb(config),
      solutionsCrumb(config),
      {
        label: sol.contest_id,
        href: toInternalPath(config, ["solutions", sol.contest_id]),
      },
      {
        label: sol.problem_code,
        href: toInternalPath(config, [
          "solutions",
          sol.contest_id,
          sol.problem_code,
        ]),
      },
      { label: sol.solution_name },
    ],
    mainInnerHtml: renderSolutionDetailMainInner(config, sol),
    mainIgnoreForPagefind: false,
    robots: "index,follow",
  });
}

// Keep import used across future edits.
void solutionsRootPath;
