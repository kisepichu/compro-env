# ce new

## 概要

既存コンテストに解法ディレクトリを追加する。別言語での解法や、追加の解法アプローチを作成する際に使う。

## シグネチャ

```
ce new <contest_id> <problem_code> [solution_name] [--lang <lang>]
```

- `contest_id`: コンテスト ID (例: `abc334`)
- `problem_code`: 問題コード (例: `a`, `ex`)
- `solution_name`: 解法名 (省略時: `main`)
- `--lang`: 言語 (省略時: config のデフォルト言語)

## 挙動

1. `solutions/{contest_id}/{lang}/{problem_code}/{solution_name}/` を作成する
2. テンプレート (`templates/{lang}/`) からファイルをコピー・置換する
3. Rust の場合: `solutions/{contest_id}/rust/Cargo.toml` の `members` に追加する
4. 作成したパスを表示する

## エラーケース

- 対象ディレクトリが既に存在する: エラーメッセージを表示して終了
- `contest_id` に対応するディレクトリがない: `ce init` を先に実行するよう促す

## 未決事項

- contest_id 等を省略できるようにするか(pwd で決める)
