# TASK-006: ce init 実装を仕様に合わせてリファクタリング

## 参照仕様

- docs/commands/init.md
- docs/spec.md

## 背景

TASK-005 で init を仮実装した後、仕様を変更した (tasks_print 1リクエスト化、ContestMeta 導入等)。
実装を現行仕様に合わせる。

## 実装チェックリスト

### domain/

- [x] `Language` を `enum {Rust, Cpp}` から String newtype (`struct Language(String)`) に変更
  - `Language::new(s: &str) -> Self`
  - `Display` / `FromStr` / `dir_name()` 相当のメソッドを維持
  - 既存テストを更新

### usecases/

- [x] `OnlineJudge` トレイトを更新
  - `ContestMeta { start_time: Option<DateTime<Utc>>, problem_id_hints: Vec<(String, String)> }` 追加
  - `get_contest_meta(contest_id: &str) -> Result<ContestMeta>` 追加
  - `get_start_time` / `wait_for_start` 削除
  - `get_problems_detail` のシグネチャに `problem_id_hints: &[(String, String)]` を追加
- [x] `Service::init` を更新
  - session optional 化: `None` でも続行 (過去コンテストは公開アクセス可)
  - `get_start_time` → `get_contest_meta` に変更
  - `meta.problem_id_hints` を `get_problems_detail` に渡す
  - テストの StubOJ を新トレイトに合わせて更新

### infrastructure/

- [x] `AtCoder::get_contest_meta` を実装 (コンテストページから開始時刻を取得、hints は空 Vec)
  - `get_start_time` の実装を移植
  - `wait_for_start` 削除
- [x] `AtCoder::get_problems_detail` を `tasks_print` 1リクエストに変更
  - `tasks_print` HTML から全問題のタイトル・サンプルを一括取得
  - `parse_tasks_print_from_html(html, contest_id, hints) -> Vec<Problem>` を実装
  - 既存の `parse_problem_list_from_html` / `parse_problem_detail_from_html` の N+1 ロジックを削除
  - テスト用 `tasks_print` HTML フィクスチャを追加
- [x] `shell/mod.rs`: `Init` コマンドに `--lang` オプションを追加
  - `--lang` 指定 → そのまま使用
  - 未指定 → `ConfigImpl.default_language()` を試みる
  - どちらもなければ stdin で確認: `Language (e.g. rust, cpp): `
  - `templates/{lang}/` が存在しなければエラー終了

## 完了条件

- [x] `Language::new("rust")` が動作し、`templates/rust/` を展開できる
- [x] `get_contest_meta` が開始時刻を返す
- [x] `get_problems_detail` が `tasks_print` 1リクエストで全問題を取得する
- [x] `ce init abc334 --lang rust` が動作する
- [x] セッションなしでも過去コンテストを `ce init` できる (過去コンテストは公開アクセス可)
- [x] `cargo fmt --all` / `cargo clippy --workspace --all-features` / `cargo test --workspace` がすべてクリア

## 作業ログ

- 2026-04-11: 作業開始・完了
