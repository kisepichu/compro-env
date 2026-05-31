# TASK-036: LibraryChecker OnlineJudge 実装 (Phase D)

LibraryChecker の `OnlineJudge` 実装を追加する。REST API (`https://v3.api.judge.yosupo.jp`)
と Firebase Auth を用い、問題取得・サンプル取得・ログイン・提出を実装する。
あわせて `Session` を enum 化して Firebase トークンを保持できるようにする (spec-update 確定)。

## 参照仕様

- docs/online_judges/README.md (OnlineJudge / Session / 提出の一般化)
- docs/online_judges/librarychecker.md
- docs/spec.md (OnlineJudge インターフェース)

## 確定事項 (spec-update 2026-05-31)

- `Session` は **enum** で OJ 別 auth を型区別する:
  - `Session::Cookie { online_judge, cookie }` — AtCoder (既存挙動維持)
  - `Session::Firebase { online_judge, id_token, refresh_token }` — LibraryChecker
- idToken のリフレッシュは **オンデマンド**: Bearer 呼び出しが 401/403 → refreshToken で更新し
  1 度だけ再試行。refreshToken も失効なら clean エラーで再ログイン誘導。
  - **永続化しない**: `OnlineJudge` は `SessionRepository` に触れないため、リフレッシュで得た新 idToken は
    プロセス内のみで使う。durable な資格情報は refreshToken (詳細は librarychecker.md)。
- ログインは email + password 前提 (GitHub OAuth ユーザーは対象外)。
- サンプルは問題ページの例セクションのみスクレイプ (GCS 公式テストは使わない)。

## 実 API メモ (調査済み)

- REST ベース: `https://v3.api.judge.yosupo.jp`
- 認証: Firebase Auth (Bearer JWT)。`signInWithPassword?key=<API_KEY>` に email+password → idToken。
  - 公開 API key: `AIzaSyCmpkoMVbKRDm2H0MJHB0iZ43uQtSqiLV0`
  - リフレッシュ: `POST https://securetoken.googleapis.com/v1/token?key=<API_KEY>`
    body `{ grant_type: "refresh_token", refresh_token }` → 新 idToken
- `GET /problems/{name}` → title, time_limit 等 (samples 含まず)
- `GET /langs` → `[{id,name,version}]` (lang_id に対応)
- `POST /submit` body `{problem, source, lang}` + Bearer → `{id}`。表示 URL `https://judge.yosupo.jp/submission/{id}`

## 実装チェックリスト (TDD: 内側レイヤーから)

### 1. domain/ — Session の enum 化

- [x] `Session` を enum 化 (`Cookie` / `Firebase`)。`online_judge()` / `set_online_judge()` アクセサを提供
- [x] 既存の `Session { online_judge, cookie }` 利用箇所を全てコンパイルが通る形に更新
- [x] テスト: enum の生成・アクセサ (entity.rs)

### 2. infrastructure/ — session の serde

- [x] `session_repository_impl.rs`: enum を session.toml の `[atcoder]`/`[librarychecker]` セクション別形式で
      シリアライズ/デシリアライズ。AtCoder の保存・読み出し挙動は不変
- [x] テスト: AtCoder cookie round-trip / LibraryChecker token round-trip / 2 OJ 併存

### 3. infrastructure/ — LibraryChecker OnlineJudge 実装

- [x] `online_judge_impl/librarychecker.rs` を新規作成し `OnlineJudge` を実装
- [x] `name()` = `"librarychecker"`, `credential_kind()` = `EmailPassword`
- [x] `login`: Firebase `signInWithPassword` に email+password → `Session::Firebase` を返す
- [x] `whoami`: `GET /auth/current_user` (Bearer) → ユーザー名。401/403 時はオンデマンドリフレッシュ
- [x] `get_contest_meta`: LC はコンテスト概念なし → start_time=None、hints 空
- [x] `get_problems_detail`: `GET /problems/{name}` + 公開バケットの例ファイルからサンプル取得し Problem 1 件
  - contest_id=問題名、problems=[1件] (問題=単問コンテスト)、id/code=問題名
  - **サンプルはバケット経由** (問題ページは SPA でスクレイプ不可と判明)。info.toml で例数→
    `v4/examples/.../in|out/example_0N` を取得
  - input_format_raw / constraints_raw = None フォールバック
  - テスト: URL ビルダ・info.toml の例数カウント・各種レスポンスパースを純粋関数として単体テスト
- [x] `submit`: `POST /submit` (Bearer) → `SubmitOutcome::Submitted { submission_url }`。401/403 リフレッシュ
- [x] オンデマンドリフレッシュのヘルパ `send_authed` (Bearer 呼び出しの共通ラッパ)

### 4. infrastructure/ — registry 登録

- [x] `registry.rs`: `OJKind::LibraryChecker => Ok(&self.librarychecker)` に置換 (clean error stub 撤去)
- [x] テスト: registry が両 OJ を解決できる

### 5. infrastructure/shell — パスワード no-echo

- [x] EmailPassword のパスワード入力を `rpassword::prompt_password` で no-echo に切り替え
- [x] Cargo.toml に `rpassword` 追加 (+ reqwest の `json` feature)

## 完了条件

- [x] `ce init <LC問題URL>` で単問コンテストとして問題・サンプルが取得できる
      (実 LC で確認: `ce init https://judge.yosupo.jp/problem/aplusb` → 2 サンプル取得・正しい出力)
- [x] `ce login librarychecker` で email+password ログインしトークン保存 (実アカウントで確認済み)
- [x] `ce sub` で実際に提出され提出 URL が表示される (実アカウントで確認済み)
- [x] `cargo test --all && cargo clippy --all --all-features -- -D warnings && cargo fmt --all --check` 通過

## 未決事項 (解決済み)

- 問題ページのサンプル HTML 構造 → **解決**: 問題ページは SPA でスクレイプ不可。公開データバケットの
  `v4/examples/{name}/{testcases_version}/in|out/example_0N.{in,out}` から取得 (info.toml で例数判定)。
  spec (librarychecker.md) を更新済み。
- input_format_raw を LC で意味ある形で取れるか → **解決**: task.md は Markdown テンプレートのため
  `None` フォールバックで確定。

## 追加対応 (2026-05-31 ユーザーフィードバック)

- [x] **contest_id を `librarychecker-` で名前空間化**。`OjDescriptor.contest_id_prefix` を追加し
      URL 抽出 id に前置。`Problem.id`/`code` は素の問題名。`get_problems_detail` で prefix を剥がす。
      名前空間付き id はプレフィックス判定でも検出。detect テスト更新。
- [x] **入力フォーマット抽出 (input_format_raw / constraints_raw)**。問題ページは SPA でスクレイプ不可
      だが statement ソース `task.md` をバケットから取得し、`@{keyword.input}` のフェンスブロックを抽出して
      `$` 除去 (パーサが期待する AtCoder 形式に揃う)。constraints は `@{param.X}` を info.toml で解決。
      → 入力コード自動生成が LC でも機能 (実測 `aplusb`:plain / `static_range_sum`:loop)。純粋関数を単体テスト。
- [x] **lang_id 自動解決**。`OnlineJudge::default_lang_id(lang)` (default None) を追加し LC は言語名を返す。
      submit は config → default_lang_id の順で解決。config 未設定でも rust/cpp は提出可能。

## 作業ログ

- 2026-05-31: 作業開始。spec-update で Session enum / リフレッシュ契機を確定。
- 2026-05-31: Session enum 化 + serde (Step 1-2)、LibraryChecker OnlineJudge 実装 (Step 3)、
  registry 登録 (Step 4)、no-echo パスワード (Step 5) を実装。実 API 調査でサンプル取得経路を
  「公開バケットの例ファイル」に確定 (問題ページ SPA のため)。`ce init` を実 LC で e2e 確認。
  全 257 テスト通過・clippy/fmt クリーン。login/submit は要実アカウント手動確認。
- 2026-05-31: ユーザーフィードバックを受け 3 点追加対応 (contest_id 名前空間化 / task.md からの入力
  フォーマット抽出 / lang_id 自動解決)。`ce init` を実 LC で再確認 (prefix・入力フォーマット抽出が機能)。
- 2026-05-31: 実アカウントで `ce login` / `ce sub` を確認済み (入力コード生成・提出 OK)。PR #36 の Copilot
  レビュー対応: パスワード trim バグ修正、lang_id 解決テスト追加、example 番号の 2 桁ゼロ埋め (idx>=10 対応)、
  whoami のエラーを 401/403 時のみ「session expired」に限定、docs/task を実装に同期。
