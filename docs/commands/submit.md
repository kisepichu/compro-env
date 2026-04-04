# ce submit (ce sub)

## 概要

解法を OJ に提出する。`ce sub` は `ce submit` のエイリアス。

## シグネチャ

```
ce submit <contest_id> <problem_code> [solution_name] [--lang <lang>]
ce sub <contest_id> <problem_code> [solution_name] [--lang <lang>]
```

- `contest_id`: コンテスト ID
- `problem_code`: 問題コード
- `solution_name`: 解法名 (省略時: `main`)
- `--lang`: 言語 (省略時: config のデフォルト言語)

## 挙動

1. 解法ファイルのパスを決定する:
   ```
   solutions/{contest_id}/{lang}/{problem_code}/{solution_name}/{submit_file}
   ```
   - `submit_file` は config の `language.{lang}.submit_file`
2. config の `submit_preprocess` コマンドがあれば実行する (バンドル等)
3. セッション (`~/.config/ce/session.toml`) を読む
4. OJ の提出 API に送信する
5. 提出 URL を表示する

## AtCoder 提出の詳細

- POST `https://atcoder.jp/contests/{contest_id}/submit`
- `REVEL_SESSION` クッキーを使用
- language_id は config で設定可能 (Rust: `5054`, C++: `5001` 等)

## エラーケース

- セッション未設定: `ce login を先に実行してください`
- 提出ファイルが存在しない: パスを表示してエラー終了
- HTTP エラー: ステータスコードとレスポンスを表示

## 将来拡張

- リアルタイムモード: `ce sub a` (カレントディレクトリから contest_id を自動検出)
- 提出後の結果をポーリングして表示

## 未決事項

- language_id の設定方法 (config に書くか、OJ 固有のマッピングを内包するか)
- contest_id を省略できるようにするか (pwd で決める)
