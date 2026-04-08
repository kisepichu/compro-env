# TASK-004: ce logout 実装

## 参照仕様

- docs/commands/logout.md

## 実装チェックリスト

### usecases/

- [x] `repository/session_repository.rs`: `SessionRepository` trait に `delete(&OJKind) -> Result<bool>` を追加
- [x] `service/login.rs` と同じファイル構成で `service/logout.rs`: `Service::logout()` を実装 (`session_repo.delete()` を呼ぶだけ)

### infrastructure/

- [x] `repository_impl/session_repository_impl.rs`: `delete()` を実装 — session.toml の該当セクションを削除、ファイルが存在しなければ `Ok(false)`
- [x] `shell/commands.rs`: `Logout` サブコマンドと `LogoutCommand` + `LogoutInput` impl を追加
- [x] `shell/mod.rs`: `Commands::Logout` アームを実装 — `logout_with_io()` 経由で呼び出し

### interfaces/

- [x] `controller/input.rs`: `LogoutInput` trait を追加
- [x] `controller.rs`: `Controller::logout()` を追加

## 完了条件

- [x] `cargo test --workspace` が全通過
- [x] `cargo fmt --all` と `cargo clippy --workspace --all-features` が警告なし
- [ ] `cargo run -- logout` でセッションが削除される (手動確認)

## 作業ログ

- 2026-04-08: 作業開始
- 2026-04-08: 全チェックリスト項目完了、cargo test 13/13 パス
