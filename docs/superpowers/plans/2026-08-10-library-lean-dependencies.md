# Lean Library Dependency Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve direct Lean module imports from parsed headers with stable locations and explicit unresolved states.

**Architecture:** The adapter maps requested repository paths to Lean module names, invokes `Parser.parseHeader`, and classifies each explicit import against the request manifest or ambient toolchain. It does not infer imports from elaborated declarations.

**Tech Stack:** Lean 4.30.0 parser APIs, Lake.

## Constraints

- **Branch:** `feat/049-library-lean-dependencies`
- **Depends on:** plan 048 merged to `main`.
- Read specification section 6.8.
- Explicit header imports are direct dependencies; implicit prelude is an ambient external dependency.
- Symbol analysis remains `partial` with a stable diagnostic until plan 050.
- Do not infer dependencies from `open`, `include`, namespaces, or theorem bodies.

### Task 1: Map repository paths and Lean modules

**Files:**
- Create: `tools/library-analyzers/lean/Analyzer/ModuleMap.lean`
- Create: `tools/library-analyzers/lean/Tests/ModuleMap.lean`

**Interfaces:**

```lean
def buildModuleMap (request : AnalysisRequest) : Except Diagnostic ModuleMap
def moduleForPath (map : ModuleMap) (path : String) : Option Name
```

- [ ] Write failing tests for nested modules, `Main.lean`, Unicode names, duplicate module ownership,
      invalid components, repository escapes, and stable path-byte ordering.
- [ ] Implement a bijective module/path map from request targets and configured roots.
- [ ] Reject duplicate module ownership before parsing any source.
- [ ] Run `lake env lean Tests/ModuleMap.lean` through the prepared toolchain.
- [ ] Invoke `/commit` with `feat: map Lean modules to managed sources`.

### Task 2: Parse explicit imports and locations

**Files:**
- Create: `tools/library-analyzers/lean/Analyzer/Dependencies.lean`
- Modify: `tools/library-analyzers/lean/Analyzer/Main.lean`
- Create: `tools/library-analyzers/lean/Tests/Dependencies.lean`
- Create: `tools/library-analyzers/protocol/fixtures/lean-dependencies-request.json`
- Create: `tools/library-analyzers/protocol/fixtures/lean-dependencies-response.json`

- [ ] Write failing fixtures for multiple imports, internal/external imports, cycles, missing modules,
      malformed headers, comments, Unicode before import, and one-based source spans.
- [ ] Use `Parser.parseHeader`; classify only imports present in the parsed header and deduplicate by key.
- [ ] Mark malformed/missing active imports `partial`; do not silently return an empty complete analysis.
- [ ] Run Lean dependency tests and compare protocol response JSON exactly.
- [ ] Invoke `/commit` with `feat: analyze direct Lean library dependencies`.

### Task 3: Deliver Lean dependencies

- [ ] Run handshake, module-map, dependency fixture, and normalized snapshot tests.
- [ ] Run rollout repository verification and `git diff --check`.
- [ ] Invoke `/commit` with `docs: record Lean dependency adapter completion`.
- [ ] Invoke `/pr --base main`; link plan 049 and state that it unblocks plan 050.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
