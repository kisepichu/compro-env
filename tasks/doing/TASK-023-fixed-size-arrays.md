# TASK-023: fixed-size arrays

## 参照仕様

- docs/commands/init.md — 「固定サイズ配列の検出ルール」「2D 固定グリッドの形式」節

## チェックリスト

### 1. 1D 固定サイズ cdots 確認テスト
- [ ] `A_1 \ldots A_3` が ok=true, vars=[a(dim=1,size=["3"])], ops=[ReadLine{dim=1,size="3"}] になることを確認するテストを追加

### 2. 1D 固定サイズ no-cdots (`A_1 A_2 A_3`)
- [ ] テスト: `A_1 A_2 A_3` → ok=true, vars=[a(dim=1,size=["3"])], ops=[ReadLine{size="3"}]
- [ ] テスト: `A_1 A_2` (2 要素) → ok=true (min=2 を満たす)
- [ ] テスト: `A_1` (1 要素) → Scalars として扱われる (配列検出しない)
- [ ] テスト: `A_1 B_1` (異なる名前) → Scalars として扱われる
- [ ] テスト: `A_1 A_3` (非連番) → Scalars として扱われる
- [ ] 実装: `try_parse_array1d_no_cdots` を `parse_var_list` の前に試みる

### 3. `{Num,Num}` 添字の読み取り (`read_subscript_value`)
- [ ] テスト: `_{1,1}` → `("1,1", advance)` として返ることを確認 (or 内部表現を確定)
- [ ] テスト: `_{1,N}` (Ident を含む) → None を返す
- [ ] 実装: `read_subscript_value` の LBrace ブランチで `{Num,Num}` を許容

### 4. `Array2DRow` の単行検出 (`parse_line`)
- [ ] テスト: `A_{1,1} A_{1,2} A_{1,3} A_{1,4} A_{1,5} A_{1,6}` → `Array2DRow { name:"A", row_idx:"1", col_count:6 }`
- [ ] テスト: `A_{1,1} A_{1,2}` (2 要素) → Array2DRow (min=2)
- [ ] テスト: `A_{1,1}` (1 要素) → Array2DRow にならない
- [ ] テスト: `A_{1,1} B_{1,2}` (異なる名前) → Array2DRow にならない
- [ ] テスト: `A_{1,1} A_{1,3}` (col 非連番) → Array2DRow にならない
- [ ] テスト: `A_{1,1} A_{2,1}` (row が異なる) → Array2DRow にならない
- [ ] 実装: `RawLine::Array2DRow { name, row_idx, col_count }` を追加
- [ ] 実装: `try_parse_array2d_row` を `parse_line` に組み込む

### 5. 多行グルーピング (`block_to_ops`)
- [ ] テスト: 3 行 `Array2DRow` 連続 (同名・row 連番) → `VarDecl(dim=2, size=["6","3"])` + `ops=[ReadLine{dim=2}]`
- [ ] テスト: 行数 1 の Array2DRow → Scalars 扱い (グルーピングしない)
- [ ] テスト: row が 1 始まりでない → グルーピングしない
- [ ] テスト: col_count が行によって異なる → ok=false
- [ ] 実装: `block_to_ops` に Array2DRow の多行グルーピングロジックを追加
- [ ] 実装: `VarDecl.dim=2`, `size=["cols","rows"]` として `ensure_var_decl` を呼ぶ
- [ ] 実装: `IntermOp::ReadGrid { name, rows, cols }` → `InputOp { tag: ReadLine, vars: [VarRef { dim=2 }] }`

### 6. テンプレート更新
- [ ] `main()`: `dim == 2` の `VarRef` を `a: [[T; cols]; rows]` として生成
- [ ] `solve()` 引数: `dim == 2` の `VarDecl` を `a: Vec<Vec<T>>` として生成
- [ ] test ハーネス: loop input 同様 `panic!` を生成 (手動記述必要)
- [ ] `templates/rust/src/main.rs.tera` を更新

### 7. `entity.rs` の `VarDecl` 確認
- [ ] `dim: u8` は既に `2` を格納可能なことを確認 (変更不要のはず)

### 8. 統合テスト
- [ ] abc456-B 相当の入力形式 `A_{1,1}...A_{1,6}` × 3 行全体を通してパースし、生成コードを目視確認

## 完了条件

- [ ] `cargo test --all` がすべて通る
- [ ] `cargo clippy --all --all-features -- -D warnings` が通る
- [ ] `cargo fmt --all --check` が通る

## 作業ログ

- 2026-05-08: 作業開始
