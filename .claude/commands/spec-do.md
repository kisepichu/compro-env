コマンドの仕様からタスクファイルを生成し、実装を開始する。

## 手順

手順に沿って進める。特に、ブランチを切ることと、チェックをすること、コミットやプッシュする前に止まる事を忘れない。

1. 引数からコマンド名を取得する (例: `/spec-do login` → `login`)
   - 引数がなければ「どのコマンドを実装しますか?」と聞く
2. `docs/commands/{command}.md` を読む。なければ `docs/spec.md` の該当部分を読む
3. `CLAUDE.md` のアーキテクチャルールを確認する
4. 実装を DDD レイヤーごとに分解してタスクを洗い出す
5. タスクファイルを `tasks/doing/TASK-NNN-{command}.md` に作成する。ブランチの切り方をユーザーに確認する
   - NNN は既存タスクの連番 (todo/ doing/ done/ を合わせて最大番号 + 1)
6. タスクファイルのチェックリスト項目ごとに以下の TDD サイクルを回す:

   **RED フェーズ** — `.claude/agents/test-writer-prompt.md` のテンプレートを使い、
   test-writer subagent を Agent ツールで起動する。

   - subagent がテストを書き、`cargo test` で失敗を確認してレポートを返す
   - テストが期待通りに失敗していることを確認してから次へ進む

   **GREEN フェーズ** — `.claude/agents/implementer-prompt.md` のテンプレートを使い、
   implementer subagent を Agent ツールで起動する。

   - test-writer のレポート（失敗したテスト名・ファイルパス）をプロンプトに含める
   - subagent が最小限の実装を書き、`cargo test` で全テスト通過を確認してレポートを返す

   **REFACTOR フェーズ** — 全テストが通る状態を維持しながらリファクタリングする

   次のチェックリスト項目へ進む前に、必ず GREEN まで完了させること

7. `cargo fmt --all` でフォーマット、`cargo clippy --workspace --all-features` で警告を確認し修正できるものはし、`cargo test --workspace` で全テスト通過を確認する

## タスクファイル形式

```markdown
# TASK-{NNN}: ce {command} 実装

## 参照仕様

- docs/commands/{command}.md

## 実装チェックリスト

### domain/

- [ ] ...

### usecases/

- [ ] ...

### interfaces/

- [ ] ...

### infrastructure/

- [ ] ...

## 完了条件

- [ ] ...

## 作業ログ

- {date}: 作業開始
```

## 注意

- 骨格 (`todo!()`) → ドメイン → usecases → infrastructure の順で実装する
- 仕様に曖昧な点があればユーザーに確認してから実装する
- 完了したチェックリスト項目はその都度 `[x]` に更新する
- 完了時は `tasks/doing/` から `tasks/done/` に移動する
