/**
 * Task 3 unit tests for the pure merge / pagination / sub-result logic.
 *
 * Covers spec §13.1 (exact-first order, page-ID dedup) and §13.2
 * (sub-result cap, pagination canonicalization). No DOM: these run in
 * plain node.
 */

import { describe, expect, it } from "vitest";

import {
  mergeResults,
  paginate,
  sortSubResults,
  type MergePage,
  type SubResult,
} from "@/search/merge.ts";

function page(id: string, overrides: Partial<MergePage> = {}): MergePage {
  return {
    page_id: id,
    url: `/${id}/`,
    title: id,
    type: "library",
    language: "rust",
    status: "verified",
    display_path: id,
    matchReasons: ["Title match"],
    subResults: [],
    ...overrides,
  };
}

describe("mergeResults — exact-first order and page-ID dedup", () => {
  it("orders exact matches by page-ID byte order regardless of caller order", () => {
    const merged = mergeResults(
      [
        page("library:rust/graph/dijkstra.rs"),
        page("library:rust/algebra/monoid.rs"),
        page("library:rust/math/mod_inv.rs"),
      ],
      [],
    );
    expect(merged.map((p) => p.page_id)).toEqual([
      "library:rust/algebra/monoid.rs",
      "library:rust/graph/dijkstra.rs",
      "library:rust/math/mod_inv.rs",
    ]);
  });

  it("drops a Pagefind result whose page_id already appeared in exact", () => {
    const merged = mergeResults(
      [page("library:rust/graph/dijkstra.rs")],
      [
        page("library:rust/graph/dijkstra.rs", { matchReasons: [] }),
        page("solution:abc300/a/dijkstra_solve"),
      ],
    );
    expect(merged.map((p) => p.page_id)).toEqual([
      "library:rust/graph/dijkstra.rs",
      "solution:abc300/a/dijkstra_solve",
    ]);
    // Only the Pagefind-only card is marked as pagefindResult.
    expect(merged[0].pagefindResult).toBeUndefined();
    expect(merged[1].pagefindResult).toBe(true);
  });

  it("preserves Pagefind relevance order for non-duplicates", () => {
    const merged = mergeResults(
      [],
      [
        page("library:rust/z_end.rs"),
        page("library:rust/a_start.rs"),
        page("library:rust/m_middle.rs"),
      ],
    );
    // Pagefind order is respected (relevance order is the input order).
    expect(merged.map((p) => p.page_id)).toEqual([
      "library:rust/z_end.rs",
      "library:rust/a_start.rs",
      "library:rust/m_middle.rs",
    ]);
  });

  it("all-duplicate-exact case: multiple exact pages with the same alias each appear", () => {
    // Both pages carry a `Dijkstra` title/basename alias and should both
    // land in the exact block, sorted by page_id byte order.
    const merged = mergeResults(
      [
        page("library:rust/graph/dijkstra.rs"),
        page("library:rust/algebra/dijkstra.rs"),
      ],
      [],
    );
    expect(merged.length).toBe(2);
    expect(merged.map((p) => p.page_id)).toEqual([
      "library:rust/algebra/dijkstra.rs",
      "library:rust/graph/dijkstra.rs",
    ]);
  });

  it("deduplicates repeated exact matches by page ID", () => {
    const merged = mergeResults(
      [page("library:rust/a.rs"), page("library:rust/a.rs")],
      [],
    );
    expect(merged.length).toBe(1);
  });

  it("filter-only queries: zero exact matches, results from Pagefind only", () => {
    const merged = mergeResults(
      [],
      [
        page("library:rust/a.rs"),
        page("library:rust/b.rs"),
        page("solution:abc300/a/main"),
      ],
    );
    expect(merged.length).toBe(3);
    expect(merged.every((p) => p.pagefindResult === true)).toBe(true);
  });

  it("Pagefind failure: empty pagefindResults still returns exact matches", () => {
    const merged = mergeResults([page("library:rust/only.rs")], []);
    expect(merged.length).toBe(1);
    expect(merged[0].page_id).toBe("library:rust/only.rs");
  });
});

describe("paginate — 20 cards per page, out-of-range canonicalization", () => {
  const items = Array.from({ length: 45 }, (_, i) => i);

  it("45 items → 3 pages", () => {
    const first = paginate(items, 1);
    expect(first.totalItems).toBe(45);
    expect(first.totalPages).toBe(3);
    expect(first.pageItems.length).toBe(20);
    expect(first.pageItems[0]).toBe(0);
  });

  it("page 3 has 5 items", () => {
    const p3 = paginate(items, 3);
    expect(p3.page).toBe(3);
    expect(p3.pageItems.length).toBe(5);
    expect(p3.pageItems[0]).toBe(40);
    expect(p3.pageItems[4]).toBe(44);
  });

  it("out-of-range page → 1", () => {
    const p9 = paginate(items, 9);
    expect(p9.page).toBe(1);
    expect(p9.pageItems[0]).toBe(0);
  });

  it("page 0 or negative → 1", () => {
    expect(paginate(items, 0).page).toBe(1);
    expect(paginate(items, -3).page).toBe(1);
  });

  it("empty items produces totalPages=1 and no pageItems", () => {
    const empty = paginate<number>([], 1);
    expect(empty.totalItems).toBe(0);
    expect(empty.totalPages).toBe(1);
    expect(empty.pageItems).toEqual([]);
  });

  it("respects custom pageSize", () => {
    const p = paginate(items, 2, 10);
    expect(p.totalPages).toBe(5);
    expect(p.pageItems.length).toBe(10);
    expect(p.pageItems[0]).toBe(10);
  });
});

function sym(
  name: string,
  overrides: Partial<SubResult> = {},
): SubResult {
  return {
    label: name,
    fragment: `symbols`,
    url: `#symbols`,
    isExactSymbol: true,
    name,
    kind: "function",
    ...overrides,
  };
}

describe("sortSubResults — order and 5-item cap", () => {
  it("shows all items when count is below the cap", () => {
    const { items, remainderCount } = sortSubResults([
      sym("a", { location: { startLine: 5 } }),
      sym("b", { location: { startLine: 3 } }),
      sym("c", { location: { startLine: 10 } }),
    ]);
    expect(items.length).toBe(3);
    expect(remainderCount).toBe(0);
    // With-location sorted by start line ascending.
    expect(items.map((s) => s.name)).toEqual(["b", "a", "c"]);
  });

  it("exact-with-location comes before exact-without-location", () => {
    const { items } = sortSubResults([
      sym("nl1"),
      sym("wl", { location: { startLine: 42 } }),
      sym("nl2"),
    ]);
    // First slot is the located one; the rest are non-located, sorted by (kind, name).
    expect(items[0].name).toBe("wl");
    expect(items[1].name).toBe("nl1");
    expect(items[2].name).toBe("nl2");
  });

  it("ties in start line break on (kind, name) byte order", () => {
    const { items } = sortSubResults([
      sym("beta", { kind: "function", location: { startLine: 1 } }),
      sym("alpha", { kind: "function", location: { startLine: 1 } }),
      sym("gamma", { kind: "trait", location: { startLine: 1 } }),
    ]);
    // 'function' < 'trait' (kind first); within function, alpha < beta.
    expect(items.map((s) => s.name)).toEqual(["alpha", "beta", "gamma"]);
  });

  it("caps at 5 items and reports the remainder count", () => {
    const many: SubResult[] = [];
    for (let i = 0; i < 8; i += 1) {
      many.push(sym(`s${i}`, { location: { startLine: i + 1 } }));
    }
    const { items, remainderCount } = sortSubResults(many);
    expect(items.length).toBe(5);
    expect(remainderCount).toBe(3);
    // Cap keeps the first five in sorted order.
    expect(items.map((s) => s.name)).toEqual(["s0", "s1", "s2", "s3", "s4"]);
  });

  it("non-exact matches come after all exact matches, in given order", () => {
    const inputs: SubResult[] = [
      { label: "L10", fragment: "L10", url: "#L10", isExactSymbol: false },
      sym("exact_one", { location: { startLine: 2 } }),
      { label: "L20", fragment: "L20", url: "#L20", isExactSymbol: false },
    ];
    const { items } = sortSubResults(inputs);
    expect(items.map((s) => s.label)).toEqual(["exact_one", "L10", "L20"]);
  });
});
