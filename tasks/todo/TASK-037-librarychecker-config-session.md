# TASK-037: LibraryChecker の config / session 対応 (Phase E)

LibraryChecker 用の config (lang_id) と session (Firebase token) を扱えるようにする。

## 参照仕様

- docs/spec.md (コンフィグ設計 / セッション)
- docs/online_judges/librarychecker.md

## 実装チェックリスト

### usecases/ (config trait)

- [ ] `Config::lang_id(lang, oj)` が LC でも機能する (キー `[language.{lang}.librarychecker].lang_id`)
  - LC の lang_id は `GET /langs` の `id` (例: `cpp`, `rust` 等)
- [ ] submit_file 等が LC でも妥当か確認

### infrastructure/

- [ ] `config_impl.rs`: `[language.{lang}.librarychecker]` セクション読み取りを確認 (oj.as_str() ベースで既に汎用なら追加不要)
- [ ] `session_repository_impl.rs`: LC セッションの保存形式を決める
  - Firebase の idToken + refreshToken を保存 (cookie 単一文字列の Session で足りるか、構造拡張が要るか検討)
  - session.toml の `[librarychecker]` セクション
- [ ] `Session` の `cookie: String` 一本で表現するか、token/refresh を持てるよう拡張するか決定し反映

## 完了条件

- [ ] LC の lang_id が config から解決できる
- [ ] LC セッション (token) が保存・読み出しでき whoami/submit で使える
- [ ] `cargo test --all && cargo clippy --all --all-features -- -D warnings && cargo fmt --all --check` 通過

## 作業ログ

- (未着手)
