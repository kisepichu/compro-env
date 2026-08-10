# C++ Library Symbol Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract searchable declarations from the active C++ AST with stable qualified names and source locations.

**Architecture:** A LibTooling AST visitor projects only declarations whose spelling location is inside the requested managed source. C++ syntax remains adapter-private; the protocol receives strings for symbol kind and names.

**Tech Stack:** LLVM/Clang 22.1.0 LibTooling ASTMatchers, C++20.

## Constraints

- **Branch:** `feat/047-library-cpp-symbols`
- **Depends on:** plan 046 merged to `main`.
- Read specification sections 6.7, 12.5, and 13.1.
- Ignore declarations originating only from included/external headers and implicit compiler declarations.
- Do not create C++-specific Web rendering or core enums.

### Task 1: Visit and normalize AST declarations

**Files:**
- Create: `tools/library-analyzers/cpp/src/symbols.cpp`
- Create: `tools/library-analyzers/cpp/include/symbols.hpp`
- Create: `tools/library-analyzers/cpp/tests/symbols.cpp`
- Create: `tools/library-analyzers/protocol/fixtures/cpp-symbols-request.json`
- Create: `tools/library-analyzers/protocol/fixtures/cpp-symbols-response.json`

**Interfaces:**

```cpp
SymbolAnalysis analyzeSymbols(
    const clang::ASTContext& context,
    const std::filesystem::path& managedSource);
```

- [ ] Write failing fixtures for namespace, class/struct, enum/enumerator, alias, concept, function,
      variable, constructor, method, template, overload, nested name, operator, and anonymous namespace.
- [ ] Implement spelling-location filtering, source-order sorting, qualified names, and deduplicated search names.
- [ ] Use stable kind tokens (`class`, `function`, `method`, `concept`, `type`, `value`) without core changes.
- [ ] Run the C++ symbol test and compare JSON fixture exactly.
- [ ] Invoke `/commit` with `feat: extract C++ library symbols`.

### Task 2: Lock source-location and recovery semantics

**Files:**
- Modify: `tools/library-analyzers/cpp/src/symbols.cpp`
- Modify: `tools/library-analyzers/cpp/tests/symbols.cpp`
- Modify: `crates/usecases/tests/library_analysis.rs`

- [ ] Add failing cases for macro spelling/expansion, CRLF, Unicode, invalid source ranges, parse recovery,
      duplicate declarations, forward declarations, and symbols from included headers.
- [ ] Prefer spelling locations in the target; omit invalid locations and mark symbol analysis `partial` on recovery.
- [ ] Prove dependency completeness is retained when AST symbol extraction is partial.
- [ ] Run C++ tests and `cargo test -p usecases library_analysis`.
- [ ] Invoke `/commit` with `test: lock C++ symbol location behavior`.

### Task 3: Deliver C++ symbols

- [ ] Run all C++ fixtures through the generic adapter build and snapshot normalization.
- [ ] Run rollout repository verification and `git diff --check`.
- [ ] Invoke `/commit` with `docs: record C++ symbol adapter completion`.
- [ ] Invoke `/pr --base main`; link plan 047 and state that it unblocks plan 048.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
