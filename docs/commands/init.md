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
"abc334"    → プレフィックス "abc"/"arc"/"ahc" → AtCoder
"aoj0000"   → プレフィックス "aoj" → AOJ (将来対応)
"https://atcoder.jp/contests/abc334" → URL パース → AtCoder, id = "abc334"
それ以外     → stdin: "OJ を選んでください [atcoder]: "
```

## 挙動

1. OJ を判定し contest_id を確定する
2. コンテスト未開始の場合:
   - `solutions/{contest_id}/` を unstarted として作成
   - 開始まで待機 (ポーリング)
   - 開始後に続行
3. OJ から問題一覧・サンプル入出力を取得する
4. `solutions/{contest_id}/testcases/{problem_code}/` にサンプルを保存:
   ```
   1.in, 1.out, 2.in, 2.out, ...
   ```
5. デフォルト言語 (config より) で全問題に解法ディレクトリを作成する:
   ```
   solutions/{contest_id}/{lang}/{problem_code}/main/
   ```
6. テンプレート (`templates/{lang}/`) からファイルをコピー・文字列置換する
7. Rust の場合: contest レベルの workspace `Cargo.toml` を生成する
8. 作成したファイル一覧を表示する

## ディレクトリ生成結果例 (Rust, abc334, 問題 a〜f)

```
solutions/abc334/
  testcases/
    a/1.in  a/1.out  a/2.in  a/2.out
    b/...
  rust/
    Cargo.toml   ← [workspace] members = ["a/main", "b/main", ...]
    Cargo.lock
    a/main/
      Cargo.toml  ← [package] name = "a"
      src/main.rs ← テンプレートから生成
    b/main/
      ...
```

## テンプレート置換変数

| 変数             | 内容                         |
| ---------------- | ---------------------------- |
| `{problem_code}` | 問題コード (例: `a`)         |
| `{contest_id}`   | コンテスト ID (例: `abc334`) |

## エラーケース

- セッションが未設定: login をする
- 問題取得失敗: エラー内容を表示して終了
- ディレクトリが既に存在する: スキップして既存ファイルを保持

## 未決事項

- 特になし (MVP スコープ確定済み)
