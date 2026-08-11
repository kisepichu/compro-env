import {
  KNOWN_FILTER_KEYS,
  STATUS_VALUES,
  TYPE_VALUES,
  type FilterKey,
  type ParsedFilters,
  type ParsedQuery,
  type QueryError,
  type QueryParseResult,
  type StatusFilterValue,
  type TypeFilterValue,
} from "./types.ts";

const KNOWN_KEYS = new Set<string>(KNOWN_FILTER_KEYS);
const STATUS_SET = new Set<StatusFilterValue>(STATUS_VALUES);
const TYPE_SET = new Set<TypeFilterValue>(TYPE_VALUES);

interface Token {
  kind: "text" | "phrase" | "filter" | "unknown-key";
  key?: string;
  value: string;
  rawText: string;
  rawStart: number;
}

function emptyFilters(): ParsedFilters {
  return {
    lang: [],
    kind: [],
    path: [],
    status: [],
    type: [],
    verified_true: false,
    verified_false: false,
  };
}

function makeError(
  raw: string,
  code: QueryError["code"],
  message: string,
  position: number,
): QueryError {
  return { ok: false, raw, code, message, position };
}

function isSpace(ch: string): boolean {
  return ch === " " || ch === "\t" || ch === "\n" || ch === "\r";
}

interface QuotedRead {
  ok: true;
  value: string;
  end: number;
}

interface QuotedFail {
  ok: false;
  error: QueryError;
}

function readQuoted(input: string, start: number, raw: string): QuotedRead | QuotedFail {
  let i = start + 1;
  let value = "";
  while (i < input.length) {
    const ch = input[i];
    if (ch === "\\") {
      const next = input[i + 1];
      if (next === '"' || next === "\\") {
        value += next;
        i += 2;
        continue;
      }
      return {
        ok: false,
        error: makeError(
          raw,
          "invalid-escape",
          `Invalid escape at position ${i}. Only \\" and \\\\ are allowed inside quotes.`,
          i,
        ),
      };
    }
    if (ch === '"') {
      return { ok: true, value, end: i + 1 };
    }
    value += ch;
    i += 1;
  }
  return {
    ok: false,
    error: makeError(
      raw,
      "unterminated-quote",
      `Unterminated quote starting at position ${start}.`,
      start,
    ),
  };
}

function tokenize(input: string): Token[] | QueryError {
  const tokens: Token[] = [];
  let i = 0;
  while (i < input.length) {
    if (isSpace(input[i]!)) {
      i += 1;
      continue;
    }
    const tokenStart = i;
    if (input[i] === '"') {
      const quoted = readQuoted(input, i, input);
      if (!quoted.ok) return quoted.error;
      if (quoted.end < input.length && !isSpace(input[quoted.end]!)) {
        return makeError(
          input,
          "text-after-quote",
          `Unexpected text after closing quote at position ${quoted.end}.`,
          quoted.end,
        );
      }
      tokens.push({
        kind: "phrase",
        value: quoted.value,
        rawText: input.slice(tokenStart, quoted.end),
        rawStart: tokenStart,
      });
      i = quoted.end;
      continue;
    }
    let keyEnd = -1;
    let j = i;
    while (j < input.length && !isSpace(input[j]!)) {
      if (input[j] === ":" && keyEnd === -1) {
        keyEnd = j;
        break;
      }
      j += 1;
    }
    if (keyEnd !== -1) {
      const rawKey = input.slice(i, keyEnd);
      const normalizedKey = rawKey.toLowerCase();
      const afterColon = keyEnd + 1;
      const known = KNOWN_KEYS.has(normalizedKey);
      if (afterColon < input.length && input[afterColon] === '"') {
        const quoted = readQuoted(input, afterColon, input);
        if (!quoted.ok) return quoted.error;
        if (quoted.end < input.length && !isSpace(input[quoted.end]!)) {
          return makeError(
            input,
            "text-after-quote",
            `Unexpected text after closing quote at position ${quoted.end}.`,
            quoted.end,
          );
        }
        const rawText = input.slice(tokenStart, quoted.end);
        if (known) {
          tokens.push({
            kind: "filter",
            key: normalizedKey,
            value: quoted.value,
            rawText,
            rawStart: tokenStart,
          });
        } else {
          tokens.push({
            kind: "unknown-key",
            key: rawKey,
            value: rawText,
            rawText,
            rawStart: tokenStart,
          });
        }
        i = quoted.end;
        continue;
      }
      let k = afterColon;
      while (k < input.length && !isSpace(input[k]!)) k += 1;
      const value = input.slice(afterColon, k);
      const rawText = input.slice(tokenStart, k);
      if (known) {
        tokens.push({
          kind: "filter",
          key: normalizedKey,
          value,
          rawText,
          rawStart: tokenStart,
        });
      } else {
        tokens.push({
          kind: "unknown-key",
          key: rawKey,
          value: rawText,
          rawText,
          rawStart: tokenStart,
        });
      }
      i = k;
      continue;
    }
    let k = i;
    while (k < input.length && !isSpace(input[k]!)) k += 1;
    const rawText = input.slice(i, k);
    tokens.push({
      kind: "text",
      value: rawText,
      rawText,
      rawStart: tokenStart,
    });
    i = k;
  }
  return tokens;
}

function normalizeStatus(value: string): StatusFilterValue | null {
  const lower = value.toLowerCase();
  const mapped = lower === "not-configured" ? "not_configured" : lower;
  if (STATUS_SET.has(mapped as StatusFilterValue)) {
    return mapped as StatusFilterValue;
  }
  return null;
}

function normalizeType(value: string): TypeFilterValue | null {
  const lower = value.toLowerCase();
  if (TYPE_SET.has(lower as TypeFilterValue)) {
    return lower as TypeFilterValue;
  }
  return null;
}

function pushUnique<T>(list: T[], value: T): void {
  if (!list.includes(value)) list.push(value);
}

export function parseSearchQuery(input: string): QueryParseResult {
  const raw = input;
  const tokens = tokenize(input);
  if (!Array.isArray(tokens)) return tokens;

  const filters = emptyFilters();
  const textPieces: string[] = [];
  const textTokens: string[] = [];

  for (const token of tokens) {
    if (token.kind === "text" || token.kind === "phrase" || token.kind === "unknown-key") {
      textPieces.push(token.rawText);
      textTokens.push(token.value);
      continue;
    }
    const key = token.key as FilterKey;
    const value = token.value;
    if (value === "") {
      return makeError(
        raw,
        "empty-filter-value",
        `Filter '${key}:' requires a value.`,
        token.rawStart,
      );
    }
    const lowered = value.toLowerCase();
    if (key === "verified") {
      if (lowered === "true") {
        filters.verified_true = true;
        pushUnique(filters.status, "verified");
      } else if (lowered === "false") {
        filters.verified_false = true;
      } else {
        return makeError(
          raw,
          "invalid-boolean",
          `Filter 'verified:' expects true or false, got '${value}'.`,
          token.rawStart,
        );
      }
    } else if (key === "status") {
      const normalized = normalizeStatus(value);
      if (normalized === null) {
        return makeError(
          raw,
          "invalid-enum",
          `Filter 'status:' does not accept '${value}'.`,
          token.rawStart,
        );
      }
      pushUnique(filters.status, normalized);
    } else if (key === "type") {
      const normalized = normalizeType(value);
      if (normalized === null) {
        return makeError(
          raw,
          "invalid-enum",
          `Filter 'type:' does not accept '${value}'.`,
          token.rawStart,
        );
      }
      pushUnique(filters.type, normalized);
    } else if (key === "lang" || key === "kind" || key === "path") {
      pushUnique(filters[key], lowered);
    }
  }

  return {
    ok: true,
    raw,
    fullText: textPieces.join(" "),
    fullTextTokens: textTokens,
    filters,
  };
}

export function canonicalPage(input: string | null): number {
  if (input === null) return 1;
  if (!/^-?\d+$/.test(input)) return 1;
  const n = parseInt(input, 10);
  if (!Number.isFinite(n) || n < 1) return 1;
  return n;
}
