# TASK-016: 非数値添字スカラー入力対応

## 参照仕様

- docs/commands/init.md — Phase 1 非対応パターン表 (非数値添字スカラー)
- docs/spec.md — 入力形式パース 未対応パターン

## 背景

`A_x A_y` (アルファベット添字) や `r_1 c_1` (数値添字・vdots なし) のように、
subscript を持つ変数が vdots ブロック外に単独で並ぶ行は現在 `ok: false` になる。

これらは「subscript が変数名の一部に過ぎないスカラー」として扱い、
`A_x` → Rust 変数 `ax`、`r_1` → `r1` のようにマッピングすれば解決できる。

## チェックリスト

### usecases/ (パーサー)

- [x] `build_intermediate()` の standalone `LoopRow` アームを修正
  - `ReadScalars(names)` を emit するよう変更 (`names = math + sub` for each var)
  - subscript がない場合は math そのまま
- [x] テスト追加:
  - `A_x A_y` 単独行 → `ok=true`, vars `ax: i64, ay: i64` (dim=0)
  - `r_1 c_1` + `r_2 c_2` の 2 行 → `ok=true`, vars `r1, c1, r2, c2` (dim=0)
  - abc246-E 形式: `N\nA_x A_y\nB_x B_y\nS_1\n\\vdots\nS_N\n` → `ok=true`, 正しい変数
  - abc176-D 形式: `H W\nr_1 c_1\nr_2 c_2\nS_{H1}...S_{HW}\n:\n...` → `ok=true`

### docs/

- [x] `docs/commands/init.md` の Phase 1 対応パターン表に追記
- [x] `docs/commands/init.md` の Phase 1 非対応表から削除
- [x] `docs/spec.md` の未対応パターン表から削除

## 完了条件

- [x] `cargo test --workspace` が全て pass
- [x] `cargo fmt --all` / `cargo clippy --workspace` がクリーン
- [x] `ce solution add abc246 e` で `ax, ay, bx, by` が生成される
- [x] `ce solution add abc176 d` で `ch, cw, dh, dw` と `s: Vec<String>` が生成される

## 作業ログ

- 2026-05-02: 作業開始
