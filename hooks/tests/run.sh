#!/usr/bin/env bash
# Regression tests for hooks/rust_expand.py.
#
# Layout: hooks/tests/fixtures/rust/<case>/{main.rs.in,main.rs.expected,...}
# For each success case we feed main.rs.in on stdin (passing an absolute
# path as argv[1] to fix the base dir) and diff stdout against
# main.rs.expected. Fail cases (cycle, missing) assert on exit code and
# stderr fragment.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
FIXTURES="$HERE/fixtures"
SCRIPT="$HERE/../rust_expand.py"

fail=0

diff_case() {
    local case_dir="$1"
    local expected_stderr_fragment="${2:-}"
    local entry="$case_dir/main.rs.in"
    local expected="$case_dir/main.rs.expected"
    local stderr_log; stderr_log="$(mktemp)"
    local actual_out; actual_out="$(mktemp)"
    # Run the bundler with its exit code captured separately so we can
    # distinguish "bundler crashed / non-zero exit" from "content mismatch".
    # Piping directly into diff would let `pipefail` mask the bundler exit
    # as a diff failure and print the misleading "(stdout diff)" label.
    local py_exit=0
    python3 "$SCRIPT" "$entry" <"$entry" >"$actual_out" 2>"$stderr_log" || py_exit=$?
    if [ "$py_exit" -ne 0 ]; then
        echo "FAIL: $case_dir (bundler exit $py_exit)" >&2
        cat "$stderr_log" >&2
        rm -f "$stderr_log" "$actual_out"
        fail=1
        return
    fi
    if ! diff -u "$expected" "$actual_out"; then
        echo "FAIL: $case_dir (stdout diff)" >&2
        cat "$stderr_log" >&2
        rm -f "$stderr_log" "$actual_out"
        fail=1
        return
    fi
    if [ -n "$expected_stderr_fragment" ]; then
        if ! grep -q -F "$expected_stderr_fragment" "$stderr_log"; then
            echo "FAIL: $case_dir stderr missing '$expected_stderr_fragment'" >&2
            cat "$stderr_log" >&2
            rm -f "$stderr_log" "$actual_out"
            fail=1
            return
        fi
    fi
    rm -f "$stderr_log" "$actual_out"
    echo "ok:   $case_dir"
}

exit_case() {
    local case_dir="$1"
    local entry="$case_dir/main.rs.in"
    local expected_exit="$2"
    local expected_stderr_fragment="$3"
    local actual_stderr
    set +e
    actual_stderr="$(python3 "$SCRIPT" "$entry" <"$entry" 2>&1 >/dev/null)"
    # NOTE: `local actual_exit=$?` は宣言と代入を 1 行で行うことで、直前の
    # コマンド置換の exit code を確実に捕捉する。もし 2 行に分けて `local
    # actual_exit` を先に書いてしまうと、`local` 自身の exit code (0) が `$?`
    # を上書きして actual_exit が常に 0 になる。触るときは注意。
    local actual_exit=$?
    set -e
    if [ "$actual_exit" -ne "$expected_exit" ]; then
        echo "FAIL: $case_dir: exit=$actual_exit (want $expected_exit)" >&2
        echo "stderr: $actual_stderr" >&2
        fail=1
        return
    fi
    case "$actual_stderr" in
        *"$expected_stderr_fragment"*)
            echo "ok:   $case_dir (exit $actual_exit)" ;;
        *)
            echo "FAIL: $case_dir: stderr missing '$expected_stderr_fragment'" >&2
            echo "stderr: $actual_stderr" >&2
            fail=1 ;;
    esac
}

diff_case "$FIXTURES/rust/basic"
diff_case "$FIXTURES/rust/nested"
diff_case "$FIXTURES/rust/passthrough" "warning: unresolved mod std_only_no_local_file"
diff_case "$FIXTURES/rust/diamond"

exit_case "$FIXTURES/rust/cycle" 2 "cycle detected"
exit_case "$FIXTURES/rust/missing" 1 "file not found"

# End-to-end via the shell entrypoint (CE_LANGUAGE=rust).
end_to_end_rust() {
    local case_dir="$FIXTURES/rust/basic"
    local entry="$case_dir/main.rs.in"
    local expected="$case_dir/main.rs.expected"
    # Pipe to diff; capturing via $() would strip the trailing newline.
    if ! CE_LANGUAGE=rust CE_SOURCE_FILE="$entry" \
            bash "$HERE/../expand-libraries.sh" <"$entry" \
            | diff -u "$expected" -; then
        echo "FAIL: shell rust end-to-end" >&2
        fail=1
    else
        echo "ok:   shell rust end-to-end"
    fi
}
end_to_end_rust

# End-to-end passthrough for cpp / lean.
passthrough_lang() {
    local lang="$1"
    local sample; sample="hello, $lang"
    local expected; expected="$(mktemp)"
    printf '%s' "$sample" >"$expected"
    # Pipe stdin/stdout directly into `diff` so a regression that appends a
    # trailing newline (or drops one) is caught. `$(…)` capture would strip
    # trailing `\n` and hide such regressions.
    if ! printf '%s' "$sample" | CE_LANGUAGE="$lang" \
            bash "$HERE/../expand-libraries.sh" \
            | diff -u "$expected" -; then
        echo "FAIL: $lang passthrough" >&2
        rm -f "$expected"
        fail=1
        return
    fi
    rm -f "$expected"
    echo "ok:   $lang passthrough"
}
passthrough_lang cpp
passthrough_lang lean
passthrough_lang unknown

exit "$fail"
