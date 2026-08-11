/**
 * Pipeline orchestration for `npm run site:build`.
 *
 * Keeps the ordering + short-circuit logic out of the CLI wrapper so it can
 * be unit-tested with injected fake runners. Every runner is called in the
 * fixed order below and receives a plain-object argument that documents
 * exactly what that stage should read/write:
 *
 *   1. `emitExactIndex`   — build the exact index in memory
 *   2. `astroBuild`       — run `astro build` into `<outDir>`
 *   3. `writeExactIndex`  — write the in-memory index to `<outDir>`
 *   4. `pagefind`         — run the Pagefind indexer against `<outDir>`
 *   5. `verify`           — run the site-verify checks against `<outDir>`
 *
 * The exact index is BUILT before Astro (so any DTO error stops the build
 * early) but WRITTEN after Astro (because `astro build` clears `<outDir>`).
 *
 * Any runner may signal failure by returning `{ status: <non-zero> }` or
 * by throwing; either short-circuits the pipeline and propagates the exit
 * code back to the CLI. When `skipPagefind` is true, the pagefind runner
 * is skipped entirely (still called only if not skipped).
 */

/**
 * @typedef {{
 *   emitExactIndex: (args: { siteData: unknown, config: { origin: string, base: string } }) => Promise<{ index: unknown, status?: number } | { status: number }>,
 *   astroBuild: (args: { outDir: string, base: string, origin: string, fixture: string }) => Promise<{ status: number }>,
 *   writeExactIndex: (args: { index: unknown, outDir: string }) => Promise<{ status?: number }>,
 *   pagefind: (args: { outDir: string }) => Promise<{ status: number }>,
 *   verify: (args: { outDir: string, base: string, fixture: string, expectPagefind: boolean }) => Promise<{ status: number }>,
 * }} Runners
 */

/**
 * Run the full pipeline. Returns `{ status, ran }` where `ran` is the
 * ordered list of stage names that were actually invoked — the test suite
 * asserts on this to verify the pipeline order.
 *
 * @param {Runners} runners
 * @param {{
 *   siteData: unknown,
 *   config: { origin: string, base: string },
 *   outDir: string,
 *   fixture: string,
 *   skipPagefind: boolean,
 * }} config
 */
export async function runPipeline(runners, config) {
  const ran = [];
  const { siteData, config: urlConfig, outDir, fixture, skipPagefind } = config;

  ran.push("emitExactIndex");
  const emitResult = await runners.emitExactIndex({
    siteData,
    config: urlConfig,
  });
  if (emitResult && typeof emitResult.status === "number" && emitResult.status !== 0) {
    return { status: emitResult.status, ran };
  }
  const index =
    emitResult && "index" in emitResult ? emitResult.index : undefined;
  // A "success" that omits `index` would silently write `undefined` to
  // exact-search-index.json downstream. Treat that as a pipeline failure
  // so the invariant (successful emit ⇒ index present) is enforced here.
  if (index === undefined) {
    return { status: 1, ran };
  }

  ran.push("astroBuild");
  const astroResult = await runners.astroBuild({
    outDir,
    base: urlConfig.base,
    origin: urlConfig.origin,
    fixture,
  });
  if (astroResult.status !== 0) {
    return { status: astroResult.status, ran };
  }

  ran.push("writeExactIndex");
  const writeResult = await runners.writeExactIndex({ index, outDir });
  if (writeResult && typeof writeResult.status === "number" && writeResult.status !== 0) {
    return { status: writeResult.status, ran };
  }

  if (!skipPagefind) {
    ran.push("pagefind");
    const pagefindResult = await runners.pagefind({ outDir });
    if (pagefindResult.status !== 0) {
      return { status: pagefindResult.status, ran };
    }
  }

  ran.push("verify");
  const verifyResult = await runners.verify({
    outDir,
    base: urlConfig.base,
    fixture,
    expectPagefind: !skipPagefind,
  });
  if (verifyResult.status !== 0) {
    return { status: verifyResult.status, ran };
  }

  return { status: 0, ran };
}
