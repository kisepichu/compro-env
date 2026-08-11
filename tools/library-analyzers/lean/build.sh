#!/usr/bin/env bash
# ce-lean adapter build driver (spec §§6.8, 6.9; plan 048 Task 2).
#
# Called by `library-adapter-build` under a cleared environment plus the
# allowlist wired in `language_plans.rs`. The parent driver picks the
# staging/bin paths and points us at the prepared Lean install; this script
# stages the analyzer sources under `<CE_ADAPTER_STAGE_DIR>/lean/`, runs
# `lake build ce-lean` inside `lake env`, and drops `ce-lean` where the
# driver expects it (`<CE_ADAPTER_STAGE_DIR>/lean/ce-lean`).
#
# Required env:
#   CE_ADAPTER_REPOSITORY_ROOT  Path to the compro-env repository root.
#   CE_ADAPTER_STAGE_DIR        Fresh per-build staging directory. We build in
#                               `<CE_ADAPTER_STAGE_DIR>/lean/`.
#   CE_LEAN_ROOT                Root of the prepared Lean 4.30.0 install (the
#                               directory that contains `bin/lean`).
set -euo pipefail

: "${CE_ADAPTER_REPOSITORY_ROOT:?CE_ADAPTER_REPOSITORY_ROOT is required}"
: "${CE_ADAPTER_STAGE_DIR:?CE_ADAPTER_STAGE_DIR is required}"
: "${CE_LEAN_ROOT:?CE_LEAN_ROOT is required}"

source_dir="${CE_ADAPTER_REPOSITORY_ROOT}/tools/library-analyzers/lean"
build_dir="${CE_ADAPTER_STAGE_DIR}/lean"

# Stage the sources next to a repository-local `.lake` directory so Lake's
# packagesDir and buildDir land under the driver-owned staging tree instead
# of `tools/library-analyzers/lean/.lake`. That keeps build output out of
# the adapter-input digest and avoids the polluted-tree case spec §6.9
# forbids.
rm -rf "${build_dir}"
mkdir -p "${build_dir}"
cp -R "${source_dir}/." "${build_dir}/"

# Force the pinned toolchain: no Elan, no ambient PATH surprises.
export PATH="${CE_LEAN_ROOT}/bin:/usr/bin:/bin"

cd "${build_dir}"

# Offline, package-manifest-preserving build. `lake build` will refuse to
# fetch remote packages because the committed `lake-manifest.json` declares
# no external packages.
lake build ce-lean

# Lake drops the executable at `.lake/build/bin/ce-lean`. Move it to the
# staging path the parent driver reads from.
install -m 0755 ".lake/build/bin/ce-lean" "${build_dir}/ce-lean"
