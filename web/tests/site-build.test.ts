/**
 * Task 4 pipeline-order tests. These lock down the *ordering contract*
 * of `npm run site:build` by injecting spy runners into `runPipeline`.
 *
 * The pipeline must always call runners in exactly this order:
 *
 *   emitExactIndex → astroBuild → writeExactIndex → pagefind → verify
 *
 * `writeExactIndex` runs AFTER `astroBuild` (since Astro clears the
 * out-dir), but `emitExactIndex` runs BEFORE it so a bad DTO stops the
 * pipeline before Astro spends time compiling.
 *
 * Additional parity test: the JS-native `buildExactIndex` in
 * `web/scripts/emit-exact-index.mjs` must produce byte-for-byte the same
 * JSON as the TypeScript `buildExactIndex` in `web/src/search/exact-index.ts`
 * for the fixture site-data — the two implementations are duplicated
 * intentionally, and this test guards against drift.
 */

// @ts-expect-error — hand-written .mjs sibling of this test, no d.ts.
import { runPipeline } from "../scripts/site-build-core.mjs";
// @ts-expect-error — hand-written .mjs sibling of this test, no d.ts.
import { buildExactIndex as buildExactIndexJs } from "../scripts/emit-exact-index.mjs";

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it, vi } from "vitest";

import { buildExactIndex as buildExactIndexTs } from "@/search/exact-index.ts";
import { assertSiteData } from "@/lib/site-data.ts";
import type { UrlConfig } from "@/lib/url.ts";

const rootConfig: UrlConfig = { origin: "https://example.test", base: "/" };

const stubSiteData = {
  libraries: [],
  solutions: [],
};

function makeSpies(overrides: Partial<Record<string, unknown>> = {}) {
  return {
    emitExactIndex: vi
      .fn()
      .mockResolvedValue({ index: { schema_version: 1, pages: [] }, status: 0 }),
    astroBuild: vi.fn().mockResolvedValue({ status: 0 }),
    writeExactIndex: vi.fn().mockResolvedValue({ status: 0 }),
    pagefind: vi.fn().mockResolvedValue({ status: 0 }),
    verify: vi.fn().mockResolvedValue({ status: 0 }),
    ...overrides,
  };
}

const baseConfig = {
  siteData: stubSiteData,
  config: rootConfig,
  outDir: "/tmp/out",
  fixture: "web/tests/fixtures/site-data.json",
  skipPagefind: false,
} as const;

describe("runPipeline — order contract", () => {
  it("runs stages in exact order: emit, astro, write, pagefind, verify", async () => {
    const runners = makeSpies();
    const result = await runPipeline(runners, baseConfig);

    expect(result.status).toBe(0);
    expect(result.ran).toEqual([
      "emitExactIndex",
      "astroBuild",
      "writeExactIndex",
      "pagefind",
      "verify",
    ]);
    // Also verify the temporal ordering via mock.invocationCallOrder.
    const orders = [
      runners.emitExactIndex.mock.invocationCallOrder[0],
      runners.astroBuild.mock.invocationCallOrder[0],
      runners.writeExactIndex.mock.invocationCallOrder[0],
      runners.pagefind.mock.invocationCallOrder[0],
      runners.verify.mock.invocationCallOrder[0],
    ];
    for (let i = 1; i < orders.length; i += 1) {
      expect(orders[i]).toBeGreaterThan(orders[i - 1]!);
    }
  });

  it("skips pagefind when --skip-pagefind is set but still runs the other four stages", async () => {
    const runners = makeSpies();
    const result = await runPipeline(runners, {
      ...baseConfig,
      skipPagefind: true,
    });
    expect(result.status).toBe(0);
    expect(result.ran).toEqual([
      "emitExactIndex",
      "astroBuild",
      "writeExactIndex",
      "verify",
    ]);
    expect(runners.pagefind).not.toHaveBeenCalled();
    expect(runners.verify).toHaveBeenCalledWith(
      expect.objectContaining({ expectPagefind: false }),
    );
  });

  it("stops before writeExactIndex / pagefind / verify when astro fails", async () => {
    const runners = makeSpies({
      astroBuild: vi.fn().mockResolvedValue({ status: 3 }),
    });
    const result = await runPipeline(runners, baseConfig);
    expect(result.status).toBe(3);
    expect(result.ran).toEqual(["emitExactIndex", "astroBuild"]);
    expect(runners.writeExactIndex).not.toHaveBeenCalled();
    expect(runners.pagefind).not.toHaveBeenCalled();
    expect(runners.verify).not.toHaveBeenCalled();
  });

  it("propagates a non-zero exit code from the emit stage", async () => {
    const runners = makeSpies({
      emitExactIndex: vi.fn().mockResolvedValue({ status: 7 }),
    });
    const result = await runPipeline(runners, baseConfig);
    expect(result.status).toBe(7);
    expect(result.ran).toEqual(["emitExactIndex"]);
    expect(runners.astroBuild).not.toHaveBeenCalled();
  });

  it("fails when emit reports success but omits the index", async () => {
    const runners = makeSpies({
      // `status: 0` without `index` would otherwise pass `undefined` into
      // writeExactIndex; the pipeline must short-circuit instead.
      emitExactIndex: vi.fn().mockResolvedValue({ status: 0 }),
    });
    const result = await runPipeline(runners, baseConfig);
    expect(result.status).not.toBe(0);
    expect(result.ran).toEqual(["emitExactIndex"]);
    expect(runners.astroBuild).not.toHaveBeenCalled();
    expect(runners.writeExactIndex).not.toHaveBeenCalled();
  });

  it("propagates a non-zero exit code from the pagefind stage", async () => {
    const runners = makeSpies({
      pagefind: vi.fn().mockResolvedValue({ status: 5 }),
    });
    const result = await runPipeline(runners, baseConfig);
    expect(result.status).toBe(5);
    expect(result.ran).toEqual([
      "emitExactIndex",
      "astroBuild",
      "writeExactIndex",
      "pagefind",
    ]);
    expect(runners.verify).not.toHaveBeenCalled();
  });

  it("propagates a non-zero exit code from the verify stage", async () => {
    const runners = makeSpies({
      verify: vi.fn().mockResolvedValue({ status: 1 }),
    });
    const result = await runPipeline(runners, baseConfig);
    expect(result.status).toBe(1);
    expect(runners.verify).toHaveBeenCalledWith(
      expect.objectContaining({
        outDir: "/tmp/out",
        base: "/",
        fixture: "web/tests/fixtures/site-data.json",
        expectPagefind: true,
      }),
    );
  });

  it("passes --out and --base into the verify stage", async () => {
    const runners = makeSpies();
    await runPipeline(runners, {
      ...baseConfig,
      config: { origin: "https://example.test", base: "/compro-env/" },
      outDir: "/tmp/other",
    });
    expect(runners.verify).toHaveBeenCalledWith(
      expect.objectContaining({ outDir: "/tmp/other", base: "/compro-env/" }),
    );
  });

  it("passes the in-memory index emitted by step 1 to the write step", async () => {
    const uniqueIndex = { schema_version: 1, pages: [{ page_id: "x" }] };
    const runners = makeSpies({
      emitExactIndex: vi.fn().mockResolvedValue({ index: uniqueIndex, status: 0 }),
    });
    await runPipeline(runners, baseConfig);
    expect(runners.writeExactIndex).toHaveBeenCalledWith(
      expect.objectContaining({ index: uniqueIndex, outDir: "/tmp/out" }),
    );
  });
});

describe("buildExactIndex — JS ↔ TS parity", () => {
  it("emits byte-identical JSON for the shipped fixture", () => {
    const raw = readFileSync(
      resolve(__dirname, "fixtures/site-data.json"),
      "utf8",
    );
    const siteData = assertSiteData(JSON.parse(raw));
    const jsIndex = buildExactIndexJs(siteData, rootConfig);
    const tsIndex = buildExactIndexTs(siteData, rootConfig);
    expect(JSON.stringify(jsIndex)).toBe(JSON.stringify(tsIndex));
  });

  it("agrees under a non-root base", () => {
    const raw = readFileSync(
      resolve(__dirname, "fixtures/site-data.json"),
      "utf8",
    );
    const siteData = assertSiteData(JSON.parse(raw));
    const cfg: UrlConfig = {
      origin: "https://example.com",
      base: "/compro-env/",
    };
    const jsIndex = buildExactIndexJs(siteData, cfg);
    const tsIndex = buildExactIndexTs(siteData, cfg);
    expect(JSON.stringify(jsIndex)).toBe(JSON.stringify(tsIndex));
  });
});
