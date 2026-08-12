/**
 * Status badge renderer per spec §12.8.
 *
 * Static badges never receive `role="alert"` or a live-region attribute.
 * The `data-status` value is the shared enum string; the label text is
 * always emitted so colour is never the sole signal.
 */

import type {
  AnalysisState,
  EvidenceStatus,
  LibraryVerificationStatus,
  SolutionVerificationStatus,
} from "../site-data-types.ts";
import { escapeAttribute, escapeHtml } from "./escape.ts";

export type StatusVariant =
  | "library-verification"
  | "solution-verification"
  | "evidence"
  | "analysis";

export type StatusValue =
  | LibraryVerificationStatus
  | SolutionVerificationStatus
  | EvidenceStatus
  | AnalysisState;

const VERIFICATION_LABELS: Record<SolutionVerificationStatus, string> = {
  verified: "Verified",
  rejected: "Rejected",
  unavailable: "Unavailable",
  stale: "Stale",
  never: "Never verified",
  not_configured: "Verification not configured",
};

const ANALYSIS_LABELS: Record<AnalysisState, string> = {
  complete: "Analysis complete",
  partial: "Analysis partial",
  failed: "Analysis failed",
};

const STATUS_SHAPES: Record<StatusValue, string> = {
  verified: `<circle cx="8" cy="8" r="5"></circle>`,
  rejected: `<rect x="3" y="3" width="10" height="10"></rect>`,
  stale: `<path d="M8 2 14 8 8 14 2 8Z"></path>`,
  unavailable: `<path d="M8 3 14 13H2Z"></path>`,
  never: `<circle cx="8" cy="8" r="4.25" fill="none" stroke-width="2"></circle>`,
  not_configured: `<rect x="2" y="7" width="12" height="2"></rect>`,
  complete: `<rect x="3.5" y="3.5" width="9" height="9" fill="none" stroke-width="2"></rect>`,
  partial: `<path d="M8 2 14 8 8 14 2 8Z"></path>`,
  failed: `<rect x="3" y="3" width="10" height="10"></rect>`,
};

export function statusLabel(
  variant: StatusVariant,
  value: StatusValue,
): string {
  if (variant === "analysis") {
    return ANALYSIS_LABELS[value as AnalysisState] ?? String(value);
  }
  return VERIFICATION_LABELS[value as SolutionVerificationStatus] ?? String(value);
}

export function renderStatus(
  variant: StatusVariant,
  value: StatusValue,
): string {
  const label = statusLabel(variant, value);
  const data = escapeAttribute(String(value));
  const variantAttr = escapeAttribute(variant);
  const shape = STATUS_SHAPES[value];
  return (
    `<span class="status-badge" data-variant="${variantAttr}" data-status="${data}">` +
    `<svg aria-hidden="true" focusable="false" viewBox="0 0 16 16">${shape}</svg>` +
    `<span class="status-badge-label">${escapeHtml(label)}</span>` +
    `</span>`
  );
}
