仕様を更新する。対象は CLI コマンド単体に限らず、OJ 連携やドメイン抽象のようなサブシステム横断トピックでもよい。

## 手順

1. 引数から対象を取得する
   - 対象は CLI コマンド名 (例: `/spec-update init` → `init`) でも、サブシステム/トピック名 (例: `/spec-update online-judge`, `/spec-update librarychecker`) でもよい
   - 引数がなければ「どの対象の仕様を更新しますか?」と聞く
2. 対象の仕様ドキュメントを読む
   - コマンドなら `docs/commands/{command}.md`
   - トピックなら `docs/{topic}.md` (例: `docs/online_judges/{name}.md`)
   - いずれもなければ `docs/spec.md` の該当部分を読む
3. 何を変更したいかをユーザーに確認し、議論して仕様を確定する
4. 対象の仕様ドキュメントを更新する (存在しなければ新規作成)
   - コマンドなら `docs/commands/{command}.md`、トピックなら `docs/{topic}.md`
5. `docs/spec.md` の該当箇所も同期して更新する
6. 変更内容をサマリーで報告する

## ファイル形式

### コマンド仕様 (docs/commands/{command}.md)

```markdown
# ce {command}

## 概要
...

## シグネチャ
`ce {command} <args> [options]`

## 挙動
...

## エラーケース
...

## 未決事項
...
```

### トピック仕様 (docs/{topic}.md)

横断トピックは上記の固定書式に縛られず、対象に合った見出しで書いてよい (例: 抽象の責務、ポート定義、各実装の差分、判定ロジック、未決事項)。

## 注意

- 仕様変更は必ず `docs/spec.md` にも反映する
- 実装済みの挙動と仕様が乖離する場合はユーザーに確認する
