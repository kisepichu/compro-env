# AtCoder

[AtCoder](https://atcoder.jp/) の OJ 実装仕様。抽象の共通設計は [README.md](./README.md) を参照。
本ドキュメントは現行実装 (`crates/infrastructure/src/online_judge_impl/atcoder.rs`) に対応する。

## 概要

- コンテスト型 OJ。`contest_id` = コンテスト ID (`abc334`)、1 コンテストに複数 `Problem`。
- Cloudflare Turnstile によりログイン・提出の HTTP 自動化が困難。ログインは手動 cookie、
  提出はブラウザ + userscript で行う。

## 判定

- URL `https://atcoder.jp/contests/{id}` → `(AtCoder, contest_id={id})` (先頭パスセグメントのみ採用)。
- プレフィックス `abc` / `arc` / `agc` / `ahc` (大文字小文字無視) → AtCoder。
- contest_id は単一パスコンポーネントであることを検証する。

## ログイン (手動 cookie)

- `ce login [atcoder]` は `REVEL_SESSION` の値を手動入力 (または `--cookie`) で受け取り、
  そのまま `~/.config/ce/session.toml` の `[atcoder]` に保存する。ネットワーク検証はしない。
  ```toml
  [atcoder]
  revel_session = "xxxxxxxx"
  ```
- 空文字はエラー。理由: Turnstile によりユーザー名/パスワード自動ログインが破綻するため
  (cookie 手動コピー方式)。

## whoami

- `GET https://atcoder.jp/home` に `Cookie: REVEL_SESSION=...` を付与。
- HTML 中の `var userScreenName = "..."` を抽出する。空ならセッション切れ
  (`session expired. Run \`ce login\` again.`)。

## 問題取得 (init)

通常ケースは **2 リクエスト**:

| # | URL | 取得 |
| --- | --- | --- |
| 1 | `https://atcoder.jp/contests/{id}` | 開始時刻 (`get_contest_meta`)・problem_id ヒント |
| 2 | `https://atcoder.jp/contests/{id}/tasks_print` | 全問題タイトル・サンプル・入力形式・制約 |

- **開始時刻**: `<time ... class="...fixtime-full...">YYYY-MM-DD HH:MM:SS+0900</time>` をパースし UTC 化。
  取得不可なら None。待機ロジック (カウントダウン/ポーリング) は `usecases/service/init.rs` 側。
- **problem_id ヒント**: ナビバードロップダウンの `href="/contests/{id}/tasks/{problem_id}"`。
  現状実装では空 Vec を返し、`get_problems_detail` 側で推定する。
- **problem_id 決定**: ヒントにあればそれを、なければ `{contest_id}_{problem_code}` (例 `abc334_a`)。
- **サンプル**: 英語セクション `<h3>Sample Input N</h3>` / `<h3>Sample Output N</h3>` の直後 `<pre>`。
  inline タグ strip + HTML エンティティ decode。
- **入力形式 (`input_format_raw`)**: `<h3>入力</h3>` / `<h3>Input</h3>` から次の `<h3>` までの
  全 `<pre>` ブロックを連結。
- **制約 (`constraints_raw`)**: `<h3>制約</h3>` / `<h3>Constraints</h3>` セクションのテキスト
  (タグ strip)。
- 未ログインで `tasks_print` が `/login` にリダイレクトされた場合は `CeError::NotLoggedIn` を返す。
- 公開コンテストは session 不要 (`Option<&Session>`)。

## 提出 (ブラウザ + userscript)

- Turnstile のため直接 POST 不可。提出ページ URL を生成してブラウザで開く:
  ```
  https://atcoder.jp/contests/{contest_id}/submit?taskScreenName={problem_id}#ce={payload}
  ```
  - `?taskScreenName={problem_id}`: 問題プルダウンを事前選択。
  - `#ce={payload}`: `{"lang_id","source"}` を URL-safe base64 (パディングあり) でエンコードした
    フラグメント。Tampermonkey userscript が読んで提出フォームへ自動入力する
    (詳細: `docs/userscript.md`)。
- base64 エンコード後の実際のフラグメント長が上限 (32 KiB) を超える場合はエラー。
- `ce sub` は Unix で提出前に `test_command` を実行し exit 0 のときのみ URL を生成する (OJ 非依存)。
- 抽象の提出一般化では「ブラウザで開く URL を返す」結果に対応する。

## 言語 (lang_id)

- config の `[language.{lang}.atcoder].lang_id` (例: Rust = `6088`)。

## 関連

- `docs/commands/login.md`, `init.md`, `submit.md`
- `docs/userscript.md` (提出フラグメントのプロトコル)
