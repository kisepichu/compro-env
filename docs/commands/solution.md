# ce solution

## 概要

解法ディレクトリの管理を行うサブコマンド群。

## サブコマンド

- `ce solution add` — 解法ディレクトリを追加する
- (将来) `ce solution rename` — 解法名を変更する

---

# ce solution add

## 概要

既存コンテストに解法ディレクトリを追加する。別言語での解法や、追加の解法アプローチを作成する際に使う。

## シグネチャ

```
ce solution add <contest_id> <problem_code> [solution_name] [--lang <lang>]
```

- `contest_id`: コンテスト ID (例: `abc334`)
- `problem_code`: 問題コード (例: `a`, `ex`)
- `solution_name`: 解法名 (省略時: `main`)
- `--lang`: 言語 (省略時: config のデフォルト言語、それもなければ stdin で確認)

## 挙動

1. `contest_id`、`problem_code`、`solution_name` を小文字に正規化する (`ce init` / `ce test` と同様)
2. `ContestRepository::exists(contest_id)` でコンテストが存在するか確認する
3. `ContestRepository::list_problem_codes(contest_id)` で `problem_code` が存在するか確認する
4. `SolutionRepository::exists(contest_id, problem_code, solution_name)` で既存チェックを行い、存在すればエラー
5. `ContestRepository::get_samples(contest_id, problem_code)` でサンプルを取得する
6. `SolutionRepository::create(&solution, &samples)` を呼ぶ
   - `templates/{lang}/` を `solutions/{contest_id}/{problem_code}/{solution_name}/` として展開
7. 作成したパス (プロジェクトルートからの相対パス) を表示する

## 出力形式

```
Created solutions/abc334/a/sol2 (rust)
```

パスはプロジェクトルートからの相対パス。

## エラーケース

- 対象ディレクトリが既に存在する: エラーメッセージを表示して終了
- `contest_id` に対応するコンテストがない: `ce init` を先に実行するよう促す
- `problem_code` に対応する問題がない: 利用可能な問題コード一覧を表示してエラー終了
- `lang` に対応するテンプレートが存在しない: 利用可能な言語一覧を表示してエラー終了

## 未決事項

- contest_id 等を省略できるようにするか (pwd で決める)
