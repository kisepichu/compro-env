# TASK-036: LibraryChecker OnlineJudge 実装 (Phase D)

LibraryChecker の `OnlineJudge` 実装を追加する。REST API (`https://v3.api.judge.yosupo.jp`)
と Firebase Auth を用い、問題取得・サンプル取得・ログイン・提出を実装する。

## 参照仕様

- docs/online_judges/librarychecker.md ← spec-update で作成予定
- docs/spec.md (OnlineJudge インターフェース)

## 実 API メモ (調査済み)

- REST ベース: `https://v3.api.judge.yosupo.jp`
- 認証: Firebase Auth (Bearer JWT)。`signInWithPassword?key=<API_KEY>` に email+password → idToken。
  - 公開 API key: `AIzaSyCmpkoMVbKRDm2H0MJHB0iZ43uQtSqiLV0`
  - idToken は短命(~1h)、refreshToken で更新
- `GET /problems/{name}` → title, time_limit 等 (samples 含まず)
- `GET /langs` → `[{id,name,version}]` (lang_id に対応)
- `POST /submit` body `{problem, source, lang}` + Bearer → `{id}`。表示 URL `https://judge.yosupo.jp/submission/{id}`
- サンプルは **問題ページの例のみ** をスクレイプ取得する方針 (GCS 公式テストは使わない。容量大回避)

## 実装チェックリスト

### domain/ (変更なし想定。必要なら Problem 等の解釈調整)

### infrastructure/

- [ ] `online_judge_impl/librarychecker.rs` を新規作成し OnlineJudge を実装
- [ ] `get_contest_meta` 相当: LC はコンテスト概念なし → start_time=None、hints 空
- [ ] `get_problems_detail` 相当: `GET /problems/{name}` + 問題ページからサンプル抽出し Problem 1 件を返す
  - contest_id=問題名、problems=[1件] (問題=単問コンテスト)
  - input_format_raw / constraints_raw は取得できれば設定 (LC 形式に注意、無理なら空)
- [ ] サンプル取得: 問題ページの Sample セクションをスクレイプ (軽量・例のみ)
- [ ] login: Firebase `signInWithPassword` に email+password → idToken/refreshToken を Session に保存
  - [ ] shell の EmailPassword パスワード入力を no-echo (例: `rpassword`) に切り替える (現状エコーあり)
- [ ] whoami: `GET /auth/current_user` (Bearer) → ユーザー名
- [ ] submit: `POST /submit` (Bearer) → 提出 id、表示 URL を返す (直接提出。Phase A の提出抽象に従う)
- [ ] registry に LibraryChecker を登録 (Phase A の仕組み)

## 完了条件

- [ ] `ce init <LC問題URL>` で単問コンテストとして問題・サンプルが取得できる
- [ ] `ce login librarychecker` で email+password ログインしトークン保存
- [ ] `ce sub` で実際に提出され提出 URL が表示される (手動確認)
- [ ] `cargo test --all && cargo clippy --all --all-features -- -D warnings && cargo fmt --all --check` 通過

## 未決事項

- 問題ページのサンプル HTML 構造 (実装時に実ページで確認)
- input_format_raw を LC で意味ある形で取れるか (取れなければ空でフォールバック)
- idToken 失効時のリフレッシュをどのコマンドで行うか

## 作業ログ

- (未着手)
