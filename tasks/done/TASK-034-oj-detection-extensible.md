# TASK-034: OJ 判定ロジックの拡張点化 (Phase B)

contest_id / URL から OJ を判定するロジックを AtCoder 決め打ちから、各 OJ が descriptor を
申告して domain の純粋関数 `OJKind::detect` が走査する拡張可能な形へ変更する。

**スコープ確定 (spec-update 2026-05-31)**: 本フェーズは既存 AtCoder 判定を descriptor 機構へ
移すリファクタに留め、**挙動は不変**とする。LibraryChecker の URL descriptor は variant が入る
TASK-035 (Phase C) で追加する (B では LibraryChecker variant をまだ持たないため)。
`--oj` 明示フラグは追加しない (判定不能時は従来どおり stdin プロンプト)。

## 参照仕様

- docs/spec.md (OJ 判定ロジック)
- docs/online_judges/README.md (OJ 判定 (init 時))

## 背景 (現状の問題)

- `infrastructure/src/shell/mod.rs` `parse_contest_input` が
  `https://atcoder.jp/contests/` と `abc/arc/agc/ahc` プレフィックスを決め打ち。
- `domain/src/entity.rs` `OJKind::from_contest_id_prefix` も AtCoder プレフィックスのみ。

## 実装チェックリスト

### domain/

- [x] descriptor 型を定義する (`OjDescriptor { kind, url_host, url_path_prefix, id_prefixes }`)
  - 各 OJ が「URL ホスト + パスパターン」「contest_id プレフィックス」を申告する
- [x] `OJKind::detect(input) -> Option<(OJKind, String)>` を追加し descriptor を走査
  - URL マッチ → ホスト/パスから contest_id を抽出 (先頭セグメントのみ・lowercase・空は None)
  - プレフィックスマッチ → 入力そのものを (lowercase して) contest_id に
  - 既存 `from_contest_id_prefix` はこの仕組みへ吸収し削除 (AtCoder の abc/arc/agc/ahc を維持)
- [x] `is_safe_path_component` 検証は infra に残す (detect は純粋に保つ)

### infrastructure/

- [x] `parse_contest_input` を `OJKind::detect` への委譲 + `is_safe_path_component` 検証に書き換え
  - AtCoder URL (`atcoder.jp/contests/{id}`) / プレフィックス判定は挙動不変
- [x] 判定不能時の挙動を維持する (stdin で OJ 名プロンプト)。`--oj` フラグは追加しない

## 完了条件

- [x] AtCoder の URL / プレフィックス判定が回帰しない (既存テストが緑のまま)
- [x] `OJKind::detect` の単体テストがある (AtCoder URL / プレフィックス / 不明入力 → None)。domain に 10 件追加
- [x] descriptor 追加だけで新 OJ の判定が組み込める構造になっている (LC は Phase C で実証)
- [x] `cargo test --all && cargo clippy --all --all-features -- -D warnings && cargo fmt --all --check` 通過

## 作業ログ

- 2026-05-31: spec-update でスコープ確定 (リファクタのみ・挙動不変)。doing へ移動し実装開始。
- 2026-05-31: TDD で domain に `OjDescriptor` + `OJKind::detect` を追加 (detect テスト 10 件)、
  `from_contest_id_prefix` を削除、infra `parse_contest_input` を委譲へ書き換え。
  全チェック通過 (test/clippy -D warnings/fmt)。done へ移動。
