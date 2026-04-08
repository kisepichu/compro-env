# ce logout

## 概要

保存されている OJ のセッション情報を削除する。

## シグネチャ

```
ce logout [oj]
```

- `oj`: 対象 OJ (省略時はデフォルト OJ を config から読む。現在は `atcoder` のみ対応)

## 挙動

1. 対象 OJ を決定する
2. `SessionRepository::delete(oj)` でセッションを削除する
3. 成功した場合: `Logged out from atcoder.` を表示して終了 (exit 0)
4. セッションが存在しなかった場合: `Already logged out.` を表示して終了 (exit 0)

## エラーケース

- ファイル削除失敗: エラーメッセージを表示して exit 1

## 未決事項

- なし
