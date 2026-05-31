#!/bin/sh
#
# ce submit preprocess hook — example.
#
# Point `[submit].preprocess` in ~/.config/ce/config.toml at this file:
#
#   [submit]
#   preprocess = "~/.config/ce/hooks/submit-preprocess.sh"
#
# Contract (see docs/commands/submit.md "提出前 preprocess フック"):
#   stdin  = the original source
#   stdout = the source to submit (this is what ce sends to the OJ)
#   exit 0 = use stdout; non-zero = abort submission (stderr is shown)
#   cwd    = the solution directory (the crate root, for Rust)
#   env    = CE_LANGUAGE CE_OJ CE_CONTEST_ID CE_PROBLEM_CODE CE_PROBLEM_ID
#            CE_SOLUTION_NAME CE_SOLUTION_DIR CE_SOURCE_FILE CE_LANG_ID
#
# Language- and OJ-specific branching lives HERE, not in the app: add a `case`
# arm per language. The app stays unchanged when you add a language or an OJ.
#
set -eu

case "${CE_LANGUAGE:-}" in
rust)
    # Expand local library crates (and the deps you use) into a single file with
    # cargo-equip, so a submission that `use`s your own library does not hit a
    # "unresolved import" compile error on the judge.
    #
    # cargo-equip prints the bundled source to stdout — exactly our submission
    # source. It also keeps your `main` readable at the top and folds the library
    # into a module below, which is nice on the submission page.
    #
    # `--check` type-checks the bundled output in a temp package before we submit.
    # If expansion produced something that won't compile, cargo-equip exits
    # non-zero, this hook exits non-zero, and ce aborts the submission. That is
    # the in-hook self-verification the design relies on (requirement 2).
    if ! command -v cargo-equip >/dev/null 2>&1; then
        echo "submit-preprocess: cargo-equip not found; install with \`cargo install cargo-equip\`" >&2
        exit 1
    fi

    # The crate's bin target is named "{problem_code}-{solution_name}"
    # (see templates/rust/Cargo.toml.tera).
    bin="${CE_PROBLEM_CODE}-${CE_SOLUTION_NAME}"

    # On Library Checker we expand everything. On AtCoder, libraries provided by
    # the judge (e.g. ac-library-rs) could be left un-expanded with `--exclude`,
    # but expanding all bundled libs is always safe, so the default needs no
    # branching. Tune the flags below to taste.
    exec cargo equip \
        --bin "$bin" \
        --resolve-cfgs \
        --remove docs comments \
        --minify libs \
        --rustfmt \
        --check
    ;;
*)
    # No expansion configured for this language: submit the source unchanged.
    # Replace this with your own toolchain (e.g. oj-bundle for C++).
    cat
    ;;
esac
