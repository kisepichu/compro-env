# C++ Library Dependency Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve active direct C++ include edges with stable source locations under a checked-in compile profile.

**Architecture:** Clang preprocesses each requested translation-unit profile. Inclusion callbacks classify managed headers as internal, non-managed headers as external, and ambiguous/missing active includes as unresolved, preserving only direct edges from the requested source.

**Tech Stack:** LLVM/Clang 22.1.0 LibTooling, C++20, CMake, Ninja.

## Constraints

- **Branch:** `feat/046-library-cpp-dependencies`
- **Depends on:** plan 045 merged to `main`.
- Read specification section 6.7.
- Use one checked-in compile profile; do not read ambient `compile_commands.json`, flags, or include paths.
- Emit direct active includes only. Transitive includes never become direct edges.
- Symbol analysis remains `partial` with a stable diagnostic until plan 047.

### Task 1: Define the compile profile

**Files:**
- Create: `tools/library-analyzers/cpp/compile-profile.toml`
- Create: `tools/library-analyzers/cpp/src/compile_profile.cpp`
- Create: `tools/library-analyzers/cpp/include/compile_profile.hpp`
- Create: `tools/library-analyzers/cpp/tests/compile_profile.cpp`
- Modify: `tools/library-analyzers/build-inputs.toml`

**Interfaces:**

```cpp
CompileProfile loadCompileProfile(const std::filesystem::path& repositoryRoot);
std::vector<std::string> buildClangArguments(const CompileProfile&, const Target&);
```

- [x] Write failing tests for C++ standard, defines, include roots, duplicate/missing keys, repository escape,
      symlink include root, environment independence, and deterministic argv.
- [x] Implement strict TOML parsing and repository-relative normalized paths.
- [x] Run the C++ compile-profile test binary; expect all cases to pass.
- [x] Invoke `/commit` with `build: define deterministic C++ analysis profile`.

### Task 2: Emit direct include dependencies

**Files:**
- Create: `tools/library-analyzers/cpp/src/dependencies.cpp`
- Create: `tools/library-analyzers/cpp/include/dependencies.hpp`
- Create: `tools/library-analyzers/cpp/tests/dependencies.cpp`
- Create: `tools/library-analyzers/protocol/fixtures/cpp-dependencies-request.json`
- Create: `tools/library-analyzers/protocol/fixtures/cpp-dependencies-response.json`

- [x] Write failing fixtures for quoted/angle includes, macro includes, inactive branches, nested transitive
      includes, cycles, missing headers, duplicate includes, Unicode paths, and one-based include locations.
- [x] Use preprocessing callbacks and the manifest path set to classify each active direct include.
- [x] Mark dependency state `partial` for active macro/missing ambiguity and preserve a stable diagnostic key.
- [x] Run C++ tests and compare the protocol response fixture byte-for-byte.
- [x] Invoke `/commit` with `feat: analyze direct C++ library dependencies`.

### Task 3: Deliver C++ dependencies

- [x] Run handshake, compile-profile, dependency fixture, and normalized snapshot tests.
- [x] Run rollout repository verification and `git diff --check`.
- [x] Invoke `/commit` with `docs: record C++ dependency adapter completion`.
- [ ] Invoke `/pr --base main`; link plan 046 and state that it unblocks plan 047.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
