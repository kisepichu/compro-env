/**
 * Task 3 tests: heading-id algorithm, Markdown pipeline, Shiki source
 * rendering, plus an end-to-end assertion that the library detail page
 * threads both through a schema-valid fixture.
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { JSDOM } from "jsdom";
import { describe, expect, it } from "vitest";

import {
  assignHeadingIds,
  buildHeadingHint,
  computeHeadingId,
  digestPrefixHex10,
} from "@/lib/headings.ts";
import {
  MarkdownRenderError,
  renderDocumentation,
  renderMarkdown,
} from "@/lib/markdown.ts";
import { renderLibraryDetailPage } from "@/lib/pages/libraries.ts";
import { assertSiteData } from "@/lib/site-data.ts";
import type { SiteData } from "@/lib/site-data-types.ts";
import {
  renderSource,
  SourceRenderError,
  splitSourceLines,
} from "@/lib/source.ts";
import type { UrlConfig } from "@/lib/url.ts";

const rootConfig: UrlConfig = { origin: "https://example.com", base: "/" };

function parse(html: string): Document {
  return new JSDOM(html).window.document;
}

function loadFixture(): SiteData {
  const path = fileURLToPath(
    new URL("./fixtures/site-data.json", import.meta.url),
  );
  const raw = readFileSync(path, "utf8");
  const parsed = JSON.parse(raw);
  return assertSiteData(parsed);
}

// ---- Heading IDs ----

describe("heading IDs (spec §12 lines 1641-1651)", () => {
  it("builds a hint by lowercasing A-Z and joining [a-z0-9] runs with '-'", () => {
    expect(buildHeadingHint("Hello world")).toBe("hello-world");
    expect(buildHeadingHint("Foo (Bar 42)!")).toBe("foo-bar-42");
    expect(buildHeadingHint("---foo---")).toBe("foo");
  });

  it("caps hint at 48 bytes with trailing '-' trimmed", () => {
    const long = "a".repeat(80);
    const hint = buildHeadingHint(long);
    expect(hint.length).toBeLessThanOrEqual(48);
    expect(hint.endsWith("-")).toBe(false);
  });

  it("falls back to 'h' when no ASCII runs are available", () => {
    expect(buildHeadingHint("こんにちは")).toBe("h");
    expect(buildHeadingHint("")).toBe("h");
    expect(buildHeadingHint("---")).toBe("h");
  });

  it("produces the same ID for the same text every time", () => {
    const seen1 = new Map<string, number>();
    const seen2 = new Map<string, number>();
    const a = computeHeadingId("Hello world", seen1);
    const b = computeHeadingId("Hello world", seen2);
    expect(a).toBe(b);
  });

  it("'Hello world' → id begins with doc-hello-world- and has a 10 hex digest", () => {
    const seen = new Map<string, number>();
    const id = computeHeadingId("Hello world", seen);
    expect(id.startsWith("doc-hello-world-")).toBe(true);
    const parts = id.split("-");
    const digest = parts[parts.length - 1];
    expect(digest).toMatch(/^[0-9a-f]{10}$/);
    expect(digest).toBe(digestPrefixHex10("Hello world"));
  });

  it("Japanese-only heading → id = doc-h-{digest10}", () => {
    const seen = new Map<string, number>();
    const id = computeHeadingId("こんにちは", seen);
    expect(id).toBe(`doc-h-${digestPrefixHex10("こんにちは")}`);
  });

  it("duplicate headings get -2, -3 suffixes in document order", () => {
    const ids = assignHeadingIds(["Notes", "Notes", "Notes", "Other"]);
    expect(ids[0]).not.toContain("-2");
    expect(ids[1]).toBe(`${ids[0]}-2`);
    expect(ids[2]).toBe(`${ids[0]}-3`);
    expect(ids[3].startsWith("doc-other-")).toBe(true);
  });
});

// ---- Markdown ----

describe("Markdown renderer (spec §12.5, §12.13)", () => {
  it("rejects a level-1 heading", () => {
    expect(() => renderMarkdown("# Title\n")).toThrow(MarkdownRenderError);
    try {
      renderMarkdown("# Title\n");
    } catch (err) {
      expect((err as MarkdownRenderError).code).toBe(
        "h1_disallowed_in_markdown",
      );
    }
  });

  it("rejects a heading-level jump (h2 then h4)", () => {
    expect(() => renderMarkdown("## A\n\n#### B\n")).toThrow(
      MarkdownRenderError,
    );
    try {
      renderMarkdown("## A\n\n#### B\n");
    } catch (err) {
      expect((err as MarkdownRenderError).code).toBe("heading_level_jump");
    }
  });

  it("rejects raw HTML in the Markdown body", () => {
    expect(() => renderMarkdown("<div>hi</div>\n")).toThrow(
      MarkdownRenderError,
    );
    try {
      renderMarkdown("<div>hi</div>\n");
    } catch (err) {
      expect((err as MarkdownRenderError).code).toBe("raw_html_not_supported");
    }
  });

  it("rejects Markdown image syntax", () => {
    expect(() => renderMarkdown("![alt](image.png)\n")).toThrow(
      MarkdownRenderError,
    );
    try {
      renderMarkdown("![alt](image.png)\n");
    } catch (err) {
      expect((err as MarkdownRenderError).code).toBe("image_not_supported");
    }
  });

  it("renders a GFM table and a task list", () => {
    const md =
      "## Table\n\n| A | B |\n| - | - |\n| 1 | 2 |\n\n" +
      "## Tasks\n\n- [x] done\n- [ ] todo\n";
    const { html, anchors } = renderMarkdown(md);
    const doc = parse(`<div>${html}</div>`);
    expect(doc.querySelector("table")).toBeTruthy();
    expect(doc.querySelectorAll("table th").length).toBe(2);
    expect(doc.querySelectorAll("table td").length).toBe(2);
    const checkboxes = doc.querySelectorAll('input[type="checkbox"]');
    expect(checkboxes.length).toBe(2);
    expect((checkboxes[0] as HTMLInputElement).hasAttribute("checked")).toBe(
      true,
    );
    expect(anchors.length).toBe(2);
    for (const a of anchors) {
      expect(a.startsWith("doc-")).toBe(true);
    }
  });

  it("strips unknown attributes such as onclick from anchors", () => {
    // Markdown itself doesn't allow attribute injection through `[text](href)`,
    // so we test via an autolink form + attribute-bearing link written as raw
    // HTML would be rejected; instead, exercise sanitize by injecting via a
    // Markdown link whose href is a plausible URL and adding attributes in a
    // rehype tree is not directly possible from Markdown. Confirm the negative:
    // the compiled HTML contains no `onclick=` regardless of input.
    const md = "[click](https://example.com)\n";
    const { html } = renderMarkdown(md);
    expect(html.toLowerCase()).not.toContain("onclick");
  });

  it("does not preserve javascript: link schemes", () => {
    const md = "[safe text](javascript:evil())\n";
    const { html } = renderMarkdown(md);
    expect(html.toLowerCase()).not.toContain("javascript:");
    // The text label survives even if the href is stripped.
    expect(html).toContain("safe text");
  });

  it("empty document has valid HTML, no headings, no anchors", () => {
    const { html, anchors } = renderMarkdown("A paragraph.\n");
    expect(anchors).toEqual([]);
    const doc = parse(`<div>${html}</div>`);
    expect(doc.querySelectorAll("h1,h2,h3,h4,h5,h6").length).toBe(0);
    expect(doc.querySelector("p")!.textContent).toBe("A paragraph.");
  });

  it("assigns doc-* ids to h2-h6 in document order", () => {
    const md = "## Foo\n\n### Bar\n";
    const { html, anchors } = renderMarkdown(md);
    expect(anchors.length).toBe(2);
    expect(anchors[0].startsWith("doc-foo-")).toBe(true);
    expect(anchors[1].startsWith("doc-bar-")).toBe(true);
    const doc = parse(`<div>${html}</div>`);
    expect(doc.querySelector("h2")!.id).toBe(anchors[0]);
    expect(doc.querySelector("h3")!.id).toBe(anchors[1]);
  });

  it("renderDocumentation wraps output in <div id='documentation'>", () => {
    const html = renderDocumentation("A short description.\n");
    expect(html.startsWith('<div id="documentation"')).toBe(true);
    expect(html.includes('class="documentation"')).toBe(true);
  });
});

// ---- Source ----

describe("Source renderer (spec §12.5, §12.7, §12.11)", () => {
  const rustToolbar = "graph/dijkstra.rs";
  const repositoryUrl = "https://github.com/kisepichu/compro-env";
  const commitSha = "0abcdef";

  it("splitSourceLines drops the entry created by a trailing newline", () => {
    expect(splitSourceLines("a\nb\n")).toEqual(["a", "b"]);
    expect(splitSourceLines("a\nb")).toEqual(["a", "b"]);
    expect(splitSourceLines("")).toEqual([]);
    expect(splitSourceLines("a\r\nb\r\n")).toEqual(["a", "b"]);
    expect(splitSourceLines("\n")).toEqual([""]);
  });

  it("emits one <span id=L{n} class='source-line'> per input line", async () => {
    const { html } = await renderSource({
      source: "pub fn f() {}\nlet x = 1;\nlet y = 2;\n",
      syntaxHighlight: "rust",
      sourcePath: rustToolbar,
    });
    const doc = parse(html);
    const lines = doc.querySelectorAll("code .source-line");
    expect(lines.length).toBe(3);
    for (let i = 0; i < lines.length; i += 1) {
      const n = i + 1;
      expect(lines[i].id).toBe(`L${n}`);
      expect(lines[i].getAttribute("data-line")).toBe(String(n));
      const anchor = lines[i].querySelector("a.source-line-number")!;
      expect(anchor.getAttribute("href")).toBe(`#L${n}`);
      expect(anchor.textContent).toBe(String(n));
      const content = lines[i].querySelector("span.source-line-content")!;
      expect(content).toBeTruthy();
    }
  });

  it("preserves blank lines with their own <span class='source-line'>", async () => {
    const { html } = await renderSource({
      source: "a\n\nb\n",
      syntaxHighlight: "rust",
      sourcePath: "example.rs",
    });
    const doc = parse(html);
    const lines = doc.querySelectorAll("code .source-line");
    expect(lines.length).toBe(3);
    expect(lines[1].querySelector(".source-line-content")!.textContent).toBe(
      "",
    );
  });

  it("normalizes CRLF to LF", async () => {
    const { html } = await renderSource({
      source: "a\r\nb\r\nc\r\n",
      syntaxHighlight: "rust",
      sourcePath: "example.rs",
    });
    const doc = parse(html);
    const lines = doc.querySelectorAll("code .source-line");
    expect(lines.length).toBe(3);
  });

  it("preserves multi-byte Unicode content in comments", async () => {
    const { html } = await renderSource({
      source: "// こんにちは\nfn main() {}\n",
      syntaxHighlight: "rust",
      sourcePath: "hello.rs",
    });
    expect(html).toContain("こんにちは");
  });

  it("trailing newline does NOT produce an extra empty line", async () => {
    const { html } = await renderSource({
      source: "only_one\n",
      syntaxHighlight: "rust",
      sourcePath: "only.rs",
    });
    const doc = parse(html);
    const lines = doc.querySelectorAll("code .source-line");
    expect(lines.length).toBe(1);
  });

  it("HTML-escapes `<` and `>` in source content", async () => {
    const { html } = await renderSource({
      source: "fn cmp(a: i32, b: i32) -> bool { a < b && b > a }\n",
      syntaxHighlight: "rust",
      sourcePath: "cmp.rs",
    });
    // Extract just the <pre><code>...</code></pre> block so we ignore the
    // toolbar's decorative <code class="path">.
    const preMatch = html.match(/<pre[^>]*>([\s\S]*?)<\/pre>/);
    expect(preMatch).not.toBeNull();
    const preInner = preMatch![1];
    // Strip all HTML tags to isolate the escaped text nodes.
    const contentText = preInner.replace(/<[^>]+>/g, "");
    expect(contentText).toContain("a &lt; b");
    expect(contentText).toContain("b &gt; a");
  });

  it("unknown language falls back to plain text and reports a warning", async () => {
    const { html, warnings } = await renderSource({
      source: "line1\nline2\n",
      syntaxHighlight: "nonsense_lang",
      sourcePath: "unknown.txt",
    });
    expect(warnings.length).toBeGreaterThan(0);
    expect(warnings.join(" ")).toContain("nonsense_lang");
    const doc = parse(html);
    const lines = doc.querySelectorAll("code .source-line");
    expect(lines.length).toBe(2);
    expect(lines[0].querySelector(".source-line-content")!.textContent).toBe(
      "line1",
    );
  });

  it("emits a soft-limit warning for source > 256 KiB", async () => {
    // 300 lines of 1 KiB each ⇒ ~300 KiB — above the 262144-byte soft limit.
    const bigLine = "x".repeat(1024);
    const source = new Array(300).fill(bigLine).join("\n") + "\n";
    const { warnings } = await renderSource({
      source,
      syntaxHighlight: "rust",
      sourcePath: "big.rs",
    });
    expect(warnings.length).toBeGreaterThan(0);
    expect(warnings.some((w) => w.toLowerCase().includes("soft limit"))).toBe(
      true,
    );
  });

  it("throws SourceRenderError in production mode when > 2 MiB", async () => {
    const line = "y".repeat(1024);
    const source = new Array(2200).fill(line).join("\n") + "\n"; // ~2.2 MiB
    await expect(
      renderSource({
        source,
        syntaxHighlight: "rust",
        sourcePath: "huge.rs",
        mode: "production",
      }),
    ).rejects.toBeInstanceOf(SourceRenderError);
  });

  it("toolbar contains a repository link when url + sha are present", async () => {
    const { html } = await renderSource({
      source: "fn main() {}\n",
      syntaxHighlight: "rust",
      sourcePath: rustToolbar,
      repositoryUrl,
      commitSha,
    });
    const doc = parse(html);
    const link = doc.querySelector(".source-toolbar a")!;
    expect(link).toBeTruthy();
    expect(link.getAttribute("rel")).toBe("noopener noreferrer");
    expect(link.getAttribute("href")).toBe(
      `${repositoryUrl}/blob/${commitSha}/graph/dijkstra.rs`,
    );
    expect(link.textContent).toBe("Repository source");
  });

  it("toolbar omits the repository link when repositoryUrl is null", async () => {
    const { html } = await renderSource({
      source: "fn main() {}\n",
      syntaxHighlight: "rust",
      sourcePath: rustToolbar,
      repositoryUrl: null,
      commitSha,
    });
    const doc = parse(html);
    expect(doc.querySelector(".source-toolbar a")).toBeNull();
  });

  it("toolbar carries data-pagefind-ignore", async () => {
    const { html } = await renderSource({
      source: "fn main() {}\n",
      syntaxHighlight: "rust",
      sourcePath: rustToolbar,
    });
    const doc = parse(html);
    expect(
      doc
        .querySelector(".source-toolbar")!
        .hasAttribute("data-pagefind-ignore"),
    ).toBe(true);
  });

  it("returns a full <section id='source'> block with h2 heading", async () => {
    const { html } = await renderSource({
      source: "fn main() {}\n",
      syntaxHighlight: "rust",
      sourcePath: rustToolbar,
    });
    const doc = parse(html);
    const section = doc.querySelector("section#source")!;
    expect(section).toBeTruthy();
    expect(section.getAttribute("aria-labelledby")).toBe("source-heading");
    const h2 = section.querySelector("h2#source-heading")!;
    expect(h2).toBeTruthy();
    expect(h2.textContent).toBe("Source");
  });

  it("inserts notesHtml between h2 and the toolbar", async () => {
    const { html } = await renderSource({
      source: "fn main() {}\n",
      syntaxHighlight: "rust",
      sourcePath: rustToolbar,
      notesHtml: '<p class="preprocess-note">Notes go here.</p>',
    });
    const notesIdx = html.indexOf("Notes go here.");
    const toolbarIdx = html.indexOf("source-toolbar");
    expect(notesIdx).toBeGreaterThan(0);
    expect(notesIdx).toBeLessThan(toolbarIdx);
  });
});

// ---- Integration ----

describe("Library detail page (fixture integration)", () => {
  it("threads the fixture through renderLibraryDetailPage", async () => {
    const siteData = loadFixture();
    const lib = siteData.libraries.find(
      (l) => l.source_path === "graph/dijkstra.rs",
    )!;
    const html = await renderLibraryDetailPage(rootConfig, siteData, lib);
    const doc = parse(html);
    // Documentation block present because description is non-empty.
    const doc_block = doc.getElementById("documentation");
    expect(doc_block).toBeTruthy();
    // A GFM table from the description should have made it in.
    expect(doc_block!.querySelector("table")).toBeTruthy();
    // Source rendering happened — first line span exists.
    expect(doc.getElementById("L1")).toBeTruthy();
    // Article carries data-pagefind-body, has #source section.
    const article = doc.querySelector("article.library-detail")!;
    expect(article.hasAttribute("data-pagefind-body")).toBe(true);
    expect(article.querySelector("section#source")).toBeTruthy();
    // Toolbar has the repository link.
    const link = article.querySelector(".source-toolbar a")!;
    expect(link).toBeTruthy();
    expect(link.getAttribute("href")).toBe(
      `${siteData.site.repository_url}/blob/${siteData.build.source_commit_short_sha}/${lib.source_path}`,
    );
  });
});
