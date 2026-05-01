# TASK-012: VarDecl.is_size フィールド追加 + solve 引数生成テンプレート

## 参照仕様

- docs/commands/init.md — 「`vars` の形式」節、「テンプレート例 (Rust)」節
- docs/spec.md — VarDecl ドメインモデル

## 実装チェックリスト

### domain/

- [x] `VarDecl` に `is_size: bool` を追加 (`entity.rs`)
  - `serde::Serialize` 済みなので自動で JSON に含まれる

### usecases/

- [x] `usecases/input_format/mod.rs` の semantic 解析で `is_size` を計算して設定
  - `VarDecl` 全件を構築した後、各 var の `name` が
    他の var の `size` に含まれる OR 任意の `InputOp::LoopBegin` の `end` と一致するなら `is_size = true`

### infrastructure/ (テンプレート)

- [x] `templates/rust/src/main.rs.tera` を仕様通りに書き換え
  - `ok=true`: `fn solve(n: usize, a: Vec<i64>)` + `main` で `input!` → `solve(...)` 呼び出し
  - `ok=false`: フォールバック (`solve<R>(src)` スタイル)
  - 型対応表に従い `is_size` / `dim` / `var_type` で型を決定

## 完了条件

- [x] `cargo test --workspace` が全て pass
- [x] `cargo fmt --all` / `cargo clippy --workspace` がクリーン
- [x] `input_format.ok=true` の問題で `solve(n, a)` 形式のコードが生成される
- [x] `input_format.ok=false` の問題でフォールバックコードが生成される

## 作業ログ

- 2026-04-30: 作業開始、feat/011-input-format-parse ブランチに追加コミット
