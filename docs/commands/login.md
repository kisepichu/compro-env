# ce login

## 概要

AtCoder の `REVEL_SESSION` クッキーを手動入力し、セッションファイルに保存する。
AtCoder が Cloudflare Turnstile を導入したため、自動ログインは使わず手動コピー方式を採用。

## シグネチャ

```
ce login [oj] [--cookie VALUE]
```

- `oj`: 対象 OJ (省略時はデフォルト OJ を config から読む。現在は `atcoder` のみ対応)
- `--cookie VALUE`: REVEL_SESSION の値を直接渡す (省略時は対話入力)

## 挙動

1. 対象 OJ を決定する
2. `--cookie` が指定されていない場合、ブラウザでの `REVEL_SESSION` 取得手順をターミナルに表示する:
   ```
   1. Open https://atcoder.jp in your browser and log in.
   2. Open DevTools -> Application -> Cookies -> https://atcoder.jp
   3. Copy the value of REVEL_SESSION.
   ```
3. `--cookie` が指定されていない場合、stdin でクッキー値を受け取る (`REVEL_SESSION: ` プロンプト)
4. `~/.config/ce/session.toml` に保存する:
   ```toml
   [atcoder]
   revel_session = "入力値"
   ```
5. 保存後、`ce whoami` で動作確認できる旨を表示する

## エラーケース

- 空文字を入力した場合: エラーメッセージを表示して終了
- ファイル書き込み失敗: エラーメッセージを表示して終了

## 未決事項

- 他 OJ 対応方法
