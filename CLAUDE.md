# compro-env

競プロ CLI ツール `ce` の開発リポジトリ。

### TDD(テスト駆動開発)を厳守

obra/superpowers の仕組みを使用する。
実装する際は、必ず以下の順序で進める:

1. テストを書く agent を起動し、テストが失敗することを確認する
2. テストを通す最小限のコードを実装する agent を起動する
3. テストが通る状態を維持しつつリファクタリングする

#### 絶対に守るルール

- テストを書く前に実装コードを書いてはならない
- テストが失敗することを確認してから実装に進む
- 既存のテストを無断で変更してはならない(テスト追加は可)
- 実装後は必ずすべてのテストを実行して確認する

## プロジェクト概要

- **ツール名**: `compro-env` / コマンド: `ce`
- **実装言語**: Rust
- **目的**: AtCoder 等のコンテスト管理、問題ディレクトリ作成、テスト、提出を行う CLI。将来的にライブラリ機能を統合。

詳細仕様: `docs/spec.md`
コマンドごとの詳細: `docs/commands/`
開発計画: `docs/PLAN.md`

## アーキテクチャ

4 層 DDD (Clean Architecture):

```
domain/         エンティティ・値オブジェクト (Contest, Problem, Solution, Sample, Session)
usecases/       ポート (OnlineJudge, ContestRepository, SolutionRepository traits) + サービス
interfaces/     Controller, Input traits (clap コマンドが実装)
infrastructure/ 実装 (AtCoder HTTP, filesystem, clap エントリポイント)
```

**依存方向**: infrastructure → interfaces → usecases → domain (domain は誰にも依存しない)

**エラー設計**: `anyhow` + `thiserror`。`E: Error + 'static` 型パラメータは使わない。

## ディレクトリ構造 (solutions/)

```
solutions/{contest_id}/{lang}/{problem_code}/{solution_name}/
solutions/{contest_id}/testcases/{problem_code}/  ← 言語共通
templates/{lang}/                                  ← ce init 用テンプレート
```

## 開発ワークフロー

1. **仕様確認・更新**: `/spec-update <command>` — 対象コマンドの仕様を議論・更新
2. **タスク生成・実装開始**: `/spec-do <command>` — タスクファイル生成 → テスト作成 → 実装
3. **レビュー・同期**: `/spec-review <command>` — 実装と仕様の整合を確認・修正

タスクファイル: `tasks/todo/` → `tasks/doing/` → `tasks/done/`

## コーディングルール

- コマンドの出力やコメントは全て英語
- 内側のレイヤーは外側を知ってはいけない (`usecases` が `infrastructure` を import しない等)
- `todo!()` で骨格を先に作り、後から実装を埋める

## 参考リポジトリ

- `ref-repos/ac` → `/home/kise/repos/ac` (Python 製旧ツール、機能の参考)
- `ref-repos/library` → `/home/kise/repos/library` (C++ 製旧ライブラリ、将来 Rust 製ライブラリを本ツールに作成統合予定)
