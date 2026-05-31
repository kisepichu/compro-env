# TASK-038: ce submit 提出前 preprocess フック

## 参照仕様

- `docs/commands/submit.md` の「提出前 preprocess フック」節 (実行契約・環境変数・config キー・エラーケース)
- `docs/spec.md` コンフィグ設計 `[submit].preprocess` / `service/submit.rs` 責務
- 既存パターン: `crates/usecases/src/service/test.rs` (`sh -c` + env 実行)

## 概要

`ce submit` が提出ソースを `OnlineJudge::submit` に渡す前に、ユーザー指定スクリプトを
`sh -c` で実行し、stdout を提出ソースに差し替える。整形・ライブラリ展開・提出レイアウトを
すべてユーザースクリプトに委ねる拡張点。アプリには言語別・OJ 別ロジックを持たない。

**初期ユースケース (アプリ仕様ではない / 受け入れデモ)**: Rust のライブラリ展開。
ユーザーが `cargo-equip` 等を呼ぶ preprocess スクリプトを書き、ローカル自作ライブラリ import を
1 ファイルに展開して提出する流れを手動で確認する。アプリ側はこのツールを一切知らない。

## 設計メモ

- 実行位置: `service/submit.rs` の手順5 (lang_id 解決) の後、手順7 (`oj.submit`) の前。
  `CE_LANG_ID` を env で渡すため lang_id 解決後である必要がある。
- 実行方式: `sh -c <command>`、`current_dir = solution_dir`、stdin に元ソースを書き込み、
  stdout を採取。exit≠0 で `bail!`(stderr はそのまま端末へ流す)。`#[cfg(unix)]` 限定で、
  非 Unix では設定があってもスキップし元ソースを提出 (test.rs と同じ方針)。
- env: `CE_LANGUAGE` `CE_OJ`(OJKind の表示文字列) `CE_CONTEST_ID` `CE_PROBLEM_CODE`
  `CE_PROBLEM_ID`(problem.id) `CE_SOLUTION_NAME` `CE_SOLUTION_DIR`(絶対)
  `CE_SOURCE_FILE`(solution_dir + solution_file) `CE_LANG_ID`(解決済み)。
- `~` を含むパスは `sh -c` がチルダ展開するので手動展開不要 (test_command と同じ)。
- trait 変更: `Config::submit_preprocess(&self) -> Option<String>` (`&Language` 引数を削除)。
  config キーは `[submit].preprocess` のみ。per-language キーは設けない。

## チェックリスト

### 1. Config trait / 実装

- [x] `crates/usecases/src/config.rs`: `submit_preprocess(&self) -> Option<String>` にシグネチャ変更
- [x] (test-first) `config_impl.rs` テスト: `[submit].preprocess` 設定時に値を返す / 未設定で `None` /
      config.toml 無しで `None` の3ケース
- [x] `crates/infrastructure/src/config_impl.rs`: `[submit].preprocess` を読み `Option<String>` を返す実装
      (読み込み/パース失敗は既存方針どおり warning + `None` にフォールバック)

### 2. submit サービス本体 (test-first)

- [x] (test-first) preprocess 設定時、フックの stdout が `oj.submit` に渡るソースになる
      (`SourceCapturingOJ` スタブを追加)
- [x] (test-first) preprocess 未設定 (`None`) 時は元ソースがそのまま渡る (後方互換)
- [x] (test-first) フックが exit≠0 → submit を中止し `oj.submit` は呼ばれない (受信ソースが None で検証)
- [x] (test-first) env (`CE_LANGUAGE` `CE_OJ` `CE_LANG_ID` 等9種) がフックに渡る
      (env を検査し `cat` で stdin を通すシェルスクリプトで確認)
- [x] `crates/usecases/src/service/submit.rs`: lang_id 解決後に preprocess を実行する処理を追加
      (`#[cfg(unix)]`、`run_preprocess_hook` で stdin 供給・stdout 採取・stderr inherit・exit≠0 で bail)

### 3. 既存スタブ追従

- [x] `service/submit.rs` テストの `StubConfig` を新シグネチャに更新 (+ `submit_preprocess` フィールド)
- [x] `service/test.rs` テストの `StubConfig` を更新
- [x] `service/init.rs` テストの `StubConfig` を更新
- [x] `service/new_solution.rs` テストの `StubConfig` を更新

### 4. 検証

- [x] `cargo test --all` — パス (新規: config_impl 3, submit preprocess 4, dry-run 1)
- [x] `cargo clippy --all --all-features -- -D warnings` — 警告なし
- [x] `cargo fmt --all --check` — クリーン

## 完了条件

- [x] `[submit].preprocess` 設定時、提出ソースがフックの stdout に差し替わる
- [x] フック exit≠0 で提出が中止され、提出 URL を生成しない
- [x] 未設定時は従来どおり元ソースを提出する (後方互換)
- [x] 9 種の env がフックに渡る
- [x] 全テスト・clippy・fmt がパス

## 作業ログ

- 2026-05-31: 仕様確定 (docs/commands/submit.md「提出前 preprocess フック」節)・タスク生成。
- 2026-05-31: 実装完了。trait を `submit_preprocess(&self) -> Option<String>` に変更、config_impl で
  `[submit].preprocess` 読込、submit.rs に `run_preprocess_hook`(sh -c + 9 env + stdin/stdout、Unix 限定)を
  追加。全ゲート (test/clippy/fmt) パス。
- 2026-06-01: `ce submit --dry-run`(OJ 非送信でソース準備のみ表示)を追加。source 準備を
  `Service::prepare_submission` に集約。`hooks/submit-preprocess.sh`(cargo-equip 例)を追加し、
  `templates/rust/Cargo.toml.tera` を edition 2021 に変更(cargo-equip が 2024 非対応)。
  実機で aplusb の `--dry-run` が cargo-equip 展開済みソースを出力することを確認済み
  (judge 上での AC は未確認)。
