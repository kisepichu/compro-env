/**
 * Base-path-safe URL helpers.
 *
 * The Astro build always ships with a `site` origin and a `base` prefix that
 * begins and ends with `/`. Every internal link, canonical URL, asset URL,
 * and search-result URL flows through the helpers below so the codebase
 * never concatenates a root-relative literal to `base`.
 */

export type UrlConfig = {
  /** Absolute site origin without trailing slash, e.g. `https://example.com`. */
  origin: string;
  /** Base path beginning and ending with `/`, e.g. `/compro-env/`. */
  base: string;
};

export class UrlEscapeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "UrlEscapeError";
  }
}

function normalizeConfig(config: UrlConfig): UrlConfig {
  const origin = config.origin.endsWith("/")
    ? config.origin.slice(0, -1)
    : config.origin;
  let base = config.base;
  if (base === "" || base === "/") {
    base = "/";
  } else {
    if (!base.startsWith("/")) base = `/${base}`;
    if (!base.endsWith("/")) base = `${base}/`;
  }
  return { origin, base };
}

function encodeSegment(segment: string): string {
  if (segment === "" || segment === "." || segment === "..") {
    throw new UrlEscapeError(
      `Path segment escapes the site root: ${JSON.stringify(segment)}`,
    );
  }
  if (segment.includes("/") || segment.includes("\\")) {
    throw new UrlEscapeError(
      `Path segment must not contain a separator: ${JSON.stringify(segment)}`,
    );
  }
  return encodeURIComponent(segment);
}

/**
 * Build an internal path (site-relative) from ordered raw segments.
 * The returned value starts with `base`, has percent-encoded segments,
 * and ends with `/` so it can be used as an `href` for a directory-style
 * Astro route or `canonical URL`.
 */
export function toInternalPath(
  config: UrlConfig,
  segments: readonly string[],
): string {
  const { base } = normalizeConfig(config);
  if (segments.length === 0) {
    return base;
  }
  const encoded = segments.map(encodeSegment).join("/");
  return `${base}${encoded}/`;
}

/**
 * Build an absolute canonical URL. The trailing slash is preserved for
 * directory-style routes; use {@link toAssetUrl} for files with an
 * extension such as `/robots.txt`.
 */
export function toCanonicalUrl(
  config: UrlConfig,
  segments: readonly string[],
): string {
  const { origin } = normalizeConfig(config);
  return `${origin}${toInternalPath(config, segments)}`;
}

/**
 * Build a base-relative asset URL (no trailing slash added). Segments are
 * still encoded and repository escapes are rejected.
 */
export function toAssetUrl(
  config: UrlConfig,
  segments: readonly string[],
): string {
  const { base } = normalizeConfig(config);
  if (segments.length === 0) {
    throw new UrlEscapeError("Asset URL requires at least one segment");
  }
  const encoded = segments.map(encodeSegment).join("/");
  return `${base}${encoded}`;
}

export function homePath(config: UrlConfig): string {
  return toInternalPath(config, []);
}

export function searchPath(config: UrlConfig): string {
  return toInternalPath(config, ["search"]);
}

export function librariesRootPath(config: UrlConfig): string {
  return toInternalPath(config, ["libraries"]);
}

export function libraryPath(
  config: UrlConfig,
  language: string,
  sourceRelativePath: string,
): string {
  const parts = sourceRelativePath.split("/").filter((p) => p.length > 0);
  return toInternalPath(config, ["libraries", language, ...parts]);
}

export function solutionsRootPath(config: UrlConfig): string {
  return toInternalPath(config, ["solutions"]);
}

export function solutionPath(
  config: UrlConfig,
  contestId: string,
  problemCode: string,
  solutionName: string,
): string {
  return toInternalPath(config, [
    "solutions",
    contestId,
    problemCode,
    solutionName,
  ]);
}

/**
 * Convert a raw string like `Some/Path/` into a normalized `href` under the
 * current base. Kept as a defence-in-depth helper for values that come from
 * data — never from a hard-coded root-relative literal.
 */
export function joinPath(config: UrlConfig, raw: string): string {
  if (raw.startsWith("/")) {
    throw new UrlEscapeError(
      "Refusing to concatenate a root-relative literal; use toInternalPath instead",
    );
  }
  const segments = raw.split("/").filter((s) => s.length > 0);
  return toInternalPath(config, segments);
}
