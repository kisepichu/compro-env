/**
 * Rich inline fixture used by Task 2's semantic-pages tests.
 * Task 3 will move to a JSON fixture under `web/tests/fixtures/`.
 */

import { SUPPORTED_SCHEMA_VERSION } from "@/lib/site-data.ts";
import type {
  LanguageSummary,
  LibraryPageData,
  SiteData,
  SolutionPageData,
} from "@/lib/site-data-types.ts";

export function fixtureLanguages(): LanguageSummary[] {
  return [
    {
      id: "rust",
      display_name: "Rust",
      syntax_highlight: "rust",
      library_count: 3,
      verification_summary: {
        verified: 1,
        rejected: 1,
        unavailable: 0,
        stale: 1,
        never: 0,
      },
    },
    {
      id: "cpp",
      display_name: "C++",
      syntax_highlight: "cpp",
      library_count: 0,
      verification_summary: {
        verified: 0,
        rejected: 0,
        unavailable: 0,
        stale: 0,
        never: 0,
      },
    },
    {
      id: "lean",
      display_name: "Lean 4",
      syntax_highlight: "lean",
      library_count: 0,
      verification_summary: {
        verified: 0,
        rejected: 0,
        unavailable: 0,
        stale: 0,
        never: 0,
      },
    },
  ];
}

function verifiedLibrary(): LibraryPageData {
  return {
    page_id: "library:rust/graph/dijkstra.rs",
    library_id: "rust/graph/dijkstra.rs",
    language: "rust",
    title: "Dijkstra",
    source_path: "graph/dijkstra.rs",
    source: "pub fn dijkstra() {}\n",
    syntax_highlight: "rust",
    updated_at: "2026-08-10T12:00:00Z",
    updated_by_commit: "0abcdef",
    description: "Shortest paths on a weighted graph.",
    symbol_analysis: {
      state: "complete",
      symbols: [
        {
          kind: "function",
          name: "dijkstra",
          qualified_name: "graph::dijkstra",
          search_names: ["dijkstra"],
          signature: "pub fn dijkstra()",
        },
      ],
    },
    dependency_analysis: {
      state: "complete",
      direct: [
        {
          library_id: "rust/util/binary_heap.rs",
          language: "rust",
          title: "Binary heap util",
          source_path: "util/binary_heap.rs",
          manual: false,
        },
      ],
      transitive: [],
      has_private_dependencies: false,
    },
    reverse_dependencies: [],
    relations: [],
    verification: {
      aggregate_status: "verified",
      evidence: [
        {
          solution_id: "abc300/a/dijkstra_solve",
          solution_page_id: "solution:abc300/a/dijkstra_solve",
          online_judge: "atcoder",
          status: "verified",
          verdict: "AC",
          judged_at: "2026-08-10T13:00:00Z",
          oj_submission_url: "https://example.com/submissions/1",
          stale_reason: null,
        },
      ],
    },
    diagnostics: [],
  };
}

function staleLibrary(): LibraryPageData {
  return {
    page_id: "library:rust/util/binary_heap.rs",
    library_id: "rust/util/binary_heap.rs",
    language: "rust",
    title: "Binary heap util",
    source_path: "util/binary_heap.rs",
    source: "pub struct Heap;\n",
    syntax_highlight: "rust",
    updated_at: "2026-08-09T09:00:00Z",
    updated_by_commit: "0abcdef",
    description: null,
    symbol_analysis: { state: "complete", symbols: [] },
    dependency_analysis: {
      state: "partial",
      direct: [],
      transitive: [],
      has_private_dependencies: true,
    },
    reverse_dependencies: [
      {
        library_id: "rust/graph/dijkstra.rs",
        language: "rust",
        title: "Dijkstra",
        source_path: "graph/dijkstra.rs",
        manual: false,
      },
    ],
    relations: [],
    verification: {
      aggregate_status: "stale",
      evidence: [],
    },
    diagnostics: [
      {
        severity: "warning",
        code: "dep.partial",
        message: "Some dependencies could not be resolved.",
      },
    ],
  };
}

function rejectedLibrary(): LibraryPageData {
  return {
    page_id: "library:rust/math/mod_inv.rs",
    library_id: "rust/math/mod_inv.rs",
    language: "rust",
    title: "Modular inverse",
    source_path: "math/mod_inv.rs",
    source: "pub fn mod_inv() {}\n",
    syntax_highlight: "rust",
    updated_at: "2026-08-08T15:00:00Z",
    updated_by_commit: "0abcdef",
    description: null,
    symbol_analysis: {
      state: "failed",
      symbols: [],
    },
    dependency_analysis: {
      state: "complete",
      direct: [],
      transitive: [],
      has_private_dependencies: false,
    },
    reverse_dependencies: [],
    relations: [],
    verification: {
      aggregate_status: "rejected",
      evidence: [
        {
          solution_id: "abc301/a/mod_inv_solve",
          solution_page_id: "solution:abc301/a/mod_inv_solve",
          online_judge: "atcoder",
          status: "rejected",
          verdict: "WA",
          judged_at: "2026-08-08T16:00:00Z",
          oj_submission_url: "https://example.com/submissions/2",
          stale_reason: null,
        },
      ],
    },
    diagnostics: [],
  };
}

export function fixtureLibraries(): LibraryPageData[] {
  return [verifiedLibrary(), staleLibrary(), rejectedLibrary()];
}

function verifiedSolution(): SolutionPageData {
  return {
    page_id: "solution:abc300/a/dijkstra_solve",
    solution_id: "abc300/a/dijkstra_solve",
    contest_id: "abc300",
    problem_code: "a",
    solution_name: "dijkstra_solve",
    online_judge: "atcoder",
    language: "rust",
    solved_at: "2026-08-10T13:00:00Z",
    source_path: "solutions/abc300/a/dijkstra_solve/main.rs",
    source: "fn main() {}\n",
    syntax_highlight: "rust",
    has_preprocess: true,
    verifies: [
      {
        library_id: "rust/graph/dijkstra.rs",
        language: "rust",
        title: "Dijkstra",
        source_path: "graph/dijkstra.rs",
        manual: false,
      },
    ],
    direct_dependencies: [
      {
        library_id: "rust/graph/dijkstra.rs",
        language: "rust",
        title: "Dijkstra",
        source_path: "graph/dijkstra.rs",
        manual: false,
      },
    ],
    has_private_dependencies: false,
    verification: {
      status: "verified",
      result: {
        attempt_id: "att-1",
        verdict: "AC",
        judged_at: "2026-08-10T13:00:00Z",
        oj_submission_url: "https://example.com/submissions/1",
        execution_time_ms: 42,
        memory_kib: 1024,
        submitted_source_hash: "hash1",
        verify_fingerprint: "fp1",
        stale_reason: null,
        testcases: [
          {
            name: "sample_00.txt",
            verdict: "AC",
            execution_time_ms: 12,
            memory_kib: 512,
          },
        ],
      },
    },
    dependency_analysis_state: "complete",
    diagnostics: [],
  };
}

function neverVerifiedSolution(): SolutionPageData {
  return {
    page_id: "solution:abc300/b/never_yet",
    solution_id: "abc300/b/never_yet",
    contest_id: "abc300",
    problem_code: "b",
    solution_name: "never_yet",
    online_judge: "atcoder",
    language: "rust",
    solved_at: "2026-08-09T10:00:00Z",
    source_path: "solutions/abc300/b/never_yet/main.rs",
    source: "fn main() {}\n",
    syntax_highlight: "rust",
    has_preprocess: false,
    verifies: [],
    direct_dependencies: [],
    has_private_dependencies: false,
    verification: {
      status: "never",
      result: null,
    },
    dependency_analysis_state: "complete",
    diagnostics: [],
  };
}

function notConfiguredSolution(): SolutionPageData {
  return {
    page_id: "solution:abc301/a/mod_inv_solve",
    solution_id: "abc301/a/mod_inv_solve",
    contest_id: "abc301",
    problem_code: "a",
    solution_name: "mod_inv_solve",
    online_judge: "atcoder",
    language: "rust",
    solved_at: "2026-08-08T16:00:00Z",
    source_path: "solutions/abc301/a/mod_inv_solve/main.rs",
    source: "fn main() {}\n",
    syntax_highlight: "rust",
    has_preprocess: false,
    verifies: [],
    direct_dependencies: [],
    has_private_dependencies: false,
    verification: {
      status: "not_configured",
      result: null,
    },
    dependency_analysis_state: "complete",
    diagnostics: [],
  };
}

export function fixtureSolutions(): SolutionPageData[] {
  return [verifiedSolution(), neverVerifiedSolution(), notConfiguredSolution()];
}

export function buildFixtureSiteData(
  overrides: Partial<SiteData> = {},
): SiteData {
  const base: SiteData = {
    schema_version: SUPPORTED_SCHEMA_VERSION,
    build: {
      schema_version: SUPPORTED_SCHEMA_VERSION,
      generated_at: "2026-08-11T00:00:00Z",
      mode: "production",
      source_commit_sha: "0abcdef1234567890abcdef1234567890abcdef1",
      source_commit_short_sha: "0abcdef",
      source_committed_at: "2026-08-11T00:00:00Z",
      uncommitted_changes: false,
      observed_toolchains: [],
      adapters: [],
    },
    site: {
      title: "compro-env fixture",
      description: "fixture site description",
      language: "en",
      repository_url: "https://example.com/repo",
    },
    languages: fixtureLanguages(),
    libraries: fixtureLibraries(),
    solutions: fixtureSolutions(),
  };
  return { ...base, ...overrides };
}
