/**
 * Documentation-heading ID algorithm per spec §12 (lines 1641–1651).
 *
 * The ID is deterministic across builds: the same heading text always
 * yields the same ID as long as the SHA-256 domain does not change.
 * A `seenCounts` map is threaded through the whole document so that
 * duplicate IDs pick up `-2`, `-3`, ... suffixes in document order,
 * starting at 2 for the second occurrence.
 *
 *   1. Extract the plain text of the heading.
 *   2. Feed the exact UTF-8 bytes into SHA-256 (no normalization).
 *   3. Build the ASCII "hint":
 *      - Lowercase A-Z; keep runs of [a-z0-9]; separate runs with `-`;
 *      - trim leading/trailing `-`; cap at 48 bytes (with any trailing
 *        `-` removed after the cap); empty result → `h`.
 *   4. Take the first 10 hex chars of the digest.
 *   5. ID: `doc-{hint}-{digest10}`.
 *   6. Suffix `-2`, `-3`, ... for repeated IDs in document order.
 */

import { createHash } from "node:crypto";

const MAX_HINT_BYTES = 48;

/** Build the ASCII hint segment for a heading text. */
export function buildHeadingHint(text: string): string {
  const lowered = text.replace(/[A-Z]/g, (c) => c.toLowerCase());
  const runs = lowered.match(/[a-z0-9]+/g) ?? [];
  let joined = runs.join("-");
  joined = joined.replace(/^-+|-+$/g, "");
  if (joined.length > MAX_HINT_BYTES) {
    joined = joined.slice(0, MAX_HINT_BYTES).replace(/-+$/g, "");
  }
  if (joined.length === 0) return "h";
  return joined;
}

/** Compute the first 10 hex chars of the SHA-256 digest of the UTF-8 bytes. */
export function digestPrefixHex10(text: string): string {
  return createHash("sha256").update(text, "utf8").digest("hex").slice(0, 10);
}

/**
 * Compute the doc-prefixed heading ID for a single heading. Mutates the
 * `seenCounts` map so subsequent identical headings get numeric suffixes.
 */
export function computeHeadingId(
  text: string,
  seenCounts: Map<string, number>,
): string {
  const hint = buildHeadingHint(text);
  const digest = digestPrefixHex10(text);
  const baseId = `doc-${hint}-${digest}`;
  const seen = seenCounts.get(baseId) ?? 0;
  seenCounts.set(baseId, seen + 1);
  if (seen === 0) return baseId;
  return `${baseId}-${seen + 1}`;
}

/**
 * Batch API: given the ordered list of heading texts in a document,
 * return the ordered list of assigned IDs.
 */
export function assignHeadingIds(headings: readonly string[]): string[] {
  const seen = new Map<string, number>();
  return headings.map((h) => computeHeadingId(h, seen));
}
