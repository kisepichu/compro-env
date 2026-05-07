# TASK-022: T-testcases 型対応

## 参照仕様

- docs/commands/init.md — パイプライン (T-testcases 型検出)、`testcase_body` の形式、Tera コンテキスト表

## チェックリスト

- [x] `domain/entity.rs`: `InputSpec` に `testcase_body: Vec<VarDecl>` フィールドを追加
- [x] `usecases/input_format/mod.rs`: テスト追加 (TDD)
  - [x] `testcase_body_abc238d` — `T\n\na s` → testcase_body=[a,s], vars=[t(is_size:true)]
  - [x] `testcase_body_empty_when_block1_not_scalar` — block 1 がスカラー以外 → testcase_body=[]
  - [x] `testcase_body_empty_for_single_block` — block が 1 つ → testcase_body=[]
- [x] `usecases/input_format/mod.rs`: T-testcases パース実装
  - [x] `not_ok` 返却を T-testcases パースに変換
  - [x] block 1 をスカラー変数リストとしてパース (`parse_scalar_block` ヘルパー)
  - [x] block 0 の変数に `is_size: true` を設定
- [x] `templates/rust/src/main.rs.tera`: `testcase_body` ブランチ追加
  - [x] `testcase_body | length > 0` のとき `for _ in 0..t { input!{...}; todo!() }` を生成
- [x] `cargo test --all` 全通過

## 完了条件

- [x] abc238-D 形式 (`T\n\na s`) で `ok: true`、`testcase_body = [{a, int}, {s, str}]`、`vars = [{t, usize}]`
- [x] テンプレートが `for _ in 0..t { input! { a: i64, s: String, } todo!() }` を生成
- [x] 全テスト通過

## 作業ログ

- 2026-05-07: 作業開始・完了
