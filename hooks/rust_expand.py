#!/usr/bin/env python3
"""Recursively inline Rust `mod` declarations for submit-time bundling.

Contract (see docs/operations/library-expand.md):
  stdin:  Rust source (typically the solution's src/main.rs).
  stdout: bundled Rust source with `#[path]` mod chains inlined.
  argv[1] (optional): entry file path used to derive the base directory for
                      relative path resolution. When absent we use
                      `$CE_SOURCE_FILE` from the environment; when that is
                      also absent we fall back to `cwd/src/main.rs`.

Exit codes:
  0 = success
  1 = file not found
  2 = cycle detected
  3 = non-UTF-8 file
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path
from typing import NoReturn

# One regex matches every `mod NAME;` declaration — with or without a leading
# `#[path = "…"]` (and any number of adjacent attributes). Anchored to a line
# start (re.MULTILINE) so we do not misfire on the tail of a string literal.
COMBINED_MOD_RE = re.compile(
    r"""^[ \t]*                                    # anchored to line start (re.MULTILINE)
        (?P<attrs>(?:\#\s*\[[^\]]*\]\s*)*)         # zero or more leading attributes (any kind, incl. #[path]); \s* between allows adjacent-with-no-space form
        (?P<vis>pub(?:\s*\(\s*[^)]+\s*\))?\s+)?
        mod\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*;""",
    re.MULTILINE | re.VERBOSE,
)

# Individual `#[path = "..."]` picker for the callback below.
PATH_ATTR_RE = re.compile(r'\#\s*\[\s*path\s*=\s*"([^"]+)"\s*\]\s*')


def die(code: int, msg: str) -> NoReturn:
    print(f"rust_expand: {msg}", file=sys.stderr)
    sys.exit(code)


def read_utf8(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError:
        die(1, f"file not found: {path}")
    except UnicodeDecodeError as e:
        die(3, f"non-UTF-8 file {path}: {e}")


def expand_source(source: str, entry_dir: Path, visited: set[Path]) -> str:
    # Single-pass expansion: one regex covers both `#[path] mod NAME;` and
    # bare `mod NAME;`. Passing them as one alternation guarantees each
    # declaration is scanned exactly once, at the correct nesting level with
    # the correct `entry_dir`. A two-pass approach would let a bare `mod`
    # that was left passthrough by a sub-file get re-scanned in the outer
    # `entry_dir` after the sub-body was spliced in, which would wrongly
    # resolve it against a same-named file in the outer directory.
    def repl(m: re.Match[str]) -> str:
        name = m.group("name")
        vis = (m.group("vis") or "").strip()
        vis_prefix = f"{vis} " if vis else ""
        # Attributes may appear in any order (e.g. `#[allow(...)] #[path = "..."]`
        # or `#[path = "..."] #[cfg(test)]`). Extract the last `#[path]` — Rust
        # only honors one — and preserve everything else on top of the expanded
        # `mod NAME { ... }` so semantics (cfg-gating, allow-lints) survive.
        attrs_raw = m.group("attrs") or ""
        path_matches = list(PATH_ATTR_RE.finditer(attrs_raw))
        rel_path = path_matches[-1].group(1) if path_matches else None
        other_attrs = PATH_ATTR_RE.sub("", attrs_raw).strip()
        extras_prefix = f"{other_attrs}\n" if other_attrs else ""
        if rel_path is not None:
            target = (entry_dir / rel_path).resolve()
            if not target.is_file():
                die(1, f"file not found: {target}")
        else:
            target = None
            for cand in (entry_dir / f"{name}.rs", entry_dir / name / "mod.rs"):
                if cand.is_file():
                    target = cand.resolve()
                    break
            if target is None:
                # Passthrough: leave the declaration verbatim (attributes and
                # all) so the caller can still compile against std / external
                # crates. Warn once.
                print(
                    f"rust_expand: warning: unresolved mod {name}",
                    file=sys.stderr,
                )
                return m.group(0)
        if target in visited:
            die(2, f"cycle detected: {target}")
        visited.add(target)
        body = read_utf8(target)
        expanded = expand_source(body, target.parent, visited)
        # NOTE: DFS 巻き戻し時に visited から抜くのは意図的。sibling 経由で同じ
        # ファイルが再展開される「diamond dependency」(A→B→D, A→C→D) を許すため。
        # Rust の module システム上 `crate::b::d` と `crate::c::d` は別 module なので
        # 両方に `mod d { … }` を emit するのが正しい。この行を削除するとリグレッション。
        visited.discard(target)
        return f"{extras_prefix}{vis_prefix}mod {name} {{\n{expanded}\n}}"

    return COMBINED_MOD_RE.sub(repl, source)


def resolve_entry_file(argv: list[str]) -> Path:
    if len(argv) >= 2:
        return Path(argv[1]).resolve()
    env = os.environ.get("CE_SOURCE_FILE")
    if env:
        return Path(env).resolve()
    return (Path.cwd() / "src" / "main.rs").resolve()


def main() -> None:
    entry = resolve_entry_file(sys.argv)
    entry_dir = entry.parent
    source = sys.stdin.read()
    visited: set[Path] = {entry}
    out = expand_source(source, entry_dir, visited)
    if not out.endswith("\n"):
        out += "\n"
    sys.stdout.write(out)


if __name__ == "__main__":
    main()
