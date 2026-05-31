# LibraryChecker

[Library Checker](https://judge.yosupo.jp/) (yosupo judge) の OJ 実装仕様。
抽象の共通設計は [README.md](./README.md) を参照。

## 概要

- コンテスト概念がない問題集型 OJ。問題は単体で URL を持つ: `https://judge.yosupo.jp/problem/{name}`
  (例: `aplusb`)。
- 本ツールでは **問題 = 単問コンテスト** として扱う。`contest_id` は **`librarychecker-` を冠した
  名前空間付き** (例 `librarychecker-aplusb`)、`problems` は 1 件。**問題 (Problem) の `id`/`code` は
  素の問題名** (`aplusb`)。素の名前は API/バケット呼び出しと提出時に prefix を剥がして復元する。
  - 名前空間化の理由: AtCoder の contest_id と衝突しないこと、id 単体で OJ が判別できること、
    将来のライブラリ verify 機能 (ファイルごとに verify 問題を個別指定) とも整合する。
  - ディレクトリ: `solutions/librarychecker-aplusb/aplusb/main/`。
- bot 対策が緩く REST API + Firebase Auth で **ログイン・提出を自動化する** (AtCoder と異なり
  ブラウザ手動提出を避ける)。

## 判定

- URL `https://judge.yosupo.jp/problem/{name}` → `(LibraryChecker, contest_id=librarychecker-{name})`
  (descriptor の `contest_id_prefix` を URL 抽出 id に前置)。
- 名前空間付き id (`librarychecker-{name}`) はプレフィックス判定でも検出する (再 init などに対応)。
  素の問題名 (`aplusb`) は命名規則がないため検出しない。

## 実装フェーズと中間状態

LibraryChecker は段階的に追加する。Phase ごとの完了時状態と判断基準は次の通り。

- **Phase C (TASK-035)**: `OJKind::LibraryChecker` variant・`as_str`/`FromStr` (`"librarychecker"`)・
  URL descriptor を追加する。**この時点では LC の `OnlineJudge` 実装はまだ無い** (Phase D)。
  - **判断基準**: `ce init <LC URL>` は `(LibraryChecker, name)` を検出するが、その先の OJ 解決
    (registry) と login の資格情報解決は **clean な anyhow エラー** (例:
    `"LibraryChecker is not yet implemented"`) を返す。`todo!()` で panic させない
    (バイナリは使用可能なまま保ち、tests/clippy を通す)。
  - AtCoder の既存挙動は一切変えない。LC 検出が増えるだけ。
- **Phase D (TASK-036)**: 上記の clean エラー stub を実際の REST/Firebase 実装に置き換える。
  あわせて `Session` を enum 化して Firebase トークンを保持できるようにする (login が生成・
  submit/whoami が消費するため Phase D に含める)。
- **Phase E (TASK-037)**: config (lang_id) の解決を LibraryChecker でも機能させる。
  Session の保存形式は Phase D で確定済みのため、E は config (lang_id) に焦点を絞る。

## エンドポイント (調査結果)

REST API ベース URL: `https://v3.api.judge.yosupo.jp` (旧 gRPC から REST へ移行済み)。
定義元: `yosupo06/library-checker-judge` の `restapi/openapi/openapi.yaml`。

| 用途 | メソッド | パス | 認証 |
| --- | --- | --- | --- |
| 問題情報 | GET | `/problems/{name}` | 不要 |
| 言語一覧 | GET | `/langs` | 不要 |
| 現在ユーザー | GET | `/auth/current_user` | Firebase Bearer |
| 提出 | POST | `/submit` | Firebase Bearer |
| 提出情報 | GET | `/submissions/{id}` | 不要 |

### 認証 (Firebase Auth)

- API は Firebase の idToken を `Authorization: Bearer <idToken>` で要求する (`bearerFormat: JWT`)。
- idToken は Firebase REST で取得する:
  - `POST https://identitytoolkit.googleapis.com/v1/accounts:signInWithPassword?key=<API_KEY>`
  - body: `{ "email", "password", "returnSecureToken": true }`
  - 返り: `idToken`, `refreshToken`, `expiresIn` (~3600s)
- 公開 Firebase 設定 (frontend の `.env.production` 由来、秘匿情報ではない):
  - API key: `AIzaSyCmpkoMVbKRDm2H0MJHB0iZ43uQtSqiLV0`
  - authDomain: `prod-library-checker-project.firebaseapp.com`
- **注意**: Firebase は username ではなく **email** でログインする。`ce login librarychecker` は
  email + password を受け取る。
- shell の EmailPassword 入力は **パスワードを no-echo で読み取る** (`rpassword::prompt_password`、
  画面非表示)。`rpassword` は末尾改行を含まない値を返すため、パスワードは trim せずそのまま渡す
  (email のみ trim)。
- idToken は短命 (~3600s)。失効時は **オンデマンドで更新する** (確定): Bearer を要するリクエスト
  (whoami/submit) が 401/403 を返したら `POST https://securetoken.googleapis.com/v1/token?key=<API_KEY>`
  に form `grant_type=refresh_token&refresh_token=<token>` を送って新 idToken を取得し、1 度だけ再試行する。
  refreshToken も失効していれば clean エラーで `ce login` を促す。
  - **注意 (実装上の制約)**: `OnlineJudge` trait は `SessionRepository` に触れないため、リフレッシュで得た
    新 idToken は **永続化しない**。永続する資格情報は refreshToken であり、`ce` の各プロセスは
    必要に応じて毎回 refreshToken から idToken を取り直す。これで十分機能する (保存した idToken が
    生きていれば最初の呼び出しはリフレッシュをスキップできる、という最適化のみ失う)。
  - リフレッシュのレスポンスは **snake_case** (`id_token` / `refresh_token`)。sign-in は camelCase
    (`idToken` / `refreshToken`) なので両者で別パース。

### 問題情報・サンプル

- `GET /problems/{name}` → `{ title, source_url, time_limit, version, overall_version, testcases_version }`。
  **samples は含まれない。**
- **サンプルは公開データバケットの「例ファイル」から取得する** (確定。当初の「問題ページをスクレイプ」は
  不可と判明: 問題ページは React SPA で、サーバ応答は ~500 byte の空シェルのみ。サンプルは取得できない)。
  公式フロントエンドと同じ経路を使う:
  - 公開バケット base: `https://storage.googleapis.com/v2-prod-library-checker-data-public/`
  - 例の個数: `<base>/v4/files/{name}/{overall_version}/{name}/info.toml` を取得し、`[[tests]]` の
    `name == "example.in"` エントリの `number` を読む (無ければ 0)。
  - 各サンプル (idx = 0..number):
    - 入力 `<base>/v4/examples/{name}/{testcases_version}/in/example_0{idx}.in`
    - 出力 `<base>/v4/examples/{name}/{testcases_version}/out/example_0{idx}.out`
  - **取得するのは小さな例ファイルのみ** (公式テストケース全体ではない)。`.out` はビルド時に生成済みで
    バケットに存在する (repo の `gen/` には `.in` しか無いため repo からは取れない)。
  - verify は実際に提出して OJ に判定させるため、ローカルに公式テスト全体を持つ必要はない。
- **`input_format_raw` / `constraints_raw` は task.md から抽出する** (確定。これにより入力コード自動生成が
  LibraryChecker でも機能する)。問題ページ HTML はスクレイプできない (SPA) が、statement のソース
  `task.md` はバケットから取得できる:
  - statement ソース: `<base>/v4/files/{name}/{overall_version}/{name}/task.md`
  - `input_format_raw`: `## @{keyword.input}` 見出し直後のフェンス済みコードブロックを取り、`$` を除去する
    (例 `$A$ $B$` → `A B`、`$N$` 改行 `$A_1$ $A_2$ $\dots$ $A_N$` → `N` 改行 `A_1 A_2 \dots A_N`)。
    パーサは `$` 無しの AtCoder 形式を期待するため、`$` 除去で両者の形式が揃う。
  - `constraints_raw`: `## @{keyword.constraints}` セクションを次の `##` 見出しまで取り、`@{param.NAME}` を
    info.toml の `[params]` で解決し `$` を除去する (best-effort)。
  - いずれも取得・抽出に失敗したら `None` フォールバック。
  - 実測: `aplusb` → `plain` 判定、`static_range_sum` → `loop` 判定で入力コード生成が成功する。

### 言語 (lang_id)

- `GET /langs` → `[{ id, name, version }]`。`id` が提出時の `lang` 値 (例 `cpp`, `cpp20`, `rust`,
  `python3`, `pypy3`)。
- **lang_id のデフォルト**: LibraryChecker の `lang` id は言語名とほぼ一致するため、config 未設定時は
  **言語名をそのまま lang_id として使う** (`OnlineJudge::default_lang_id`)。`rust` → `rust`、`cpp` → `cpp` で
  動く。LC id が言語名と異なる場合 (例 Python → `python3`/`pypy3`) は config で明示する。
  - 解決順 (submit サービス): `config.lang_id(lang, oj)` → 無ければ `oj.default_lang_id(lang)` → それも
    無ければエラー。AtCoder は `default_lang_id` = None のままなので従来通り config 必須。
- config で上書きする場合: `[language.{lang}.librarychecker].lang_id = "{id}"`。
- 不正な lang_id は提出 API がエラーを返す。

### 提出

- `POST /submit` body: `{ "problem": "{name}", "source": "...", "lang": "{lang_id}" }` + Firebase Bearer。
  - 制約: `source` 最大 1 MiB、`lang` 最大 64 文字。
- 返り: `{ "id": <int> }`。
- 表示 URL: `https://judge.yosupo.jp/submission/{id}` を標準出力に出す (任意でブラウザを開く)。
- 直接提出のため、抽象の「提出済み」結果として提出 URL を返す (README のログイン/提出一般化を参照)。

## Session の保存

- session.toml の `[librarychecker]` セクションに保存する。
- 保持する値: idToken と refreshToken。
- **`Session` は enum で OJ 別の認証材料を型区別する** (確定)。例:
  - `Session::Cookie { online_judge, cookie }` — AtCoder (既存挙動を維持)
  - `Session::Firebase { online_judge, id_token, refresh_token }` — LibraryChecker
- serde で session.toml に `[atcoder]` / `[librarychecker]` のセクション別形式でシリアライズする。
  AtCoder の保存・読み出し挙動は不変。
- この型変更は Phase D (TASK-036) で行う (login の生成物 = submit/whoami の消費物のため)。

## 検証 (受け入れ基準)

- `ce init https://judge.yosupo.jp/problem/aplusb` で `aplusb` が単問コンテストとして作成され、
  問題ページのサンプルが取得される。
- `ce login librarychecker` で email + password を入力しトークンが保存される。
- `ce whoami librarychecker` でユーザー名が表示される。
- `ce sub aplusb {code}` で提出が成功し提出 URL が表示される (実 OJ で手動確認)。

## 未決事項

- (なし)

## 確定済み (旧未決)

- 問題ページのサンプル/statement 取得: 問題ページは SPA でスクレイプ不可。サンプルは公開バケットの
  例ファイル、input_format/constraints は statement ソース `task.md` から取得 (上記「問題情報・サンプル」)。
- `input_format_raw` / `constraints_raw`: task.md から抽出し `$` 除去・`@{param}` 解決で取得 (取れなければ
  `None`)。入力コード自動生成が機能する (実測 `aplusb`:plain / `static_range_sum`:loop)。
- `Session` の表現: enum で OJ 別 auth を型区別 (`Session::Firebase { id_token, refresh_token }`)。Phase D。
- idToken のリフレッシュ契機: オンデマンド更新 (Bearer 呼び出しが 401/403 → refreshToken で更新し 1 度再試行)。
- GitHub OAuth ログインのユーザーは email/password ログイン不可の場合がある → 今回は **email/password 前提**で確定。
