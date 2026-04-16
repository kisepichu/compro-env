# TASK-009: ce solution add 実装

## 参照仕様

- docs/commands/solution.md
- docs/spec.md (SolutionRepository インターフェース)

## 背景・現状

- domain / usecases / interfaces / infrastructure の骨格は全て作成済み
- `Controller::new_solution` は完成済み (Solution 構築 → service 呼び出し)
- `SolutionRepositoryImpl::create` / `exists` / `solution_dir` は実装済み
- `ContestRepositoryImpl::exists` / `list_problem_codes` / `get_samples` は実装済み
- 未実装: `Service::new_solution` (todo!()), shell `New` ハンドラ (todo!())
- 要修正: `SolutionRepository::exists` の `lang: &Language` 引数を削除 (仕様変更)

## 実装チェックリスト

### usecases/ — repository インターフェース修正

- [x] `SolutionRepository::exists` から `lang: &Language` 引数を削除する
  - `crates/usecases/src/repository/solution_repository.rs`

### infrastructure/ — impl 側を trait に合わせる

- [x] `SolutionRepositoryImpl::exists` のシグネチャを更新 (`_lang: &Language` を削除)
  - `crates/infrastructure/src/repository_impl/solution_repository_impl.rs`

### usecases/ — Service::new_solution 実装

- [x] `Service::new_solution` を実装する
  - ファイル: `crates/usecases/src/service/new_solution.rs`
  - 手順:
    1. `contest_repo.exists(contest_id)` — なければ「`ce init` を実行」エラー
    2. `contest_repo.list_problem_codes(contest_id)` — problem_code がなければ利用可能コード一覧を示すエラー
    3. `solution_repo.exists(contest_id, problem_code, name)` — 存在すればエラー
    4. `contest_repo.get_samples(contest_id, problem_code)` — サンプル取得
    5. `solution_repo.create(&solution, &samples)` — テンプレート展開

- [x] `Service::new_solution` のユニットテストを書く
  - テストケース:
    - 正常系: コンテスト・問題・解法名が全て有効 → Ok(())
    - エラー: contest が存在しない → "ce init" を含むエラー
    - エラー: problem_code がない → 利用可能コード一覧を含むエラー
    - エラー: 解法ディレクトリが既に存在する → エラー

### infrastructure/ — shell ハンドラ実装

- [x] `new_solution_with_io` 関数を実装する
  - ファイル: `crates/infrastructure/src/shell/mod.rs`
  - `resolve_new_solution_args` (cfg(test)) ヘルパーでテストカバレッジを確保
  - テスト: 正常系・パストラバーサル・スラッシュ含む problem_code・不明言語

- [x] `Commands::Solution { subcommand: SolutionSubcommand::Add { .. } }` のハンドラを `new_solution_with_io` 呼び出しに切り替える (`Commands::New` を削除して `SolutionSubcommand` に置き換え)

### フォーマット・静的解析

- [x] `cargo fmt --all` を通す
- [x] `cargo clippy --workspace --all-features` の警告を修正する
- [x] `cargo test --workspace` で全テスト通過を確認する (84 tests)

## 完了条件

- [x] `cargo test --workspace` が全て通る
- [x] コンテストが存在しない場合に適切なエラーが出る
- [x] 解法が既に存在する場合に適切なエラーが出る

## 作業ログ

- 2026-04-16: 作業開始、ブランチ feat/009-solution-add 作成
- 2026-04-16: 実装完了 (19 usecases + 48 infrastructure + 17 domain tests all passing)
