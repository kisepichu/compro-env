#!/usr/bin/env bash
# Elaborate every Lean library source file.
#
# Why not `lake build`: the previous `check_command` was `lake build`, but
# `libraries/lean/` holds no lakefile (the only Lake package in this
# repository is the analyzer at `tools/library-analyzers/lean/`). Lake does
# not search parent directories, so the command failed with
# `no configuration file with a supported extension` on every run — nothing
# was ever checked (issue #122). Calling `lean` per file needs no package
# metadata, so dropping a new file under the library root is enough to get it
# elaborated.
#
# `-DwarningAsError=true`: plain `lean` exits 0 on a file whose proofs are
# `sorry`, which would let an unproven theorem pass the check. Promoting
# warnings turns `declaration uses 'sorry'` into an error, so an incomplete
# proof fails here.
#
# Assumption: every library file is self-contained and imports only modules
# shipped with the toolchain. `lean <file>` resolves `import` through
# `LEAN_PATH`, and this repository compiles no `.olean` artifacts for
# `libraries/lean`, so a file importing a sibling library module fails with an
# unknown-module error. Introducing cross-file imports therefore means
# introducing a lakefile for the library root and revisiting this script.
#
# Invoked by `ce check` (see docs/commands/check.md) with CE_REPOSITORY_ROOT /
# CE_LIBRARY_ROOT / CE_LANGUAGE set; the fallbacks below keep it runnable by
# hand (`bash scripts/check-lean-libraries.sh`).

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="${CE_REPOSITORY_ROOT:-$(dirname -- "$script_dir")}"
library_root="${CE_LIBRARY_ROOT:-$repository_root/libraries/lean}"
label="${CE_LANGUAGE:-lean}-lib"

if [ ! -d "$library_root" ]; then
  echo "[$label] library root not found: $library_root" >&2
  exit 1
fi

files=()
while IFS= read -r file; do
  files+=("$file")
done < <(find "$library_root" -type f -name '*.lean' | LC_ALL=C sort)

if [ "${#files[@]}" -eq 0 ]; then
  echo "[$label] no *.lean found under $library_root"
  echo "[$label] 0 passed, 0 failed"
  exit 0
fi

passed=0
failed=0

for file in "${files[@]}"; do
  display="${file#"$repository_root"/}"

  if ! lean -DwarningAsError=true "$file"; then
    echo "[$label] $display: FAILED (elaboration)"
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
