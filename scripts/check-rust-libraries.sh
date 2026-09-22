#!/usr/bin/env bash
# Compile and run the unit tests of every Rust library source file.
#
# Why not `cargo test`: `libraries/rust/` has no Cargo.toml and is not a
# workspace member, so cargo walks up to the repository-root Cargo.toml and
# runs the workspace tests instead — the `#[cfg(test)]` modules inside
# `libraries/rust/**/*.rs` are never compiled. This script compiles each file
# standalone with `rustc --test`, so dropping a new file under the library
# root is enough to get its tests executed.
#
# Assumption: every library file is self-contained or pulls its dependencies
# in with `#[path = "..."]`. `rustc --test <file>` resolves `#[path]` relative
# to the file itself, so such files keep working without extra wiring.
#
# Invoked by `ce check` (see docs/commands/check.md) with CE_REPOSITORY_ROOT /
# CE_LIBRARY_ROOT / CE_LANGUAGE set; the fallbacks below keep it runnable by
# hand (`bash scripts/check-rust-libraries.sh`).

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="${CE_REPOSITORY_ROOT:-$(dirname -- "$script_dir")}"
library_root="${CE_LIBRARY_ROOT:-$repository_root/libraries/rust}"
label="${CE_LANGUAGE:-rust}-lib"

# Keep in sync with the workspace edition (Cargo.toml `edition`).
edition="2024"

if [ ! -d "$library_root" ]; then
  echo "[$label] library root not found: $library_root" >&2
  exit 1
fi

files=()
while IFS= read -r file; do
  files+=("$file")
done < <(find "$library_root" -type f -name '*.rs' | LC_ALL=C sort)

if [ "${#files[@]}" -eq 0 ]; then
  echo "[$label] no *.rs found under $library_root"
  echo "[$label] 0 passed, 0 failed"
  exit 0
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

passed=0
failed=0

for file in "${files[@]}"; do
  display="${file#"$repository_root"/}"
  relative="${file#"$library_root"/}"
  binary="${relative//\//_}"
  binary="${binary//./_}"

  if ! rustc --edition "$edition" --test -o "$workdir/$binary" "$file"; then
    echo "[$label] $display: FAILED (compile)"
    failed=$((failed + 1))
    continue
  fi

  if ! "$workdir/$binary"; then
    echo "[$label] $display: FAILED (test)"
    failed=$((failed + 1))
    continue
  fi

  echo "[$label] $display: ok"
  passed=$((passed + 1))
done

echo "[$label] $passed passed, $failed failed"

if [ "$failed" -gt 0 ]; then
  exit 1
fi
