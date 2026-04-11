# TASK-005: ce init 実装

## 参照仕様

- docs/commands/init.md

## 実装チェックリスト

### usecases/

- [x] `OnlineJudge` trait に `get_start_time(contest_id: &str) -> Result<Option<DateTime<Utc>>>` を追加
- [x] `Service::init()` を実装 (コンテスト初期化フロー全体)
  - セッション取得 → OJ 判定 → 開始時刻取得 → 待機 → 問題取得 → contest_repo.create → solution_repo.create × N

### infrastructure/

- [x] `ConfigImpl::default_language()` を実装 (config.toml から読み込む)
- [x] `AtCoder::get_start_time()` を実装 (コンテストページから開始時刻をスクレイピング)
- [x] `AtCoder::get_problems_detail()` を実装 (問題一覧・サンプル I/O をスクレイピング)
- [x] `ContestRepositoryImpl::exists()` を実装 (`.ce.toml` の有無)
- [x] `ContestRepositoryImpl::exists_unstarted()` を実装 (ディレクトリあり・`.ce.toml` なし)
- [x] `ContestRepositoryImpl::create_unstarted()` を実装 (空ディレクトリ作成)
- [x] `ContestRepositoryImpl::create()` を実装 (`.ce.toml` + testcases ファイル生成)
- [x] `SolutionRepositoryImpl::exists()` を実装 (解法ディレクトリの有無)
- [x] `SolutionRepositoryImpl::create()` を実装 (Tera テンプレート展開 + workspace Cargo.toml 更新)
- [x] `templates/rust/Cargo.toml` → `Cargo.toml.tera` にリネームし Tera 構文 (`{{problem.code}}` 等) に変更
- [x] Shell `commands.rs` の `Init` アームを実装 (contest_id/URL パース → controller.init → 結果表示)

## 完了条件

- [x] `ce init abc334` を実行すると `solutions/abc334/` 以下にディレクトリ・ファイルが生成される
- [x] 冪等性: `.ce.toml` 既存時は上書きせず、既存解法ディレクトリはスキップ
- [x] デフォルト言語未設定時に適切なエラーメッセージで exit 1
- [x] セッション未設定時に適切なエラーメッセージで exit 1
- [x] `cargo fmt --all` / `cargo clippy --workspace --all-features` / `cargo test --workspace` がすべてクリア

## 作業ログ

- 2026-04-11: 作業開始・完了
