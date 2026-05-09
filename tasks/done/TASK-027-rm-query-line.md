# TASK-027: {query}_N 形式 QueryLine 対応 (abc453-G スタイル)

## 参照仕様

- docs/commands/init.md — Parser 節「{query}_N (大文字小文字不問; {\rm Query}_N 等 LaTeX 書式コマンドでラップされた形を含む) → QueryLine」

## 背景

abc453-G (Copy Query) の入力形式:
```
N M Q
{\rm Query}_1
{\rm Query}_2
\vdots
{\rm Query}_Q

1 X_i Y_i
2 X_i Y_i Z_i
3 X_i L_i R_i
```

`{\rm Query}_Q` のような LaTeX 書式コマンドラッパーが付いたクエリマーカーを QueryLine として認識できていない。
トークナイザーは `\rm` を未知コマンドとして無視するため、`{\rm Query}_Q` は
`[LBrace, Space, Ident("Query"), RBrace, Subscript, Ident("Q")]` とトークン化される。
`parse_line` の QueryLine 検出では先頭が `Ident` であることを期待しているため、`LBrace` が先頭だと認識失敗し `ok=false` になる。

## 実装チェックリスト

### usecases/

- [x] `parse_line` に「先頭 `{Ident}` グルーピングを剥がす」前処理を追加
  - `tokens` が `[LBrace, (Space*), Ident(x), (Space*), RBrace, ...]` で始まる場合、
    `[Ident(x), ...]` に置き換えてから既存の QueryLine 検出を走らせる
  - 対象: `{\rm Query}_Q` → `Query_Q` と等価に処理
  - 副作用なし: Ident が "query" 以外なら下流の処理 (scalar/subscripted var) がそのまま扱う

## 完了条件

- [x] `abc453g_rm_query_numbered_subtypes` テストが GREEN
- [x] 既存テスト全通過 (193 tests)
- [x] `cargo clippy --workspace --all-features -- -D warnings` 通過
- [x] `cargo fmt --all --check` 通過

## 作業ログ

- 2026-05-09: 作業開始
