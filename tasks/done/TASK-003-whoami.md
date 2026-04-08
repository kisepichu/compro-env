# TASK-003: ce whoami 実装

## 参照仕様

- docs/commands/whoami.md

## 実装チェックリスト

### infrastructure/

- [x] `online_judge_impl/atcoder.rs`: `AtCoder::whoami()` — GET https://atcoder.jp/home を REVEL_SESSION 付きで送り、HTML からユーザー名を抽出する。抽出失敗時は `Err("session expired. Run \`ce login\` again.")`
- [x] `shell/mod.rs`: `Commands::Whoami` アーム — OJ 解決・service 構築 (project root 不要)・`controller.whoami()` 呼び出し・`CeError::SessionNotFound` は `(not logged in)` + ログイン促しメッセージで exit 0・その他エラーは exit 1

## 完了条件

- [x] `cargo test --workspace` が全通過
- [x] `cargo fmt --all` と `cargo clippy --workspace --all-features` が警告なし (既存の dead_code は他コマンドの todo!() 由来)
- [ ] `cargo run -- whoami` で実際にユーザー名が表示される (手動確認)

## 作業ログ

- 2026-04-07: 作業開始
- 2026-04-07: 全チェックリスト項目完了、cargo test 9/9 パス
