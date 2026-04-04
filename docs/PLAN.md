# PLAN

壁打ちしながら更新。未決事項は `docs/spec.md` の Q リストを参照。

## 開発フロー (予定)

1. **壁打ち・仕様まとめ** ← 今ここ
   - IDEA.md をもとに壁打ちし `docs/spec.md` に仕様を固める
2. **ドメインモデル設計**
   - エンティティ・値オブジェクト・集約を確定 (spec.md のモデルをベースに)
3. **ディレクトリ骨格 + `todo!()` 実装**
4. **MVP 機能を順に実装**
   - login → init → test → submit
5. **仕様と実装の同期を保ちながら拡張**

## 確定事項

| 項目 | 決定内容 |
|------|----------|
| ツール名 | `compro-env` / コマンド `ce` |
| 実装言語 | Rust |
| MVP スコープ | login, init, test, submit |
| AtCoder ログイン | `REVEL_SESSION` 手動コピー (aclogin 方式を自前実装) |
| oj 連携 | なし (Cloudflare Turnstile で破綻中) |
| ディレクトリ構造 | `solutions/{contest_id}/{lang}/{problem_code}/{solution_name}/` |
| テストケース置き場 | `solutions/{contest_id}/testcases/{problem_code}/` (言語共通) |
| solution_name デフォルト | `main` |
| init の挙動 | サンプル取得 + ディレクトリ作成 (MVP)。テスト生成は将来 |
| OJ 判定 | プレフィックスで自動判定 + URL 対応 + 不明時は stdin |
| コンフィグ | グローバル `~/.config/ce/` + プロジェクトローカル 2 段階 |
| Rust Cargo 構成 | コンテストレベル workspace、問題ごとに package |
| エラー設計 | `anyhow` + `thiserror`、型パラメータなし |
| コンテキスト検出 | カレントディレクトリから `contest_id` を自動検出 |

## giming から引き継ぐ設計

- 4 層構造: domain / usecases / interfaces / infrastructure
- `OnlineJudge` trait をポートとして usecases に置く
- Repository trait を usecases に置き、実装は infrastructure

## giming から改善する設計

| giming の問題点 | 改善方針 |
|----------------|----------|
| `E: Error + 'static` が全体に伝播 | `anyhow` + `thiserror` に統一 |
| `WorkProblem<'p>` がライフタイム参照 | データを所有する形に変更 |
| `IOSpec` が domain 層にある | MVP では不要、将来拡張で追加 |
| `Solution` エンティティが未定義 | domain に `Solution` を追加、`SolutionRepository` も定義 |
| `WorkspaceRepository` のみ | `ContestRepository` + `SolutionRepository` に分割 |
