# TASK-033: OnlineJudge の動的解決と login/submit 一般化 (Phase A)

複数 OJ を扱えるよう、OnlineJudge 実装を OJKind から動的解決する仕組みを導入し、
trait を「OJ ごとに異なるログイン方式」「直接提出して提出 id/URL を返す提出方式」に対応できる形へ一般化する。
これは LibraryChecker 追加の前提となるコア・リファクタ。

## 参照仕様

- docs/spec.md (OnlineJudge インターフェース / アーキテクチャ層構成) ← spec-update で更新予定
- docs/online_judges/README.md (OJ 抽象の責務) ← spec-update で作成予定

## 背景 (現状の問題)

- `infrastructure/src/shell/mod.rs` の `build_controller` / `build_controller_no_root` が
  `Box::new(AtCoder::new()?)` を固定注入し、`Service` は OnlineJudge を 1 個しか持たない。
- `usecases/src/service/submit.rs` は `.ce.toml` から `oj_kind` を読むのに、提出は
  `self.online_judge` (固定の AtCoder) を使う → LC コンテストで誤った OJ に提出してしまう。
- `OnlineJudge::build_submit_url` はブラウザ + AtCoder userscript 前提。直接提出 (LC の REST) を表現できない。
- ログインは AtCoder の手動 cookie 方式のみ想定 (trait に login がない)。

## 実装チェックリスト

### usecases/

- [ ] OnlineJudge 解決ポートを導入する (案: `OnlineJudgeRegistry` trait or `fn online_judge(&OJKind) -> &dyn OnlineJudge`)
  - `Service` が単一 `online_judge` を持つのをやめ、OJKind から解決する形にする
- [ ] `submit` / `init` / `whoami` 等が対象 OJ を OJKind から解決して使うよう修正する
  - 特に `submit` は `.ce.toml` の `get_oj_kind` 結果で OJ を選ぶ
- [ ] 提出の抽象を一般化する (`build_submit_url` 一本化をやめる)
  - 案: `submit(...) -> SubmitOutcome` で「ブラウザで開く URL」または「直接提出済みの提出 id/URL」を表現
  - AtCoder は従来通りブラウザ URL、LC は直接提出を返す
- [ ] ログイン能力を trait に追加 (OJ ごとに方式が異なる)
  - AtCoder: 手動 cookie (既存維持)、LC: email+password → token
  - 案: `login(credentials) -> Session` を OnlineJudge に追加、または capability を分離
- [ ] 既存サービスのテストスタブ (StubOJ 等) を新 trait 形状に追従させる

### interfaces/

- [ ] Controller / Input trait に提出方式・ログイン方式の差を吸収する変更があれば反映

### infrastructure/

- [ ] `shell/mod.rs` の `build_controller*` を registry/factory ベースに書き換える
  - 既存 OJ (AtCoder) を registry に登録
- [ ] AtCoder 実装を新 trait 形状に追従させる (挙動は不変)

## 完了条件

- [ ] `.ce.toml` の OJ に応じて提出先 OJ が切り替わる (AtCoder コンテストは従来通り動作)
- [ ] 既存コマンド (login/whoami/logout/init/test/submit on AtCoder) の挙動が回帰しない
- [ ] `cargo test --all && cargo clippy --all --all-features -- -D warnings && cargo fmt --all --check` 通過

## 作業ログ

- (未着手)
