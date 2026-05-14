# TASK-018: クエリ型入力対応

## 参照仕様

- docs/commands/init.md — Phase 1 非対応パターン表 (クエリ型)
- docs/spec.md — 入力形式パース 未対応パターン

## 背景

`\text{query}_i` / `\mathrm{Query}_i` を含む行や、
複数 `<pre>` ブロック + 数字始まりサブ形式は現在 `ok: false` になる。

典型パターン:
```
N Q
\text{query}_1
\vdots
\text{query}_Q

1 x

2 x k
```

これらはプリアンブル変数 (N, Q) を抽出し、
Q 回のクエリループとして `LoopBegin + LoopEnd` を emit することで
最低限有用なコード生成 (preamble vars + loop stub) が可能になる。

## チェックリスト

### usecases/ (パーサー)

- [x] `RawLine::QueryLine { loop_bound: String }` を追加
- [x] `parse_line()` の `\text{}/\mathrm{}` 処理を修正:
  - `Err` から `Ok(RawLine::QueryLine { loop_bound })` に変更
  - subscript なし/malformed は `Err(ParseError::Unknown)` のまま
- [x] `is_loop_or_grid` クロージャに `QueryLine` を追加
- [x] vdots ブロック拡張ロジックに query kind を追加
  - `RowKind` enum (Loop/Grid/Query) で kind を管理
- [x] vdots ブロック処理に `is_query` 分岐を追加:
  - QueryLine ブロック → `LoopBegin + LoopEnd`(body なし)
  - loop_bound は last after-row (なければ last before-row) の subscript
- [x] standalone `QueryLine` (vdots ブロック外) → `Err(ParseError::Unknown)`
- [x] `parse()` の早期検出を修正:
  - `block0.contains("\\text{")` / `"\\mathrm{"` の early rejection を削除
  - 複数ブロック + digit 先頭の early rejection: block0 に query marker がある場合は通過させる
- [x] テスト追加:
  - `\text{query}_Q` 形式 → `ok=true`, vars `n,q`, ops `[ReadLine, LoopBegin(q), LoopEnd]`
  - `\mathrm{Query}_Q` 形式 → 同上
  - 複数ブロック形式 → 同上 (block[1..] 無視)
  - `\text{query}` 添字なし → `ok=false`
  - マーカーなし multi-block → `ok=false`

### templates/rust/src/main.rs.tera

- [x] `op.tag == "loop_begin"` の分岐に empty-body 検出を追加:
  - `body_op.tag == "loop_end"` のとき: TODO コメント + ループ雛形を生成

### docs/

- [x] `docs/commands/init.md` の Phase 1 対応パターン表に追記
- [x] `docs/commands/init.md` の Phase 1 非対応表から削除 (abc241-D, abc248-D のみ。typical90-L は残す)
- [x] `docs/spec.md` の未対応パターン表から削除 (同上)

## 完了条件

- [x] `cargo test --workspace` が全て pass
- [x] `cargo fmt --all` / `cargo clippy --workspace` がクリーン
- [x] abc241-D 形式の raw で `ok=true`, vars=[n,q], LoopBegin(q) が生成される
- [x] abc248-D 形式の raw で `ok=true`, vars=[n,q], LoopBegin(q) が生成される

## 作業ログ

- 2026-05-03: 作業開始
