# ce check

## 概要

`[library.languages]` に定義した各言語の `check_command` を実行するライブラリプラットフォーム向けコマンド。
`cargo test` / property-based testing / `lake build` / 証明チェック / lint 等を、アプリはその中身を知らずに委譲する（詳細: `docs/spec.md` §7.1）。

- 結果は端末または CI ログにのみ出力する。結果ファイルや Web 公開データには含めない。
- 通常の公開 solution の `test_command` は `ce check` から実行しない (それは `ce test` の役割)。
- 現在の `ce check` は Unix-like shell (`sh`) が利用できる環境のみ対応する。

## シグネチャ

```
ce check [--language <id>]
```

- `--language <id>`: 単一言語だけを実行する。省略時は `[library.languages]` に登録された全言語を実行する。

CI の通常 check と公開 build は言語 filter なしで実行する。filter 付き check の成功だけを repository 全体の公開可否には使わない。

## 挙動

1. `templates/` を含む project root を探索し、`config.toml` の `[library]` を strict にロードする。
2. `[library.languages]` の各言語を **`LanguageId` 昇順 (UTF-8 バイト順)** で 1 回ずつ処理する。
3. 各言語について:
   - `check_command` 未設定なら `[<lang>] skipped (no check_command configured)` を stderr に出し、`Skipped` として集計する（失敗にしない）。
   - `check_command` が設定されていれば `sh -c <command>` で実行する。
     - 作業ディレクトリ: `<repository_root>/<language.root>` (spec §7.1「check の作業ディレクトリは言語 root」)
     - タイムアウト: `check_timeout_seconds` (既定 600 秒、正の整数のみ許可)
     - 標準出力・標準エラーは実行中に親プロセスへそのままストリーミングする。
4. 全言語を実行し終えたら、行ごとに 1 行の集約結果を stdout に出す。
5. 1 件でも `Failed` / `TimedOut` があれば exit 1、そうでなければ exit 0。

タイムアウトは check failure として扱い、残りの言語も引き続き実行する。1 言語が失敗しても他の言語の実行は止めない (spec §7.1)。

## 環境変数

各 check_command は sanitized な環境で起動する。親プロセスの環境を丸ごと継承させない代わりに、以下だけを明示的に渡す:

| 変数                  | 内容                                                                        |
| --------------------- | --------------------------------------------------------------------------- |
| `CE_REPOSITORY_ROOT`  | project root の絶対パス                                                     |
| `CE_LIBRARY_ROOT`     | 現在処理中の言語 root (`<repository_root>/<language.root>`) の絶対パス      |
| `CE_LANGUAGE`         | 現在処理中の `LanguageId` (例: `rust`)                                      |
| `PATH`                | 親プロセスから継承 (`sh` および外部ツールを解決するために必須)              |
| `HOME`                | 親プロセスから継承 (未設定なら渡さない)                                     |
| `TERM`                | 親プロセスから継承 (未設定なら渡さない)                                     |

OJ credential / GitHub token / cloud credential 等の secret は check には渡さない (spec §6 の sanitized environment 方針に一致)。

## タイムアウトとプロセスグループ

- 各 command は独立したプロセスグループに置き、`check_timeout_seconds` 超過時にプロセスグループ全体へ `SIGTERM` を送り、5 秒後にも残っていれば `SIGKILL` を送る (詳細: `usecases::command_runner::CommandRunner` 実装、spec §7.1)。
- shell だけを終了して compiler や test runner を残さない。

## エラーケース

- project root が見つからない (`templates/` を上位に見つけられない): エラー終了
- `config.toml` が無い、または strict loader が拒否した: エラー終了
- `--language <id>` の id が `[library.languages]` に存在しない: エラー終了
- `check_timeout_seconds` が非正整数: `config.toml` の loader 側で拒否
- `sh` が見つからない等の起動失敗: エラー終了

## 例

```
$ ce check
[cpp] skipped (no check_command configured)
[lean] passed
[rust] passed
```

```
$ ce check --language rust
[rust] passed
```

## 関連

- `docs/commands/test.md` — solution 個別の `ce test`。`ce check` からは呼ばない。
- `docs/spec.md` §7.1 — check と verify の設計要件。
