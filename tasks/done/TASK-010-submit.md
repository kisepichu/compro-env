# TASK-010: ce submit 実装

## 参照仕様

- docs/commands/submit.md

## 設計メモ

- `Service::submit` のシグネチャは `test` と同様に `(contest_id, problem_code, solution_name)` を受け取る形に変更する。
  service が ce.toml から language を読むため `SubmitInput::language()` は不要。
- `SolutionRepository::get_source` にはファイルパスを渡す引数を追加する:
  `fn get_source(&self, solution: &Solution, file_path: &str) -> Result<String>`
  サービスが `Config::submit_file` でパスを決定し、リポジトリに渡す。
- `ContestRepository::get_problem` は trait に未定義なので追加・実装する。
- AtCoder submit は GET (CSRF 取得) → POST → Location ヘッダー取得の 2 ステップ。

## 実装チェックリスト

### interfaces/

- [x] `SubmitInput` から `fn language(&self) -> Language` を削除する
- [x] `Controller::submit` を `service.submit(contest_id, problem_code, solution_name)` を呼ぶ形に変更する

### usecases/

- [x] `ContestRepository` trait に `fn get_problem(&self, contest_id: &str, problem_code: &str) -> Result<Problem>` を追加する
- [x] `SolutionRepository::get_source` のシグネチャを `fn get_source(&self, solution: &Solution, file_path: &str) -> Result<String>` に変更し、全スタブを更新する
- [x] `Service::submit(contest_id, problem_code, solution_name)` を実装する:
  1. solution_dir を取得し ce.toml の存在確認
  2. ce.toml から `language` を読む (直接 `std::fs` + toml)
  3. `contest_repo.get_oj_kind(contest_id)` → OJKind
  4. `contest_repo.get_problem(contest_id, problem_code)` → Problem (problem_id を取得)
  5. `config.submit_file(&language)` → file_path; ソースファイルの存在確認
  6. `solution_repo.get_source(&solution, &file_path)` → source
  7. `config.lang_id(&language, &oj_kind)` → lang_id (None はエラー)
  8. `session_repo.get(&oj_kind)` → Session (None は "Run 'ce login' first." エラー)
  9. `oj.submit(contest_id, &problem.id, &lang_id, &source, &session)` → SubmitResult

### infrastructure/

- [x] `ContestRepository::get_problem` を実装する:
  - `CeTomlOwned` に `problems: Vec<CeTomlProblemOwned>` フィールドを追加
  - `.ce.toml` の `[[problems]]` から `code` 一致エントリを返す。見つからない場合はエラー
- [x] `SolutionRepository::get_source` を実装する:
  - `solution_dir(solution).join(file_path)` のファイルを読む
- [x] `Config::submit_file(lang)` を実装する:
  - `[language.{lang}].solution_file` を読む。未設定なら `"src/main.rs"` をデフォルトとする
- [x] `Config::lang_id(lang, oj)` を実装する:
  - `[language.{lang}.{oj_name}].lang_id` を読む。未設定なら `None` を返す
- [x] `AtCoder::submit(...)` を実装する:
  1. GET `https://atcoder.jp/contests/{contest_id}/submit` で CSRF トークンを抽出
  2. POST (フォームデータ: csrf_token, data.TaskScreenName, data.LanguageId, sourceCode, クッキー: REVEL_SESSION)
  3. redirect を follow せず `Location` ヘッダーを提出 URL として `SubmitResult` に格納
  4. 非 302 はステータスコードとボディを含むエラーを返す
- [x] Shell Submit handler を実装する:
  - `SubmitCommand` から `language` フィールドと `lang` CLI arg を削除する
  - `contest_id` と `problem_code` を lowercase 正規化する
  - `controller.submit(args)` を呼び、`submission_url` を `println!` する

## 完了条件

- [x] `cargo test --workspace` が全て通る
- [x] `cargo clippy --workspace --all-features` で warning なし
- [ ] `ce sub abc001 a` で AtCoder に提出できる (手動確認)

## 作業ログ

- 2026-04-16: 作業開始・完了
