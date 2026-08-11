/**
 * Task 2 tests for `buildExactIndex` and the detail-page Pagefind
 * annotations. Verifies page IDs, aliases, fragments, path prefixes,
 * filters, duplicate names, symbol-punctuation, private exclusion, and
 * anchor validity against rendered HTML.
 */

import { JSDOM } from "jsdom";
import { describe, expect, it } from "vitest";

import {
  buildExactIndex,
  pathFilterValues,
  type ExactIndexPage,
} from "@/search/exact-index.ts";
import { renderLibraryDetailMainInner } from "@/lib/pages/libraries.ts";
import { renderSolutionDetailMainInner } from "@/lib/pages/solutions.ts";
import type {
  LibraryPageData,
  SiteData,
  SolutionPageData,
  SymbolPublic,
} from "@/lib/site-data-types.ts";
import type { UrlConfig } from "@/lib/url.ts";
import { buildFixtureSiteData } from "./helpers/fixture.ts";

const rootConfig: UrlConfig = { origin: "https://example.com", base: "/" };
const projectConfig: UrlConfig = {
  origin: "https://example.com",
  base: "/compro-env/",
};

function pageBy(index: { pages: ExactIndexPage[] }, id: string): ExactIndexPage {
  const p = index.pages.find((e) => e.page_id === id);
  if (!p) throw new Error(`Expected page ${id} in index`);
  return p;
}

// ---- Basic page set ----

describe("buildExactIndex — page set", () => {
  it("emits one entry per public library and solution", () => {
    const siteData = buildFixtureSiteData();
    const index = buildExactIndex(siteData, rootConfig);
    expect(index.schema_version).toBe(1);
    const expected = new Set<string>([
      ...siteData.libraries.map((l) => l.page_id),
      ...siteData.solutions.map((s) => s.page_id),
    ]);
    const got = new Set(index.pages.map((p) => p.page_id));
    expect(got).toEqual(expected);
  });

  it("preserves every source DTO page_id verbatim", () => {
    const siteData = buildFixtureSiteData();
    const index = buildExactIndex(siteData, rootConfig);
    for (const lib of siteData.libraries) {
      const entry = pageBy(index, lib.page_id);
      expect(entry.type).toBe("library");
      expect(entry.page_id).toBe(lib.page_id);
    }
    for (const sol of siteData.solutions) {
      const entry = pageBy(index, sol.page_id);
      expect(entry.type).toBe("solution");
      expect(entry.page_id).toBe(sol.page_id);
    }
  });

  it("URLs respect the configured base", () => {
    const siteData = buildFixtureSiteData();
    const rooted = buildExactIndex(siteData, rootConfig);
    const project = buildExactIndex(siteData, projectConfig);
    const libRooted = pageBy(rooted, "library:rust/graph/dijkstra.rs");
    const libProject = pageBy(project, "library:rust/graph/dijkstra.rs");
    expect(libRooted.url).toBe("/libraries/rust/graph/dijkstra.rs/");
    expect(libProject.url).toBe("/compro-env/libraries/rust/graph/dijkstra.rs/");
    const solRooted = pageBy(rooted, "solution:abc300/a/dijkstra_solve");
    expect(solRooted.url).toBe("/solutions/abc300/a/dijkstra_solve/");
  });
});

// ---- Aliases ----

describe("buildExactIndex — aliases", () => {
  it("libraries include title, basename with extension, and basename without last extension", () => {
    const siteData = buildFixtureSiteData();
    const index = buildExactIndex(siteData, rootConfig);
    const lib = pageBy(index, "library:rust/graph/dijkstra.rs");
    expect(lib.aliases).toEqual(["Dijkstra", "dijkstra.rs", "dijkstra"]);
  });

  it("dedupes aliases when the title equals the basename", () => {
    const siteData = buildFixtureSiteData();
    const patched = siteData.libraries[0];
    patched.title = "dijkstra.rs";
    const index = buildExactIndex(siteData, rootConfig);
    const lib = pageBy(index, "library:rust/graph/dijkstra.rs");
    expect(lib.aliases).toEqual(["dijkstra.rs", "dijkstra"]);
  });

  it("only the last extension is stripped: `foo.test.cpp` → `foo.test`, never `foo`", () => {
    const siteData = buildFixtureSiteData();
    const extra: LibraryPageData = {
      page_id: "library:cpp/test/foo.test.cpp",
      library_id: "cpp/test/foo.test.cpp",
      language: "cpp",
      title: "Foo test",
      source_path: "test/foo.test.cpp",
      source: "// noop\n",
      syntax_highlight: "cpp",
      updated_at: "2026-08-11T00:00:00Z",
      updated_by_commit: "0abcdef",
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
    };
    siteData.libraries.push(extra);
    const index = buildExactIndex(siteData, rootConfig);
    const lib = pageBy(index, "library:cpp/test/foo.test.cpp");
    expect(lib.aliases).toEqual(["Foo test", "foo.test.cpp", "foo.test"]);
  });

  it("solutions use solution_name as both title and basename (deduped)", () => {
    const siteData = buildFixtureSiteData();
    const index = buildExactIndex(siteData, rootConfig);
    const sol = pageBy(index, "solution:abc300/a/dijkstra_solve");
    expect(sol.aliases).toEqual(["dijkstra_solve"]);
  });
});

// ---- Symbol fragments ----

describe("buildExactIndex — symbol fragments", () => {
  it("locationless symbols use the `symbols` fragment", () => {
    const siteData = buildFixtureSiteData();
    const index = buildExactIndex(siteData, rootConfig);
    const lib = pageBy(index, "library:rust/graph/dijkstra.rs");
    expect(lib.symbols.length).toBe(1);
    expect(lib.symbols[0].fragment).toBe("symbols");
    expect(lib.symbols[0].name).toBe("dijkstra");
    expect(lib.symbols[0].qualified_name).toBe("graph::dijkstra");
  });

  it("located symbols use `L{line}` for the fragment", () => {
    const siteData = buildFixtureSiteData();
    const lib = siteData.libraries[0];
    const symWithLoc: SymbolPublic = {
      kind: "function",
      name: "run",
      search_names: ["run"],
      location: { start: { line: 42 }, end: { line: 44 } },
    };
    lib.symbol_analysis = {
      state: "complete",
      symbols: [...lib.symbol_analysis.symbols, symWithLoc],
    };
    const index = buildExactIndex(siteData, rootConfig);
    const entry = pageBy(index, lib.page_id);
    const runSym = entry.symbols.find((s) => s.name === "run")!;
    expect(runSym.fragment).toBe("L42");
  });

  it("keeps punctuation-only names verbatim (no tokenization)", () => {
    const siteData = buildFixtureSiteData();
    const lib = siteData.libraries[0];
    lib.symbol_analysis = {
      state: "complete",
      symbols: [
        { kind: "operator", name: "+", search_names: ["+", "add"] },
        { kind: "namespace", name: "::", search_names: ["::"] },
      ],
    };
    const index = buildExactIndex(siteData, rootConfig);
    const entry = pageBy(index, lib.page_id);
    const names = entry.symbols.map((s) => s.name);
    expect(names).toEqual(["+", "::"]);
    expect(entry.symbols[0].search_names).toContain("+");
    expect(entry.symbols[1].search_names).toContain("::");
  });
});

// ---- Path filter prefixes ----

describe("buildExactIndex — path filter values", () => {
  it("libraries: emits segments and cumulative prefixes lowercased", () => {
    const siteData = buildFixtureSiteData();
    const index = buildExactIndex(siteData, rootConfig);
    const lib = pageBy(index, "library:rust/graph/dijkstra.rs");
    expect(lib.filters.path).toEqual([
      "graph",
      "dijkstra.rs",
      "graph/dijkstra.rs",
    ]);
  });

  it("solutions: emits segments and cumulative prefixes from solution_id", () => {
    const siteData = buildFixtureSiteData();
    const index = buildExactIndex(siteData, rootConfig);
    const sol = pageBy(index, "solution:abc300/a/dijkstra_solve");
    expect(sol.filters.path).toEqual([
      "abc300",
      "a",
      "dijkstra_solve",
      "abc300/a",
      "abc300/a/dijkstra_solve",
    ]);
  });

  it("pathFilterValues lowercases uppercase segments", () => {
    expect(pathFilterValues(["Graph", "Dijkstra.RS"])).toEqual([
      "graph",
      "dijkstra.rs",
      "graph/dijkstra.rs",
    ]);
  });
});

// ---- Filter values ----

describe("buildExactIndex — filters", () => {
  it("verified library sets verified=true and status=verified", () => {
    const siteData = buildFixtureSiteData();
    const index = buildExactIndex(siteData, rootConfig);
    const lib = pageBy(index, "library:rust/graph/dijkstra.rs");
    expect(lib.filters.verified).toBe("true");
    expect(lib.filters.status).toBe("verified");
    expect(lib.filters.lang).toBe("rust");
    expect(lib.filters.type).toBe("library");
    expect(lib.filters.kind).toEqual(["function"]);
  });

  it("stale library sets verified=false", () => {
    const siteData = buildFixtureSiteData();
    const index = buildExactIndex(siteData, rootConfig);
    const lib = pageBy(index, "library:rust/util/binary_heap.rs");
    expect(lib.filters.verified).toBe("false");
    expect(lib.filters.status).toBe("stale");
  });

  it("not_configured solution has verified=false and status=not_configured", () => {
    const siteData = buildFixtureSiteData();
    const index = buildExactIndex(siteData, rootConfig);
    const sol = pageBy(index, "solution:abc301/a/mod_inv_solve");
    expect(sol.filters.verified).toBe("false");
    expect(sol.filters.status).toBe("not_configured");
    expect(sol.filters.type).toBe("solution");
  });

  it("collects and dedupes lowercase symbol kinds", () => {
    const siteData = buildFixtureSiteData();
    const lib = siteData.libraries[0];
    lib.symbol_analysis = {
      state: "complete",
      symbols: [
        { kind: "Function", name: "a", search_names: ["a"] },
        { kind: "function", name: "b", search_names: ["b"] },
        { kind: "Trait", name: "T", search_names: ["T"] },
      ],
    };
    const index = buildExactIndex(siteData, rootConfig);
    const entry = pageBy(index, lib.page_id);
    expect(entry.filters.kind).toEqual(["function", "trait"]);
  });
});

// ---- Duplicate names ----

describe("buildExactIndex — duplicate names", () => {
  it("keeps both entries when two libraries share the same title and basename", () => {
    const siteData = buildFixtureSiteData();
    const dup: LibraryPageData = {
      ...siteData.libraries[0],
      page_id: "library:rust/algebra/dijkstra.rs",
      library_id: "rust/algebra/dijkstra.rs",
      source_path: "algebra/dijkstra.rs",
    };
    siteData.libraries.push(dup);
    const index = buildExactIndex(siteData, rootConfig);
    const same = index.pages.filter(
      (p) => p.aliases.includes("Dijkstra") && p.aliases.includes("dijkstra.rs"),
    );
    expect(same.length).toBe(2);
    const ids = same.map((p) => p.page_id).sort();
    expect(ids).toEqual(
      ["library:rust/algebra/dijkstra.rs", "library:rust/graph/dijkstra.rs"].sort(),
    );
  });
});

// ---- Private / non-detail exclusion ----

describe("buildExactIndex — exclusion invariants", () => {
  it("only detail-page IDs appear; no root/language/directory synthetic IDs", () => {
    const siteData = buildFixtureSiteData();
    const index = buildExactIndex(siteData, rootConfig);
    for (const p of index.pages) {
      expect(
        p.page_id.startsWith("library:") || p.page_id.startsWith("solution:"),
      ).toBe(true);
    }
  });

  it("`has_private_dependencies` does not add private-target records", () => {
    const siteData = buildFixtureSiteData();
    // The stale library carries has_private_dependencies=true but the index
    // must still contain only the public libraries listed in siteData.
    const index = buildExactIndex(siteData, rootConfig);
    const publicIds = new Set([
      ...siteData.libraries.map((l) => l.page_id),
      ...siteData.solutions.map((s) => s.page_id),
    ]);
    for (const p of index.pages) expect(publicIds.has(p.page_id)).toBe(true);
  });
});

// ---- Detail-page Pagefind attributes and fragment validity ----

describe("detail page — Pagefind annotations", () => {
  it("library detail article carries data-pagefind-body, meta, filter", async () => {
    const siteData = buildFixtureSiteData();
    const lib = siteData.libraries[0];
    const html = await renderLibraryDetailMainInner(rootConfig, siteData, lib);
    const doc = new JSDOM(html).window.document;
    const article = doc.querySelector("article.library-detail")!;
    expect(article.hasAttribute("data-pagefind-body")).toBe(true);
    const meta = article.getAttribute("data-pagefind-meta")!;
    expect(meta).toContain("title:Dijkstra");
    expect(meta).toContain("type:library");
    expect(meta).toContain("language:rust");
    expect(meta).toContain("status:verified");
    expect(meta).toContain(`page_id:${lib.page_id}`);
    expect(meta).toContain("display_path:graph/dijkstra.rs");
    expect(meta).toContain("url:/libraries/rust/graph/dijkstra.rs/");
    const filter = article.getAttribute("data-pagefind-filter")!;
    expect(filter).toContain("lang:rust");
    expect(filter).toContain("type:library");
    expect(filter).toContain("status:verified");
    expect(filter).toContain("verified:true");
    // Inner hidden filters for kinds and paths.
    const inner = article.innerHTML;
    expect(inner).toContain('data-pagefind-filter="kind:function"');
    expect(inner).toContain('data-pagefind-filter="path:graph"');
    expect(inner).toContain('data-pagefind-filter="path:dijkstra.rs"');
    expect(inner).toContain('data-pagefind-filter="path:graph/dijkstra.rs"');
  });

  it("library h1, symbol name, description, and source carry weight attributes", async () => {
    const siteData = buildFixtureSiteData();
    const lib = siteData.libraries[0];
    const html = await renderLibraryDetailMainInner(rootConfig, siteData, lib);
    const doc = new JSDOM(html).window.document;
    expect(doc.querySelector("h1")!.getAttribute("data-pagefind-weight")).toBe(
      "10",
    );
    const codeName = doc.querySelector("code.name")!;
    expect(codeName.getAttribute("data-pagefind-weight")).toBe("10");
    const documentation = doc.querySelector("#documentation")!;
    expect(documentation.getAttribute("data-pagefind-weight")).toBe("5");
    const source = doc.querySelector("#source")!;
    expect(source.getAttribute("data-pagefind-weight")).toBe("1");
  });

  it("solution detail article carries Pagefind meta / filter attributes", async () => {
    const siteData = buildFixtureSiteData();
    const sol: SolutionPageData = siteData.solutions[2]; // not_configured
    const html = await renderSolutionDetailMainInner(rootConfig, siteData, sol);
    const doc = new JSDOM(html).window.document;
    const article = doc.querySelector("article.solution-detail")!;
    const meta = article.getAttribute("data-pagefind-meta")!;
    expect(meta).toContain("type:solution");
    expect(meta).toContain("status:not_configured");
    expect(meta).toContain("display_path:abc301/a/mod_inv_solve");
    const filter = article.getAttribute("data-pagefind-filter")!;
    expect(filter).toContain("type:solution");
    expect(filter).toContain("status:not_configured");
    expect(filter).toContain("verified:false");
    const inner = article.innerHTML;
    // path filters derived from solution_id, not source_path
    expect(inner).toContain('data-pagefind-filter="path:abc301"');
    expect(inner).toContain('data-pagefind-filter="path:abc301/a/mod_inv_solve"');
    // No source_path segments should leak in as filters.
    expect(inner).not.toContain('data-pagefind-filter="path:solutions"');
  });

  it("every symbol fragment resolves to a real anchor in the rendered HTML", async () => {
    const siteData = buildFixtureSiteData();
    // Add a located symbol to exercise the L{n} branch.
    const lib = siteData.libraries[0];
    lib.symbol_analysis = {
      state: "complete",
      symbols: [
        ...lib.symbol_analysis.symbols,
        {
          kind: "function",
          name: "on_line_one",
          search_names: ["on_line_one"],
          location: { start: { line: 1 } },
        },
      ],
    };
    const html = await renderLibraryDetailMainInner(rootConfig, siteData, lib);
    const doc = new JSDOM(html).window.document;
    const index = buildExactIndex(siteData, rootConfig);
    const entry = pageBy(index, lib.page_id);
    expect(entry.symbols.length).toBeGreaterThan(0);
    for (const sym of entry.symbols) {
      // Each fragment must correspond to an existing element id in the doc.
      expect(doc.getElementById(sym.fragment)).not.toBeNull();
    }
  });
});
