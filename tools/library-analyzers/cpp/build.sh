#!/usr/bin/env bash
# ce-cpp adapter build driver (spec §§6.7, 6.9; plan 045 Task 2).
#
# Called by `library-adapter-build` under a cleared environment plus the
# allowlist wired in `language_plans.rs`. The parent driver picks the
# staging/bin paths and points us at the prepared LLVM install; this script
# just runs the two-step CMake invocation and drops `cpp-analyzer` where the
# driver expects it.
#
# Required env:
#   CE_ADAPTER_REPOSITORY_ROOT  Path to the compro-env repository root.
#   CE_ADAPTER_STAGE_DIR        Fresh per-build staging directory. We build in
#                               `<CE_ADAPTER_STAGE_DIR>/cpp/`.
#   CE_LLVM_DIR                 Root of the prepared LLVM 22.1.0 install (the
#                               directory that contains `lib/cmake/llvm`).
#
# Optional env:
#   CE_CPP_BUILD_JOBS           Parallelism (default: 4).
set -euo pipefail

: "${CE_ADAPTER_REPOSITORY_ROOT:?CE_ADAPTER_REPOSITORY_ROOT is required}"
: "${CE_ADAPTER_STAGE_DIR:?CE_ADAPTER_STAGE_DIR is required}"
: "${CE_LLVM_DIR:?CE_LLVM_DIR is required}"

jobs="${CE_CPP_BUILD_JOBS:-4}"
source_dir="${CE_ADAPTER_REPOSITORY_ROOT}/tools/library-analyzers/cpp"
build_dir="${CE_ADAPTER_STAGE_DIR}/cpp"

mkdir -p "${build_dir}"

cmake -S "${source_dir}" -B "${build_dir}" \
    -DCMAKE_BUILD_TYPE=Release \
    -DLLVM_DIR="${CE_LLVM_DIR}/lib/cmake/llvm" \
    -DClang_DIR="${CE_LLVM_DIR}/lib/cmake/clang"

cmake --build "${build_dir}" --target cpp-analyzer -- -j"${jobs}"
