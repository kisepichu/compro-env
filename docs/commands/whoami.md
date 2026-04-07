# ce whoami

## 概要

現在保存されているセッションで OJ にアクセスし、ログイン中のユーザー名を表示する。

## シグネチャ

```
ce whoami [oj]
```

- `oj`: 対象 OJ (省略時はデフォルト OJ を config から読む。現在は `atcoder` のみ対応)

## 挙動

1. 対象 OJ を決定する
2. `SessionRepository::get(oj)` でセッションを読み込む
3. セッションが存在しない場合:
   ```
   (not logged in)
   Run `ce login` to save your session.
   ```
   を表示して終了 (exit 0)
4. セッションが存在する場合: `OnlineJudge::whoami(&session)` を呼ぶ
5. 成功した場合: ユーザー名を1行で表示する
   ```
   kisepichu
   ```
6. 接続失敗・認証失敗の場合: エラーメッセージを表示して exit 1

## AtCoder 実装

`OnlineJudge::whoami` の AtCoder 実装:

- `GET https://atcoder.jp/home` を REVEL_SESSION クッキー付きで送る
- レスポンス HTML から `class="username"` を持つ `<a href="/users/{username}">` パターンでユーザー名を抽出する
- 抽出できなかった場合 (セッション切れ等): `Err` を返す

## エラーケース

- セッションなし: `(not logged in)` を表示して exit 0
- HTTP エラー / パース失敗: エラーメッセージを表示して exit 1
- セッション切れ (ページからユーザー名を抽出できない): `"session expired. Run \`ce login\` again."` を表示して exit 1

## 未決事項

- なし
