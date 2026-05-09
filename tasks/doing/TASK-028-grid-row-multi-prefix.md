# TASK-028: GridRow 先頭複数要素対応 (abc453-D スタイル)

## 参照仕様

- docs/commands/init.md — Parser 節「GridRow: 先頭の Ident_{2D_left} [Ident_{Num,Num}]* Cdots Ident_{2D_right}」
- docs/commands/init.md — 「文字グリッド (2D 添字) の検出」節

## 背景

abc453-D (Go Straight) の入力形式:
```
H W
S_{1,1} S_{1,2} \ldots S_{1,W}
S_{2,1} S_{2,2} \ldots S_{2,W}
\vdots
S_{H,1} S_{H,2} \ldots S_{H,W}
```

`S_{1,1} S_{1,2} \ldots S_{1,W}` のように cdots の前に複数の `Ident_{Num,Num}` 要素が並ぶ形式を
現在の `try_parse_grid_row` が認識できない。
- 既存: `Ident_{2D_left} Cdots Ident_{2D_right}` (先頭1要素のみ)
- 必要: `Ident_{2D_left} [Ident_{Num,Num}]* Cdots Ident_{2D_right}` (先頭複数要素)

## 実装チェックリスト

### usecases/

- [x] `try_parse_grid_row` に「cdots 前に Ident_{Num,Num} が複数続く」前処理を追加
  - tokens が `[Ident_sub_{Num,Num}, Space, Ident_sub_{Num,Num}, ..., Space, Cdots, ...]` のとき、
    先頭の追加 `Ident_{Num,Num}` 要素を消費して既存の `leftvar Cdots rightvar` 検出に合流させる
  - 追加要素は全て名前・行インデックスが一致していること、を検証する
  - 副作用なし: 先頭1要素の既存パターンはそのまま通過する

## 完了条件

- [x] `abc453d_grid_row_multi_prefix` テストが GREEN
- [x] 既存テスト全通過
- [x] `cargo clippy --workspace --all-features -- -D warnings` 通過
- [x] `cargo fmt --all --check` 通過

## 作業ログ

- 2026-05-09: 作業開始
