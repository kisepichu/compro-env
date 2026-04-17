# ce submit (ce sub)

## 概要

解法をブラウザで提出する。`ce sub` は `ce submit` のエイリアス。

AtCoder は Cloudflare Turnstile を導入しており、HTTP 直接送信でのボット提出がブロックされる。
そのため `ce submit` は提出内容を URL フラグメントに埋め込んでブラウザで提出ページを開く。
ブラウザに Tampermonkey userscript を導入することで問題選択・ソースコード注入を自動化できる。

## シグネチャ

```
ce submit <contest_id> <problem_code> [solution_name]
ce sub <contest_id> <problem_code> [solution_name]
```

- `contest_id`: コンテスト ID
- `problem_code`: 問題コード
- `solution_name`: 解法名 (省略時: `main`)

## 挙動

1. 解法ディレクトリの `ce.toml` から `language` を読む:
   ```
   solutions/{contest_id}/{problem_code}/{solution_name}/ce.toml
   ```
   `language` フィールドは `templates/{lang}/ce.toml.tera` で定義され `ce init` / `ce solution add` 時に生成される (詳細: `docs/commands/test.md`)。
2. `.ce.toml` から `OJKind` と `problem_id` を取得する:
   - `ContestRepository::get_oj_kind(contest_id)` で `OJKind` を得る
   - `ContestRepository::get_problem(contest_id, problem_code)` で `problem_id` を得る
3. config の `language.{language}.solution_file` からファイルパスを決定し、`SolutionRepository::get_source(solution)` でソースを読む:
   ```
   solutions/{contest_id}/{problem_code}/{solution_name}/{solution_file}
   ```
4. config の `language.{language}.{oj}.lang_id` を取得する
5. 提出ページ URL を生成して標準出力に表示する
6. 提出ページ URL をブラウザで開く (詳細: 次節)

ステップ 5 の URL を開いた後、Tampermonkey userscript が問題選択・ソースコード注入を行う (詳細: `docs/userscript.md`)。

## 提出 URL の生成

```
https://atcoder.jp/contests/{contest_id}/submit?taskScreenName={problem_id}#ce={payload}
```

- `?taskScreenName={problem_id}`: AtCoder の submit ページが対応している既存クエリパラメータ。問題プルダウンを `problem_id` で事前選択する
- `#ce={payload}`: userscript が読む URL フラグメント

`payload` は以下の JSON を URL-safe base64 (RFC 4648 §5、パディング `=` あり) でエンコードしたもの:

```json
{"lang_id": "6088", "source": "fn main() { ... }"}
```

ブラウザで URL を開く際は OS のデフォルトブラウザを使用する:
- Linux: `xdg-open <url>`
- macOS: `open <url>`
- Windows: `cmd /c start <url>`

## エラーケース

- 解法の `ce.toml` が存在しない: パスを表示してエラー終了
- 提出ファイルが存在しない: パスを表示してエラー終了
- `lang_id` が config に未設定: エラー終了

## 将来拡張

- リアルタイムモード: `ce sub a` (cwd から `contest_id` を自動検出)
- 提出後の結果をポーリングして表示 (ブラウザから提出 URL を受け取る方法が別途必要)
- `submit_preprocess`: バンドル等を実行して stdout を提出内容とする
