# TASK-029: GridRow 先頭要素スペースなし隣接対応 (abc450-C スタイル)

## 参照仕様

- docs/commands/init.md — Parser 節「GridRow: 先頭の Ident_{2D_left} [Ident_{2D}]* Cdots Ident_{2D_right}」
- docs/commands/init.md — 「文字グリッド (2D 添字) の検出」節

## 背景

abc450-C の入力形式:
```
H W
S_{1,1}S_{1,2}\dots S_{1,W}
\vdots
S_{H,1}S_{H,2}\dots S_{H,W}
```

`S_{1,1}S_{1,2}\dots S_{1,W}` のように cdots の前の `Ident_{2D}` 要素がスペースなし隣接している
形式を現在の `try_parse_grid_row` が認識できない。

- TASK-028 で追加した `had_space` ガードが、スペースなし隣接を `ok: false` にフォールバックさせている
- 必要: スペースあり・なし両方を GridRow として認識する
- 検出条件は「同名・2D 添字・行インデックス一致」のみで十分 (スペース有無は問わない)

## 実装チェックリスト

### usecases/

- [x] `try_parse_grid_row` の `had_space` ガードを削除し、スペースなし隣接も許容する
  - 負のテスト `grid_row_multi_prefix_no_space_falls_through` を削除または反転する
  - `abc450c_grid_row_no_space` テストが GREEN になること
  - 既存のスペースあり形式 (`abc453d_grid_row_multi_prefix`) も引き続き GREEN であること

## 完了条件

- [x] `abc450c_grid_row_no_space` テストが GREEN
- [x] 既存テスト全通過
- [x] `cargo clippy --workspace --all-features -- -D warnings` 通過
- [x] `cargo fmt --all --check` 通過

## 作業ログ

- 2026-05-10: 作業開始
