/**
 * HTML escaping helpers used by page templates. Every user-controlled
 * string that lands in generated HTML must flow through these — the
 * pure page renderers never inject unsanitized data.
 */

export function escapeHtml(input: string): string {
  return input
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/** Attribute value context — same escape rules apply. */
export function escapeAttribute(input: string): string {
  return escapeHtml(input);
}
