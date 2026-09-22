#!/bin/sh
# ce submit preprocess hook — repository-local, language-agnostic.
#
# Wire this from <repository_root>/config.toml:
#
#   [submit]
#   preprocess = "hooks/expand-libraries.sh"
#
# Contract (see docs/commands/submit.md "提出前 preprocess フック"):
#   stdin  = original source
#   stdout = bundled source
#   exit 0 = adopt stdout; non-zero = abort submission
#   cwd    = solution directory
#   env    = CE_LANGUAGE CE_OJ CE_CONTEST_ID CE_PROBLEM_CODE CE_PROBLEM_ID
#            CE_SOLUTION_NAME CE_SOLUTION_DIR CE_SOURCE_FILE CE_LANG_ID
#            CE_PROJECT_ROOT
#
# Language branches live HERE. Adding a language means adding a case arm and
# a hooks/<lang>_expand.{py,sh} sibling; the Rust binaries stay unchanged.
set -eu

here="$(cd "$(dirname "$0")" && pwd)"

case "${CE_LANGUAGE:-}" in
rust)
    exec python3 "$here/rust_expand.py"
    ;;
cpp|lean)
    # TODO(#<follow-up issue>): implement C++ / Lean bundlers (oj-bundle etc.).
    # Passthrough keeps the hook contract intact so submissions of these
    # languages continue to work with their raw source.
    exec cat
    ;;
*)
    exec cat
    ;;
esac
