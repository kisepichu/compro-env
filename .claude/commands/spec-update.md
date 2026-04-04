コマンド仕様を更新する。

## 手順

1. 引数からコマンド名を取得する (例: `/spec-update init` → `init`)
   - 引数がなければ「どのコマンドの仕様を更新しますか?」と聞く
2. `docs/commands/{command}.md` が存在すれば読む。なければ `docs/spec.md` の該当部分を読む
3. 何を変更したいかをユーザーに確認し、議論して仕様を確定する
4. `docs/commands/{command}.md` を更新する (存在しなければ新規作成)
5. `docs/spec.md` の該当箇所も同期して更新する
6. 変更内容をサマリーで報告する

## ファイル形式 (docs/commands/{command}.md)

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

## 注意

- 仕様変更は必ず `docs/spec.md` にも反映する
- 実装済みの挙動と仕様が乖離する場合はユーザーに確認する
