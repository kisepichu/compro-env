# TASK-019: クエリ型 sub-block 自動コード生成

## 参照仕様

- docs/commands/init.md — 入力形式パース > `query_types` 形式 / テンプレート例
- docs/spec.md — ドメインモデル > InputSpec / QueryTypeDecl

## 概要

`\text{query}_Q` 形式のクエリ型入力について、追加ブロック (`1 x`, `2 x k` 等) を
解析して `query_types: Vec<QueryTypeDecl>` を構築し、テンプレートで
match dispatch または loop stub を自動生成する。

典型フロー:
```
N Q
\text{query}_1
\vdots
\text{query}_Q

1 x

2 x k
```
→ solve 本体に `for _ in 0..q { match query_type { 1 => input!{x}, 2 => input!{x,k}, ... } }`

## チェックリスト

### domain/entity.rs

- [x] `QueryTypeDecl` 構造体を追加
  - `type_id: String`, `ok: bool`, `vars: Vec<VarDecl>`
- [x] `InputSpec` に `query_types: Vec<QueryTypeDecl>` フィールドを追加
  - デフォルト値は `vec![]`

### usecases/input_format/mod.rs (テスト先行)

- [x] テスト: `text_query_multi_block_numbered` — abc241-D 形式
  - raw: `"N Q\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n\n1 x\n\n2 x k\n\n3 l r\n"`
  - 期待: `ok=true`, `query_types.len() == 3`
  - type_id `"1"`: `ok=true`, vars=`[x:int]`
  - type_id `"2"`: `ok=true`, vars=`[x:int, k:int]`
  - type_id `"3"`: `ok=true`, vars=`[l:int, r:int]` (constraints で \leq があれば int)
- [x] テスト: `query_subblock_non_numeric_skipped` — abc334-D 形式
  - raw: `"N Q\nR_1 \\ldots R_N\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n\nX\n"`
  - 期待: `ok=true`, `query_types.len() == 0` (X は数字始まりでないためスキップ)
- [x] テスト: `query_no_subblocks_empty_query_types` — sub-block なし
  - raw: `"Q\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n"`
  - 期待: `ok=true`, `query_types.len() == 0`
- [x] テスト: `query_subblock_type_inference` — 型推定
  - constraints に \leq x → x: int

- [x] 実装: `parse_query_subblocks()` 関数
  - 各 sub-block を行分割しトークナイズ
  - 先頭トークンが `Num` → type_id、残りをスカラー変数リストとして解析
  - 先頭が `Num` でない / 空 → スキップ (エントリ生成しない)
  - 変数パース: `parse_var_list` ベース
  - 型推定: メインと同一 `infer_types` を適用
  - `QueryTypeDecl { type_id, ok, vars }` を構築
- [x] `parse()` 内で `has_query_marker` が true のとき `parse_query_subblocks` を呼ぶ

### templates/rust/src/main.rs.tera

- [x] solve 本体前に `set_global query_loop_end` でループ走査 (empty-body LoopBegin 検出)
- [x] solve 本体: `query_loop_end != ""` のとき:
  - `query_types` non-empty → match dispatch 生成
  - `query_types` empty → loop stub 生成 (`for _ in 0..q { todo!() }`)
- [x] solve 本体: `query_loop_end == ""` → 従来の `todo!()` スタブ
- [x] main(): empty-body LoopBegin をスキップ (コメントのみ、コード生成しない)
- [x] main(): 非空ループ (LoopBegin 直後が ReadLine) は従来通り TODO スタブ生成

### docs/

- [x] `docs/commands/init.md` のテンプレート Tera 例・仕様を実装に合わせて確認・反映済み

## 完了条件

- [x] `cargo test --workspace` が全て pass
- [x] `cargo fmt --all` / `cargo clippy --workspace` がクリーン
- [x] abc241-D 形式で `query_types.len() == 3`、match dispatch 生成を手動確認
- [x] abc334-D 形式で `query_types.len() == 0`、loop stub 生成を手動確認

## 作業ログ

- 2026-05-07: 作業開始・完了
