/**
 * Aggregate helpers for computing counts across public data.
 * Public-only projection is guaranteed by the schema; these helpers only
 * count what is already present in the site data.
 */

import type {
  LanguageSummary,
  LibraryPageData,
  SiteData,
  SolutionPageData,
  SolutionVerificationStatus,
  VerificationCounts,
} from "../site-data-types.ts";

export interface LibraryVerificationTotals extends VerificationCounts {
  total: number;
}

export function sumLanguageVerification(
  languages: readonly LanguageSummary[],
): LibraryVerificationTotals {
  const totals: LibraryVerificationTotals = {
    verified: 0,
    rejected: 0,
    unavailable: 0,
    stale: 0,
    never: 0,
    total: 0,
  };
  for (const lang of languages) {
    totals.verified += lang.verification_summary.verified;
    totals.rejected += lang.verification_summary.rejected;
    totals.unavailable += lang.verification_summary.unavailable;
    totals.stale += lang.verification_summary.stale;
    totals.never += lang.verification_summary.never;
    totals.total += lang.library_count;
  }
  return totals;
}

export interface SolutionVerificationTotals {
  verified: number;
  rejected: number;
  unavailable: number;
  stale: number;
  never: number;
  not_configured: number;
  total: number;
}

export function sumSolutionVerification(
  solutions: readonly SolutionPageData[],
): SolutionVerificationTotals {
  const totals: SolutionVerificationTotals = {
    verified: 0,
    rejected: 0,
    unavailable: 0,
    stale: 0,
    never: 0,
    not_configured: 0,
    total: solutions.length,
  };
  for (const sol of solutions) {
    const key = sol.verification.status as SolutionVerificationStatus;
    totals[key] += 1;
  }
  return totals;
}

/** Public libraries flagged as attention-required (status or failed analysis). */
export function attentionLibraries(
  libraries: readonly LibraryPageData[],
): LibraryPageData[] {
  return libraries.filter((lib) => {
    const status = lib.verification.aggregate_status;
    if (status === "stale" || status === "rejected" || status === "unavailable") {
      return true;
    }
    if (
      lib.symbol_analysis.state === "failed" ||
      lib.symbol_analysis.state === "partial" ||
      lib.dependency_analysis.state === "failed" ||
      lib.dependency_analysis.state === "partial"
    ) {
      return true;
    }
    return false;
  });
}

/** Sort libraries by (updated_at desc, library_id asc). */
export function recentLibraries(
  libraries: readonly LibraryPageData[],
  limit: number,
): LibraryPageData[] {
  const copy = [...libraries];
  copy.sort((a, b) => {
    if (a.updated_at === b.updated_at) {
      return a.library_id < b.library_id ? -1 : a.library_id > b.library_id ? 1 : 0;
    }
    return a.updated_at < b.updated_at ? 1 : -1;
  });
  return copy.slice(0, limit);
}

/** Sort solutions by (solved_at desc, solution_id asc). */
export function recentSolutions(
  solutions: readonly SolutionPageData[],
  limit: number,
): SolutionPageData[] {
  const copy = [...solutions];
  copy.sort((a, b) => {
    if (a.solved_at === b.solved_at) {
      return a.solution_id < b.solution_id ? -1 : a.solution_id > b.solution_id ? 1 : 0;
    }
    return a.solved_at < b.solved_at ? 1 : -1;
  });
  return copy.slice(0, limit);
}

export function librariesForLanguage(
  siteData: SiteData,
  languageId: string,
): LibraryPageData[] {
  return siteData.libraries.filter((lib) => lib.language === languageId);
}
