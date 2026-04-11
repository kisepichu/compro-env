# ce init

## 概要

コンテスト用ディレクトリを作成し、ジャッジから問題一覧・サンプル入出力を取得する。

## シグネチャ

```
ce init <contest_id_or_url> [--lang <lang>]
```

- `contest_id_or_url`: コンテスト ID (`abc334`) または URL (`https://atcoder.jp/contests/abc334`)
- `--lang <lang>`: 解法ディレクトリの言語 (省略時は config のデフォルト言語、それもなければ stdin で確認)

## OJ 判定ロジック

```
"abc334"    → プレフィックス "abc"/"arc"/"agc"/"ahc" → AtCoder
"aoj0000"   → プレフィックス "aoj" → AOJ (将来対応)
"https://atcoder.jp/contests/abc334" → URL パース → AtCoder, id = "abc334"
それ以外     → stdin: "OJ (e.g. atcoder): " (空 Enter でデフォルト atcoder)
```

## HTTP リクエスト構成 (AtCoder)

コンテスト開始後の通常ケースでは以下の **2 リクエスト** のみでコンテスト情報を取得する:

| # | URL | 取得情報 |
| - | --- | ------- |
| 1 | `https://atcoder.jp/contests/{id}` | 開始時刻・problem_id ヒント (後述) |
| 2 | `https://atcoder.jp/contests/{id}/tasks_print` | 全問題タイトル・サンプル I/O |

コンテスト待機中 (残り ≤ 10 秒) は `get_problems_detail` をポーリングするため、リクエスト数は 2 を超える。

### problem_id の決定ロジック

`tasks_print` には `<span class="h2">A - Title</span>` しかなく、OJ 固有の `problem_id` (例: `abc334_a`) は含まれない。  
problem_id は次の優先順位で決定する:

1. **ヒントあり**: リクエスト 1 のコンテストページのナビバードロップダウンに  
   `href="/contests/{id}/tasks/{problem_id}"` が含まれる場合 (ABC/ARC 同時開催の旧コンテスト等)、  
   その problem_id を使用する。
2. **ヒントなし**: ドロップダウンがない場合 (現行のほとんどのコンテスト)、  
   `{contest_id}_{problem_code}` (例: `abc334_a`) と推定する。

`OnlineJudge::get_contest_meta` がリクエスト 1 の結果として開始時刻と problem_id ヒントを両方返す。  
`get_problems_detail` はそのヒントを受け取り、空なら推定する。

### `tasks_print` のサンプル取得

`tasks_print` は日本語 (`入力例 N`) と英語 (`Sample Input N`) の両方のサンプルセクションを含む。  
英語セクション (`<h3>Sample Input N</h3>` / `<h3>Sample Output N</h3>`) のみを使用する。

### problem_code の変換

`tasks_print` の `<span class="h2">A - Title</span>` から problem_code を決定する:

- `<span class="h2">` のテキストの `-` 前の部分をトリムして小文字化する
- 例: `"A - Christmas Present"` → `"a"`, `"EX - Extra"` → `"ex"`

## 挙動

1. OJ を判定し contest_id を確定する
2. 言語を決定する:
   - `--lang` が指定されていればそれを使用
   - 指定なし・config にデフォルト言語あり → config の値を使用
   - 指定なし・config にデフォルト言語なし → stdin で確認:
     ```
     Language (e.g. rust, cpp): 
     ```
   - いずれの場合も `templates/{lang}/` ディレクトリが存在しなければエラー終了:
     ```
     Error: unknown language "{lang}". Available: {templates/ 以下のディレクトリ一覧}
     ```
3. セッションを取得する (`SessionRepository::get`、失敗しない — `Option<Session>` として保持)
4. `OnlineJudge::get_contest_meta(contest_id)` を呼ぶ:
   - 開始時刻 (`Option<DateTime<Utc>>`) と problem_id ヒント (`Vec<(code, problem_id)>`) を取得
   - 取得できない場合、または開始時刻が現在時刻以前の場合はそのまま step 6 へ進む
5. 現在時刻 < 開始時刻の場合 (未開始):
   - `ContestRepository::create_unstarted()` でディレクトリを作成する (既存なら何もしない)
   - 開始まで待機 (コンテスト開始待機の挙動を参照)
6. `OnlineJudge::get_problems_detail(contest_id, session.as_ref(), &meta.problem_id_hints)` を呼ぶ
   - session が `None` のままでも呼ぶ (過去コンテストは公開アクセス可)
   - AtCoder 実装側でログインが必要と判断した場合は `Error: not logged in. Run \`ce login\` first.` を返す
7. `ContestRepository::create(&contest)` を呼ぶ:
   - `.ce.toml` を生成する (online_judge, contest_id, problems)
   - `solutions/{contest_id}/testcases/{problem_code}/` にサンプルを保存:
     ```
     1.in, 1.out, 2.in, 2.out, ...
     ```
8. 決定した言語で全問題に解法ディレクトリを作成する (MVP では 1 言語のみ):
   - 各問題について `SolutionRepository::create(&solution)` を呼ぶ (solution name: `main`)
9. 結果サマリーを表示する

### コンテスト開始待機の挙動

`get_contest_meta` で取得した開始時刻をもとに、以下のスケジュールで表示・ポーリングを切り替える:

| タイミング | 動作 |
| --- | --- |
| 残り > 1 分 | 1 分ごとに現在時刻・開始時刻・残り時間を表示 |
| 1 分 ≥ 残り > 10 秒 | 1 秒ごとに残り時間を表示 |
| 残り ≤ 10 秒 | 1 秒ごとにポーリング開始 (問題一覧が取れたら開始とみなす) |

待機ロジックは `usecases/service/init.rs` に実装する。Ctrl-C でキャンセル可能。

### unstarted 状態の定義

`solutions/{contest_id}/` ディレクトリが存在するが `.ce.toml` がない = unstarted。
`ce init` を再実行した場合、unstarted なら待機ループに入り直す。

### 冪等性

`.ce.toml` が既に存在する (= 初期化済み) 場合:

- `.ce.toml` は上書きしない (既存を保持)
- `testcases/` は既存ファイルをスキップ (新規サンプルは追加)
- 解法ディレクトリ (`{problem_code}/{solution_name}/`) が既に存在する問題はスキップ (ファイル単位の追加はしない)

## ディレクトリ生成結果例 (lang: rust, abc334, 問題 a〜f)

```
solutions/abc334/
  .ce.toml
  testcases/
    a/1.in  a/1.out  a/2.in  a/2.out
    b/...
  a/
    main/           ← templates/rust/ から展開
      Cargo.toml
      src/main.rs
  b/main/
    ...
```

## 出力形式

```
Initialized abc334 (AtCoder) — 6 problems: a b c d e f
  testcases   12 files
  {lang}      6 solutions (a/main … f/main)
```

`{lang}` は使用した言語名 (例: `rust`)。

## テンプレートシステム

`templates/{lang}/` の内容を `solutions/{contest_id}/{problem_code}/{solution_name}/` に展開する。
言語はユーザーが `templates/` 以下にディレクトリを追加することで自由に定義できる。

### Tera レンダリング

- `.tera` 拡張子: ファイル名・内容ともに Tera でレンダリングし、`.tera` を除いたパスで保存
  - 例: `Cargo.toml.tera` → `Cargo.toml`
- それ以外: 静的コピー

### Tera コンテキスト

| 変数 | 内容 | ソース |
| --- | --- | --- |
| `contest.id` | コンテスト ID (例: `abc334`) | ドメインオブジェクト |
| `problem.code` | 問題コード (例: `a`) | ドメインオブジェクト |
| `problem.title` | 問題タイトル | ドメインオブジェクト |
| `solution.name` | 解法名 (例: `main`) | ドメインオブジェクト |

### テンプレート例 (Rust)

```
templates/rust/
  Cargo.toml.tera     ← [package] name = "{{problem.code}}-{{solution.name}}"
  src/main.rs         ← 静的ファイル
```

`Cargo.toml.tera` 例:
```toml
[package]
name = "{{problem.code}}-{{solution.name}}"
version = "0.1.0"
edition = "2021"
```

## エラーケース

- テンプレートが存在しない言語を指定: `Error: unknown language "{lang}". Available: {templates/ 以下のディレクトリ一覧}` を表示して exit 1
- 問題取得失敗 (ログイン必要): `Error: not logged in. Run \`ce login\` first.` を表示して exit 1
- 問題取得失敗 (その他): エラー内容を表示して exit 1
- ディレクトリが既に存在する: 冪等性の節に従って動作する (エラーにはならない)

## 未決事項

### problem_id ヒントの完全実装 (Phase 2)

現在 `get_contest_meta` は `problem_id_hints: vec![]` を返す (推定のみ)。  
将来的にコンテストページのナビバードロップダウンを解析して、ABC/ARC 同時開催等の旧コンテストでも正しい problem_id を使えるようにする。  
インターフェース変更は不要 — `get_contest_meta` の実装を拡張するだけ。
