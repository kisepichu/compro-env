# TASK-021: plain-text query marker 対応 (query_i パターン)

## 参照仕様

- docs/commands/init.md — 入力形式パース > パイプライン / Phase 1 対応パターン

## 概要

abc212_d 形式の `query_i` (plain ident + subscript、LaTeX マーカーなし) を
QueryLine として認識し、既存の sub-block 解析コードパスに乗せる。

典型入力:
```
Q
query_1
query_2
:
query_Q

1 X_i

2 X_i

3
```

## チェックリスト

### usecases/input_format/mod.rs (テスト先行)

- [x] テスト: `plain_query_marker_abc212d` — `query_i` パターン
- [x] テスト: `plain_query_marker_not_triggered_for_short_ident` — `q_i` は対象外
- [x] テスト: `multi_block_digit_no_query_marker_not_ok` の入力を `q_i` に差し替え

### 実装: `has_query_marker`

- [x] block0 の単語を split して `"query"` (case-insensitive) が含まれれば `true`

### 実装: `parse_line` の QueryLine 検出

- [x] `Ident(s) where s.to_ascii_lowercase() == "query"` を検出条件に追加

## 完了条件

- [x] `cargo test --workspace` が全て pass (65 tests)
- [x] `cargo fmt --all` / `cargo clippy --workspace` がクリーン
- [x] abc212_d 形式 (`plain_query_marker_abc212d`) で `query_types.len() == 3` を確認

## 作業ログ

- 2026-05-07: 作業開始・完了
