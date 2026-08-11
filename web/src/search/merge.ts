/**
 * Pure merge/pagination/sub-result logic for the static search page.
 *
 * Per spec §13.1 the search-page union is:
 *   1. exact file / symbol matches, grouped by canonical page ID
 *   2. exact matches between themselves ordered by canonical page ID
 *      UTF-8 byte order
 *   3. Pagefind relevance-ordered results
 *   4. Pagefind results whose canonical page ID is already covered by
 *      the exact block are dropped
 *   5. the union is paginated in blocks of 20
 *
 * JavaScript `<` compares UTF-16 code units. For the ASCII/BMP page IDs
 * that ce emits (library:/solution: prefix + path segments) this yields
 * the same order as a UTF-8 byte comparison, so we use plain `<` and
 * document the requirement here.
 */

export type MatchReason = "Title match" | "File match" | "Symbol match";

/** A symbol/source-line sub-result rendered inside a page card. */
export interface SubResult {
  /** Short display label, e.g. `dijkstra` or `line 42`. */
  label: string;
  /** Detail-page fragment (without leading `#`), e.g. `L42` or `symbols`. */
  fragment: string;
  /** Full URL including fragment. */
  url: string;
  /** True when this is an exact symbol-name match from the exact index. */
  isExactSymbol: boolean;
  /** Symbol kind (used only for tie-break sorting). */
  kind?: string;
  /** Symbol name (used only for tie-break sorting). */
  name?: string;
  /** When present the sub-result anchors to a specific source line. */
  location?: { startLine: number };
}

/** Fields required to render a single search-result card. */
export interface MergePage {
  page_id: string;
  url: string;
  title: string;
  type: "library" | "solution";
  language: string;
  status: string;
  display_path: string;
  matchReasons: MatchReason[];
  subResults: SubResult[];
  /** Sanitized HTML excerpt (from Pagefind). */
  excerpt?: string;
}

export interface MergedPage extends MergePage {
  /** Set when Pagefind also produced a result for this page (unused by
   * dropped duplicates — only meaningful when a Pagefind-only card is
   * emitted, but exposed for the client if it wants to badge it). */
  pagefindResult?: true;
}

/**
 * Compare two page IDs using JavaScript's default lexicographic order.
 * For ASCII page IDs this matches UTF-8 byte order (spec §13.1 step 2).
 */
function comparePageId(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

function compareKindName(a: SubResult, b: SubResult): number {
  const ka = a.kind ?? "";
  const kb = b.kind ?? "";
  if (ka !== kb) return ka < kb ? -1 : 1;
  const na = a.name ?? a.label;
  const nb = b.name ?? b.label;
  if (na === nb) return 0;
  return na < nb ? -1 : 1;
}

/**
 * Merge exact and Pagefind result sets by canonical page ID.
 *
 * All filter predicates must already have been applied by the caller.
 * The function sorts exact matches by page ID byte order defensively
 * even though callers usually pre-sort — the invariant is easier to
 * keep by re-doing it once here.
 */
export function mergeResults(
  exactMatches: MergePage[],
  pagefindResults: MergePage[],
): MergedPage[] {
  const seen = new Set<string>();
  const out: MergedPage[] = [];
  // Exact block: dedup by page_id, then sort by page ID byte order.
  const exactSeen = new Set<string>();
  const exactDeduped: MergePage[] = [];
  for (const page of exactMatches) {
    if (exactSeen.has(page.page_id)) continue;
    exactSeen.add(page.page_id);
    exactDeduped.push(page);
  }
  exactDeduped.sort((a, b) => comparePageId(a.page_id, b.page_id));
  for (const page of exactDeduped) {
    seen.add(page.page_id);
    out.push({ ...page });
  }
  // Pagefind block: preserve relevance order; drop pages already covered.
  for (const page of pagefindResults) {
    if (seen.has(page.page_id)) continue;
    seen.add(page.page_id);
    out.push({ ...page, pagefindResult: true });
  }
  return out;
}

export interface Paginated<T> {
  pageItems: T[];
  totalPages: number;
  /** Canonicalized 1-based page number actually used. */
  page: number;
  totalItems: number;
}

/**
 * Paginate `items` in blocks of `pageSize` (default 20 per spec §13.1).
 * Invalid or out-of-range `page` inputs canonicalize to 1 (spec §13.2).
 * When `items` is empty, `totalPages` is 1 and `pageItems` is empty.
 */
export function paginate<T>(
  items: readonly T[],
  page: number,
  pageSize = 20,
): Paginated<T> {
  const totalItems = items.length;
  const totalPages = Math.max(1, Math.ceil(totalItems / pageSize));
  const canonical =
    !Number.isFinite(page) || page < 1 || page > totalPages ? 1 : Math.floor(page);
  const start = (canonical - 1) * pageSize;
  const pageItems = items.slice(start, start + pageSize);
  return { pageItems, totalPages, page: canonical, totalItems };
}

/**
 * Order sub-results per spec §13.2 and cap at 5:
 *   1. exact symbol matches with a source location, ordered by start line
 *   2. exact symbol matches without a location
 *   3. remaining (non-exact) sub-results in insertion order
 * Within groups 1 and 2, ties break on (kind, name) byte order.
 * Items past the fifth are dropped and returned in `remainderCount`.
 */
export function sortSubResults(
  subResults: readonly SubResult[],
): { items: SubResult[]; remainderCount: number } {
  const exactWithLoc: SubResult[] = [];
  const exactNoLoc: SubResult[] = [];
  const nonExact: SubResult[] = [];
  for (const sub of subResults) {
    if (sub.isExactSymbol && sub.location !== undefined) {
      exactWithLoc.push(sub);
    } else if (sub.isExactSymbol) {
      exactNoLoc.push(sub);
    } else {
      nonExact.push(sub);
    }
  }
  exactWithLoc.sort((a, b) => {
    const la = a.location!.startLine;
    const lb = b.location!.startLine;
    if (la !== lb) return la - lb;
    return compareKindName(a, b);
  });
  exactNoLoc.sort(compareKindName);
  const combined = [...exactWithLoc, ...exactNoLoc, ...nonExact];
  const items = combined.slice(0, 5);
  const remainderCount = Math.max(0, combined.length - 5);
  return { items, remainderCount };
}
