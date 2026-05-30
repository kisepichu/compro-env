# TASK-035: OJKind に LibraryChecker を追加 (Phase C)

`OJKind` enum に `LibraryChecker` variant を追加し、各 match サイトを網羅修正する。
このタスクはコンパイラの非網羅 match エラーに従って機械的に潰す作業が中心。

## 参照仕様

- docs/spec.md (ドメインモデル > OJKind)
- docs/online_judges/librarychecker.md (実装フェーズと中間状態)

## 実装チェックリスト

### domain/

- [x] `OJKind` に `LibraryChecker` variant 追加
- [x] `OJKind::as_str` に対応文字列追加 (`"librarychecker"`、config/session キーに使用)
- [x] `OJKind::FromStr` に対応追加 (別名 `lc`/`yosupo` は **追加しない**。`as_str` と
  ラウンドトリップする正準キーのみ受理し最小に保つ)
- [x] LC の判定 descriptor を追加 (`judge.yosupo.jp/problem/{name}` URL → `(LibraryChecker, name)`)
  - Phase B で入れた descriptor 機構に 1 件足すだけ。プレフィックスは持たない (`id_prefixes: &[]`)
- [x] `OJKind::detect` の単体テスト追加: LC 問題 URL → `(LibraryChecker, "aplusb")`
  (trailing slash / 追加セグメント / 空 id / プレフィックス非検出 も網羅)

### infrastructure/

- [x] `shell/mod.rs` の `oj_display` match に `LibraryChecker => "Library Checker"` を追加
- [x] `online_judge_impl/registry.rs` `get()`: `LibraryChecker` arm は clean な anyhow エラー
  (`bail!("LibraryChecker is not yet implemented (TASK-036)")`) を返す。`todo!()` で panic させない。
- [x] `shell/mod.rs` `credential_kind_for`: `LibraryChecker => CredentialKind::EmailPassword`
- [x] その他 OJKind を match する箇所をコンパイラ警告に従って網羅対応
  - `session_repository_impl.rs` の save/delete/get の 3 match: LC arm は clean エラー bail
    (セッション保存形式は Phase E / TASK-037 で確定するため Phase C では未実装)
- [x] config_impl / session_repository_impl で OJKind 文字列キーが LC でも機能することを確認
  - `config_impl::lang_id` は `oj.as_str()` でキーを引くため `[language.{lang}.librarychecker]` が
    自動で機能する (変更不要)

## 完了条件

- [x] `OJKind::LibraryChecker` を含め全 match が網羅される (ビルド通過)
- [x] `as_str` / `FromStr` のラウンドトリップが LC でも成立
- [x] `OJKind::detect("https://judge.yosupo.jp/problem/aplusb")` → `(LibraryChecker, "aplusb")`
- [x] `registry.get(&LibraryChecker)` が `Err` を返す (panic しない) ことをテストで確認
- [x] AtCoder の既存テスト・挙動は不変
- [x] `cargo test --all && cargo clippy --all --all-features -- -D warnings && cargo fmt --all --check` 通過

## 作業ログ

- 2026-05-31: 着手。TDD で domain のテスト (detect LC URL 群・as_str/FromStr ラウンドトリップ) を
  先に追加 → red 確認 → variant + descriptor + as_str/FromStr を実装し green。
- 2026-05-31: variant 追加で壊れた infra の 6 match を網羅対応。registry/session は clean エラー
  bail (Phase C 中間状態の決定どおり panic させない)、shell の credential_kind/oj_display は実値。
  registry の LC エラーパスにテスト追加。
- 2026-05-31: `cargo test --all` (全 pass) / `cargo clippy --all --all-features -- -D warnings`
  (警告ゼロ) / `cargo fmt --all --check` (差分なし) すべて通過。完了。
