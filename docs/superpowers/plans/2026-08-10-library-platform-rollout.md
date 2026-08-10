# Library Platform Rollout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the repository-specific Rust, C++, and Lean library platform as small, independently reviewed PRs without a long-lived integration branch.

**Architecture:** A language-independent Rust core discovers managed sources and exchanges strict JSON with repository-local adapters. Rust produces normalized site data for an Astro/Pagefind static site, while verification and GitHub automation remain separate consumers of the same immutable analysis snapshot. Every implementation PR is based on the latest `main`; incomplete capabilities stay unreachable until their activation PR.

**Tech Stack:** Rust 1.92.0 / Cargo, serde and JSON Schema, LLVM/Clang 22.1.0 LibTooling, Lean 4.30.0/Lake, Astro, TypeScript, npm, Pagefind, GitHub Actions and GitHub App APIs.

## Global Constraints

- The approved specification is `docs/superpowers/specs/2026-08-10-library-platform-design.md`.
- The semantic Web contract is `docs/superpowers/specs/2026-08-10-library-web-structure-handoff.md`.
- MVP languages are exactly Rust, C++, and Lean; core language IDs remain open string slugs.
- One library page maps to one source file, identified by its repository-relative path.
- `main` is the only source of truth; do not use `dev`, `develop`, or an epic integration branch.
- Each implementation branch is created from current `origin/main` only after all prerequisite PRs are merged.
- Keep unfinished commands and workflows unreachable; every merged PR must pass the existing CLI test suite.
- Use TDD: observe each focused test fail before adding the minimal implementation.
- Do not add `--force` verification, a fourth language, a dynamic Web backend, or an artifact branch.
- Do not expose private library paths, sources, symbols, diagnostics, or dependency counts in public DTOs.
- Adapter protocol version 1 is strict; adapters return direct dependencies only.
- Build and analysis jobs are secretless; OJ and repository-write credentials never share a job.
- Run `cargo test --all`, `cargo clippy --all --all-features -- -D warnings`, and `cargo fmt --all --check` before every PR.
- Run `prek run --all-files` before every commit when `prek` is available.

---

## Branch and PR Protocol

Each worker must perform this sequence for one leaf plan only.

- [ ] **Step 1: Confirm prerequisites on GitHub**

Read the plan's `Depends on` field. Use `gh pr view <number>` when the rollout table has a PR number, or inspect `main` for the prerequisite plan's completion commit. Do not branch from an unmerged feature branch.

- [ ] **Step 2: Create an isolated worktree**

Invoke `superpowers:using-git-worktrees`. The project-local `.worktrees/` directory is ignored. Create the plan's exact branch from the latest `origin/main`, then run the baseline:

```bash
cargo test --all
cargo clippy --all --all-features -- -D warnings
cargo fmt --all --check
```

Expected: all commands exit 0 before implementation starts. If dependency preparation would require remote Git transfer from a restricted machine, stop and give the user the documented human preparation command.

- [ ] **Step 3: Execute only the selected plan**

Use `superpowers:subagent-driven-development` or `superpowers:executing-plans`. Check off steps in that plan as they pass. Do not opportunistically implement a later plan.

- [ ] **Step 4: Commit at each task boundary**

Invoke `/commit`. Stage explicit paths only, preserve unrelated files, run repository checks, and use the commit message written in the task.

- [ ] **Step 5: Verify the branch delivery gate**

Run the plan-specific integration command followed by:

```bash
cargo test --all
cargo clippy --all --all-features -- -D warnings
cargo fmt --all --check
git diff --check origin/main...HEAD
```

Expected: all commands exit 0 and the diff contains only the plan's declared files plus checked-off plan progress.

- [ ] **Step 6: Open the PR against main**

Invoke `/pr` with `main` as the explicit base. The PR body must link the approved spec and selected plan, list the test commands actually run, and state which later branches it unblocks.

- [ ] **Step 7: Complete AI review**

Invoke `/pr-review`. Address or explain every comment, resolve only replied threads, and continue the Copilot re-review loop until it reports no comments or no new comments.

- [ ] **Step 8: Merge before unlocking dependents**

After required checks and review pass, merge to `main`. Dependent workers must verify the merged state themselves rather than relying only on a plan checkbox.

## Agent Handoff Contract

Assign exactly one leaf plan to one implementation agent. Give the agent this prompt with the concrete
plan path substituted; do not ask an agent to choose multiple ready plans itself.

```text
Read docs/superpowers/plans/2026-08-10-library-platform-rollout.md and <leaf-plan-path> completely.
Confirm every Depends on item is merged to origin/main. Use superpowers:using-git-worktrees, create the
leaf plan's exact branch from current origin/main, and execute only that plan with TDD. Use /commit at
task boundaries, /pr --base main, and /pr-review until no new comments. Merge only after CI and review
pass. Preserve unrelated files and report any HUMAN GATE without weakening it.
```

The coordinator may start only these independent ready sets:

- Initially: 039 only.
- After 039: 040 and 054.
- After 040: 041; after 041: 042; continue language plans 043 through 050 in order.
- After 054: 055; after 055: 056.
- After 056: 051 and 057.
- Continue 051 -> 052 -> 053 and 057 -> 058 -> 059 independently.
- After 050, 053, and 059: 060, then 061, then 062.

Do not reserve or create a dependent branch early. A worker starts by fetching the prerequisite merge and
creating its worktree from that new `origin/main`.

## PR Dependency Graph

```text
039 foundation
 |-- 040 adapter process -- 041 adapter prepare -- 042 adapter build
 |      `-- 043 Rust dependencies -- 044 Rust symbols
 |             `-- 045 C++ toolchain -- 046 C++ dependencies -- 047 C++ symbols
 |                    `-- 048 Lean toolchain -- 049 Lean dependencies -- 050 Lean symbols
 `-- 054 check -- 055 verification state -- 056 verification planning
        |-- 051 site data -- 052 static Web -- 053 search
        `-- 057 submission ports -- 058 LibraryChecker -- 059 verify CLI

050 + 053 + 059 -- 060 safe automation -- 061 platform activation -- 062 verify activation
```

Plans sharing a parent may run in parallel only after that parent is merged. Plans never use stacked PR bases.

## Plan Index

| ID | Branch | Plan | Depends on | Merge boundary |
|---|---|---|---|---|
| 039 | `feat/039-library-foundation` | `2026-08-10-library-foundation.md` | PR #41 | Strict project config, discovery, protocol types, fixture snapshot |
| 040 | `feat/040-library-adapter-process` | `2026-08-10-library-adapter-process.md` | 039 | Deterministic inputs, manifests, process protocol |
| 041 | `feat/041-library-adapter-prepare` | `2026-08-10-library-adapter-prepare.md` | 040 | HTTPS/checksum/archive-safe dependency preparation |
| 042 | `feat/042-library-adapter-build` | `2026-08-10-library-adapter-build.md` | 041 | Offline builds, handshake, atomic current publication |
| 043 | `feat/043-library-rust-dependencies` | `2026-08-10-library-rust-dependencies.md` | 042 | Rust modules and direct dependency edges |
| 044 | `feat/044-library-rust-symbols` | `2026-08-10-library-rust-symbols.md` | 043 | Rust symbol and source-location extraction |
| 045 | `feat/045-library-cpp-toolchain` | `2026-08-10-library-cpp-toolchain.md` | 044 | Exact Clang toolchain and adapter handshake |
| 046 | `feat/046-library-cpp-dependencies` | `2026-08-10-library-cpp-dependencies.md` | 045 | C++ direct include dependency edges |
| 047 | `feat/047-library-cpp-symbols` | `2026-08-10-library-cpp-symbols.md` | 046 | C++ LibTooling symbols and locations |
| 048 | `feat/048-library-lean-toolchain` | `2026-08-10-library-lean-toolchain.md` | 047 | Exact Lean/Lake toolchain and adapter handshake |
| 049 | `feat/049-library-lean-dependencies` | `2026-08-10-library-lean-dependencies.md` | 048 | Lean header imports and direct dependency edges |
| 050 | `feat/050-library-lean-symbols` | `2026-08-10-library-lean-symbols.md` | 049 | Lean elaborated symbols and source locations |
| 051 | `feat/051-library-site-data` | `2026-08-10-library-site-data.md` | 056 | Privacy-safe normalized public DTO and `ce site-data` |
| 052 | `feat/052-library-static-web` | `2026-08-10-library-static-web.md` | 051 | Static semantic routes and source pages |
| 053 | `feat/053-library-search` | `2026-08-10-library-search.md` | 052 | Exact/Pagefind search and filter UI |
| 054 | `feat/054-library-check` | `2026-08-10-library-check.md` | 039 | Reusable process runner and local/CI `ce check` |
| 055 | `feat/055-library-verification-state` | `2026-08-10-library-verification-state.md` | 054 | Result schema, capabilities, atomic/CAS repository |
| 056 | `feat/056-library-verification-planning` | `2026-08-10-library-verification-planning.md` | 055 | Fingerprints, statuses, immutable plans and transitions |
| 057 | `feat/057-library-submission-ports` | `2026-08-10-library-submission-ports.md` | 056 | Reusable start/poll/recovery ports and submit migration |
| 058 | `feat/058-librarychecker-lifecycle` | `2026-08-10-librarychecker-lifecycle.md` | 057 | LibraryChecker polling and conservative recovery |
| 059 | `feat/059-library-verify-command` | `2026-08-10-library-verify-command.md` | 058 | Resumable local `ce verify` orchestration |
| 060 | `feat/060-library-safe-automation` | `2026-08-10-library-safe-automation.md` | 050, 053, 059 | Disabled secret-safe workflows, policy, state writer |
| 061 | `feat/061-library-platform-activation` | `2026-08-10-library-platform-activation.md` | 060 | Pinned normal CI, Pages publication, mixed acceptance |
| 062 | `feat/062-library-verify-activation` | `2026-08-10-library-verify-activation.md` | 061 + human gate | Live credential-separated verify automation |

## Specification Coverage

| Specification sections | Owning plans |
|---|---|
| 4-5, 6.1-6.5, 16 | 039 |
| 6.9, 14 | 040-042 |
| 6.6, 16.1 Rust | 043-044 |
| 6.7, 16.1 C++ | 045-047 |
| 6.8, 16.1 Lean | 048-050 |
| 12.0-12.14 | 051-052 |
| 12.15, 13 | 053 |
| 7.1 | 054 |
| 9-11 | 055-056 |
| 8, 7.2 | 057-059 |
| 15.1-15.4 | 060, 062 |
| 12.15, 15, 15.5, 17 | 061 |

## Coordinator Acceptance

- [ ] Every indexed file exists and starts with the required Superpowers plan header.
- [ ] Every leaf plan names exact files, interfaces, failing tests, commands, expected outcomes, and commits.
- [ ] No leaf plan contains placeholder markers or an unspecified implementation step.
- [ ] Interfaces consumed by later plans use the same signatures as the producing plan.
- [ ] Every specification section maps to at least one plan in the coverage matrix.
- [ ] Every plan ends with `/pr --base main` and `/pr-review` delivery instructions.
