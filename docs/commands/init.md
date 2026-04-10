# ce init

## 概要

コンテスト用ディレクトリを作成し、ジャッジから問題一覧・サンプル入出力を取得する。

## シグネチャ

```
ce init <contest_id_or_url>
```

- `contest_id_or_url`: コンテスト ID (`abc334`) または URL (`https://atcoder.jp/contests/abc334`)

## OJ 判定ロジック

```
"abc334"    → プレフィックス "abc"/"arc"/"agc"/"ahc" → AtCoder
"aoj0000"   → プレフィックス "aoj" → AOJ (将来対応)
"https://atcoder.jp/contests/abc334" → URL パース → AtCoder, id = "abc334"
それ以外     → stdin: "OJ を選んでください [atcoder]: "
```

## 挙動

1. OJ を判定し contest_id を確定する
2. config からデフォルト言語を取得する。未設定の場合はエラー終了:
   ```
   Error: default language is not set. Add `language = "..."` to ~/.config/ce/config.toml
   ```
3. コンテストページから開始時刻を取得する (`OnlineJudge::get_start_time`)
   - 取得できない場合、または開始時刻が現在時刻以前の場合はそのまま step 5 へ進む
4. 現在時刻 < 開始時刻の場合 (未開始):
   - `ContestRepository::create_unstarted()` でディレクトリを作成する (既存なら何もしない)
   - 開始まで待機 (コンテスト開始待機の挙動を参照)
5. OJ から問題一覧・サンプル入出力を取得する (`OnlineJudge::get_problems_detail`)
6. `ContestRepository::create(&contest)` を呼ぶ:
   - `.ce.toml` を生成する (online_judge, contest_id, problems)
   - `solutions/{contest_id}/testcases/{problem_code}/` にサンプルを保存:
     ```
     1.in, 1.out, 2.in, 2.out, ...
     ```
7. デフォルト言語で全問題に解法ディレクトリを作成する (MVP では 1 言語のみ):
   - 各問題について `SolutionRepository::create(&solution)` を呼ぶ (solution name: `main`)
8. 結果サマリーを表示する

### コンテスト開始待機の挙動

`get_start_time` で取得した開始時刻をもとに、以下のスケジュールで表示・ポーリングを切り替える:

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

`{lang}` はデフォルト言語名 (例: `rust`)。

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

- デフォルト言語未設定: `Error: default language is not set. Add \`language = "..."\` to ~/.config/ce/config.toml` を表示して exit 1
- セッション未設定: `Error: not logged in. Run \`ce login\` first.` を表示して exit 1
- 問題取得失敗: エラー内容を表示して exit 1
- ディレクトリが既に存在する: 冪等性の節に従って動作する (エラーにはならない)

## 未決事項

- 特になし (MVP スコープ確定済み)
