# ce submit (ce sub)

## 概要

解法をブラウザで提出する。`ce sub` は `ce submit` のエイリアス。

AtCoder は Cloudflare Turnstile を導入しており、HTTP 直接送信でのボット提出がブロックされる。
そのため `ce submit` は提出内容を URL フラグメントに埋め込んでブラウザで提出ページを開く。
ブラウザに Tampermonkey userscript を導入することで問題選択・ソースコード注入を自動化できる。

## シグネチャ

```
ce submit <contest_id> <problem_code> [solution_name]
ce sub <contest_id> <problem_code> [solution_name]
```

- `contest_id`: コンテスト ID
- `problem_code`: 問題コード
- `solution_name`: 解法名 (省略時: `main`)

## 挙動

1. Unix 環境では、提出前に `ce test <contest_id> <problem_code> [solution_name]` 相当のテストを実行する:
   - 実行内容は `docs/commands/test.md` と同じ
   - `test_command` の標準出力・標準エラーはそのまま端末に流す
   - 終了コードが `0` の場合のみ次のステップへ進む
   - 終了コードが `0` 以外の場合は提出 URL を生成せず、ブラウザも開かずにエラー終了する
   - 非 Unix 環境では、`ce test` が未対応のため提出前テストをスキップし、従来通り提出 URL 生成へ進む
2. 解法ディレクトリの `ce.toml` から `language` を読む:
   ```
   solutions/{contest_id}/{problem_code}/{solution_name}/ce.toml
   ```
   `language` フィールドは `templates/{lang}/ce.toml.tera` で定義され `ce init` / `ce solution add` 時に生成される (詳細: `docs/commands/test.md`)。
3. `.ce.toml` から `OJKind` と `problem_id` を取得する:
   - `ContestRepository::get_oj_kind(contest_id)` で `OJKind` を得る
   - `ContestRepository::get_problem(contest_id, problem_code)` で `problem_id` を得る
4. config の `language.{language}.solution_file` からファイルパスを決定し、`SolutionRepository::get_source(solution, file_path)` でソースを読む:
   ```
   solutions/{contest_id}/{problem_code}/{solution_name}/{solution_file}
   ```
5. config の `language.{language}.{oj}.lang_id` を取得する (詳細: `docs/commands/submit.md` の lang_id 解決順)
6. **preprocess フック**が設定されていれば実行し、その標準出力を提出ソースとして採用する (詳細: 「提出前 preprocess フック」節)。未設定なら手順 4 で読んだソースをそのまま使う
7. 提出ページ URL を生成して標準出力に表示する
8. 提出ページ URL をブラウザで開く (詳細: 次節)

ステップ 7 の URL を開いた後、Tampermonkey userscript が問題選択・ソースコード注入を行う (詳細: `docs/userscript.md`)。

> 手順 6 は提出ソースの内容のみを差し替える。OJ への提出方式 (`SubmitOutcome`) や URL 生成・ブラウザ起動は変わらない。AtCoder では preprocess 後のソースが URL フラグメントの `source` に入る。

## 提出 URL の生成

```
https://atcoder.jp/contests/{contest_id}/submit?taskScreenName={problem_id}#ce={payload}
```

- `?taskScreenName={problem_id}`: AtCoder の submit ページが対応している既存クエリパラメータ。問題プルダウンを `problem_id` で事前選択する
- `#ce={payload}`: userscript が読む URL フラグメント

`payload` は以下の JSON を URL-safe base64 (RFC 4648 §5、パディング `=` あり) でエンコードしたもの:

```json
{ "lang_id": "6088", "source": "fn main() { ... }" }
```

ブラウザで URL を開く際は OS のデフォルトブラウザを使用する:

- Linux: `xdg-open <url>`
- macOS: `open <url>`
- Windows: `explorer.exe <url>`

## 提出前 preprocess フック

提出するソースを、実際に送る前にユーザー指定のスクリプトで整形・変換する仕組み。
整形・ライブラリ展開・提出ページ向けのレイアウト変更などを、すべてユーザースクリプト側に委ねる。

**設計原則**: アプリ側にバンドル・展開・言語別ロジックを一切持たない。言語や OJ を増やしても
アプリのコードは変更不要で、ユーザーは config に1行加えてスクリプト内で分岐するだけでよい。
各言語の成熟したツール (Rust = `cargo-equip`、C++ = `oj-bundle` 等) をスクリプトから呼ぶことを想定する。

### 実行契約

| チャネル         | 内容                                                                                             |
| ---------------- | ------------------------------------------------------------------------------------------------ |
| stdin            | 手順 4 で読んだ元ソース全文                                                                      |
| stdout           | 提出するソース全文 (これを `OnlineJudge::submit` に渡す)                                         |
| exit 0           | stdout を提出ソースとして採用する                                                                |
| exit ≠0          | 提出を中止する。スクリプトの stderr をそのまま表示し、URL は生成しない                           |
| 作業ディレクトリ | 解法ディレクトリ (`ce test` と同じ。`cargo-equip` 等のパス系ツールが crate ルートを前提にできる) |

実行方式は `ce test` と同じく `sh -c <command>` を用い、Unix-like shell が前提。
非 Unix 環境では preprocess フックを実行できないため、設定されていてもスキップし、元ソースをそのまま提出する。

### 環境変数

スクリプトは以下の環境変数でコンテキストを受け取り、自身で言語・OJ 分岐を行う:

| 変数               | 内容                                                               |
| ------------------ | ------------------------------------------------------------------ |
| `CE_LANGUAGE`      | 言語名 (例: `rust`)                                                |
| `CE_OJ`            | OJKind::as_str (例: `atcoder` / `librarychecker`)。OJ 別分岐に使用 |
| `CE_CONTEST_ID`    | コンテスト ID                                                      |
| `CE_PROBLEM_CODE`  | 問題コード                                                         |
| `CE_PROBLEM_ID`    | OJ 固有の問題 ID (例: `abc334_a`)                                  |
| `CE_SOLUTION_NAME` | 解法名                                                             |
| `CE_SOLUTION_DIR`  | 解法ディレクトリの絶対パス                                         |
| `CE_SOURCE_FILE`   | 提出元ソースファイルの絶対パス                                     |
| `CE_LANG_ID`       | 手順 5 で解決した提出言語 ID                                       |

### config キー

```toml
[submit]
preprocess = "~/.config/ce/hooks/submit-preprocess.sh"   # 全言語共通の1本
```

キーは `[submit].preprocess` のみ。未設定なら preprocess を行わず元ソースをそのまま提出する (後方互換)。
`Config::submit_preprocess(&self) -> Option<String>` を返し、未設定時は `None` とする (`&Language` 引数は取らない)。

**言語別の分岐はアプリではなくスクリプト側で行う。** 言語は `CE_LANGUAGE` env で渡るので、
1 本のスクリプト内で `case "$CE_LANGUAGE" in rust) ... ;; cpp) ... esac` のように分岐する。
言語ごとにファイルを分けたいユーザーは、メインスクリプトに `exec "$(dirname "$0")/hooks/$CE_LANGUAGE.sh"` の
1 行を書けば自前でディスパッチでき、アプリ側の支援は不要 (per-language config キーは設けない)。

### ユースケース対応

- **整形**: `rustfmt` 等をスクリプト内で実行する。
- **ライブラリ展開 (compile error 回避)**: ローカル自作ライブラリの import や OJ 側に無い外部ライブラリを
  1 ファイルに展開する。`$CE_OJ` を見て「AtCoder では ACL を展開しない」等の最適化も可能。展開が
  compile error を生む事故は、スクリプト自身が展開後にコンパイル/サンプル確認して失敗時に exit ≠0 で
  防ぐ責務を負う (アプリは終了コードを見るだけ。専用の post-verify フックは将来拡張)。
- **提出ページの可読性**: 上部に元コードをコメント (`/* ... */`) や到達不能コード (`#if 0 ... #endif`) で
  残し、下に展開済み本体を置く、といったレイアウトはスクリプト内の文字列組み立てで行う。
  ※ `cargo-equip` の素の出力は「上 = 自分のコード / 下 = ライブラリを畳んだ module」で既に可読なため、
  元コードの重複掲載は必須でない (提出サイズ上限に注意)。

### スクリプト例

Rust ライブラリ展開 (cargo-equip) を行う実例は `hooks/submit-preprocess.sh` にある。
`$CE_LANGUAGE` で分岐し、Rust は `cargo equip --check` で展開後コンパイルまで検証してから提出ソースを
stdout に出す。他言語は `cat` で素通しする雛形。

## エラーケース

- Unix 環境で提出前テストが失敗した: 終了コードを表示してエラー終了し、提出 URL は生成しない
- preprocess フックが exit ≠0 で終了した: スクリプトの stderr を表示してエラー終了し、提出 URL は生成しない
- Unix 環境で提出前テストを起動できない、または `test_command` が未定義: `ce test` と同じエラーとして終了し、提出 URL は生成しない
- 解法の `ce.toml` が存在しない: パスを表示してエラー終了
- 提出ファイルが存在しない: パスを表示してエラー終了
- `lang_id` が config に未設定: エラー終了

## 将来拡張

- リアルタイムモード: `ce sub a` (cwd から `contest_id` を自動検出)
- 提出後の結果をポーリングして表示 (ブラウザから提出 URL を受け取る方法が別途必要)
- post-verify フック: preprocess の展開後ソースに対してアプリがサンプルを再実行する第 2 フック。
  当面は preprocess スクリプト内の自己検証で代替する
- バイナリ埋め込み提出: 展開ツールが無い言語向けに「compile → strip → base64 を汎用言語に埋め込み実行」を
  preprocess スクリプトの 1 分岐として行う。クロスコンパイル/libc 不一致に脆いため第一推奨にはしないが、
  アプリ無変更で実現できる。可読性のため上部に元コードを書く

## 既知の技術的負債

- **`Config` trait の戻り値**: `lang_id()` / `submit_file()` は設定読み込み失敗時に `eprintln!` して `None`/デフォルト値にフォールバックする。`Result` を返して呼び出し側でハンドリングすべき。変更には `usecases/src/config.rs` の trait 定義と 4 サービスのテストスタブ変更が必要。
- **`submit_file()` のデフォルト値**: 言語問わず `"src/main.rs"` にフォールバックする。C++ テンプレートは `main.cpp` を使うため不整合。言語ごとのデフォルトパスを持つべき。
