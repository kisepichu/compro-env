/**
 * Scheme allowlist for external URLs that appear inside `<a href>` in the
 * generated HTML. HTML escaping alone does not defeat `javascript:` or
 * other dangerous schemes; we accept only `http` and `https` and reject
 * everything else (control characters, protocol-relative `//foo`, `data:`,
 * `javascript:`, `file:`, and malformed inputs).
 *
 * Callers are expected to omit the link entirely when this returns `null`.
 */

const ALLOWED_SCHEMES = new Set(["http:", "https:"]);

function containsControlChars(input: string): boolean {
  for (const ch of input) {
    const code = ch.codePointAt(0);
    if (code === undefined) continue;
    if (code < 0x20 || code === 0x7f) return true;
  }
  return false;
}

/**
 * Return a trusted, http(s)-only URL string, or `null` when the input is
 * not a safe external URL.
 *
 * Pass `{ stripTrailingSlash: true }` when building a base URL that will
 * later be composed as `{base}/blob/{sha}/{path}` — the trailing `/` is
 * removed so the result never contains `//blob/…`.
 */
export function sanitizeExternalUrl(
  input: string | null | undefined,
  options: { stripTrailingSlash?: boolean } = {},
): string | null {
  if (input === null || input === undefined) return null;
  const trimmed = input.trim();
  if (trimmed.length === 0) return null;
  if (containsControlChars(trimmed)) return null;
  // Protocol-relative URLs (`//foo`) are rejected — they inherit the
  // current page's scheme, which is not something we want to trust for
  // externally sourced strings.
  if (trimmed.startsWith("//")) return null;
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    return null;
  }
  if (!ALLOWED_SCHEMES.has(parsed.protocol)) return null;
  const serialized = parsed.toString();
  if (options.stripTrailingSlash === true && serialized.endsWith("/")) {
    return serialized.slice(0, -1);
  }
  return serialized;
}
