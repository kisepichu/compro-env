# ce test

## 概要

サンプルテストケースを使って解法を実行し、期待出力と照合する。

## シグネチャ

```
ce test <contest_id> <problem_code> [solution_name] [--lang <lang>]
```

- `contest_id`: コンテスト ID
- `problem_code`: 問題コード
- `solution_name`: 解法名 (省略時: `main`)
- `--lang`: 言語 (省略時: config のデフォルト言語)

## 挙動

1. `solutions/{contest_id}/testcases/{problem_code}/` からテストケースを読む
2. config のテストコマンドを実行する:
   ```toml
   [language.rust]
   test = "cargo test -p {problem}"
   ```
   - `{problem}` → problem_code に置換
   - `{file}` → 解法ファイルの絶対パスに置換
3. 結果を表示する (MVP: シンプルなパス/フェイル表示、将来: カラー AC/WA/TLE)

## 出力形式 (MVP)

```
[1] AC  (0.012s)
[2] WA
  expected: 5
  actual:   4
[3] AC  (0.011s)

2/3 passed
```

## エラーケース

- テストケースが存在しない: エラーメッセージを表示
- テストコマンドが config にない: デフォルト言語設定を確認するよう促す

## 将来拡張

- カラー表示 (AC: 緑、WA: 赤、TLE: 黄)
- TLE 判定 (time_limit を testcase メタデータから取得)
