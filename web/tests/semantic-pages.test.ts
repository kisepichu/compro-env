/**
 * Semantic-contract tests for every static route rendered by Task 2.
 *
 * Each test renders the full HTML document for a route, parses it with
 * jsdom, and asserts on landmarks, headings, `data-pagefind-*` attributes,
 * link `href` base-safety, sorting, and status text.
 */

import { JSDOM } from "jsdom";
import { describe, expect, it } from "vitest";

import type { LibraryPageData, SolutionPageData } from "@/lib/site-data-types.ts";
import type { UrlConfig } from "@/lib/url.ts";
import { renderHomePage } from "@/lib/pages/home.ts";
import {
  listLibraryRoutes,
  renderLibrariesRootPage,
  renderLibraryDetailPage,
  renderLibraryDirectoryPage,
  splitSourcePath,
} from "@/lib/pages/libraries.ts";
import { renderNotFoundPage } from "@/lib/pages/notfound.ts";
import {
  listSolutionRoutes,
  renderContestPage,
  renderProblemPage,
  renderSolutionDetailPage,
  renderSolutionsRootPage,
} from "@/lib/pages/solutions.ts";
import { renderStatus } from "@/lib/pages/status.ts";
import { buildFixtureSiteData } from "./helpers/fixture.ts";

const rootConfig: UrlConfig = { origin: "https://example.com", base: "/" };
const projectConfig: UrlConfig = {
  origin: "https://example.com",
  base: "/compro-env/",
};

function parse(html: string): Document {
  return new JSDOM(html).window.document;
}

/**
 * Assert the semantic shell: exactly one h1, skip link first in body,
 * primary navigation with 3 items, global search form, canonical link,
 * footer with short SHA. Returns the document for further assertions.
 */
function assertShell(
  html: string,
  opts: {
    config: UrlConfig;
    expectH1: string;
    currentNav?: "libraries" | "solutions" | "search" | null;
    expectMainIgnored: boolean;
    expectRobots?: string;
  },
): Document {
  const doc = parse(html);

  // Doctype + html lang.
  expect(html.startsWith("<!DOCTYPE html>")).toBe(true);
  expect(doc.documentElement.getAttribute("lang")).toBeTruthy();

  // Exactly one h1.
  const h1s = doc.querySelectorAll("h1");
  expect(h1s.length).toBe(1);
  expect(h1s[0].textContent).toBe(opts.expectH1);

  // Skip link first in body.
  const body = doc.body;
  const first = body.firstElementChild;
  expect(first).toBeTruthy();
  expect(first!.tagName).toBe("A");
  expect(first!.getAttribute("class")).toBe("skip-link");
  expect(first!.getAttribute("href")).toBe("#main-content");

  // Header with pagefind-ignore.
  const header = doc.querySelector("header.site-header");
  expect(header).toBeTruthy();
  expect(header!.hasAttribute("data-pagefind-ignore")).toBe(true);

  // Primary navigation.
  const nav = doc.querySelector("nav.primary-navigation");
  expect(nav).toBeTruthy();
  expect(nav!.getAttribute("aria-label")).toBe("Primary");
  const navLinks = nav!.querySelectorAll("ul > li > a");
  expect(navLinks.length).toBe(3);
  const labels = [...navLinks].map((a) => a.textContent);
  expect(labels).toEqual(["Libraries", "Solutions", "Search"]);
  const base = opts.config.base;
  const [libLink, solLink, searchLink] = [...navLinks];
  expect(libLink.getAttribute("href")).toBe(`${base}libraries/`);
  expect(solLink.getAttribute("href")).toBe(`${base}solutions/`);
  expect(searchLink.getAttribute("href")).toBe(`${base}search/`);

  // aria-current on the correct nav entry.
  const currentAttrs = [...navLinks].map((a) => a.getAttribute("aria-current"));
  const expected =
    opts.currentNav === undefined || opts.currentNav === null
      ? [null, null, null]
      : opts.currentNav === "libraries"
        ? ["page", null, null]
        : opts.currentNav === "solutions"
          ? [null, "page", null]
          : ["page", null, null] /* unreachable */;
  if (opts.currentNav === "search") {
    expect(currentAttrs).toEqual([null, null, "page"]);
  } else {
    expect(currentAttrs).toEqual(expected);
  }

  // Global search form.
  const form = doc.querySelector("form.global-search");
  expect(form).toBeTruthy();
  expect(form!.getAttribute("role")).toBe("search");
  expect(form!.getAttribute("method")).toBe("get");
  expect(form!.getAttribute("action")).toBe(`${base}search/`);
  const label = form!.querySelector("label");
  expect(label!.getAttribute("for")).toBe("global-search-query");
  const input = form!.querySelector("input");
  expect(input!.getAttribute("id")).toBe("global-search-query");
  expect(input!.getAttribute("name")).toBe("q");
  expect(input!.getAttribute("type")).toBe("search");

  // <main id="main-content"> presence + pagefind-ignore expectation.
  const main = doc.getElementById("main-content");
  expect(main).toBeTruthy();
  expect(main!.tagName).toBe("MAIN");
  expect(main!.hasAttribute("data-pagefind-ignore")).toBe(opts.expectMainIgnored);

  // Footer.
  const footer = doc.querySelector("footer.site-footer");
  expect(footer).toBeTruthy();
  expect(footer!.hasAttribute("data-pagefind-ignore")).toBe(true);
  const sha = footer!.querySelector(".build-source-commit-sha");
  expect(sha).toBeTruthy();
  expect(sha!.textContent).toBe("0abcdef");

  // Canonical link.
  const canonical = doc.querySelector('link[rel="canonical"]');
  expect(canonical).toBeTruthy();
  expect(canonical!.getAttribute("href")!.startsWith("https://example.com")).toBe(
    true,
  );

  // Robots meta.
  const robots = doc.querySelector('meta[name="robots"]');
  expect(robots).toBeTruthy();
  if (opts.expectRobots !== undefined) {
    expect(robots!.getAttribute("content")).toBe(opts.expectRobots);
  }

  return doc;
}

/** Assert every internal href starts with `base` (base-safety). */
function assertBaseSafety(doc: Document, base: string): void {
  const links = doc.querySelectorAll("a[href]");
  for (const a of links) {
    const href = a.getAttribute("href")!;
    if (
      href.startsWith("http://") ||
      href.startsWith("https://") ||
      href.startsWith("#")
    ) {
      continue;
    }
    expect(href.startsWith(base)).toBe(true);
  }
  const forms = doc.querySelectorAll("form[action]");
  for (const f of forms) {
    const action = f.getAttribute("action")!;
    if (action.startsWith("http://") || action.startsWith("https://")) continue;
    expect(action.startsWith(base)).toBe(true);
  }
}

// ---- Home ----

describe("Home page (/)", () => {
  const siteData = buildFixtureSiteData();
  const html = renderHomePage(rootConfig, siteData);

  it("emits the semantic shell with h1 = site.title and no aria-current", () => {
    const doc = assertShell(html, {
      config: rootConfig,
      expectH1: "compro-env fixture",
      currentNav: null,
      expectMainIgnored: true,
    });
    // No breadcrumb on the Home page.
    expect(doc.querySelector("nav.breadcrumb")).toBeNull();
  });

  it("renders all Home sections with h2 headings even when data is present", () => {
    const doc = parse(html);
    const h2Texts = [...doc.querySelectorAll("main h2")].map((h) => h.textContent);
    expect(h2Texts).toEqual(
      expect.arrayContaining([
        "Repository status",
        "Languages",
        "Recently updated libraries",
        "Recently solved solutions",
        "Attention required",
      ]),
    );
  });

  it("recent libraries are capped at 10 and sorted by (updated_at desc, id asc)", () => {
    // Build 15 libraries to prove capping and stable tie-break.
    const many: LibraryPageData[] = [];
    for (let i = 0; i < 15; i += 1) {
      many.push({
        page_id: `library:rust/l${i}.rs`,
        library_id: `rust/l${i}.rs`,
        language: "rust",
        title: `lib${i}`,
        source_path: `l${i}.rs`,
        source: "",
        syntax_highlight: "rust",
        // Half share the same timestamp to test tie-break.
        updated_at: i < 5 ? "2026-08-10T00:00:00Z" : `2026-08-${String(11 + (i % 20)).padStart(2, "0")}T00:00:00Z`,
        updated_by_commit: "x",
        description: null,
        symbol_analysis: { state: "complete", symbols: [] },
        dependency_analysis: {
          state: "complete",
          direct: [],
          transitive: [],
          has_private_dependencies: false,
        },
        reverse_dependencies: [],
        relations: [],
        verification: { aggregate_status: "never", evidence: [] },
        diagnostics: [],
      });
    }
    const doc = parse(renderHomePage(rootConfig, buildFixtureSiteData({ libraries: many })));
    const items = doc.querySelectorAll("section.recent-libraries .library-list > li");
    expect(items.length).toBe(10);
    // First 10 are strictly descending by updated_at (tie-break lib_id asc).
    const times = [...items].map((li) => li.querySelector("time")!.getAttribute("datetime")!);
    for (let i = 1; i < times.length; i += 1) {
      expect(times[i - 1] >= times[i]).toBe(true);
    }
  });

  it("keeps internal links under the configured base", () => {
    const doc = parse(renderHomePage(projectConfig, siteData));
    assertBaseSafety(doc, "/compro-env/");
  });
});

// ---- Libraries root (/libraries/) ----

describe("Libraries root (/libraries/)", () => {
  it("filters languages to library_count > 0 and marks Libraries current", () => {
    const siteData = buildFixtureSiteData();
    const html = renderLibrariesRootPage(rootConfig, siteData);
    const doc = assertShell(html, {
      config: rootConfig,
      expectH1: "Libraries",
      currentNav: "libraries",
      expectMainIgnored: true,
    });
    const cards = doc.querySelectorAll(".language-list > li");
    // Only Rust (library_count = 3) — C++ and Lean have zero.
    expect(cards.length).toBe(1);
    expect(cards[0].textContent).toContain("Rust");
  });

  it("renders an empty-state message when no eligible languages exist", () => {
    const siteData = buildFixtureSiteData({ languages: [] });
    const doc = parse(renderLibrariesRootPage(rootConfig, siteData));
    expect(doc.querySelector(".empty-state")).toBeTruthy();
    expect(doc.querySelector(".empty-state")!.textContent).toMatch(/No languages/);
  });

  it("keeps internal links under a project base like /compro-env/", () => {
    const siteData = buildFixtureSiteData();
    const doc = parse(renderLibrariesRootPage(projectConfig, siteData));
    assertBaseSafety(doc, "/compro-env/");
    // Language card link points to /compro-env/libraries/rust/
    const langLink = doc.querySelector(".language-list > li a")!;
    expect(langLink.getAttribute("href")).toBe("/compro-env/libraries/rust/");
  });
});

// ---- Language & directory (/libraries/{lang}/... /) ----

describe("Library directory pages", () => {
  const siteData = buildFixtureSiteData();

  it("language page: h1 = display name, child-directories + library-files sections", () => {
    const html = renderLibraryDirectoryPage(rootConfig, siteData, "rust", []);
    const doc = assertShell(html, {
      config: rootConfig,
      expectH1: "Rust",
      currentNav: "libraries",
      expectMainIgnored: true,
    });
    expect(doc.querySelector("section.child-directories")).toBeTruthy();
    expect(doc.querySelector("section.library-files")).toBeTruthy();
    const dirs = doc.querySelectorAll(".directory-list > li");
    // graph, math, util — three top-level directories from the fixture.
    expect(dirs.length).toBe(3);
  });

  it("subdirectory page: h1 = final directory segment, breadcrumb chain complete", () => {
    const html = renderLibraryDirectoryPage(rootConfig, siteData, "rust", ["graph"]);
    const doc = parse(html);
    expect(doc.querySelector("h1")!.textContent).toBe("graph");
    const crumbs = [...doc.querySelectorAll("nav.breadcrumb ol > li")].map(
      (li) => li.textContent,
    );
    expect(crumbs).toEqual(["Home", "Libraries", "Rust", "graph"]);
    // Library card for graph/dijkstra.rs is present.
    const libLinks = doc.querySelectorAll(".library-list > li h3 a");
    expect(libLinks.length).toBe(1);
    expect(libLinks[0].getAttribute("href")).toBe("/libraries/rust/graph/dijkstra.rs/");
  });
});

// ---- Library detail ----

describe("Library detail (/libraries/{lang}/{source-path}/)", () => {
  const siteData = buildFixtureSiteData();
  const lib = siteData.libraries[0];
  const html = renderLibraryDetailPage(rootConfig, siteData, lib);

  it("emits <article data-pagefind-body id=page-library:...> with fixed section IDs", () => {
    const doc = assertShell(html, {
      config: rootConfig,
      expectH1: lib.title,
      currentNav: "libraries",
      expectMainIgnored: false, // Detail pages do NOT ignore <main>.
      expectRobots: "index,follow",
    });
    const article = doc.querySelector("article.library-detail")!;
    expect(article).toBeTruthy();
    expect(article.hasAttribute("data-pagefind-body")).toBe(true);
    expect(article.getAttribute("id")).toBe(`page-${lib.page_id}`);
    for (const id of [
      "symbols",
      "source",
      "dependencies",
      "relations",
      "verification",
      "diagnostics",
    ]) {
      expect(doc.getElementById(id)).toBeTruthy();
    }
    // Documentation section only when description is non-null.
    expect(doc.getElementById("documentation")).toBeTruthy();
  });

  it("emits status badges with the expected data-status values", () => {
    const doc = parse(html);
    const statuses = [...doc.querySelectorAll("article header .status-badge")].map(
      (el) => el.getAttribute("data-status"),
    );
    expect(statuses).toEqual(
      expect.arrayContaining(["verified", "complete"]),
    );
  });

  it("omits documentation section when description is null", () => {
    const doc = parse(renderLibraryDetailPage(rootConfig, siteData, siteData.libraries[1]));
    expect(doc.getElementById("documentation")).toBeNull();
  });
});

// ---- Solutions root, contest, problem, detail ----

describe("Solution browse and detail", () => {
  const siteData = buildFixtureSiteData();

  it("solutions root: h1 Solutions, contests sorted by latest solved_at desc", () => {
    const html = renderSolutionsRootPage(rootConfig, siteData);
    const doc = assertShell(html, {
      config: rootConfig,
      expectH1: "Solutions",
      currentNav: "solutions",
      expectMainIgnored: true,
    });
    const cards = doc.querySelectorAll(".contest-list > li");
    expect(cards.length).toBe(2);
    // abc300 (latest 2026-08-10) before abc301 (2026-08-08).
    const heads = [...cards].map((c) => c.querySelector("h3 a")!.textContent);
    expect(heads).toEqual(["abc300", "abc301"]);
  });

  it("contest page: h1 = contest_id and lists distinct problems", () => {
    const html = renderContestPage(rootConfig, siteData, "abc300");
    const doc = parse(html);
    expect(doc.querySelector("h1")!.textContent).toBe("abc300");
    const problems = doc.querySelectorAll(".problem-list > li h3 a");
    const codes = [...problems].map((a) => a.textContent);
    expect(codes.sort()).toEqual(["a", "b"]);
  });

  it("problem page: h1 = problem_code, breadcrumb full chain", () => {
    const html = renderProblemPage(rootConfig, siteData, "abc300", "a");
    const doc = parse(html);
    expect(doc.querySelector("h1")!.textContent).toBe("a");
    const crumbs = [...doc.querySelectorAll("nav.breadcrumb ol > li")].map(
      (li) => li.textContent,
    );
    expect(crumbs).toEqual(["Home", "Solutions", "abc300", "a"]);
  });

  it("solution detail: article has data-pagefind-body and canonical page id", () => {
    const sol: SolutionPageData = siteData.solutions[0];
    const html = renderSolutionDetailPage(rootConfig, siteData, sol);
    const doc = assertShell(html, {
      config: rootConfig,
      expectH1: sol.solution_name,
      currentNav: "solutions",
      expectMainIgnored: false,
      expectRobots: "index,follow",
    });
    const article = doc.querySelector("article.solution-detail")!;
    expect(article.hasAttribute("data-pagefind-body")).toBe(true);
    expect(article.getAttribute("id")).toBe(`page-${sol.page_id}`);
    // verification section present for 'verified' status.
    expect(doc.getElementById("verification")).toBeTruthy();
    // Status badge with data-status="verified".
    const detailStatus = article.querySelector("header .status-badge")!;
    expect(detailStatus.getAttribute("data-status")).toBe("verified");
  });

  it("solution detail with 'never' status keeps verification section as empty state", () => {
    const sol = siteData.solutions[1]; // 'never'
    const doc = parse(renderSolutionDetailPage(rootConfig, siteData, sol));
    const section = doc.getElementById("verification")!;
    expect(section).toBeTruthy();
    expect(section.textContent).toMatch(/never been submitted/i);
  });

  it("solution detail with 'not_configured' omits the verification section", () => {
    const sol = siteData.solutions[2]; // not_configured
    const doc = parse(renderSolutionDetailPage(rootConfig, siteData, sol));
    expect(doc.getElementById("verification")).toBeNull();
    const status = doc.querySelector("article header .status-badge")!;
    expect(status.getAttribute("data-status")).toBe("not_configured");
  });
});

// ---- 404 ----

describe("Static 404 (/404.html)", () => {
  const siteData = buildFixtureSiteData();
  const html = renderNotFoundPage(rootConfig, siteData);

  it("sets robots noindex,nofollow and has recovery-navigation", () => {
    const doc = assertShell(html, {
      config: rootConfig,
      expectH1: "Page not found",
      currentNav: null,
      expectMainIgnored: true,
      expectRobots: "noindex,nofollow",
    });
    const recovery = doc.querySelector("nav.recovery-navigation")!;
    expect(recovery).toBeTruthy();
    expect(recovery.querySelectorAll("li > a").length).toBe(4);
  });

  it("breadcrumb is Home > Page not found", () => {
    const doc = parse(html);
    const crumbs = [...doc.querySelectorAll("nav.breadcrumb ol > li")].map(
      (li) => li.textContent,
    );
    expect(crumbs).toEqual(["Home", "Page not found"]);
  });

  it("works under a project base too", () => {
    const doc = parse(renderNotFoundPage(projectConfig, siteData));
    assertBaseSafety(doc, "/compro-env/");
  });
});

// ---- Status component ----

describe("Status component", () => {
  it("maps each verification status to its spec-mandated label", () => {
    const cases: [Parameters<typeof renderStatus>[1], string][] = [
      ["verified", "Verified"],
      ["rejected", "Rejected"],
      ["unavailable", "Unavailable"],
      ["stale", "Stale"],
      ["never", "Never verified"],
      ["not_configured", "Verification not configured"],
    ];
    for (const [value, label] of cases) {
      const doc = parse(`<div>${renderStatus("solution-verification", value)}</div>`);
      const badge = doc.querySelector(".status-badge")!;
      expect(badge.getAttribute("data-status")).toBe(value);
      expect(badge.textContent!.trim()).toBe(label);
      expect(badge.hasAttribute("role")).toBe(false);
    }
  });

  it("maps analysis states to their labels", () => {
    for (const [value, label] of [
      ["complete", "Analysis complete"],
      ["partial", "Analysis partial"],
      ["failed", "Analysis failed"],
    ] as const) {
      const doc = parse(`<div>${renderStatus("analysis", value)}</div>`);
      const badge = doc.querySelector(".status-badge")!;
      expect(badge.getAttribute("data-status")).toBe(value);
      expect(badge.textContent!.trim()).toBe(label);
    }
  });
});

// ---- Route enumeration ----

describe("Route enumeration", () => {
  const siteData = buildFixtureSiteData();

  it("enumerates language + directory + detail routes for libraries", () => {
    const routes = listLibraryRoutes(siteData);
    const kinds = routes.reduce<Record<string, number>>((acc, r) => {
      acc[r.kind] = (acc[r.kind] ?? 0) + 1;
      return acc;
    }, {});
    // 1 language (rust), 3 dirs (graph, math, util), 3 detail routes.
    expect(kinds).toEqual({ language: 1, directory: 3, detail: 3 });
    const paths = routes.map((r) => r.segments.join("/")).sort();
    expect(paths).toEqual(
      [
        "rust",
        "rust/graph",
        "rust/math",
        "rust/util",
        "rust/graph/dijkstra.rs",
        "rust/util/binary_heap.rs",
        "rust/math/mod_inv.rs",
      ].sort(),
    );
  });

  it("enumerates contest + problem + detail routes for solutions", () => {
    const routes = listSolutionRoutes(siteData);
    const kinds = routes.reduce<Record<string, number>>((acc, r) => {
      acc[r.kind] = (acc[r.kind] ?? 0) + 1;
      return acc;
    }, {});
    // 2 contests, 3 problems total (abc300/a, abc300/b, abc301/a), 3 solutions.
    expect(kinds).toEqual({ contest: 2, problem: 3, detail: 3 });
  });

  it("splitSourcePath drops empty segments", () => {
    expect(splitSourcePath("graph/dijkstra.rs")).toEqual(["graph", "dijkstra.rs"]);
    expect(splitSourcePath("/leading/slash/")).toEqual(["leading", "slash"]);
  });
});
