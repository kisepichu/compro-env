import { describe, expect, it } from "vitest";

import { canonicalPage, parseSearchQuery } from "@/search/query.ts";
import type { ParsedQuery, QueryError } from "@/search/types.ts";

function ok(result: ReturnType<typeof parseSearchQuery>): ParsedQuery {
  if (!result.ok) {
    throw new Error(`Expected parse success, got ${result.code}: ${result.message}`);
  }
  return result;
}

function err(result: ReturnType<typeof parseSearchQuery>): QueryError {
  if (result.ok) {
    throw new Error("Expected parse error, got success");
  }
  return result;
}

describe("parseSearchQuery — bare terms and phrases", () => {
  it("keeps bare terms as full-text tokens", () => {
    const r = ok(parseSearchQuery("monoid fenwick"));
    expect(r.fullTextTokens).toEqual(["monoid", "fenwick"]);
    expect(r.fullText).toBe("monoid fenwick");
    expect(r.filters.lang).toEqual([]);
  });

  it("treats quoted phrases as a single full-text token with unescaped value", () => {
    const r = ok(parseSearchQuery('"fenwick tree"'));
    expect(r.fullTextTokens).toEqual(["fenwick tree"]);
  });

  it("keeps the raw quoted form inside fullText for downstream engines", () => {
    const r = ok(parseSearchQuery('foo "fenwick tree"'));
    expect(r.fullText).toBe('foo "fenwick tree"');
  });

  it("empty input yields an empty parse", () => {
    const r = ok(parseSearchQuery(""));
    expect(r.fullTextTokens).toEqual([]);
    expect(r.fullText).toBe("");
  });

  it("whitespace-only input yields an empty parse", () => {
    const r = ok(parseSearchQuery("   \t  "));
    expect(r.fullTextTokens).toEqual([]);
  });
});

describe("parseSearchQuery — filters", () => {
  it("parses bare filter values and lowercases them", () => {
    const r = ok(parseSearchQuery("lang:CPP"));
    expect(r.filters.lang).toEqual(["cpp"]);
    expect(r.fullText).toBe("");
  });

  it("parses quoted filter values", () => {
    const r = ok(parseSearchQuery('path:"Data Structures/Fenwick Tree.cpp"'));
    expect(r.filters.path).toEqual(["data structures/fenwick tree.cpp"]);
  });

  it("bare value stops at the next whitespace, not the next colon", () => {
    const r = ok(parseSearchQuery("path:foo:bar"));
    expect(r.filters.path).toEqual(["foo:bar"]);
  });

  it("treats mixed filters as AND across keys", () => {
    const r = ok(parseSearchQuery("lang:cpp kind:trait"));
    expect(r.filters.lang).toEqual(["cpp"]);
    expect(r.filters.kind).toEqual(["trait"]);
  });

  it("collects repeated keys as OR values in order", () => {
    const r = ok(parseSearchQuery("lang:cpp lang:rust"));
    expect(r.filters.lang).toEqual(["cpp", "rust"]);
  });

  it("deduplicates repeated identical filter values", () => {
    const r = ok(parseSearchQuery("lang:cpp lang:CPP"));
    expect(r.filters.lang).toEqual(["cpp"]);
  });

  it("mixes filter and text tokens", () => {
    const r = ok(parseSearchQuery("monoid lang:rust kind:trait"));
    expect(r.fullTextTokens).toEqual(["monoid"]);
    expect(r.filters.lang).toEqual(["rust"]);
    expect(r.filters.kind).toEqual(["trait"]);
  });

  it("accepts filter-only queries", () => {
    const r = ok(parseSearchQuery("lang:cpp"));
    expect(r.fullTextTokens).toEqual([]);
    expect(r.filters.lang).toEqual(["cpp"]);
  });

  it("preserves case in unrelated tokens", () => {
    const r = ok(parseSearchQuery("Monoid"));
    expect(r.fullTextTokens).toEqual(["Monoid"]);
  });
});

describe("parseSearchQuery — verified aliasing and status/type", () => {
  it("verified:true implies status:verified and sets the flag", () => {
    const r = ok(parseSearchQuery("verified:true"));
    expect(r.filters.verified_true).toBe(true);
    expect(r.filters.status).toEqual(["verified"]);
  });

  it("verified:false sets only the negative flag", () => {
    const r = ok(parseSearchQuery("verified:false"));
    expect(r.filters.verified_false).toBe(true);
    expect(r.filters.status).toEqual([]);
  });

  it("verified:TRUE lowercases", () => {
    const r = ok(parseSearchQuery("verified:TRUE"));
    expect(r.filters.verified_true).toBe(true);
  });

  it("status accepts all enum values", () => {
    const r = ok(
      parseSearchQuery("status:verified status:rejected status:unavailable status:stale status:never status:not-configured"),
    );
    expect(r.filters.status).toEqual([
      "verified",
      "rejected",
      "unavailable",
      "stale",
      "never",
      "not_configured",
    ]);
  });

  it("type accepts library and solution", () => {
    const r = ok(parseSearchQuery("type:library type:solution"));
    expect(r.filters.type).toEqual(["library", "solution"]);
  });
});

describe("parseSearchQuery — unknown keys", () => {
  it("treats unknown key:value as full-text with the raw token preserved", () => {
    const r = ok(parseSearchQuery("foo:bar"));
    expect(r.fullTextTokens).toEqual(["foo:bar"]);
    expect(r.fullText).toBe("foo:bar");
  });

  it("preserves unknown key quoted values as full-text raw text", () => {
    const r = ok(parseSearchQuery('foo:"bar baz"'));
    expect(r.fullTextTokens).toEqual(['foo:"bar baz"']);
  });

  it("does not populate any filter for unknown key", () => {
    const r = ok(parseSearchQuery("foo:cpp"));
    expect(r.filters.lang).toEqual([]);
  });
});

describe("parseSearchQuery — escapes inside quotes", () => {
  it("unescapes \\\" and \\\\ inside phrases", () => {
    const r = ok(parseSearchQuery('"a\\"b\\\\c"'));
    expect(r.fullTextTokens).toEqual(['a"b\\c']);
  });

  it("unescapes inside quoted filter values", () => {
    const r = ok(parseSearchQuery('path:"a\\"b"'));
    expect(r.filters.path).toEqual(['a"b']);
  });
});

describe("parseSearchQuery — invalid forms", () => {
  it("empty value for a known key is an error", () => {
    const e = err(parseSearchQuery("lang:"));
    expect(e.code).toBe("empty-filter-value");
  });

  it("empty quoted value for a known key is an error", () => {
    const e = err(parseSearchQuery('lang:""'));
    expect(e.code).toBe("empty-filter-value");
  });

  it("invalid boolean is an error", () => {
    const e = err(parseSearchQuery("verified:maybe"));
    expect(e.code).toBe("invalid-boolean");
  });

  it("invalid status enum is an error", () => {
    const e = err(parseSearchQuery("status:unknown"));
    expect(e.code).toBe("invalid-enum");
  });

  it("invalid type enum is an error", () => {
    const e = err(parseSearchQuery("type:book"));
    expect(e.code).toBe("invalid-enum");
  });

  it("unterminated quote is an error", () => {
    const e = err(parseSearchQuery('"abc'));
    expect(e.code).toBe("unterminated-quote");
  });

  it("unterminated quote after key: is an error", () => {
    const e = err(parseSearchQuery('path:"abc'));
    expect(e.code).toBe("unterminated-quote");
  });

  it("invalid escape inside quotes is an error", () => {
    const e = err(parseSearchQuery('"a\\bc"'));
    expect(e.code).toBe("invalid-escape");
  });

  it("text after closing quote in the same token is an error (phrase)", () => {
    const e = err(parseSearchQuery('"ab"cd'));
    expect(e.code).toBe("text-after-quote");
  });

  it("text after closing quote in the same token is an error (filter)", () => {
    const e = err(parseSearchQuery('path:"ab"cd'));
    expect(e.code).toBe("text-after-quote");
  });
});

describe("canonicalPage", () => {
  it("null → 1", () => {
    expect(canonicalPage(null)).toBe(1);
  });
  it("missing/non-numeric → 1", () => {
    expect(canonicalPage("")).toBe(1);
    expect(canonicalPage("abc")).toBe(1);
    expect(canonicalPage("1.5")).toBe(1);
  });
  it("zero and negative → 1", () => {
    expect(canonicalPage("0")).toBe(1);
    expect(canonicalPage("-5")).toBe(1);
  });
  it("valid positive integer → itself", () => {
    expect(canonicalPage("1")).toBe(1);
    expect(canonicalPage("42")).toBe(42);
  });
});
