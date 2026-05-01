# TASK-013: input format Phase 1 — 単一変数 vdots フラット化・`:` vdots 正規化

## 参照仕様

- docs/commands/init.md — Phase 1 対応パターン節
- docs/spec.md — InputSpec / InputOp / VarRef ドメインモデル

## 対象問題 (test_problems.yml より)

| 問題 | input_format_raw 概要 | 期待 ok |
| --- | --- | --- |
| abc246-f | `N L\nS_1\n\\dots\nS_N` | true |
| abc246-e | `N\nA_x A_y\nB_x B_y\nS_1\n\\vdots\nS_N` | false (A_x 等が非数値添字スカラーで未対応) |
| ukuku09-c | `H W Q\nS_1\n:\nS_H\n...` | 部分対応 (S_1..S_H 節は ok、後続ループ節は false) |
| typical90-060 | `N\nA_1 A_2 \\cdots A_N` | true (既存) |
| abc154-e | `N\nK` | true (既存) |

## 実装チェックリスト

### usecases/

- [x] `preprocess()` で行が `:` のみの場合を `\\vdots` に正規化する
  - トークナイザーレベルで `:` のみの行を Vdots トークンと等価に処理 (preprocess 変更不要だった)
- [x] 単一変数 vdots ループを 1D 配列読み込みへフラット化する
  - `flatten_single_var_loops(ops)` を実装: `LoopBegin + ReadLine(1 var, index) + LoopEnd` → `ReadLine(dim=1, size=loop_end)` に変換
  - フラット化できないループ(複数変数行など)は `not_ok` のまま
  - テスト 3 件追加・全 pass

### docs/

- [x] `docs/commands/init.md` の Phase 1 対応パターン表に単一変数 vdots / `:` 区切りを追加

## 完了条件

- [x] `cargo test --workspace` が全て pass (134 tests)
- [x] `cargo fmt --all` / `cargo clippy --workspace` がクリーン
- [x] abc246-f 相当の入力 (`N L\nS_1\n\\dots\nS_N`) で `ok=true`、`s: Vec<String>` が生成される
- [x] abc242-d 相当の2変数ループ入力で `ok=false` が維持される
- [x] `:` と `\\vdots` が同等に扱われる

## 作業ログ

- 2026-05-01: 作業開始
