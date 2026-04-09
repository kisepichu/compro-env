# ce solution

## 概要

解法ディレクトリの管理を行うサブコマンド群。

## サブコマンド

- `ce solution new` — 解法ディレクトリを追加する
- (将来) `ce solution rename` — 解法名を変更する

---

# ce solution new

## 概要

既存コンテストに解法ディレクトリを追加する。別言語での解法や、追加の解法アプローチを作成する際に使う。

## シグネチャ

```
ce solution new <contest_id> <problem_code> [solution_name] [--lang <lang>]
```

- `contest_id`: コンテスト ID (例: `abc334`)
- `problem_code`: 問題コード (例: `a`, `ex`)
- `solution_name`: 解法名 (省略時: `main`)
- `--lang`: 言語 (省略時: config のデフォルト言語)

## 挙動

1. `ContestRepository::get(contest_id)` で `Contest` を取得する
2. `SolutionRepository::create(&solution)` を呼ぶ
   - `templates/{lang}/__problem__/__solution__/` を `{problem_code}/{solution_name}/` として展開
   - 既に存在するファイルはスキップ
3. `SolutionRepository::sync_contest_templates(&contest, lang)` を呼ぶ
   - contest-level `.tera` ファイルを再レンダリング
4. 作成したパスを表示する

## エラーケース

- 対象ディレクトリが既に存在する: エラーメッセージを表示して終了
- `contest_id` に対応するコンテストがない: `ce init` を先に実行するよう促す

## 未決事項

- contest_id 等を省略できるようにするか (pwd で決める)
