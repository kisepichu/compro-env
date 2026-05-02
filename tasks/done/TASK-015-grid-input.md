# TASK-015: 文字グリッド (2D添字) 入力対応

## 参照仕様

- docs/commands/init.md — 文字グリッド (2D添字) の検出ルール節

## チェックリスト

### usecases/ (パーサー)

- [x] `RawLine::GridRow(RawVar)` バリアントを追加
- [x] `try_parse_grid_row()` 実装: `X_{2D} Cdots X_{2D}` パターン検出、行インデックス抽出
  - 両側の変数名が一致すること
  - 少なくとも一方の添字が「複数部分」(複数トークン or カンマ区切り or 2文字以上の単一トークン)
  - 行全体がこのパターンのみ
- [x] `build_intermediate()` に `GridRow` → ループ行として処理する分岐を追加
  - `GridRow` を `LoopRow` 相当として vdots ブロック検出に含める
- [x] `IntermOp::ReadGridRow(String)` バリアントを追加
- [x] `parse()` に `ReadGridRow` → `VarDecl(dim=1, var_type=Str)` + `InputOp` の変換を追加
- [x] テスト追加:
  - `H W\nS_{11}...S_{1W}\n:\nS_{H1}...S_{HW}\n` → `ok=true`, `s: Vec<String>`, `[String; h]` にフラット化
  - `H W\nS_{i1}...S_{iW}\n\vdots\nS_{H1}...S_{HW}\n` (loop variable形式)
  - `H W\nS_{1,1}...S_{1,W}\n:\nS_{H,1}...S_{H,W}\n` (カンマ区切り形式)

## 完了条件

- [x] `cargo test --workspace` が全て pass
- [x] `cargo fmt --all` / `cargo clippy --workspace` がクリーン
- [x] `ce init abc151` で `s: Vec<String>` と `s: [String; h]` が生成される

## 作業ログ

- 2026-05-01: 作業開始
