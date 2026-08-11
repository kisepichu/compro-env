/**
 * Persist the exact-match index to disk.
 *
 * Per spec §13.1 the JSON is not committed to git; the build emits it into
 * the static output directory next to Pagefind's own bundle so the search
 * client fetches both via relative URLs under the configured base.
 */

import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

import type { ExactIndex } from "./exact-index.ts";

export const EXACT_INDEX_FILENAME = "exact-search-index.json";

/**
 * Write `exact-search-index.json` into {@link outDir}, creating the
 * directory if needed. The file is JSON.stringify'd without indentation
 * to minimize download size — the search page never renders it directly.
 */
export async function writeExactIndex(
  index: ExactIndex,
  outDir: string,
): Promise<void> {
  await mkdir(outDir, { recursive: true });
  const target = join(outDir, EXACT_INDEX_FILENAME);
  await writeFile(target, JSON.stringify(index), "utf8");
}
