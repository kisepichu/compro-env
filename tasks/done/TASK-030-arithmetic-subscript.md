# TASK-030: 算術式添字対応 (ce init 入力形式パース)

## 参照仕様

- docs/commands/init.md — 「添字の分類ルール」「算術式添字の構築ルール」「Semantic Analysis」「Phase 1 対応パターン」

## 背景

`{N-1}`, `{2N}` 等の算術式を含む添字が `ok: false` になる問題を修正する。

- abc448_d: `U_{N-1} V_{N-1}` のループ上限 `{N-1}` が `"N1"` にパースされて valid_loop_bounds 検証を通過できない
- tupc2024_k: `A_{2N}` の配列サイズ `{2N}` が `"2N"` にパースされて size 解決が壊れる

## 実装チェックリスト

### usecases/ (crates/usecases/src/input_format/mod.rs)

- [x] Lexer: `Token` に `Plus`, `Minus`, `Star` を追加; `tokenize_line` で `+`, `-`, `*` をトークン化
- [x] `read_subscript_value` `{...}` ブランチ: 算術式を構築する
  - `Plus`/`Minus`/`Star` → そのまま式文字列に連結
  - 隣接する `Num Ident` (演算子なし) → `*` を自動挿入
  - 隣接する `Ident Num` (演算子なし) → `*` を自動挿入
  - 全 Ident を小文字化して返す (e.g. `{2N}` → `"2*n"`, `{N-1}` → `"n-1"`)
- [x] `valid_loop_bounds`: 算術式対応 — 式中の全識別子が宣言済みスカラーなら OK
- [x] `is_size` 計算: size/end 文字列から識別子を抽出して判定 (e.g. `"2*n"` → `n`)

## 完了条件

- [x] `cargo test --workspace` 全通過
- [x] abc448_d 形式 (`N / A_1 \dots A_N / U_1 V_1 / \vdots / U_{N-1} V_{N-1}`) が `ok: true`、`end = "n-1"`
- [x] tupc2024_k 形式 (`N / A_1 A_2 \ldots A_{2N}`) が `ok: true`、size = `"2*n"`
- [x] `cargo clippy --all --all-features -- -D warnings` 警告なし

## 作業ログ

- 2026-05-10: 作業開始・完了
