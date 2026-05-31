# TASK-037: LibraryChecker の config / session 対応 (Phase E)

LibraryChecker 用の config (lang_id) と session (Firebase token) を扱えるようにする。

## 参照仕様

- docs/spec.md (コンフィグ設計 / セッション)
- docs/online_judges/librarychecker.md

## 実装チェックリスト

### usecases/ (config trait)

- [x] `Config::lang_id(lang, oj)` が LC でも機能する (キー `[language.{lang}.librarychecker].lang_id`)
  - LC の lang_id は `GET /langs` の `id` (例: `cpp`, `rust` 等)。Phase D で汎用実装済み
  - 解決順: `config.lang_id(lang, oj)` → 無ければ `OnlineJudge::default_lang_id(lang)` (LC=言語名) → エラー
- [x] submit_file 等が LC でも妥当か確認 (OJ 非依存のため変更不要)

### infrastructure/

- [x] `config_impl.rs`: `[language.{lang}.librarychecker]` セクション読み取りを確認 (`oj.as_str()` ベースで既に汎用。Phase E で LC 明示テストを追加)
- [x] `session_repository_impl.rs`: LC セッションの保存形式を決定 (Phase D で実装済み)
  - Firebase の idToken + refreshToken を `[librarychecker]` セクションに保存。round-trip テストあり
  - session.toml の `[librarychecker]` セクション
- [x] `Session` を enum 化 (`Cookie` / `Firebase`) して token/refresh を保持 (Phase D で実装済み)

## 完了条件

- [x] LC の lang_id が config から解決できる
- [x] LC セッション (token) が保存・読み出しでき whoami/submit で使える
- [x] `cargo test --all && cargo clippy --all --all-features -- -D warnings && cargo fmt --all --check` 通過

## 作業ログ

- Phase E の実装 (Session enum・session.toml `[librarychecker]` 保存/読出・`config.lang_id` の
  OJ 別解決・LC `default_lang_id`・submit の解決順) は **Phase D (TASK-036) で先取り実装された**。
  Session enum 化は login の生成物 = submit/whoami の消費物のため D に含める必要があり、あわせて
  lang_id 解決も実装されていた。
- Phase E はクローズ作業として実施:
  - 仕様ドリフト解消: `docs/spec.md` の Session ドメインモデル・session.toml 例・実装状況ノート、
    `docs/online_judges/README.md`・`librarychecker.md` を実装に同期。
  - `config_impl.rs` に LC 固有の lang_id 解決テストを 2 件追加
    (`lang_id_returns_librarychecker_value` / `lang_id_is_scoped_per_oj`)。
  - フル検証 (test 268 件 / clippy / fmt) 通過。
