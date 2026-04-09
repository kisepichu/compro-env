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
2. コンテストページから開始時刻を取得する
3. 現在時刻 < 開始時刻の場合 (未開始):
   - `ContestRepository::create_unstarted()` でディレクトリを作成する (既存なら何もしない)
   - 開始まで待機 (コンテスト開始待機の挙動を参照)
4. OJ から問題一覧・サンプル入出力を取得する
5. `ContestRepository::create(&contest)` を呼ぶ:
   - `.ce.toml` を生成する (online_judge, contest_id, problems)
   - `solutions/{contest_id}/testcases/{problem_code}/` にサンプルを保存:
     ```
     1.in, 1.out, 2.in, 2.out, ...
     ```
6. デフォルト言語 (config より) で全問題に解法ディレクトリを作成する:
   - 各問題について `SolutionRepository::create(&solution)` を呼ぶ (solution name: `main`)
   - 全問題分の作成後、`SolutionRepository::sync_contest_templates(&contest, lang)` を呼ぶ
7. 結果サマリーを表示する

### コンテスト開始待機の挙動

step 2 で取得した開始時刻をもとに、以下のスケジュールで表示・ポーリングを切り替える:

| タイミング | 動作 |
| --- | --- |
| 残り > 1 分 | 1 分ごとに現在時刻・開始時刻・残り時間を表示 |
| 1 分 ≥ 残り > 10 秒 | 1 秒ごとに残り時間を表示 |
| 残り ≤ 10 秒 | 1 秒ごとにポーリング開始 (問題一覧が取れたら開始とみなす) |

Ctrl-C でキャンセル可能。

### unstarted 状態の定義

`solutions/{contest_id}/` ディレクトリが存在するが `.ce.toml` がない = unstarted。
`ce init` を再実行した場合、unstarted なら待機ループに入り直す。

### 冪等性

`.ce.toml` が既に存在する (= 初期化済み) 場合:

- `.ce.toml` は上書きしない (既存を保持)
- `testcases/` は既存ファイルをスキップ (新規サンプルは追加)
- `__problem__/__solution__/` 展開ディレクトリは既存ファイルをスキップ
- `sync_contest_templates()` は常に再レンダリング (contest-level `.tera` を最新状態に更新)

## ディレクトリ生成結果例 (Rust, abc334, 問題 a〜f)

```
solutions/abc334/
  .ce.toml
  testcases/
    a/1.in  a/1.out  a/2.in  a/2.out
    b/...
  rust/
    Cargo.toml        ← sync_contest_templates() でレンダリング
    a/
      main/           ← __problem__/__solution__/ から展開
        Cargo.toml
        src/main.rs
    b/main/
      ...
```

## 出力形式

```
Initialized abc334 (AtCoder) — 6 problems: a b c d e f
  testcases  12 files
  rust       6 solutions (a/main … f/main)
```

## テンプレートシステム

`templates/{lang}/` を `solutions/{contest_id}/{lang}/` に展開する。

### 特殊ディレクトリ

| 名前 | 意味 |
| --- | --- |
| `__problem__/` | 各問題ごとに `{problem.code}/` にリネームして展開 |
| `__solution__/` | `__problem__/` 内に置く。各 solution 名にリネームして展開。`ce solution new` でも再利用 |

`__problem__/` 直下かつ `__solution__/` 外のファイルは、問題ごとに 1 回だけ生成される (solution 追加時はスキップ)。

### Tera レンダリング

- `.tera` 拡張子: ファイル名・内容ともに Tera でレンダリングし、`.tera` を除いたパスで保存
  - 例: `__problem__/{{problem.code}}_notes.md.tera` → `a/a_notes.md`
- それ以外: 静的コピー

### Tera コンテキスト

| 変数 | スコープ | 内容 | ソース |
| --- | --- | --- | --- |
| `contest.id` | 全体 | コンテスト ID (例: `abc334`) | ドメインオブジェクト |
| `contest.problems` | 全体 | 問題の配列 | ドメインオブジェクト |
| `problem.code` | `__problem__/` 内 | 問題コード (例: `a`) | ドメインオブジェクト |
| `problem.title` | `__problem__/` 内 | 問題タイトル | `Contest` ドメインオブジェクト |
| `problem.solutions` | `sync_contest_templates()` 内 | 問題ごとの solutions 配列 | infra 層でファイルシステムスキャン |
| `solution.name` | `__solution__/` 内 | 解法名 (例: `main`) | ドメインオブジェクト |

`sync_contest_templates()` は `Contest` ドメインオブジェクトを引数で受け取り、`problem.solutions` のみ infra 層でファイルシステムスキャンして補完する。`ce init` / `ce solution new` どちらから呼ばれても一貫した結果になる。

### テンプレート例 (Rust)

```
templates/rust/
  Cargo.toml.tera           ← コンテストレベル: workspace members を全解法分列挙
  __problem__/
    __solution__/
      Cargo.toml.tera       ← [package] name = "{{problem.code}}-{{solution.name}}"
      src/main.rs           ← 静的ファイル
```

`Cargo.toml.tera` (コンテストレベル):
```toml
[workspace]
members = [
{% for p in contest.problems %}{% for s in p.solutions %}  "{{ p.code }}/{{ s.name }}",
{% endfor %}{% endfor %}]
```

## エラーケース

- セッションが未設定: login をする
- 問題取得失敗: エラー内容を表示して終了
- ディレクトリが既に存在する: 冪等性の節に従って動作する (エラーにはならない)

## 未決事項

- 特になし (MVP スコープ確定済み)
