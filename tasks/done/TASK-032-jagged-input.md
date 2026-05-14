# TASK-032: jagged array input 対応

## 参照仕様

- docs/commands/init.md (入力形式パース > JaggedRow, loop_jagged)

## 実装チェックリスト

### domain/

- [x] `VarDecl` に `is_jagged: bool` フィールド追加
- [x] `OpTag` に `LoopJagged` variant 追加
- [x] `InputOp` に `scalars: Vec<VarRef>`, `size_var: Option<VarRef>`, `elem_var: Option<VarRef>` フィールド追加
- [x] `InputFormatKind` に `Jagged` variant 追加
- [x] `InputSpec::kind()` に `Jagged` 判定追加 (Triangle の次、Iter の前)
- [x] `Display for InputFormatKind` に `Jagged => "jagged"` 追加

### usecases/

- [x] `RawLine::JaggedRow` variant 追加
- [x] `IntermOp::LoopJagged` variant 追加
- [x] `try_parse_jagged_row(tokens)` 実装: 右端が `{row_idx, SIZE_VAR_{row_idx}}` のパターン検出
- [x] `parse_line` から `try_parse_jagged_row` を呼ぶ
- [x] `build_intermediate` vdots ブロック処理に JaggedRow 検出を追加
- [x] セマンティック解析で `IntermOp::LoopJagged` → `InputOp { tag: LoopJagged, ... }` 変換
- [x] セマンティック解析で `is_size=true` (size_var)、`is_jagged=true` (elem_var) を設定

### interfaces/ (変更なし)

### infrastructure/ (template)

- [x] `templates/rust/src/main.rs.tera` に `loop_jagged` op の Rust コード生成を追加
  - `main` fn: `let mut ...: Vec<_> = Vec::new();` + `for _ in 0..end { input!{...} ... .push(...) }`
  - `solve` fn 引数: `is_jagged=true` → `Vec<Vec<T>>`、`is_size=true,dim=1` → `Vec<usize>` 対応

## 完了条件

- [x] abc457_b (`scalars=[], size_var=l, elem_var=a`) の入力形式が正しくパースされる
- [x] abc446_b (2行ボディ、`scalars=[], size_var=l, elem_var=x`) が正しくパースされる
- [x] abc226_c (`scalars=[t], size_var=k, elem_var=a`) が正しくパースされる
- [x] `InputFormatKind::kind()` が `Jagged` を返す
- [x] `cargo test --all && cargo clippy --all --all-features -- -D warnings && cargo fmt --all --check` 通過

## 作業ログ

- 2026-05-14: 作業開始
