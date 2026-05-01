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

### `tasks_print` の入力形式・制約取得

同じ `tasks_print` ページから、各問題の入力形式と制約も取得する (追加リクエスト不要)。

- **入力形式**: `<h3>入力</h3>` から次の `<h3>` までの範囲にある **全 `<pre>` ブロック** を取得
  - `<var>` 等の inline HTML タグは strip してテキストを得る
  - 変数は underscore 記法 (`A_1`, `A_{1,1}`)、省略は LaTeX 記法 (`\ldots`, `\vdots`)
  - クエリ型問題では複数 `<pre>` ブロックが存在する (pre[0]: メイン形式、pre[1..]: クエリサブ形式)
  - 全ブロックを `\n\n` で結合して `input_format_raw` に格納する
  - 例 (通常): `"N M\nA_1 A_2 \\ldots A_N\n"`
  - 例 (クエリ型): `"Q\nquery_1\n\\vdots\nquery_Q\n\n1 x\n\n2 x k\n\n3 x k\n"`
- **制約**: `<h3>制約</h3>` の直後のテキスト (HTML タグ strip 済み)
  - 型推定に使用する (詳細は「入力形式パース」節を参照)

取得失敗・セクション不在の場合は `None` として扱い、`ce init` 全体は失敗させない。

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
   - `.ce.toml` を生成する (online_judge, contest_id, problems, input_format_raw, constraints_raw)
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
| `input_format.raw` | 入力形式の生テキスト (常に設定、取得できなければ空文字) | パーサー |
| `input_format.ok` | パース成功フラグ (`bool`) | パーサー |
| `input_format.vars` | 変数宣言リスト (詳細後述) | パーサー |
| `input_format.ops` | 読み取り命令列 (詳細後述) | パーサー |

`input_format.ok` が `false` のとき `vars` と `ops` は空リスト。テンプレートは `{% if input_format.ok %}` で分岐する。

### 入力形式パース

`.ce.toml` に保存した `input_format_raw` を `SolutionRepository::create` 時に毎回オンザフライでパースし、Tera コンテキストに注入する。パース処理は `usecases/input_format/` モジュールが担う。

#### パイプライン

```
input_format_raw (文字列、複数 pre ブロックは \n\n 区切りで結合済み)
  │
  ▼  前処理
  │    \hspace{...}\vdots → \vdots に正規化 (LaTeX spacing コマンドを除去)
  │    \n\n で分割してブロック列にする
  │
  ▼  Phase 2 早期検出 (ok: false にフォールバック)
  │    ブロック[0] に \text{query} / \mathrm{Query} → クエリ型
  │    ブロック数 > 1 かつブロック[1] が "1 " / "2 " で始まる → クエリサブ形式
  │    ブロック数 > 1 かつブロック[0] が単一変数のみ → T-testcases 型
  │
  ▼  Lexer  (ブロック[0] を行単位でトークン列化)
  │    IDENT       変数名 (大文字・小文字・複合: A, N, ra)
  │    NUM         数字リテラル (1, 2, ...)
  │    SUBSCRIPT   _ (添字開始)
  │    LBRACE/RBRACE  { }
  │    COMMA       ,
  │    CDOTS       \ldots \dots \cdots ...
  │    VDOTS       \vdots : ⋮  (正規化済み)
  │    NEWLINE
  │    SPACE
  │
  ▼  Parser  (行パターンマッチ)
  │    スカラー列 / 1D配列(cdots) / vdots → ForLoop
  │    添字が非数値 (アルファベット) → Phase 2 → ok: false
  │    空白なし隣接要素 (S_{1,1}S_{1,2}) → Phase 2 → ok: false
  │
  ▼  Semantic Analysis
  │    変数テーブル (dim / size 解決)
  │    制約テキストから型推定
  │    subscript → loop_var / begin / end 解決 (0-indexed に正規化)
  │
  ▼  InputSpec  (vars + ops)
```

#### 変数名の小文字化規則

- 数学表記の変数名を小文字化してコード変数名 (`name`) とする
  - 例: `N` → `name: "n"`, `math: "N"`
- **衝突時**: 同じコンテキスト内に `N` と `n` が別変数として登場する場合は、大文字の方をそのまま残す
  - 例: `N` と `n` が両方出現 → `name: "N"`, `name: "n"` (小文字化しない)
- 非数値の添字を持つ表記 (`S_X`, `S_Y` など) は現状未対応で、パースは失敗する (`ok: false`)

#### ループのインデックス正規化

添字が 1-origin (`A_1 ... A_N`) であっても、loop は常に 0-indexed に正規化する:
- `loop_begin`: `begin: "0"`, `end: "n"` (大文字の N → 小文字化済み)
- `read_line` 内 VarRef の `index` はループ変数名 (`"i"`, `"j"`, ...)

#### `vars` の形式 (JSON 例)

```json
[
  { "name": "n",  "math": "N",  "var_type": "int", "dim": 0, "size": [],    "is_size": true  },
  { "name": "k",  "math": "K",  "var_type": "int", "dim": 0, "size": [],    "is_size": false },
  { "name": "a",  "math": "A",  "var_type": "int", "dim": 1, "size": ["n"], "is_size": false }
]
```

- `var_type`: `"int"` | `"str"` | `"unknown"`
- `dim`: `0` = スカラー, `1` = 1D配列 (Phase 1 上限)
- `size`: dim ごとのサイズ式 (小文字化済み変数名)
- `is_size`: 他の var の `size` または `loop_begin` の `end` に自分の `name` が現れるなら `true`。テンプレートで `usize` / `Vec<T>` の型決定に使用する

#### `ops` の形式 (JSON 例)

```json
[
  { "tag": "read_line", "depth": 0,
    "vars": [{"name":"n","dim":0}, {"name":"k","dim":0}] },
  { "tag": "read_line", "depth": 0,
    "vars": [{"name":"a","dim":1,"size":"k"}] }
]
```

ループあり (abc334-F 相当) — Phase 2 以降は多変数ループも `ok: true` で返し、`loop_begin`/`loop_end` を含む ops がそのまま Tera コンテキストに渡される。テンプレートがループコードを生成する。

```json
[
  { "tag": "read_line",  "depth": 0, "vars": [{"name":"n"},{"name":"k"}] },
  { "tag": "read_line",  "depth": 0, "vars": [{"name":"sx"},{"name":"sy"}] },
  { "tag": "loop_begin", "depth": 0, "loop_var":"i","begin":"0","end":"n" },
  { "tag": "read_line",  "depth": 1, "vars": [{"name":"x","dim":1,"index":"i"},{"name":"y","dim":1,"index":"i"}] },
  { "tag": "loop_end",   "depth": 0 }
]
```

VarRef フィールド:
- `dim == 0` (スカラー): `{"name":"n","dim":0}`
- `dim == 1`, 一括読み (水平 cdots): `{"name":"a","dim":1,"size":"n"}` — 1行を split して配列全体を読む
- `dim == 1`, 要素読み (ループ内): `{"name":"a","dim":1,"index":"i"}` — `a[i]` を1つ読む

#### 型推定 (制約テキストから)

制約テキストを走査し、以下のヒューリスティックで `vars[*].var_type` を設定する:

| 制約テキストのパターン | 結果 |
| --- | --- |
| `整数` / `integers` が出現 | 対象変数を `int` |
| `\leq` / `<` / `≤` が変数に直接かかる | 対象変数を `int` |
| `文字列` / `string` が出現 | 対象変数を `str` |
| `英小文字` / `英大文字` / `lowercase` / `uppercase` が出現 | 対象変数を `str` |
| `All input values are integers` | 全変数を `int` |
| (マッチなし) | `unknown` |

#### Phase 1 対応パターン

実際の AtCoder 問題で確認したパターン:

| パターン | 例 | 確認問題 |
| --- | --- | --- |
| スカラー列 | `N M K` | abc334-A,B |
| 1D 配列 (水平 cdots) | `A_1 A_2 \ldots A_N` | abc334-C, abc360-C |
| 複数配列 (水平) | `A_1 \ldots A_N` + `W_1 \ldots W_N` | abc360-C |
| 単独文字列 | `S` (型推定で `str`) | abc360-A |
| 1D 配列 (垂直 vdots, 単一変数) | `S_1` / `\vdots` / `S_N` | abc246-F |
| `:` 区切り (垂直 vdots 等価) | `S_1` / `:` / `S_H` | ukuku09-C |
| 複数変数ループ (`\vdots`) | `t_1 k_1` / `\vdots` / `t_Q k_Q` | abc242-D |

**前処理**: `\hspace{0.4cm}\vdots` は `\vdots` に正規化する。また `:` のみの行も `\vdots` と等価に扱う (トークナイザーレベルで正規化)。

**単一変数ループのフラット化**: `\vdots` または `:` で囲まれたブロックが「1行1変数」の繰り返しのみで構成される場合、`[T; N]` の一括読み込みに変換する (`flatten_single_var_loops`)。

**複数変数ループのコード生成**: 1行複数変数のループは `Vec::new()` 宣言 + `for _ in 0..N { input!{...} ... .push(...) }` を生成する。テストハーネスではループ入力は `panic!` を生成するため、サンプルテストは手動で記述する。

#### Phase 1 非対応 → `ok: false` にフォールバック

実際の AtCoder 問題で確認:

| 非対応パターン | 例 | 確認問題 |
| --- | --- | --- |
| クエリ型 (複数 pre ブロック + `\text{query}`) | `Q\nquery_1\n\vdots` | abc241-D, typical90-L |
| クエリ型 (`\mathrm{Query}`) | `\mathrm{Query}_1` | abc248-D |
| T-testcases 型 (pre[0]=`T`, pre[1]=形式) | `T\n\n a s` | abc238-D |
| 空白なし文字グリッド | `S_{11}...S_{1W}` / `:` | abc151-D, abc176-D |
| 可変長行 (サイズが変数) | `T_i K_i A_{i,1} \ldots A_{i,K_i}` | abc226-C |
| 斜め・上三角行列 | `A_{1,2} \cdots A_{1,2N}` / `\vdots` | abc236-D |
| 非数値添字スカラー | `A_x A_y` | abc246-E, abc176-D |
| ネストループ 2段以上 | — | — |

### テンプレート例 (Rust)

```
templates/rust/
  Cargo.toml.tera     ← [package] name = "{{problem.code}}-{{solution.name}}"
  src/main.rs.tera    ← 入力コード自動生成テンプレート
```

`Cargo.toml.tera` 例:
```toml
[package]
name = "{{problem.code}}-{{solution.name}}"
version = "0.1.0"
edition = "2021"
```

`src/main.rs.tera` 例 (proconio 使用):

`ok=true` のとき: `solve` を純粋な引数関数として生成し、`main` で `input!` → `solve(args...)` と呼ぶ。
`ok=false` のとき: フォールバックとして `solve` が `src` を受け取る従来スタイルを生成する。

```tera
use proconio::input;

{% if input_format.ok -%}
fn solve(
    {% for v in input_format.vars -%}
    {{ v.name }}: {% if v.is_size %}usize{% elif v.dim == 1 %}Vec<{% if v.var_type == "str" %}String{% else %}i64{% endif %}>{% elif v.var_type == "str" %}String{% else %}i64{% endif %},
    {% endfor -%}
) -> String {
    todo!()
}

fn main() {
    input! {
        {% for op in input_format.ops -%}
        {% if op.tag == "read_line" -%}
        {% for v in op.vars -%}
        {% set vd = input_format.vars | filter(attribute="name", value=v.name) | first -%}
        {% if v.dim == 0 -%}
        {{ v.name }}: {% if vd.is_size %}usize{% elif vd.var_type == "str" %}String{% else %}i64{% endif %},
        {% elif v.size -%}
        {{ v.name }}: [{% if vd.var_type == "str" %}String{% else %}i64{% endif %}; {{ v.size }}],
        {% endif -%}
        {% endfor -%}
        {% elif op.tag == "loop_begin" -%}
        // TODO: loop {{ op.loop_var }} in {{ op.begin }}..{{ op.end }} — write manually
        {% endif -%}
        {% endfor -%}
    }
    print!("{}", solve({{ input_format.vars | map(attribute="name") | join(sep=", ") }}));
}
{% else -%}
fn solve<R: std::io::BufRead>(src: &mut impl proconio::source::Source<R>) -> String {
    // TODO: input_format.ok = false — write input manually
    // raw: {{ input_format.raw }}
    todo!()
}

fn main() {
    use proconio::source::line::LineSource;
    use std::io::BufReader;
    let src = &mut LineSource::new(BufReader::new(std::io::stdin()));
    print!("{}", solve(src));
}
{% endif -%}

#[cfg(test)]
mod tests { ... }
```

**型対応表** (テンプレート内の判定ロジック):

| `dim` | `is_size` | `var_type` | Rust 型 |
|-------|-----------|------------|---------|
| 0 | true | any | `usize` |
| 0 | false | `"str"` | `String` |
| 0 | false | other | `i64` |
| 1 | false | `"str"` | `Vec<String>` |
| 1 | false | other | `Vec<i64>` |

## エラーケース

- テンプレートが存在しない言語を指定: `Error: unknown language "{lang}". Available: {templates/ 以下のディレクトリ一覧}` を表示して exit 1
- 問題取得失敗 (ログイン必要): `Error: not logged in. Run \`ce login\` first.` を表示して exit 1
- 問題取得失敗 (その他): エラー内容を表示して exit 1
- ディレクトリが既に存在する: 冪等性の節に従って動作する (エラーにはならない)

## 未決事項

### 入力形式パース非対応パターン (Phase 2)

以下のパターンは Phase 1 では `ok: false` にフォールバックする。  
詳細は `docs/spec.md` の「入力形式パース 未対応パターン」節を参照。

TDD: `ac/test/data/test_problems.yml` の問題を個別ページから取得してパーサーのテストケースに利用する。
各問題について `input_format_raw` の期待値と `InputSpec` の期待出力を fixtures として管理する。

### problem_id ヒントの完全実装 (Phase 2)

現在 `get_contest_meta` は `problem_id_hints: vec![]` を返す (推定のみ)。  
将来的にコンテストページのナビバードロップダウンを解析して、ABC/ARC 同時開催等の旧コンテストでも正しい problem_id を使えるようにする。  
インターフェース変更は不要 — `get_contest_meta` の実装を拡張するだけ。
