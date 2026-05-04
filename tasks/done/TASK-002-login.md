# TASK-002: ce login 実装

## 参照仕様

- docs/commands/login.md

## 実装チェックリスト

### infrastructure/

- [x] `repository_impl/session_repository_impl.rs`: `save()` — Session を `~/.config/ce/session.toml` に TOML 形式で書き込む
- [x] `repository_impl/session_repository_impl.rs`: `get()` — `~/.config/ce/session.toml` から Session を読み込む (ファイルなし → None)
- [x] `config_impl.rs`: `default_online_judge()` — MVP では `OJKind::AtCoder` を返す
- [x] `shell/mod.rs`: `Commands::Login` アーム — OJ 解決・手順表示・stdin 入力・controller.login() 呼び出し・完了メッセージ

## 完了条件

- [x] `cargo test --workspace` が全通過
- [x] `cargo fmt --all` と `cargo clippy --workspace --all-features` が警告なし (既存の dead_code は他コマンドの todo!() 由来)
- [ ] `ce login` を実行すると `~/.config/ce/session.toml` に保存される (手動確認)

## 作業ログ

- 2026-04-07: 作業開始
- 2026-04-07: 全チェックリスト項目完了、cargo test 6/6 パス
