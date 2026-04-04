# compro-env

競プロ CLI ツール `ce` の開発リポジトリ。

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
2. **タスク生成・実装開始**: `/spec-do <command>` — タスクファイル生成 → 実装
3. **レビュー・同期**: `/spec-review <command>` — 実装と仕様の整合を確認・修正

タスクファイル: `tasks/todo/` → `tasks/doing/` → `tasks/done/`

## コーディングルール

- コマンドの出力やコメントは全て英語
- 内側のレイヤーは外側を知ってはいけない (`usecases` が `infrastructure` を import しない等)
- `todo!()` で骨格を先に作り、後から実装を埋める

## 参考リポジトリ

- `ref-repos/ac` → `/home/kise/repos/ac` (Python 製旧ツール、機能の参考)
- `ref-repos/library` → `/home/kise/repos/library` (C++ 製旧ライブラリ、将来 Rust 製ライブラリを本ツールに作成統合予定)
