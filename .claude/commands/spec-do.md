仕様からタスクファイルを生成し、実装を開始する。対象は CLI コマンド単体に限らず、OJ 連携やドメイン抽象のようなサブシステム横断トピックでもよい。

## 手順

手順に沿って進める。特に、ブランチを切ることと、チェックをすること、コミットやプッシュする前に止まることを忘れない。

1. 引数から対象を取得する
   - 対象は CLI コマンド名 (例: `/spec-do login` → `login`) でも、サブシステム/トピック名 (例: `/spec-do online-judge`, `/spec-do librarychecker`) でもよい
   - 引数がなければ「どの対象を実装しますか?」と聞く
2. 対象の仕様ドキュメントを読む
   - コマンドなら `docs/commands/{command}.md`、トピックなら `docs/{topic}.md` (例: `docs/online_judges/{name}.md`)
   - なければ `docs/spec.md` の該当部分を読む
3. `CLAUDE.md` のアーキテクチャルールを確認する
4. 実装を DDD レイヤーごとに分解してタスクを洗い出す
5. タスクファイルを `tasks/doing/TASK-NNN-{slug}.md` に作成する。ブランチの切り方をユーザーに確認して、 dev から切る
   - `{slug}` は対象を表す短い識別子 (コマンド名、または `oj-librarychecker` 等のトピックスラグ)
   - NNN は既存タスクの連番 (todo/ doing/ done/ を合わせて最大番号 + 1)
   - 横断トピックで作業が大きい場合は、レイヤーや機能単位で複数タスクに分割してよい
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
# TASK-{NNN}: {対象} 実装

## 参照仕様

- docs/commands/{command}.md または docs/{topic}.md (+ docs/spec.md の該当セクション)

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
