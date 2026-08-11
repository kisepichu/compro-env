/**
 * Source-code rendering with Shiki per spec §12.5, §12.7, §12.11 and
 * the semantic handoff §13.
 *
 * Contract:
 *   - Every public source is rendered whole. No truncation, wrapping,
 *     or virtualization.
 *   - The output shape is a `<section id="source">` containing a
 *     `<h2>`, optional notes (e.g. the solution preprocess disclaimer),
 *     a `<div class="source-toolbar">`, and a `<pre><code>...</code></pre>`.
 *   - Every line — including blank ones — is a `<span id="L{n}"
 *     class="source-line" data-line="{n}">` with an inner
 *     `<a class="source-line-number">` before the content span, and
 *     the two adjacent `<span class="source-line">` elements are joined
 *     by a literal newline so `pre`'s whitespace preservation gives a
 *     copy-paste-friendly result.
 *   - Line numbers are 1-based. Trailing `\n` is a line terminator, not
 *     an extra blank line. CRLF is normalized to LF.
 *   - Byte-length gate: > 256 KiB → warning; > 2 MiB in production
 *     mode → hard `SourceRenderError('source_too_large')`.
 *   - Unknown / unloadable language falls back to plain-text tokens and
 *     records a warning (does NOT throw).
 */

import type { ThemedToken } from "@shikijs/types";
import { createHighlighter, type BundledLanguage, type Highlighter } from "shiki";

import { escapeAttribute, escapeHtml } from "./pages/escape.ts";
import { sanitizeExternalUrl } from "./safe-url.ts";

// ---- Errors ----

export type SourceRenderErrorCode = "source_too_large";

export class SourceRenderError extends Error {
  readonly code: SourceRenderErrorCode;
  constructor(code: SourceRenderErrorCode, message?: string) {
    super(message ?? code);
    this.name = "SourceRenderError";
    this.code = code;
  }
}

// ---- Constants ----

const SIZE_WARNING_BYTES = 256 * 1024;
const SIZE_HARD_LIMIT_BYTES = 2 * 1024 * 1024;
const THEME = "github-light";

/**
 * Languages we eagerly pre-load so the very first render is warm.
 * Additional languages are lazy-loaded via `loadLanguage`.
 */
const EAGER_LANGS: BundledLanguage[] = [
  "rust",
  "cpp",
  "c",
  "python",
  "javascript",
  "typescript",
  "go",
  "java",
  "kotlin",
  "lean",
];

// ---- Shiki singleton ----

let highlighterPromise: Promise<Highlighter> | null = null;

async function getHighlighter(): Promise<Highlighter> {
  if (highlighterPromise === null) {
    highlighterPromise = createHighlighter({
      themes: [THEME],
      langs: EAGER_LANGS,
    });
  }
  return highlighterPromise;
}

/**
 * Ensure a language is loaded; return true on success. Never throws — a
 * failure translates into fallback plain-text rendering upstream.
 */
async function ensureLanguageLoaded(lang: string): Promise<boolean> {
  const highlighter = await getHighlighter();
  if (highlighter.getLoadedLanguages().includes(lang)) return true;
  try {
    await highlighter.loadLanguage(lang as BundledLanguage);
    return highlighter.getLoadedLanguages().includes(lang);
  } catch {
    return false;
  }
}

// ---- Types ----

export interface RenderSourceOptions {
  readonly source: string;
  readonly syntaxHighlight: string;
  readonly sourcePath: string;
  readonly repositoryUrl?: string | null;
  readonly commitSha?: string | null;
  readonly mode?: "production" | "preview";
  /**
   * HTML inserted inside the `<section>` right after the `<h2>` and
   * before the toolbar — used by the solution detail page to surface the
   * "displayed source is the pre-preprocess entry file" disclaimer.
   */
  readonly notesHtml?: string;
}

export interface RenderSourceResult {
  readonly html: string;
  readonly warnings: string[];
}

// ---- Line splitting ----

/**
 * Normalize newlines, split on `\n`, and drop the empty entry created by
 * a trailing newline (per spec: `\n` is a terminator, not a blank line).
 */
export function splitSourceLines(source: string): string[] {
  if (source.length === 0) return [];
  const normalized = source.replace(/\r\n/g, "\n");
  const lines = normalized.split("\n");
  if (normalized.endsWith("\n") && lines.length > 0) {
    lines.pop();
  }
  return lines;
}

// ---- Rendering ----

function utf8ByteLength(input: string): number {
  return Buffer.byteLength(input, "utf8");
}

function renderTokensShiki(tokens: readonly ThemedToken[]): string {
  return tokens
    .map((t) => {
      const color = t.color;
      const style =
        typeof color === "string" && color.length > 0
          ? ` style="color:${escapeAttribute(color)}"`
          : "";
      return `<span${style}>${escapeHtml(t.content)}</span>`;
    })
    .join("");
}

function renderLine(
  lineNumber: number,
  contentHtml: string,
): string {
  const anchor =
    `<a class="source-line-number" href="#L${lineNumber}"` +
    ` aria-label="Line ${lineNumber}" data-pagefind-ignore>${lineNumber}</a>`;
  const contentSpan = `<span class="source-line-content">${contentHtml}</span>`;
  return (
    `<span id="L${lineNumber}" class="source-line" data-line="${lineNumber}">` +
      anchor +
      contentSpan +
    `</span>`
  );
}

function renderToolbar(opts: RenderSourceOptions): string {
  const langLabel = `<span class="language">${escapeHtml(opts.syntaxHighlight)}</span>`;
  const pathLabel = `<code class="path">${escapeHtml(opts.sourcePath)}</code>`;
  const safeRepoBase = sanitizeExternalUrl(opts.repositoryUrl, {
    stripTrailingSlash: true,
  });
  let repoLink = "";
  if (
    safeRepoBase !== null &&
    opts.commitSha !== null &&
    opts.commitSha !== undefined &&
    opts.commitSha.length > 0
  ) {
    const url =
      `${safeRepoBase}/blob/${encodeURIComponent(opts.commitSha)}/` +
      opts.sourcePath
        .split("/")
        .filter((p) => p.length > 0)
        .map((p) => encodeURIComponent(p))
        .join("/");
    repoLink =
      ` <a href="${escapeAttribute(url)}" rel="noopener noreferrer">` +
        `Repository source` +
      `</a>`;
  }
  return (
    `<div class="source-toolbar" data-pagefind-ignore>` +
      langLabel +
      ` ` +
      pathLabel +
      repoLink +
    `</div>`
  );
}

/**
 * Render a source file into a full `<section id="source">` block. Async
 * because Shiki loads highlighting grammars on demand.
 */
export async function renderSource(
  opts: RenderSourceOptions,
): Promise<RenderSourceResult> {
  const warnings: string[] = [];
  const bytes = utf8ByteLength(opts.source);
  const mode = opts.mode ?? "preview";
  if (mode === "production" && bytes > SIZE_HARD_LIMIT_BYTES) {
    throw new SourceRenderError(
      "source_too_large",
      `Source ${JSON.stringify(opts.sourcePath)} is ${bytes} bytes; the hard limit is ${SIZE_HARD_LIMIT_BYTES} bytes.`,
    );
  }
  if (bytes > SIZE_WARNING_BYTES) {
    warnings.push(
      `Source ${JSON.stringify(opts.sourcePath)} is ${bytes} bytes; exceeds ${SIZE_WARNING_BYTES}-byte soft limit.`,
    );
  }

  const lines = splitSourceLines(opts.source);

  // Attempt to load the language; fall back to plain text on failure.
  let tokensPerLine: ThemedToken[][] | null = null;
  const langOk = await ensureLanguageLoaded(opts.syntaxHighlight);
  if (!langOk) {
    warnings.push(
      `Language ${JSON.stringify(opts.syntaxHighlight)} is not supported by the highlighter; ` +
        `falling back to plain text for ${JSON.stringify(opts.sourcePath)}.`,
    );
  } else if (lines.length > 0) {
    const highlighter = await getHighlighter();
    try {
      const joined = lines.join("\n");
      tokensPerLine = highlighter.codeToTokensBase(joined, {
        lang: opts.syntaxHighlight as BundledLanguage,
        theme: THEME,
        includeExplanation: false,
      });
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err);
      warnings.push(
        `Syntax highlighting failed for ${JSON.stringify(opts.sourcePath)}: ${detail}; falling back to plain text.`,
      );
      tokensPerLine = null;
    }
  }

  const contentHtmls = lines.map((rawLine, index) => {
    const tokens = tokensPerLine !== null ? tokensPerLine[index] : undefined;
    if (tokens === undefined || tokens.length === 0) {
      return escapeHtml(rawLine);
    }
    return renderTokensShiki(tokens);
  });

  const lineHtmls: string[] = contentHtmls.map((c, i) => renderLine(i + 1, c));

  const toolbar = renderToolbar(opts);
  const notesHtml = opts.notesHtml ?? "";
  const codeInner = lineHtmls.join("\n");
  const langAttr = escapeAttribute(opts.syntaxHighlight);
  const html =
    `<section id="source" aria-labelledby="source-heading">` +
      `<h2 id="source-heading">Source</h2>` +
      notesHtml +
      toolbar +
      `<pre class="source-code" tabindex="0" aria-labelledby="source-heading">` +
        `<code data-language="${langAttr}">${codeInner}</code>` +
      `</pre>` +
    `</section>`;

  return { html, warnings };
}

/**
 * Alias for {@link renderSource}. Kept for symmetry with the plan's
 * `renderSourceBlock` name; both return the full `<section id="source">`.
 */
export const renderSourceBlock = renderSource;
