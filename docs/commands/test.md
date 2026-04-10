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
2. config のテストコマンドをテストケースごとに実行する:
   - コマンド内のプレースホルダーを置換して実行
   - 標準出力を `expected` と照合する
3. 結果を表示する

## テストコマンドのプレースホルダー

| プレースホルダー | 内容 |
| --- | --- |
| `{problem}` | problem_code (例: `a`) |
| `{solution}` | solution_name (例: `main`) |
| `{dir}` | 解法ディレクトリの絶対パス (`solutions/{contest_id}/{problem_code}/{solution_name}`) |
| `{file}` | 解法ファイルの絶対パス (`{dir}/{solution_file}`) |
| `{input_file}` | テストケース入力ファイルの絶対パス (テストケースごとに展開・実行) |

`{input_file}` を含むコマンドはテストケース 1 件ずつ実行される。

### config 例

```toml
[language.rust]
solution_file = "src/main.rs"
test = "cargo run --manifest-path {dir}/Cargo.toml < {input_file}"

[language.rust.atcoder]
lang_id = "5054"
```

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
- TLE 判定 (time_limit をテストケースメタデータから取得)
