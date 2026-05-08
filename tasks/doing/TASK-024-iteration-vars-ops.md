# TASK-024: iteration_vars / iteration_ops

## 参照仕様

- docs/commands/init.md — 「繰り返し本体 (sub-block) 解析」「`iteration_vars` / `iteration_ops` の形式」節

## 背景

abc456_e のようにループ本体 (block[1]) がスカラーのみでなく配列・ネストループを含む場合、
現状は scalar 解析失敗で `query_body = []` になり solve が空スタブになる。
仕様では block[1] を再帰的に `parse()` して `iteration_vars` / `iteration_ops` に格納する。

## チェックリスト

### 1. entity.rs — InputSpec にフィールド追加

- [x] `InputSpec` に `iteration_vars: Vec<VarDecl>` / `iteration_ops: Vec<InputOp>` を追加
- [x] `not_ok` / 空 InputSpec 構築箇所をすべて修正 (コンパイルエラー解消)

### 2. tests — 期待動作のテストを先に書く

- [ ] `N K\nA_1 A_2 \\ldots A_N` 単独 (block[1] 相当) → `parse()` が ok=true でループ含む ops を返すことを確認
- [ ] abc456_f 形式: `T\n\\mathrm{case}_T\n\nN K\nA_1 A_2 \\dots A_N`
      → `iteration_vars = [n, k, a]`, `iteration_ops = [ReadLine(n,k), ReadLine(a, size=n)]`
      → `query_body = []`
- [ ] abc456_e 形式 (複数ループ含む body) → `iteration_vars` 非空, `iteration_ops` 非空, `query_body = []`
- [ ] abc334_d 形式 (scalar sub-block) → `query_body` 非空, `iteration_vars = []`, `iteration_ops = []` (回帰)
- [ ] numbered query_types 非空 → `iteration_vars = []`, `iteration_ops = []` (query_types 優先)

### 3. parser — parse_query_subblocks 修正

- [ ] 戻り型を `(Vec<QueryTypeDecl>, Vec<VarDecl>, Vec<VarDecl>, Vec<InputOp>)` に変更
- [ ] 非数値 sub-block の scalar 解析失敗時: `parse(block, constraints)` 再帰呼び出し
      → `ok=true` なら `iteration_vars = mini.vars`, `iteration_ops = mini.ops`
      → `ok=false` なら `iteration_vars = []`, `iteration_ops = []`
- [ ] `query_types` 非空 → `iteration_vars`/`iteration_ops` を空に (spec 優先順)
- [ ] `iteration_ops` 非空 → `query_body` を空に
- [ ] `parse()` 関数: `InputSpec` 構築に `iteration_vars` / `iteration_ops` を追加

### 4. テンプレート更新 — templates/rust/src/main.rs.tera

- [ ] `{% elif input_format.iteration_ops | length > 0 %}` ブランチを追加 (docs 記載のコード通り)
- [ ] `solve()` 引数: `iteration_vars` は含めない (template 側は現状で OK のはず)

### 5. テスト確認 & 整合

- [ ] `cargo test --all` 全通過
- [ ] `cargo clippy --all --all-features -- -D warnings` 通過
- [ ] `cargo fmt --all --check` 通過
- [ ] abc456_e で生成される `main.rs` を目視確認 (ループ本体が正しく展開される)

## 完了条件

- [ ] `cargo test --all` 全通過
- [ ] abc456_f: `iteration_vars = [n,k,a]`, `iteration_ops = [ReadLine(n,k), ReadLine(a,size=n)]`
- [ ] abc456_e: `iteration_vars` / `iteration_ops` 非空、`query_body = []`

## 作業ログ

- 2026-05-08: 作業開始
