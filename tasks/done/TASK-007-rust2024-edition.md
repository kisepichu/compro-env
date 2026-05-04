# TASK-007: Rust 2024 edition への移行 (issue #5)

## 参照

- [GitHub issue #5](https://github.com/kisepichu/compro-env/issues/5)

## 背景

全クレートが `edition = "2021"` を使用している。Rust 1.85 (2025年2月) で Rust 2024 edition が安定化された。
コードベースのスキャンにより、破壊的変更を要するパターン（`unsafe`、`static mut`、FFI 等）は存在しないことが確認済み。

## 実装チェックリスト

- [x] `cargo fix --edition` を実行し、変更差分を確認する
  - `std::env::set_var` / `remove_var` を `unsafe {}` でラップ (3 ファイル × 各箇所)
  - drop order 警告 (`init.rs:74`) は `anyhow::Error` のメモリ解放のみで動作影響なし
- [x] ワークスペースルート + 4 クレートの `Cargo.toml` で `edition = "2024"` に更新
  - `Cargo.toml` (workspace root)
  - `crates/domain/Cargo.toml`
  - `crates/usecases/Cargo.toml`
  - `crates/interfaces/Cargo.toml`
  - `crates/infrastructure/Cargo.toml`
- [x] `cargo test --workspace` で全テストがパスすることを確認 (71 tests passed)
- [x] `rust-toolchain.toml` — 現行 rustc 1.92.0 >= 1.85 のためスキップ


## 完了条件

- [x] `cargo fmt --all` / `cargo clippy --workspace --all-features` / `cargo test --workspace` がすべてクリア
- [x] 全 `Cargo.toml` が `edition = "2024"` になっている
- [x] 意図しないコード変更がないこと (差分を確認)

## 作業ログ

- 2026-04-14: タスクファイル作成
- 2026-04-14: `mod.rs` の廃止は今回スコープ外と確定 (issue #5 コメントにて確認)
- 2026-04-14: 実装完了
