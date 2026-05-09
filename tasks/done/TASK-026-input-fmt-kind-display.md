# TASK-026: input fmt kind 表示

## 参照仕様

- docs/commands/init.md — 「出力形式」節・「input fmt 行の形式」節
- docs/spec.md — `InputFormatKind` 導出値

## 実装チェックリスト

### domain/ — `InputFormatKind` + `InputSpec::kind()`

- [x] `InputFormatKind` enum を `domain/src/entity.rs` に追加
  - variants: `Plain`, `Loop`, `Iter`, `Testcase`, `Query`, `QueryTypes(usize)`, `Fail`
- [x] `impl std::fmt::Display for InputFormatKind` を実装
  - `Plain` → `"plain"`, `Loop` → `"loop"`, `Iter` → `"iter"`, `Testcase` → `"testcase"`
  - `Query` → `"query"`, `QueryTypes(n)` → `"query({n})"`, `Fail` → `"FAIL"`
- [x] `InputSpec::kind() -> InputFormatKind` メソッドを実装 (優先順に従う)
  - `!ok` → `Fail`
  - `query_types` 非空 → `QueryTypes(query_types.len())`
  - `query_body` 非空 → `Query`
  - `testcase_body` 非空 → `Testcase`
  - `iteration_ops` 非空 → `Iter`
  - `ops` に `LoopBegin` あり → `Loop`
  - それ以外 → `Plain`

### usecases/ — `InitResult` に kind リストを追加

- [x] `InitResult` に `input_fmt_kinds: Vec<(String, InputFormatKind)>` フィールドを追加
  - タプルは `(problem_code, kind)`
- [x] `build_result()` で各問題について `crate::input_format::parse()` → `spec.kind()` を呼び、`input_fmt_kinds` を構築する
- [x] `already_initialized` 分岐の `InitResult` にも空 vec を設定する

### infrastructure/ — サマリー出力

- [x] `shell/mod.rs` の init サマリー出力に `input fmt` 行を追加
  - `  input fmt   {a:plain  b:plain  ...}  [{ok_count}/{total} ok]`
  - `FAIL` のみで `ok_count` にカウントしない
  - `input_fmt_kinds` が空の場合は行を出力しない

## 完了条件

- [x] `cargo test --workspace` が全て pass
- [x] `cargo clippy --workspace --all-features -- -D warnings` がクリーン
- [x] `cargo fmt --all --check` がクリーン

## 作業ログ

- 2026-05-09: 作業開始・完了
