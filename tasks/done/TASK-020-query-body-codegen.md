# TASK-020: 単一形式クエリの変数自動生成 (query_body)

## 参照仕様

- docs/commands/init.md — Query sub-block 解析 > `query_body` の形式 / テンプレート例
- docs/spec.md — ドメインモデル > InputSpec / query_body

## 概要

abc334_d 形式のような「クエリは1種類だが入力変数がある」ケースで、
non-numeric 先頭の sub-block (`X` 等) を解析して `query_body: Vec<VarDecl>` に格納し、
テンプレートで `input! { x: i64, }` 付きループを自動生成する。

## チェックリスト

### domain/entity.rs

- [x] `InputSpec` に `query_body: Vec<VarDecl>` フィールドを追加
  - デフォルト値は `vec![]`
  - 全既存 `InputSpec { ... }` 構築に `query_body: vec![]` を追加

### usecases/input_format/mod.rs (テスト先行)

- [x] テスト: `query_body_single_var` — abc334_d 形式 (既存 `query_subblock_non_numeric_skipped` を更新)
  - raw: `"N Q\nR_1 \\ldots R_N\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n\nX\n"`
  - 期待: `ok=true`, `query_types.len() == 0`, `query_body.len() == 1`
  - `query_body[0]`: `name="x"`, `var_type="int"`, `dim=0`
- [x] テスト: `query_body_multi_var` — 複数変数の non-numeric sub-block
- [x] テスト: `query_body_ignored_when_query_types_present` — query_types 非空のとき query_body は空

- [x] 実装: `parse_query_subblocks()` を更新して `(Vec<QueryTypeDecl>, Vec<VarDecl>)` を返す
  - 先頭トークンが Num でない sub-block かつ `query_types` 空 かつ `query_body` 未設定 → `query_body` に設定
  - `query_types` 非空の場合は `query_body` を空にして返す

### templates/rust/src/main.rs.tera

- [x] solve 本体: `query_types` 空 かつ `query_body` 非空 のとき `input! { ... }` 付きループ生成

## 完了条件

- [x] `cargo test --workspace` が全て pass (63 tests)
- [x] `cargo fmt --all` / `cargo clippy --workspace` がクリーン
- [x] abc334_d 形式で `query_body.len() == 1`、`input! { x: i64, }` 付きループ生成を手動確認

## 作業ログ

- 2026-05-07: 作業開始・完了
