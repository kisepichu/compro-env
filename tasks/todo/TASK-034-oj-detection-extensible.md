# TASK-034: OJ 判定ロジックの拡張点化 (Phase B)

contest_id / URL から OJ を判定するロジックを AtCoder 決め打ちから、
複数 OJ を登録できる拡張可能な形へ変更する。LibraryChecker は命名規則を持たないため
URL / 明示指定 / プロンプトで判定する。

## 参照仕様

- docs/spec.md (OJ 判定ロジック) ← spec-update で更新予定
- docs/online_judges/librarychecker.md ← spec-update で作成予定

## 背景 (現状の問題)

- `infrastructure/src/shell/mod.rs` `parse_contest_input` が
  `https://atcoder.jp/contests/` と `abc/arc/agc/ahc` プレフィックスを決め打ち。
- `domain/src/entity.rs` `OJKind::from_contest_id_prefix` も AtCoder プレフィックスのみ。

## 実装チェックリスト

### domain/

- [ ] OJ 判定を OJ ごとに拡張できる形にする
  - 案: 各 OJ が「URL ホスト/パターン」「contest_id プレフィックス」を申告し、判定器が走査
  - LibraryChecker: `judge.yosupo.jp/problem/{name}` URL を解釈、プレフィックスは持たない

### infrastructure/

- [ ] `parse_contest_input` を OJ 拡張テーブル/レジストリ経由の判定へ書き換える
  - AtCoder URL/プレフィックス判定は維持
  - LC URL (`https://judge.yosupo.jp/problem/{name}`) → (LibraryChecker, problem 名)
- [ ] 判定不能時の挙動を維持/拡張する (現状: stdin で OJ 名プロンプト)
  - `--oj` 明示指定の受け口があれば検討 (未決: init のシグネチャ変更要否)

## 完了条件

- [ ] AtCoder の URL / プレフィックス判定が回帰しない
- [ ] LC の問題 URL から (LibraryChecker, 問題名) が得られる
- [ ] `cargo test --all && cargo clippy --all --all-features -- -D warnings && cargo fmt --all --check` 通過

## 作業ログ

- (未着手)
