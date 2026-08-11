/**
 * Markdown → sanitized HTML pipeline per spec §12, §12.5, §12.13.
 *
 * Pipeline:
 *   remark-parse → remark-gfm → validate mdast → remark-rehype
 *   → rehype-sanitize (customized) → assign heading IDs → rehype-stringify
 *
 * Enforced structural invariants (build errors, not silent drops):
 *   - `<h1>` in Markdown is rejected (page owns its `<h1>`).
 *   - Heading-level jumps deeper than +1 from the running level are rejected.
 *   - Image nodes are rejected — Rust generates static assets separately.
 *   - Raw HTML nodes are rejected (Markdown must not embed HTML).
 *
 * Sanitization narrows the default GitHub schema:
 *   - Only `http`, `https`, `mailto` are accepted as link schemes.
 *   - `id` is dropped from the `clobber` list so our `doc-*` heading anchors
 *     survive verbatim (the fragment scheme is `#doc-*` per spec §12.13).
 *   - Tag allowlist keeps h2-h6, p, ul, ol, li, code, pre, blockquote,
 *     strong, em, a, tables (GFM), task-list `<input>`, `<span>`, `<hr>`,
 *     `<br>`, `<del>`.
 */

import type { Root as HastRoot } from "hast";
import type { Root as MdastRoot } from "mdast";

import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import rehypeStringify from "rehype-stringify";
import { unified, type Plugin } from "unified";
import { visit } from "unist-util-visit";
import { toString as mdastToString } from "mdast-util-to-string";

import { computeHeadingId } from "./headings.ts";

export type MarkdownRenderErrorCode =
  | "h1_disallowed_in_markdown"
  | "heading_level_jump"
  | "image_not_supported"
  | "raw_html_not_supported";

export class MarkdownRenderError extends Error {
  readonly code: MarkdownRenderErrorCode;
  constructor(code: MarkdownRenderErrorCode, message?: string) {
    super(message ?? code);
    this.name = "MarkdownRenderError";
    this.code = code;
  }
}

export interface RenderMarkdownOptions {
  /** Reserved for future overrides (e.g. protocols allowlist). Task 3 has none. */
  readonly _reserved?: never;
}

export interface RenderMarkdownResult {
  readonly html: string;
  /** Ordered list of `doc-*` IDs assigned to h2-h6, in document order. */
  readonly anchors: string[];
}

// ---- Custom sanitize schema (narrower than defaultSchema) ----

/**
 * Allow list mirrors the GitHub schema minus everything we don't need.
 * Notably drops: `img`, `picture`, `source`, `iframe`, `script`, `div`,
 * `section` (page code owns sections), `h1` (owned by the page).
 */
const ALLOWED_TAG_NAMES = [
  "a",
  "blockquote",
  "br",
  "code",
  "del",
  "em",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "hr",
  "input", // GFM task-list checkbox
  "li",
  "ol",
  "p",
  "pre",
  "span",
  "strong",
  "sup",
  "sub",
  "table",
  "tbody",
  "td",
  "tfoot",
  "th",
  "thead",
  "tr",
  "ul",
];

const buildSanitizeSchema = () => {
  const schema = {
    ...defaultSchema,
    tagNames: ALLOWED_TAG_NAMES,
    protocols: {
      ...(defaultSchema.protocols ?? {}),
      href: ["http", "https", "mailto"],
      cite: ["http", "https"],
    },
    // Drop `id` from clobber so `doc-*` heading anchors remain intact.
    clobber: (defaultSchema.clobber ?? []).filter((c) => c !== "id"),
  };
  return schema;
};

const SANITIZE_SCHEMA = buildSanitizeSchema();

// ---- Validation plugin (runs on mdast) ----

const validateMdast: Plugin<[], MdastRoot> = () => {
  return (tree) => {
    let currentLevel = 1; // Virtual <h1> owned by the page.
    visit(tree, (node) => {
      if (node.type === "image" || node.type === "imageReference") {
        throw new MarkdownRenderError(
          "image_not_supported",
          "Markdown image syntax is not supported in library documentation.",
        );
      }
      if (node.type === "html") {
        throw new MarkdownRenderError(
          "raw_html_not_supported",
          "Raw HTML is not permitted in library documentation.",
        );
      }
      if (node.type === "heading") {
        const depth = (node as { depth: number }).depth;
        if (depth === 1) {
          throw new MarkdownRenderError(
            "h1_disallowed_in_markdown",
            "Documentation must not include a level-1 heading; the page owns the <h1>.",
          );
        }
        if (depth > currentLevel + 1) {
          throw new MarkdownRenderError(
            "heading_level_jump",
            `Heading level ${depth} jumps more than +1 from the running level ${currentLevel}.`,
          );
        }
        currentLevel = depth;
      }
    });
  };
};

// ---- Attach IDs to h2-h6 in document order (runs on hast, after sanitize) ----

const attachHeadingIds =
  (recordInto: string[]): Plugin<[], HastRoot> =>
  () => {
    return (tree) => {
      const seen = new Map<string, number>();
      visit(tree, "element", (node) => {
        const name = (node as { tagName: string }).tagName;
        if (
          name !== "h2" &&
          name !== "h3" &&
          name !== "h4" &&
          name !== "h5" &&
          name !== "h6"
        ) {
          return;
        }
        const text = mdastToString(node as unknown);
        const id = computeHeadingId(text, seen);
        // hast property name for `id` is `id`.
        const properties =
          (node as { properties?: Record<string, unknown> }).properties ??
          ({} as Record<string, unknown>);
        properties.id = id;
        (node as { properties?: Record<string, unknown> }).properties =
          properties;
        recordInto.push(id);
      });
    };
  };

// ---- Public API ----

export function renderMarkdown(
  input: string,
  _opts?: RenderMarkdownOptions,
): RenderMarkdownResult {
  const anchors: string[] = [];
  const processor = unified()
    .use(remarkParse)
    .use(remarkGfm)
    .use(validateMdast)
    .use(remarkRehype, { allowDangerousHtml: false })
    .use(rehypeSanitize, SANITIZE_SCHEMA)
    .use(attachHeadingIds(anchors))
    .use(rehypeStringify);
  const file = processor.processSync(input);
  return { html: String(file), anchors };
}

/**
 * Render library documentation Markdown into the `<div id="documentation">`
 * wrapper used by library detail pages. Returns just the HTML string; the
 * caller composes the surrounding sections.
 */
export function renderDocumentation(description: string): string {
  const { html } = renderMarkdown(description);
  return `<div id="documentation" class="documentation">${html}</div>`;
}
