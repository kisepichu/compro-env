import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import {
  SiteDataSchemaError,
  SUPPORTED_SCHEMA_VERSION,
  assertSiteData,
  loadSiteData,
} from "@/lib/site-data.ts";
import {
  UrlEscapeError,
  homePath,
  libraryPath,
  searchPath,
  solutionPath,
  toAssetUrl,
  toCanonicalUrl,
  toInternalPath,
} from "@/lib/url.ts";

function minimalSiteData(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    schema_version: SUPPORTED_SCHEMA_VERSION,
    build: {
      schema_version: SUPPORTED_SCHEMA_VERSION,
      generated_at: "2026-08-11T00:00:00Z",
      mode: "production",
      source_commit_sha: "0000000000000000000000000000000000000000",
      source_commit_short_sha: "0000000",
      source_committed_at: "2026-08-11T00:00:00Z",
      uncommitted_changes: false,
      observed_toolchains: [],
      adapters: [],
    },
    site: {
      title: "compro-env fixture",
      description: "fixture site data",
      language: "en",
    },
    languages: [],
    libraries: [],
    solutions: [],
    ...overrides,
  };
}

describe("site-data loader", () => {
  it("accepts a well-formed minimal document", () => {
    const raw = minimalSiteData();
    const value = assertSiteData(raw);
    expect(value.schema_version).toBe(SUPPORTED_SCHEMA_VERSION);
    expect(value.libraries).toHaveLength(0);
  });

  it("rejects a schema_version mismatch with a specific message", () => {
    const raw = minimalSiteData({ schema_version: SUPPORTED_SCHEMA_VERSION + 1 });
    expect(() => assertSiteData(raw)).toThrow(SiteDataSchemaError);
    try {
      assertSiteData(raw);
    } catch (err) {
      expect((err as Error).message).toMatch(/Unsupported site-data schema_version/);
    }
  });

  it("rejects a missing schema_version integer", () => {
    const raw = minimalSiteData({ schema_version: "1" });
    expect(() => assertSiteData(raw)).toThrow(SiteDataSchemaError);
  });

  it("rejects a document containing a private-only field", () => {
    // spec §4.4 forbids private data leaking; the schema uses
    // additionalProperties=false so any extra key must fail.
    const raw = minimalSiteData();
    (raw.libraries as unknown[]).push({
      page_id: "library:rust/foo.rs",
      library_id: "rust/foo.rs",
      language: "rust",
      title: "foo",
      source_path: "foo.rs",
      source: "pub fn foo() {}\n",
      syntax_highlight: "rust",
      updated_at: "2026-08-11T00:00:00Z",
      updated_by_commit: "0000000",
      // Private diagnostic — must be rejected because additionalProperties=false
      _private_diagnostic: "leaked internal path",
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
    expect(() => assertSiteData(raw)).toThrow(SiteDataSchemaError);
    try {
      assertSiteData(raw);
    } catch (err) {
      const issues = (err as SiteDataSchemaError).issues;
      expect(
        issues.some((issue) => issue.includes("_private_diagnostic")),
      ).toBe(true);
    }
  });

  it("loadSiteData reads and validates a JSON file at an explicit path", () => {
    const dir = mkdtempSync(join(tmpdir(), "ce-site-data-"));
    const path = join(dir, "site-data.json");
    writeFileSync(path, JSON.stringify(minimalSiteData()), "utf8");
    const value = loadSiteData(path);
    expect(value.site.title).toBe("compro-env fixture");
  });

  it("loadSiteData surfaces JSON parse errors with the file path", () => {
    const dir = mkdtempSync(join(tmpdir(), "ce-site-data-"));
    const path = join(dir, "site-data.json");
    writeFileSync(path, "{ this is not json", "utf8");
    expect(() => loadSiteData(path)).toThrow(SiteDataSchemaError);
  });
});

describe("URL helpers", () => {
  const rootConfig = { origin: "https://example.com", base: "/" };
  const projectConfig = {
    origin: "https://example.com",
    base: "/compro-env/",
  };

  it("builds root-base internal paths ending with `/`", () => {
    expect(toInternalPath(rootConfig, [])).toBe("/");
    expect(toInternalPath(rootConfig, ["libraries"])).toBe("/libraries/");
    expect(
      toInternalPath(rootConfig, ["libraries", "rust", "graph"]),
    ).toBe("/libraries/rust/graph/");
  });

  it("preserves the project base prefix without duplication", () => {
    expect(toInternalPath(projectConfig, [])).toBe("/compro-env/");
    expect(toInternalPath(projectConfig, ["libraries"])).toBe(
      "/compro-env/libraries/",
    );
    expect(homePath(projectConfig)).toBe("/compro-env/");
    expect(searchPath(projectConfig)).toBe("/compro-env/search/");
  });

  it("percent-encodes each path segment individually", () => {
    // Slashes stay unencoded because they are hierarchy separators.
    expect(
      toInternalPath(rootConfig, ["libraries", "c++", "graph 1"]),
    ).toBe("/libraries/c%2B%2B/graph%201/");
    // Unicode segments are encoded as UTF-8 percent bytes.
    expect(
      toInternalPath(rootConfig, ["libraries", "日本語"]),
    ).toBe("/libraries/%E6%97%A5%E6%9C%AC%E8%AA%9E/");
  });

  it("adds a trailing slash for canonical URLs", () => {
    expect(toCanonicalUrl(rootConfig, ["libraries"])).toBe(
      "https://example.com/libraries/",
    );
    expect(toCanonicalUrl(projectConfig, [])).toBe(
      "https://example.com/compro-env/",
    );
  });

  it("rejects repository-escape segments", () => {
    expect(() => toInternalPath(rootConfig, [".."])).toThrow(UrlEscapeError);
    expect(() => toInternalPath(rootConfig, ["ok", "..", "escape"]))
      .toThrow(UrlEscapeError);
    expect(() => toInternalPath(rootConfig, [""])).toThrow(UrlEscapeError);
    expect(() =>
      toInternalPath(rootConfig, ["contains/slash"]),
    ).toThrow(UrlEscapeError);
  });

  it("computes library and solution detail paths", () => {
    expect(libraryPath(projectConfig, "rust", "graph/dijkstra.rs")).toBe(
      "/compro-env/libraries/rust/graph/dijkstra.rs/",
    );
    expect(
      solutionPath(projectConfig, "abc300", "a", "main"),
    ).toBe("/compro-env/solutions/abc300/a/main/");
  });

  it("asset URLs do not add a trailing slash", () => {
    expect(toAssetUrl(rootConfig, ["robots.txt"])).toBe("/robots.txt");
    expect(toAssetUrl(projectConfig, ["sitemap.xml"])).toBe(
      "/compro-env/sitemap.xml",
    );
  });
});
