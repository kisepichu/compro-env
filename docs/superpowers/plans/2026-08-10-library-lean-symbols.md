# Lean Library Symbol Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Elaborate Lean sources and expose stable declarations, qualified names, kinds, and source locations.

**Architecture:** The adapter creates a fresh Lean environment for each request batch, elaborates modules in deterministic dependency order, and diffs environments to identify declarations owned by each target. It projects Lean declaration kinds to open string tokens without teaching the core Lean syntax.

**Tech Stack:** Lean 4.30.0 elaborator/environment APIs, Lake.

## Constraints

- **Branch:** `feat/050-library-lean-symbols`
- **Depends on:** plan 049 merged to `main`.
- Read specification sections 6.8, 12.5, and 13.1.
- Keep declarations only when source information points to the requested managed file.
- Generated/internal declarations may be omitted or locationless; never fabricate a source location.
- Elaboration failure affects symbol state only when header dependency analysis succeeded.

### Task 1: Elaborate modules deterministically

**Files:**
- Create: `tools/library-analyzers/lean/Analyzer/Elaboration.lean`
- Create: `tools/library-analyzers/lean/Tests/Elaboration.lean`

**Interfaces:**

```lean
def elaborateTargets
    (request : AnalysisRequest)
    (moduleMap : ModuleMap) : IO (Array ElaboratedTarget)
```

- [x] Write failing tests for topological ordering, cycles, independent modules, same name in namespaces,
      theorem errors, missing imports, no user-global search path, and repeatable output.
- [x] Build a fresh search path from the prepared toolchain/packages and request roots.
- [x] Elaborate in stable strongly-connected-component order and capture target-scoped diagnostics.
- [x] Run `lake env lean Tests/Elaboration.lean` twice and compare normalized output.
- [x] Invoke `/commit` with `feat: elaborate Lean library targets deterministically`.

### Task 2: Project declarations and locations

**Files:**
- Create: `tools/library-analyzers/lean/Analyzer/Symbols.lean`
- Modify: `tools/library-analyzers/lean/Analyzer/Main.lean`
- Create: `tools/library-analyzers/lean/Tests/Symbols.lean`
- Create: `tools/library-analyzers/protocol/fixtures/lean-symbols-request.json`
- Create: `tools/library-analyzers/protocol/fixtures/lean-symbols-response.json`

- [x] Write failing fixtures for definition, theorem, axiom, structure, class, inductive/constructors,
      instance, notation, namespace qualification, generated declaration, and Unicode source spans.
- [x] Diff environments per target, use declaration ranges when valid, sort by location/name, and deduplicate.
- [x] Emit stable kind strings and search names; mark recoverable elaboration errors as symbol `partial`.
- [x] Run Lean symbol tests and compare checked-in protocol JSON exactly.
- [x] Invoke `/commit` with `feat: extract Lean library symbols`.

### Task 3: Deliver Lean symbols

- [x] Run all three language fixtures through build, handshake, normalization, and toolchain identity checks.
- [x] Run rollout repository verification and `git diff --check`.
- [x] Invoke `/commit` with `docs: record Lean symbol adapter completion`.
- [x] Invoke `/pr --base main`; link plan 050 and state that it satisfies the language prerequisite of plan 060.
- [x] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
