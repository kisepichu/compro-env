# TASK-011: input format parse & Tera context generate

## 参照仕様

- docs/commands/init.md — 「入力形式パース」節、「Tera コンテキスト」節
- docs/spec.md — ドメインモデル (InputSpec 等)、入力形式パース未対応パターン

## 概要

`ce init` 時に AtCoder の tasks_print から入力形式・制約を取得し、
パースして Tera テンプレートコンテキストに `input_format.*` として注入する。

## 実装チェックリスト

### domain/

- [x] `Problem` に `input_format_raw: Option<String>`, `constraints_raw: Option<String>` を追加
- [x] `InputSpec`, `VarDecl`, `VarType`, `InputOp`, `OpTag`, `VarRef` を `entity.rs` に追加
  - すべて `serde::Serialize` を derive (Tera JSON 化のため)
  - `InputSpec { raw: String, ok: bool, vars: Vec<VarDecl>, ops: Vec<InputOp> }`
  - `VarDecl { name: String, math: String, var_type: VarType, dim: u8, size: Vec<String> }`
  - `VarType` enum: `Int | Str | Unknown`
  - `InputOp { tag: OpTag, depth: u8, vars: Vec<VarRef>, loop_var: Option<String>, begin: Option<String>, end: Option<String> }`
  - `OpTag` enum: `ReadLine | LoopBegin | LoopEnd`
  - `VarRef { name: String, dim: u8, size: Option<String>, index: Option<String> }`

### usecases/ — input_format パーサー

- [x] `crates/usecases/src/input_format/mod.rs` 作成
  - `pub fn parse(raw: &str, constraints: &str) -> InputSpec`
  - `raw` が空なら `InputSpec { raw: "", ok: false, vars: [], ops: [] }` を返す
  - lexer / parser / semantic を同一ファイルに実装

- [x] Lexer: 1 行のテキストをトークン列に変換
  - 前処理: `\hspace{...}\vdots` → `\vdots` (正規化)
  - トークン種: `Ident(String)`, `Num(String)`, `Subscript`, `LBrace`, `RBrace`, `Comma`,
    `Cdots` (`\ldots` `\dots` `\cdots` `...`), `Vdots` (`\vdots` `:` `⋮`)

- [x] Parser: トークン列 + 行リストからパターン検出
  - Phase 2 早期検出 → `ok: false`:
    - `raw` を `\n\n` で split、ブロック数 > 1 かつ後続ブロックが数字で始まる → クエリ型
    - ブロック数 > 1 かつ先頭ブロックが単一変数のみ → T-testcases 型
    - いずれかの行に `\text{` / `\mathrm{` が含まれる → クエリ型
  - 各行のパターン: スカラー列 / 1D 配列 (水平 cdots) / vdots → ForLoop
  - 非数値添字スカラー (アルファベット添字) → `ok: false`
  - 隣接要素 (スペースなし) → `ok: false`

- [x] Semantic: パース結果から `vars` テーブルを構築、型推定
  - 変数名小文字化 (衝突時は大文字のまま)
  - loop 0-indexed 正規化 (`begin: "0"`)
  - 制約テキストから型推定 (ヒューリスティック)

### usecases/ — インターフェース変更

- [x] `SolutionRepository::create` のシグネチャに `input_format_raw: &str` を追加

### infrastructure/ — HTML 抽出

- [x] `atcoder.rs` の `parse_tasks_print_from_html` を拡張:
  - 各問題の `<h3>入力</h3>` セクションから全 `<pre>` ブロックを取得、`\n\n` で結合
  - 各問題の `<h3>制約</h3>` セクションからテキストを取得 (HTML タグ strip)
  - `Problem.input_format_raw` / `Problem.constraints_raw` に設定

### infrastructure/ — 永続化

- [x] `contest_repository_impl.rs` の `.ce.toml` スキーマに `input_format_raw: String` を追加
  - serialize: `Option<String>` → 空文字にフォールバック
  - deserialize: `#[serde(default)]` で既存 `.ce.toml` との後方互換
- [x] `ContestRepository::get_problem` が `input_format_raw` を返すよう更新

### infrastructure/ — テンプレート展開

- [x] `SolutionRepositoryImpl::create` を更新:
  - `usecases::input_format::parse(input_format_raw, "")` を呼んで `InputSpec` 取得
  - `serde_json::to_value` で JSON 化して Tera コンテキストの `input_format` に注入

### service 層 — 呼び出し箇所の更新

- [x] `usecases/service/init.rs`: `solution_repo.create` に `input_format_raw` を渡す
- [x] `usecases/service/new_solution.rs`: `ContestRepository::get_problem` 経由で取得して渡す

## 完了条件

- [x] `cargo test --workspace` が全て pass (125 tests)
- [x] `cargo fmt --all` / `cargo clippy --workspace` がクリーン
- [x] `templates/rust/src/main.rs.tera` (サンプル) で `input_format.ok`, `input_format.vars`, `input_format.ops` が展開される
- [x] `ok: false` 問題 (クエリ型等) で `ce init` がエラーにならずフォールバックする

## 作業ログ

- 2026-04-30: 作業開始、ブランチ `feat/011-input-format-parse`
- 2026-04-30: 全チェックリスト完了、125 tests pass
