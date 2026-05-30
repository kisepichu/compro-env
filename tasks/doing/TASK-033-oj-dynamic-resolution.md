# TASK-033: OnlineJudge の動的解決と login/submit 一般化 (Phase A)

複数 OJ を扱えるよう、OnlineJudge 実装を OJKind から動的解決する仕組みを導入し、
trait を「OJ ごとに異なるログイン方式」「直接提出して提出 id/URL を返す提出方式」に対応できる形へ一般化する。
これは LibraryChecker 追加の前提となるコア・リファクタ。

## 参照仕様

- docs/spec.md (OnlineJudge インターフェース / アーキテクチャ層構成)
- docs/online_judges/README.md (OJ 抽象の責務・一般化方針)

## 背景 (現状の問題)

- `infrastructure/src/shell/mod.rs` の `build_controller` / `build_controller_no_root` が
  `Box::new(AtCoder::new()?)` を固定注入し、`Service` は OnlineJudge を 1 個しか持たない。
- `usecases/src/service/submit.rs` は `.ce.toml` から `oj_kind` を読むのに、提出は
  `self.online_judge` (固定の AtCoder) を使う → LC コンテストで誤った OJ に提出してしまう。
- `OnlineJudge::build_submit_url` はブラウザ + AtCoder userscript 前提。直接提出 (LC の REST) を表現できない。
- ログインは AtCoder の手動 cookie 方式のみ想定 (trait に login がない)。

## 確定した設計 (2026-05-30)

- 動的解決: usecases に `OnlineJudgeRegistry` trait を定義し `Service` が `Box<dyn>` で保持。未対応 OJ は `Err`。
- ログイン一般化: `CredentialKind { Cookie, EmailPassword }`、`Credentials { Cookie(String), Password{identifier,password} }`、`OnlineJudge::credential_kind()` + `login(&Credentials) -> Session`。
- 提出一般化: `SubmitOutcome { OpenBrowser{url}, Submitted{submission_url} }`、`submit(...) -> SubmitOutcome` が `build_submit_url` を置換。URL フラグメント長ガードは atcoder.rs へ移す。
- Session は本タスクでは `cookie: String` 据え置き (LC トークン表現は TASK-037)。
- 各段階でビルド緑・clippy 警告なしを保つため A1→A2→A3 の順で増分実装する。

## 実装チェックリスト

### A1: registry 導入 (trait 署名は変えない) ✅

- [x] usecases に `OnlineJudgeRegistry` trait 追加 (`get(&self, oj: &OJKind) -> Result<&dyn OnlineJudge>`) + `SingleOnlineJudge` ヘルパ
- [x] `Service` を `Box<dyn OnlineJudge>` 保持から `Box<dyn OnlineJudgeRegistry>` 保持へ変更 + `Service::online_judge(&oj)` 解決ヘルパ
- [x] 各サービス (whoami/init/submit) が `OJKind` で OJ を解決して使う
  - submit は `.ce.toml` の `get_oj_kind` 結果で解決 (固定注入バグ解消)。新テスト `submit_resolves_online_judge_from_contest_oj_kind` 追加
- [x] サービス側テストスタブを `SingleOnlineJudge` でラップする形に更新 (submit/test/new_solution/init)
- [x] infrastructure に registry 実装 `OnlineJudgeRegistryImpl` (AtCoder を登録)、`shell/mod.rs` の `build_controller*` を registry 経由へ
- [x] AtCoder 挙動不変・既存テスト緑を確認 (test 115 / clippy 警告なし / fmt クリーン)

### A2: login 一般化

- [ ] usecases に `CredentialKind` / `Credentials` 追加、`OnlineJudge` に `credential_kind()` + `login(&Credentials) -> Result<Session>`
- [ ] `service/login.rs`: cookie 直保存をやめ `oj.login(creds)` → 保存
- [ ] interfaces `LoginInput`: cookie 単体から `Credentials` 供給へ一般化
- [ ] infrastructure: AtCoder `credential_kind=Cookie` / `login` は Session を包むだけ (ネットワーク不要)
- [ ] `shell/mod.rs` login: `credential_kind` に応じて入力を出し分け (現状 cookie プロンプトは維持)

### A3: submit 一般化

- [ ] usecases に `SubmitOutcome` 追加、`OnlineJudge::submit(...) -> SubmitOutcome` で `build_submit_url` 置換
- [ ] `service/submit.rs`: `oj.submit()` を呼び `SubmitOutcome` を返す。URL フラグメント長ガードは AtCoder 実装へ移動
- [ ] interfaces/Controller: `SubmitResult` を `SubmitOutcome` に合わせて調整
- [ ] infrastructure: AtCoder `submit` は `OpenBrowser{url}` を返す (URL 構築 + サイズガードを内包)
- [ ] `shell/mod.rs` submit: `SubmitOutcome` で「開く/提出URL表示」を出し分け (AtCoder は従来通り URL 表示 + ブラウザ起動)

## 完了条件

- [ ] `.ce.toml` の OJ に応じて提出先 OJ が切り替わる (AtCoder コンテストは従来通り動作)
- [ ] 既存コマンド (login/whoami/logout/init/test/submit on AtCoder) の挙動が回帰しない
- [ ] `cargo test --all && cargo clippy --all --all-features -- -D warnings && cargo fmt --all --check` 通過

## 作業ログ

- 2026-05-30: 作業開始。設計を A1/A2/A3 増分に整理。
- 2026-05-30: A1 完了 (registry 導入)。OnlineJudgeRegistry/SingleOnlineJudge 追加、Service が OJKind で解決、submit が .ce.toml の OJ を使用。test 115 / clippy / fmt 緑。
