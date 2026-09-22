#!/usr/bin/env bash
# Syntax-check every C++ library source file.
#
# Why a script: the previous `check_command` named
# `libraries/cpp/algebra/monoid.hpp` directly, so a second library file was
# never checked (issue #122). Enumerating the library root instead means
# dropping a new file under it is enough to get it compiled.
#
# Compiler: `clang++` from PATH, not the LLVM 22.1.0 install of the prepared
# adapter set (`tools/library-analyzers/dependencies.toml`). `ce check` is a
# cheap pre-flight gate that must run without the ~700MB prepare download, and
# its sanitized environment (docs/commands/check.md) forwards no path to the
# prepared set. The design doc §6 requirement that a `check_command` verify
# the pinned toolchain version and fail on mismatch therefore stays
# unimplemented here, exactly as it does in check-rust-libraries.sh; the
# pinned identity is still enforced where it gates publication, i.e. against
# the analyzer's observed toolchain in site-data / `ce verify`. Consequence:
# the exact warning set depends on the local clang version, so `-Werror` can
# fire locally on a warning CI does not see (and the reverse). Keep library
# headers warning-clean under both.
#
# Flags mirror `tools/library-analyzers/cpp/compile-profile.toml`, which the
# analyzer reads, so a file that passes analysis also passes here:
# `cxx_standard` as `-std`, `include_roots` as `-I <library root>` (one library
# can include another by root-relative path). `defines` is deliberately not
# mirrored: `CE_LIBRARY_ANALYSIS=1` marks the analysis pass, and the check
# should compile the code a solution author gets.
#
# Assumption: every header is self-contained (includes what it uses), because
# `-fsyntax-only` compiles it as a standalone translation unit.
#
# Invoked by `ce check` (see docs/commands/check.md) with CE_REPOSITORY_ROOT /
# CE_LIBRARY_ROOT / CE_LANGUAGE set; the fallbacks below keep it runnable by
# hand (`bash scripts/check-cpp-libraries.sh`).

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="${CE_REPOSITORY_ROOT:-$(dirname -- "$script_dir")}"
library_root="${CE_LIBRARY_ROOT:-$repository_root/libraries/cpp}"
label="${CE_LANGUAGE:-cpp}-lib"

# Keep in sync with `cxx_standard` in
# `tools/library-analyzers/cpp/compile-profile.toml`.
std="c++20"

if [ ! -d "$library_root" ]; then
  echo "[$label] library root not found: $library_root" >&2
  exit 1
fi

files=()
while IFS= read -r file; do
  files+=("$file")
done < <(find "$library_root" -type f \( -name '*.hpp' -o -name '*.cpp' \) | LC_ALL=C sort)

if [ "${#files[@]}" -eq 0 ]; then
  echo "[$label] no *.hpp or *.cpp found under $library_root"
  echo "[$label] 0 passed, 0 failed"
  exit 0
fi

passed=0
failed=0

for file in "${files[@]}"; do
  display="${file#"$repository_root"/}"

  # No `-x`: the driver's extension mapping parses `.hpp` as a header (so
  # `#pragma once` is legal) and `.cpp` as a translation unit. Forcing
  # `-x c++` would make every header a main file and trip
  # `-Wpragma-once-outside-header`.
  #
  # `-Wno-unused-command-line-argument`: a wrapped compiler (nix, ccache,
  # Homebrew) injects link-time flags that `-fsyntax-only` never consumes.
  # That warning describes the driver invocation, not the library source, so
  # under `-Werror` it would fail the check for reasons a contributor cannot
  # fix in the library.
  if ! clang++ -std="$std" -I "$library_root" \
    -Wall -Wextra -Werror -Wno-unused-command-line-argument \
    -fsyntax-only "$file"; then
    echo "[$label] $display: FAILED (syntax)"
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
