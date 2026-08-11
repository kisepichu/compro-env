export type StatusFilterValue =
  | "verified"
  | "rejected"
  | "unavailable"
  | "stale"
  | "never"
  | "not_configured";

export type TypeFilterValue = "library" | "solution";

export type FilterKey =
  | "lang"
  | "kind"
  | "path"
  | "verified"
  | "status"
  | "type";

export interface ParsedFilters {
  lang: string[];
  kind: string[];
  path: string[];
  status: StatusFilterValue[];
  type: TypeFilterValue[];
  verified_true: boolean;
  verified_false: boolean;
}

export interface ParsedQuery {
  ok: true;
  raw: string;
  fullText: string;
  fullTextTokens: string[];
  filters: ParsedFilters;
}

export interface QueryError {
  ok: false;
  raw: string;
  code:
    | "empty-filter-value"
    | "invalid-boolean"
    | "invalid-enum"
    | "unterminated-quote"
    | "invalid-escape"
    | "text-after-quote";
  message: string;
  position: number;
}

export type QueryParseResult = ParsedQuery | QueryError;

export const KNOWN_FILTER_KEYS: readonly FilterKey[] = [
  "lang",
  "kind",
  "path",
  "verified",
  "status",
  "type",
];

export const STATUS_VALUES: readonly StatusFilterValue[] = [
  "verified",
  "rejected",
  "unavailable",
  "stale",
  "never",
  "not_configured",
];

export const TYPE_VALUES: readonly TypeFilterValue[] = ["library", "solution"];
