/**
 * Schema-validated loader for `site-data.json`.
 *
 * The static build refuses to run against an input whose declared
 * `schema_version` differs from the version this build was compiled
 * against, and refuses any input that fails JSON-Schema validation.
 * Fixture JSONs used in tests share the same schema.
 */

import { readFileSync } from "node:fs";
import { resolve as resolvePath } from "node:path";

import Ajv2020, { type ErrorObject, type ValidateFunction } from "ajv/dist/2020.js";
import addFormats from "ajv-formats";

import siteDataSchema from "../../schema/site-data-v1.schema.json" with { type: "json" };
import type {
  AnalysisState,
  DiagnosticSeverity,
  EvidenceStatus,
  LibraryVerificationStatus,
  SolutionVerificationStatus,
  SiteData,
} from "./site-data-types.ts";

export const SUPPORTED_SCHEMA_VERSION = 1 as const;

export class SiteDataSchemaError extends Error {
  constructor(
    message: string,
    readonly issues: readonly string[] = [],
  ) {
    super(
      issues.length === 0 ? message : `${message}\n${issues.join("\n")}`,
    );
    this.name = "SiteDataSchemaError";
  }
}

let cachedValidator: ValidateFunction | null = null;

function getValidator(): ValidateFunction {
  if (cachedValidator !== null) return cachedValidator;
  const ajv = new Ajv2020({
    allErrors: true,
    strict: false,
    allowUnionTypes: true,
  });
  addFormats(ajv);
  cachedValidator = ajv.compile(siteDataSchema as object);
  return cachedValidator;
}

function formatErrors(errors: readonly ErrorObject[] | null): string[] {
  if (errors === null) return [];
  return errors.map((err) => {
    const at = err.instancePath === "" ? "<root>" : err.instancePath;
    const params = err.params as Record<string, unknown> | undefined;
    const extra =
      err.keyword === "additionalProperties" && params !== undefined
        ? ` (offending key: ${String(params.additionalProperty ?? "?")})`
        : "";
    return `${at}: ${err.message ?? "invalid"}${extra}`;
  });
}

/**
 * Validate a raw parsed JSON value and return it as a strongly-typed
 * `SiteData`. Throws {@link SiteDataSchemaError} on schema-version mismatch
 * or JSON Schema violations.
 */
export function assertSiteData(value: unknown): SiteData {
  if (
    value === null ||
    typeof value !== "object" ||
    Array.isArray(value)
  ) {
    throw new SiteDataSchemaError("site-data.json must be a JSON object");
  }
  const declared = (value as Record<string, unknown>).schema_version;
  if (typeof declared !== "number" || !Number.isInteger(declared)) {
    throw new SiteDataSchemaError(
      "site-data.json is missing an integer schema_version",
    );
  }
  if (declared !== SUPPORTED_SCHEMA_VERSION) {
    throw new SiteDataSchemaError(
      `Unsupported site-data schema_version ${declared} — this build only supports ${SUPPORTED_SCHEMA_VERSION}`,
    );
  }
  const validator = getValidator();
  if (!validator(value)) {
    throw new SiteDataSchemaError(
      "site-data.json failed JSON Schema validation",
      formatErrors(validator.errors ?? null),
    );
  }
  return value as SiteData;
}

/**
 * Read the site-data JSON pointed to by `CE_SITE_DATA_PATH`, or an explicit
 * path when provided. Repository directories default to
 * `target/ce-site-data/site-data.json`.
 */
export function loadSiteData(explicitPath?: string): SiteData {
  const path =
    explicitPath ??
    process.env.CE_SITE_DATA_PATH ??
    resolvePath(process.cwd(), "target", "ce-site-data", "site-data.json");
  const raw = readFileSync(path, "utf8");
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new SiteDataSchemaError(
      `site-data.json at ${path} is not valid JSON: ${detail}`,
    );
  }
  return assertSiteData(parsed);
}

export {
  type AnalysisState,
  type DiagnosticSeverity,
  type EvidenceStatus,
  type LibraryVerificationStatus,
  type SolutionVerificationStatus,
  type SiteData,
};
