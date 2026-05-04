# TASK-013: input format Phase 1+2 — vdots フラット化・多変数ループコード生成

## 参照仕様

- docs/commands/init.md — Phase 1 対応パターン節
- docs/spec.md — InputSpec / InputOp / VarRef ドメインモデル

## 対象問題 (test_problems.yml より)

| 問題 | input_format_raw 概要 | 期待 ok |
| --- | --- | --- |
| abc246-f | `N L\nS_1\n\\dots\nS_N` | true (Phase 1) |
| abc242-d | `S\nQ\nt_1 k_1\n\\vdots\nt_Q k_Q` | true (Phase 2) |
| ukuku09-c | `H W Q\nS_1\n:\nS_H\n...` | true (Phase 1+2) |
| abc246-e | `N\nA_x A_y\nB_x B_y\nS_1\n\\vdots\nS_N` | false (非数値添字スカラー未対応) |
| typical90-060 | `N\nA_1 A_2 \\cdots A_N` | true (既存) |
| abc154-e | `N\nK` | true (既存) |

## 実装チェックリスト

### usecases/

- [x] 単一変数 vdots ループを 1D 配列読み込みへフラット化 (Phase 1)
  - `flatten_single_var_loops()`: `LoopBegin + ReadLine(1 var) + LoopEnd` → `ReadLine(dim=1, size=loop_end)`
  - フラット化失敗時は元の ops を返す (`Err(original_ops)`)
- [x] 多変数ループは ops を保持して `ok=true` (Phase 2)
  - フラット化失敗時に `not_ok` 返す代わりに元 ops をそのまま保持
  - `has_loop → not_ok` チェックを削除
  - テンプレート側でループコードを生成

### infrastructure/ (テンプレート)

- [x] `templates/rust/src/main.rs.tera` を per-op 生成に変更 (Phase 2)
  - `loop_begin`: `Vec::new()` 宣言 + `for _ in 0..end { input! {`
  - `read_line` (depth>0): `__tmp_x: T,` を input! 内に出力
  - `loop_end`: `}` + `.push(__tmp_x)` を出力

### docs/

- [x] `docs/commands/init.md` Phase 1 対応パターン表を更新

## 完了条件

- [x] `cargo test --workspace` が全て pass
- [x] `cargo fmt --all` / `cargo clippy --workspace` がクリーン
- [x] abc246-f 相当で `ok=true`、`s: Vec<String>` 生成
- [x] abc242-d 相当の2変数ループで `ok=true`、ループコード生成
- [x] `:` と `\\vdots` が同等に扱われる

## 作業ログ

- 2026-05-01: 作業開始・Phase 1 完了・Phase 2 完了
