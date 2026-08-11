# C++ Library Toolchain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pin LLVM/Clang 22.1.0, compile the C++ adapter against prepared LibTooling, and pass the protocol handshake.

**Architecture:** Preparation selects one checked-in official LLVM artifact by target triple. CMake receives only paths from that prepared set. The first executable handles empty requests and reports exact Clang identity; dependency and AST extraction land separately.

**Tech Stack:** LLVM/Clang 22.1.0, CMake, Ninja, C++20, nlohmann/json generated protocol bindings.

## Constraints

- **Branch:** `feat/045-library-cpp-toolchain`
- **Depends on:** plan 044 merged to `main`.
- Read specification sections 6.7 and 6.9.
- Supported prepared artifacts are official release archives:
  - Linux x86_64: `LLVM-22.1.0-Linux-X64.tar.xz`, SHA-256
    `8d662e425e46c48b45f5f970770b5e37f323607c8c2cbc371593fc9c4ba1e7b3`.
  - Linux aarch64: `LLVM-22.1.0-Linux-ARM64.tar.xz`, SHA-256
    `e3b4205fe45d5561dec9d46465873a79c26b25b028b310515b38c34f668c6aec`.
  - macOS aarch64: `LLVM-22.1.0-macOS-ARM64.tar.xz`, SHA-256
    `cd5e615f4dab23d0239359cd343202c5f6ceeaf072c245a3c685d73afac09646`.
- Other platforms fail before download; never fall back to Apple Clang, Homebrew, or `/usr/bin/clang`.

### Task 1: Prepare and validate exact LLVM artifacts

**Files:**
- Modify: `tools/library-analyzers/dependencies.toml`
- Modify: `tools/library-analyzers/build-inputs.toml`
- Create: `crates/infrastructure/tests/cpp_toolchain.rs`

- [x] Write failing selection tests for all three supported triples, unsupported triples, wrong archive digest,
      missing `clang`, missing `llvm-config`, and reported version not exactly `22.1.0`.
- [x] Add the three official HTTPS URLs and digests to the dependency manifest.
- [x] Validate executable/library/header layout after safe extraction and record target plus version in manifest.
- [x] Run `cargo test -p infrastructure --test cpp_toolchain`.
- [x] Invoke `/commit` with `build: pin LLVM 22.1.0 for library analysis`.

### Task 2: Build a handshaking C++ adapter

**Files:**
- Create: `tools/library-analyzers/cpp/CMakeLists.txt`
- Create: `tools/library-analyzers/cpp/src/main.cpp`
- Create: `tools/library-analyzers/cpp/src/protocol.cpp`
- Create: `tools/library-analyzers/cpp/include/protocol.hpp`
- Create: `tools/library-analyzers/cpp/tests/handshake.cpp`
- Modify: `tools/library-analyzers/build-inputs.toml`

- [x] Write a failing empty-request test expecting adapter `ce-cpp`, protocol v1, toolchain
      `clang=22.1.0`, and empty target arrays.
- [x] Configure CMake from prepared `LLVM_DIR`/`Clang_DIR`, compile with C++20, and link only required libraries.
- [x] Parse/serialize strict protocol fields and reject unknown/request version fields before analysis.
- [x] Run `tools/library-analyzers/prepare && tools/library-analyzers/build`; expect all handshakes to pass.
- [x] Invoke `/commit` with `feat: add C++ analyzer protocol executable`.

### Task 3: Deliver the C++ toolchain boundary

- [x] Run C++ unit tests and the generic adapter build/process suites.
- [x] Run rollout repository verification and `git diff --check`.
- [x] Invoke `/commit` with `docs: record C++ toolchain completion`.
- [ ] Invoke `/pr --base main`; link plan 045 and state that it unblocks plan 046.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
