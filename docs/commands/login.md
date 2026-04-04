# ce login

## 概要

AtCoder の `REVEL_SESSION` クッキーを手動入力し、セッションファイルに保存する。
AtCoder が Cloudflare Turnstile を導入したため、自動ログインは使わず手動コピー方式を採用。

## シグネチャ

```
ce login [oj]
```

- `oj`: 対象 OJ (省略時はデフォルト OJ を config から読む。現在は `atcoder` のみ対応)

## 挙動

1. 対象 OJ を決定する
2. ブラウザでの `REVEL_SESSION` 取得手順をターミナルに表示する:
   ```
   1. ブラウザで https://atcoder.jp にログインする
   2. DevTools → Application → Cookies → https://atcoder.jp
   3. REVEL_SESSION の値をコピーする
   ```
3. stdin でクッキー値を受け取る (`REVEL_SESSION の値: ` プロンプト)
4. `~/.config/ce/session.toml` に保存する:
   ```toml
   [atcoder]
   revel_session = "入力値"
   ```
5. 保存後、`ce whoami` で動作確認できる旨を表示する

## エラーケース

- 空文字を入力した場合: エラーメッセージを表示して終了
- ファイル書き込み失敗: エラーメッセージ + パスを表示

## 未決事項

- `ce whoami` コマンドは MVP に含めるか? (ログイン確認用) > 含めて良い
- 他 OJ 対応方法
