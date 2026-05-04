# TASK-008: ce test 実装

## 参照仕様

- docs/commands/test.md
- docs/spec.md

## 実装チェックリスト

### usecases/ — インターフェース変更

- [x] `SolutionRepository::create` に `samples: &[Sample]` を追加
- [x] `SolutionRepository` に `fn solution_dir` を追加
- [x] `ContestRepository` に `fn testcases_dir` を追加
- [x] `Config::test_command` / `run_command` を trait から削除
- [x] `service/test.rs` を新設計で実装
      (ce.toml 読み取り → sh -c 実行 → exit code 返却)

### infrastructure/ — リポジトリ実装更新

- [x] `solution_repository_impl::create` に samples を Tera コンテキスト追加
- [x] `solution_repository_impl::solution_dir` を実装
- [x] `contest_repository_impl::testcases_dir` を実装
- [x] `config_impl::test_command` / `run_command` を削除
- [x] `shell/commands.rs`: `Commands::Test` から `lang` を削除、`TestInput` 更新
- [x] `shell/mod.rs`: `Commands::Test` ハンドラを実装 (exit code で process::exit)

### usecases/ — 呼び出し側修正

- [x] `service/init.rs`: `solution_repo.create(solution, &problem.samples)` に更新
- [x] `service/new_solution.rs`: `contest_repo.get_samples()` を追加し `create(solution, &samples)` に更新 (TASK-009 で対応済み)

### templates/

- [x] `templates/rust/ce.toml.tera` を追加 (`test_command = "cargo test"`)
- [x] `templates/rust/src/main.rs` → `main.rs.tera` に変換 (samples ループ付き)

## 完了条件

- [x] `cargo test --workspace` 全通過 (71 tests)
- [x] `cargo clippy --workspace --all-features -- -D warnings` 警告なし
- [x] `cargo fmt --all` 適用済み
- [x] `ce test abc334 a` が実際に動作する（手動確認）

## 作業ログ

- 2026-04-15: 作業開始・実装完了・手動確認完了
