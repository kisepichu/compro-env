# TASK-035: OJKind に LibraryChecker を追加 (Phase C)

`OJKind` enum に `LibraryChecker` variant を追加し、各 match サイトを網羅修正する。
このタスクはコンパイラの非網羅 match エラーに従って機械的に潰す作業が中心。

## 参照仕様

- docs/spec.md (ドメインモデル > OJKind) ← spec-update で更新予定

## 実装チェックリスト

### domain/

- [ ] `OJKind` に `LibraryChecker` variant 追加
- [ ] `OJKind::as_str` に対応文字列追加 (案: `"librarychecker"`、config/session キーに使用)
- [ ] `OJKind::FromStr` に対応追加 (受理する別名があれば検討: `lc` / `yosupo` 等)
- [ ] LC の判定 descriptor を追加 (`judge.yosupo.jp/problem/{name}` URL → `(LibraryChecker, name)`)
  - Phase B で入れた descriptor 機構に 1 件足すだけ。プレフィックスは持たない (URL のみ)
- [ ] `OJKind::detect` の単体テスト追加: LC 問題 URL → `(LibraryChecker, "aplusb")`

### infrastructure/

- [ ] `shell/mod.rs` の `oj_display` match (`OJKind::AtCoder => "AtCoder"`) に `LibraryChecker => "Library Checker"` を追加
- [ ] その他 OJKind を match する箇所をコンパイラ警告に従って網羅対応
- [ ] config_impl / session_repository_impl で OJKind 文字列キーが LC でも機能することを確認

## 完了条件

- [ ] `OJKind::LibraryChecker` を含め全 match が網羅される (ビルド通過)
- [ ] `as_str` / `FromStr` のラウンドトリップが LC でも成立
- [ ] `cargo test --all && cargo clippy --all --all-features -- -D warnings && cargo fmt --all --check` 通過

## 作業ログ

- (未着手)
