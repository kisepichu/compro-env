# LibraryChecker

[Library Checker](https://judge.yosupo.jp/) (yosupo judge) の OJ 実装仕様。
抽象の共通設計は [README.md](./README.md) を参照。

## 概要

- コンテスト概念がない問題集型 OJ。問題は単体で URL を持つ: `https://judge.yosupo.jp/problem/{name}`
  (例: `aplusb`)。
- 本ツールでは **問題 = 単問コンテスト** として扱う。`contest_id` = 問題名、`problems` は 1 件。
- bot 対策が緩く REST API + Firebase Auth で **ログイン・提出を自動化する** (AtCoder と異なり
  ブラウザ手動提出を避ける)。

## 判定

- URL `https://judge.yosupo.jp/problem/{name}` → `(LibraryChecker, contest_id={name})`。
- contest_id のプレフィックス命名規則は持たない。プレフィックス判定では検出しない。

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
- **Phase E (TASK-037)**: config (lang_id) と session (Firebase token) の保存形式を確定する。

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
- **既知の制約**: 現状の shell の EmailPassword 入力はパスワードを端末エコーありで読み取る
  (画面に表示される)。LC ログイン実装時 (TASK-036) に no-echo 入力 (例: `rpassword`) へ切り替える。
- idToken は短命。失効時は refreshToken で更新する (`securetoken.googleapis.com`)。更新タイミングは未決。

### 問題情報・サンプル

- `GET /problems/{name}` → `{ title, source_url, time_limit, version, ... }`。**samples は含まれない。**
- サンプルは **問題ページの例セクションのみ** をスクレイプして取得する (軽量方針)。
  - 公式テストケース (公開 GCS バケット `storage.googleapis.com/v2-prod-library-checker-data-public/`)
    は容量が大きいため **使わない**。ローカルのサンプルテストは小さく行う。
  - verify は実際に提出して OJ に判定させるため、ローカルに公式テストを持つ必要はない。
- `input_format_raw` / `constraints_raw` は LibraryChecker の問題文形式に依存する。意味ある形で
  取得できなければ空文字でフォールバックする (`InputFormatKind::Fail` 相当でも可)。

### 言語 (lang_id)

- `GET /langs` → `[{ id, name, version }]`。`id` が提出時の `lang` 値。
- config では `[language.{lang}.librarychecker].lang_id = "{id}"` で対応付ける。

### 提出

- `POST /submit` body: `{ "problem": "{name}", "source": "...", "lang": "{lang_id}" }` + Firebase Bearer。
  - 制約: `source` 最大 1 MiB、`lang` 最大 64 文字。
- 返り: `{ "id": <int> }`。
- 表示 URL: `https://judge.yosupo.jp/submission/{id}` を標準出力に出す (任意でブラウザを開く)。
- 直接提出のため、抽象の「提出済み」結果として提出 URL を返す (README のログイン/提出一般化を参照)。

## Session の保存

- session.toml の `[librarychecker]` セクションに保存する。
- 保持する値: idToken と refreshToken (cookie 単一文字列では不足する可能性あり)。
  Session 型を cookie 文字列のまま使うか拡張するかは未決 (TASK-037)。

## 検証 (受け入れ基準)

- `ce init https://judge.yosupo.jp/problem/aplusb` で `aplusb` が単問コンテストとして作成され、
  問題ページのサンプルが取得される。
- `ce login librarychecker` で email + password を入力しトークンが保存される。
- `ce whoami librarychecker` でユーザー名が表示される。
- `ce sub aplusb {code}` で提出が成功し提出 URL が表示される (実 OJ で手動確認)。

## 未決事項

- 問題ページのサンプル HTML 構造 (実装時に実ページで確認)。
- `input_format_raw` を LibraryChecker で意味ある形で取得できるか。
- idToken のリフレッシュ契機。
- GitHub OAuth ログインのユーザーは email/password ログインできない場合がある (今回は email/password 前提)。
