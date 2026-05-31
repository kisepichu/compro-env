# Online Judge 抽象

複数の Online Judge (OJ) を扱うためのポート設計と責務をまとめる。
個別 OJ の仕様は `docs/online_judges/{name}.md` に置く (例: `librarychecker.md`)。

> このドキュメントは「期待する状態と判断基準」を書く。具体的な実装手順はタスクファイル
> (`tasks/`) に置く。現行コードの該当箇所は `docs/spec.md` の「アーキテクチャ層構成」を参照。

## 設計の前提

- `OnlineJudge` は usecases 層のポート (trait)。実装は infrastructure 層 (`online_judge_impl/`)。
- ツールは最初からマルチ OJ を想定する。`.ce.toml` に `online_judge` を保存し、`ce test` /
  `ce sub` 時に `ContestRepository::get_oj_kind` で復元する。
- AtCoder と LibraryChecker の両方が実装済み。AtCoder 前提が残っていた箇所
  (OnlineJudge の固定注入・ブラウザ提出固定・手動 cookie ログイン固定) は一般化済み。
- **実装状況**: Phase A〜E すべて実装済み。下記「動的解決」「ログインの一般化」「提出の一般化」は
  TASK-033 (Phase A)、「OJ 判定」の拡張点化 (descriptor + `OJKind::detect`) は TASK-034 (Phase B)、
  `OJKind::LibraryChecker` 追加と LC URL 判定は TASK-035 (Phase C)、LC の REST/Firebase 実装は
  TASK-036 (Phase D)、config(lang_id)/session は TASK-037 (Phase E) で実装済み。
  config/session は Phase D で先取り実装された (Session enum 化は login の生成物 = submit/whoami の
  消費物のため D に含めた)。詳細は [librarychecker.md](./librarychecker.md) の「実装フェーズと中間状態」。

## 「コンテスト」の一般化

`contest_id` と `Contest` 集約は OJ 横断の「取得単位」として扱う。

- AtCoder: `contest_id` = コンテスト ID (`abc334`)、1 コンテストに複数 `Problem`。
- LibraryChecker: コンテスト概念がないため **問題 = 単問コンテスト**。`contest_id` は
  `librarychecker-` を冠した名前空間付き (`librarychecker-aplusb`)、`problems` は 1 件で
  `Problem.id`/`code` は素の問題名 (`aplusb`)。名前空間化で AtCoder の contest_id と衝突せず、
  id 単体で OJ が判別できる。ディレクトリ構造 (`solutions/{contest_id}/{problem_code}/`) はそのまま
  再利用 (`solutions/librarychecker-aplusb/aplusb/`)。将来のライブラリ verify 機能とも整合する。
  - descriptor に `contest_id_prefix` を持たせ、URL 抽出 id に前置する (OJKind::detect)。

## OJ の動的解決 (registry)

- `Service` は単一の `OnlineJudge` を固定で持たない。`OJKind` から対象 OJ 実装を解決する。
- 解決手段は usecases 層のポートとして定義する (例: `OnlineJudgeRegistry`)。実装は
  infrastructure が各 OJ を登録して提供する。
- `ce sub` / `ce test` は `.ce.toml` の `OJKind` に従って OJ を選ぶ。
  - **判断基準**: AtCoder で初期化したコンテストは従来通り AtCoder へ、LibraryChecker で
    初期化した問題は LibraryChecker へ提出される (固定注入による誤提出が起きない)。

## OJ 判定 (init 時)

`ce init <contest_id_or_url>` の入力から OJ と取得単位 ID を判定する。各 OJ が判定材料 (descriptor)
を申告し、domain の純粋関数 `OJKind::detect` が走査して `(OJKind, contest_id)` を返す。判定は
I/O を伴わないため domain 層に置き、infrastructure の `parse_contest_input` は委譲のみとする。

- descriptor が申告する材料: 「URL ホスト + パスパターン」と「contest_id プレフィックス」。
- AtCoder: `atcoder.jp/contests/{id}` URL、または `abc/arc/agc/ahc` プレフィックス。
- LibraryChecker: `judge.yosupo.jp/problem/{name}` URL。命名規則 (プレフィックス) は持たない。
- いずれにも該当しない場合: stdin で OJ 名を尋ねる (既存挙動)。`--oj` 明示フラグは追加しない。
- **判断基準**: OJ を追加する変更が descriptor 1 件の追加に閉じ、判定ロジック本体や match の
  散在を増やさない。

**実装フェーズ**: 判定機構の拡張点化は TASK-034 (Phase B) で行うが、B は既存 AtCoder 判定を
この機構へ移すリファクタに留め挙動を変えない。LibraryChecker の URL descriptor は variant を
追加する TASK-035 (Phase C) で足す。

## ログインの一般化

ログイン方式は OJ ごとに異なる。OJ は必要な資格情報の種別を申告し、`ce login` はそれに応じて
入力を促す。

| OJ | 方式 | 入力 | 検証 |
| --- | --- | --- | --- |
| AtCoder | 手動 cookie | `REVEL_SESSION` を貼り付け | ネットワーク不要。貼り付け値をそのまま保存 |
| LibraryChecker | パスワード | email + password | Firebase に問い合わせてトークン取得。失敗時はエラー |

- ポートは「資格情報種別の申告」と「資格情報 → `Session` の生成」を表現する
  (例: `credential_kind()` と `login(credentials) -> Session`)。
- `Session` は OJ 固有の認証材料を保持する。**enum で OJ 別の認証材料を型で区別する**
  (確定): AtCoder は cookie 文字列、LibraryChecker は Firebase の idToken + refreshToken。
  session.toml には OJ ごとのセクションで保存する。この型変更は Phase D (TASK-036) で行う
  (login がトークンを生成し submit/whoami が消費するため、Phase E に分離できない)。

## 提出の一般化

提出は「ブラウザで開く URL を返す」方式に固定しない。提出結果を表現する型を返す。

- AtCoder: Cloudflare Turnstile のため直接 POST 不可。提出内容を URL フラグメントに載せて
  ブラウザの submit ページを開く (Tampermonkey userscript が自動入力)。→ 「開く URL」を返す。
- LibraryChecker: bot 対策が緩く REST で直接提出できる。→ 「提出済み (提出 id/URL)」を返す。

shell 層は返り値の種別に応じて、URL を開く / 提出 URL を表示する、を出し分ける。
`ce sub` の提出前テスト (Unix で `test_command` を実行し exit 0 のみ続行) は OJ 非依存で維持する。

**lang_id の解決**: submit は `config.lang_id(lang, oj)` を優先し、無ければ OJ の
`OnlineJudge::default_lang_id(lang)` にフォールバックする。AtCoder は default = None (config 必須)。
LibraryChecker は lang id が言語名とほぼ一致するため、言語名をデフォルトに使う (config 上書き可)。

## 各 OJ ドキュメント

- [AtCoder](./atcoder.md)
- [LibraryChecker](./librarychecker.md)

## 未決事項

- `ce init` の `--oj` 明示フラグ: **追加しない**で確定 (判定不能時は stdin プロンプト)。

## 確定済み (旧未決)

- `Session` の表現: **enum で OJ 別 auth を型区別する**で確定 (AtCoder=Cookie / LibraryChecker=Firebase
  idToken+refreshToken)。Phase D で実装。
- LibraryChecker の idToken 失効時のリフレッシュ: **オンデマンド更新**で確定。Bearer リクエストが
  401/403 を返したら refreshToken で idToken を更新 (`securetoken.googleapis.com`) し 1 度だけ
  再試行する。明示的な `ce login` 再実行は不要。refreshToken も失効していれば clean エラーで再ログインを促す。
  - 永続する資格情報は **refreshToken**。`OnlineJudge` trait は `SessionRepository` に触れないため、
    リフレッシュで得た新 idToken はプロセス内のみで使い、保存はしない (詳細は librarychecker.md)。
