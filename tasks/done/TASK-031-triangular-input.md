# TASK-031: ce init — 上三角行列入力形式対応

## 参照仕様

- docs/commands/init.md (入力形式パース → TriangularMatrix 節、triangular の形式節)

## 概要

abc236_d / abc451_e のような上三角行列形式を `ok: true` でパースし、
テンプレートが正しい Rust 読み取りコードを生成できるようにする。

```
N
A_{1, 2} A_{1, 3} \ldots A_{1, N}    ← TriangularRow (first_idx=1, bound=n)
A_{2, 3} \ldots A_{2, N}              ← TriangularRow (first_idx=2)
\vdots
A_{N-1,N}                             ← 末尾要素
```

## 実装チェックリスト

### domain/

- [x] `TriangularSpec { name: String, math: String, var_type: VarType, bound: String }` を `entity.rs` に追加
- [x] `InputSpec.triangular: Option<TriangularSpec>` フィールド追加 (既存の `InputSpec` 初期化箇所を `triangular: None` で更新)
- [x] `InputFormatKind::Triangle` バリアント追加 (`Testcase` と `Iter` の間)
- [x] `Display for InputFormatKind`: `Triangle => "triangle"`
- [x] `InputSpec::kind()`: `triangular.is_some()` → `Triangle` (priority 5)

### usecases/input_format/mod.rs

- [x] 前処理: `…` (U+2026) → `\ldots` 正規化 (テキストレベル、tokenize 前)
- [x] 前処理: CDOTS-only 行 → VDOTS 正規化 (detect_triangular 内で処理)
- [x] Phase 2 早期検出: `detect_triangular(block0_lines)` 関数を実装
- [x] `parse()` の Phase 2 先頭で `detect_triangular` を呼び、Some なら早期 return
- [x] `not_ok()` に `triangular: None` を追加
- [x] 全 `InputSpec { ... }` 構築箇所に `triangular: None` を追加

### usecases/input_format/ (テスト)

- [x] `triangular_abc451e_bound_n`: bound="n" のケース
- [x] `triangular_abc236d_bound_2n`: bound="2*n" の算術式ケース
- [x] `triangular_cdots_only_line_as_vdots`: CDOTS-only 行が VDOTS として扱われること
- [x] `triangular_unicode_ellipsis`: `…` (U+2026) が CDOTS として処理されること
- [x] 既存テストが全て通ること

### templates/rust/src/main.rs.tera

- [x] `fn solve` 引数末尾に `triangular` が非空のとき `Vec<Vec<T>>` を追加
- [x] `fn main` の末尾に三角行列読み取りループを追加 (既存 ops ループの後)
- [x] `print!` 呼び出しで triangular.name を末尾引数として渡す
- [x] テストハーネス: `LineSource` を使った実際の読み取りコードを生成 (panic stub ではなく)

## 完了条件

- [x] `cargo test --workspace` 全通過
- [x] `cargo clippy --all --all-features -- -D warnings` 警告なし
- [x] `cargo fmt --all --check` 通過
- [ ] `ce init abc451` で `triangle` ラベルが表示される (手動確認)
- [ ] `ce init abc236` で `triangle` ラベルが表示される (手動確認)
- [x] 生成された `src/main.rs` に `for _i in 0..n-1 { input! { _row: [i64; n-1-_i] } }` が含まれる

## 作業ログ

- 2026-05-10: 作業開始
