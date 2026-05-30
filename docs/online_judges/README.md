# Online Judge 抽象

複数の Online Judge (OJ) を扱うためのポート設計と責務をまとめる。
個別 OJ の仕様は `docs/online_judges/{name}.md` に置く (例: `librarychecker.md`)。

> このドキュメントは「期待する状態と判断基準」を書く。具体的な実装手順はタスクファイル
> (`tasks/`) に置く。現行コードの該当箇所は `docs/spec.md` の「アーキテクチャ層構成」を参照。

## 設計の前提

- `OnlineJudge` は usecases 層のポート (trait)。実装は infrastructure 層 (`online_judge_impl/`)。
- ツールは最初からマルチ OJ を想定する。`.ce.toml` に `online_judge` を保存し、`ce test` /
  `ce sub` 時に `ContestRepository::get_oj_kind` で復元する。
- 現状は AtCoder のみ実装されている。LibraryChecker を追加するにあたり、AtCoder 前提が
  残る箇所 (OnlineJudge の固定注入・ブラウザ提出固定・手動 cookie ログイン固定) を一般化する。

## 「コンテスト」の一般化

`contest_id` と `Contest` 集約は OJ 横断の「取得単位」として扱う。

- AtCoder: `contest_id` = コンテスト ID (`abc334`)、1 コンテストに複数 `Problem`。
- LibraryChecker: コンテスト概念がないため **問題 = 単問コンテスト**。`contest_id` = 問題名
  (`aplusb`)、`problems` は 1 件。ディレクトリ構造 (`solutions/{contest_id}/{problem_code}/`)
  をそのまま再利用する。将来のライブラリ verify 機能 (ファイルごとに verify 問題を個別指定)
  とも整合する。

## OJ の動的解決 (registry)

- `Service` は単一の `OnlineJudge` を固定で持たない。`OJKind` から対象 OJ 実装を解決する。
- 解決手段は usecases 層のポートとして定義する (例: `OnlineJudgeRegistry`)。実装は
  infrastructure が各 OJ を登録して提供する。
- `ce sub` / `ce test` は `.ce.toml` の `OJKind` に従って OJ を選ぶ。
  - **判断基準**: AtCoder で初期化したコンテストは従来通り AtCoder へ、LibraryChecker で
    初期化した問題は LibraryChecker へ提出される (固定注入による誤提出が起きない)。

## OJ 判定 (init 時)

`ce init <contest_id_or_url>` の入力から OJ と取得単位 ID を判定する。各 OJ が判定材料を申告し、
判定器が走査する形にする。

- AtCoder: `atcoder.jp/contests/{id}` URL、または `abc/arc/agc/ahc` プレフィックス。
- LibraryChecker: `judge.yosupo.jp/problem/{name}` URL。命名規則 (プレフィックス) は持たない。
- いずれにも該当しない場合: stdin で OJ 名を尋ねる (既存挙動)。`--oj` 明示指定の要否は未決。

## ログインの一般化

ログイン方式は OJ ごとに異なる。OJ は必要な資格情報の種別を申告し、`ce login` はそれに応じて
入力を促す。

| OJ | 方式 | 入力 | 検証 |
| --- | --- | --- | --- |
| AtCoder | 手動 cookie | `REVEL_SESSION` を貼り付け | ネットワーク不要。貼り付け値をそのまま保存 |
| LibraryChecker | パスワード | email + password | Firebase に問い合わせてトークン取得。失敗時はエラー |

- ポートは「資格情報種別の申告」と「資格情報 → `Session` の生成」を表現する
  (例: `credential_kind()` と `login(credentials) -> Session`)。
- `Session` は OJ 固有の認証材料を保持する。AtCoder は cookie 文字列、LibraryChecker は
  Firebase の idToken (+ refreshToken)。session.toml には OJ ごとのセクションで保存する。

## 提出の一般化

提出は「ブラウザで開く URL を返す」方式に固定しない。提出結果を表現する型を返す。

- AtCoder: Cloudflare Turnstile のため直接 POST 不可。提出内容を URL フラグメントに載せて
  ブラウザの submit ページを開く (Tampermonkey userscript が自動入力)。→ 「開く URL」を返す。
- LibraryChecker: bot 対策が緩く REST で直接提出できる。→ 「提出済み (提出 id/URL)」を返す。

shell 層は返り値の種別に応じて、URL を開く / 提出 URL を表示する、を出し分ける。
`ce sub` の提出前テスト (Unix で `test_command` を実行し exit 0 のみ続行) は OJ 非依存で維持する。

## 各 OJ ドキュメント

- [AtCoder](./atcoder.md)
- [LibraryChecker](./librarychecker.md)

## 未決事項

- `Session` を cookie 単一文字列のまま OJ 固有トークンも表現するか、enum 等へ拡張するか。
- LibraryChecker の idToken 失効時のリフレッシュをどのコマンドで行うか。
- `ce init` に `--oj` 明示フラグを追加するか。
