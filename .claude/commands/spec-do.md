コマンドの仕様からタスクファイルを生成し、実装を開始する。

## 手順

1. 引数からコマンド名を取得する (例: `/spec-do login` → `login`)
   - 引数がなければ「どのコマンドを実装しますか?」と聞く
2. `docs/commands/{command}.md` を読む。なければ `docs/spec.md` の該当部分を読む
3. `CLAUDE.md` のアーキテクチャルールを確認する
4. 実装を DDD レイヤーごとに分解してタスクを洗い出す
5. タスクファイルを `tasks/doing/TASK-NNN-{command}.md` に作成する
   - NNN は既存タスクの連番 (todo/ doing/ done/ を合わせて最大番号 + 1)
6. タスクファイルを作成したら、 TDD を厳守して実装を開始する。各部分について、テストを書き、テストが失敗することを確認してから実装を行う

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
