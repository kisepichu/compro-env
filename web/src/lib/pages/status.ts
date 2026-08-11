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
  return (
    `<span class="status-badge" data-variant="${variantAttr}" data-status="${data}">` +
    `<svg aria-hidden="true" focusable="false" viewBox="0 0 16 16"></svg>` +
    `<span class="status-badge-label">${escapeHtml(label)}</span>` +
    `</span>`
  );
}
