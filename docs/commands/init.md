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

| #   | URL                                            | 取得情報                           |
| --- | ---------------------------------------------- | ---------------------------------- |
| 1   | `https://atcoder.jp/contests/{id}`             | 開始時刻・problem_id ヒント (後述) |
| 2   | `https://atcoder.jp/contests/{id}/tasks_print` | 全問題タイトル・サンプル I/O       |

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

| タイミング          | 動作                                                      |
| ------------------- | --------------------------------------------------------- |
| 残り > 1 分         | 1 分ごとに現在時刻・開始時刻・残り時間を表示              |
| 1 分 ≥ 残り > 10 秒 | 1 秒ごとに残り時間を表示                                  |
| 残り ≤ 10 秒        | 1 秒ごとにポーリング開始 (問題一覧が取れたら開始とみなす) |

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
  input fmt   a:plain  b:plain  c:loop  d:query(3)  e:iter  f:FAIL  [5/6 ok]
```

`{lang}` は使用した言語名 (例: `rust`)。

### input fmt 行の形式

各問題コードの後にコロンと format kind ラベルを付けて空白区切りで並べる。末尾に `[{ok件数}/{全件数} ok]` を付ける。

#### format kind ラベル

`InputSpec` のフィールドから以下の優先順で決定する:

| 優先 | 条件 | ラベル |
| ---- | ---- | ------ |
| 1 | `ok = false` | `FAIL` (大文字) |
| 2 | `query_types` が非空 | `query({n})` — n は種別数 |
| 3 | `query_body` が非空 | `query` |
| 4 | `testcase_body` が非空 | `testcase` |
| 5 | `triangular` が非空 | `triangle` |
| 6 | `ops` に `loop_jagged` を含む | `jagged` |
| 7 | `iteration_ops` が非空 | `iter` |
| 8 | `ops` に `loop_begin` を含む | `loop` |
| 9 | それ以外 (`ok=true`、ループなし) | `plain` |

`input_format_raw` が取得できなかった問題は `ok=false` 扱いとして `FAIL` を表示する。

## テンプレートシステム

`templates/{lang}/` の内容を `solutions/{contest_id}/{problem_code}/{solution_name}/` に展開する。
言語はユーザーが `templates/` 以下にディレクトリを追加することで自由に定義できる。

### Tera レンダリング

- `.tera` 拡張子: ファイル名・内容ともに Tera でレンダリングし、`.tera` を除いたパスで保存
  - 例: `Cargo.toml.tera` → `Cargo.toml`
- それ以外: 静的コピー

### Tera コンテキスト

| 変数                       | 内容                                                                            | ソース               |
| -------------------------- | ------------------------------------------------------------------------------- | -------------------- |
| `contest.id`               | コンテスト ID (例: `abc334`)                                                    | ドメインオブジェクト |
| `problem.code`             | 問題コード (例: `a`)                                                            | ドメインオブジェクト |
| `problem.title`            | 問題タイトル                                                                    | ドメインオブジェクト |
| `solution.name`            | 解法名 (例: `main`)                                                             | ドメインオブジェクト |
| `input_format.raw`         | 入力形式の生テキスト (常に設定、取得できなければ空文字)                         | パーサー             |
| `input_format.ok`          | パース成功フラグ (`bool`)                                                       | パーサー             |
| `input_format.vars`        | 変数宣言リスト (詳細後述)                                                       | パーサー             |
| `input_format.ops`         | 読み取り命令列 (詳細後述)                                                       | パーサー             |
| `input_format.query_types`    | クエリ種別リスト (詳細後述、クエリ型以外は空リスト)                                                                                                    | パーサー             |
| `input_format.query_body`     | 単一形式ループの変数リスト (スカラーのみの非数値先頭 sub-block、`query_types` 非空または `iteration_ops` 非空のときは空)                                 | パーサー             |
| `input_format.testcase_body`  | 簡易 T-testcases 型の本体変数リスト (ブロック[0]=単一スカラー かつ ブロック[1] がスカラーのみ、それ以外は空リスト)                                       | パーサー             |
| `input_format.triangular`     | 三角行列仕様 (`{name, math, var_type, bound}`)。三角行列型以外は `null`。`triangular` が非空のとき `vars`/`ops` は size 変数のみ含む                    | パーサー             |
| `input_format.iteration_vars` | 複雑な繰り返し本体の変数リスト (最初に採用された非数値 sub-block にループ・配列が含まれる場合。`query_types`/`query_body`/`testcase_body` が非空のときは空) | パーサー             |
| `input_format.iteration_ops`  | 複雑な繰り返し本体の読み取り命令列 (`vars`・`ops` と同形式。`query_types`/`query_body`/`testcase_body` が非空のときは空。テンプレートが for ループ内でレンダリングする) | パーサー             |

`input_format.ok` が `false` のとき `vars`・`ops`・`query_types`・`query_body`・`testcase_body`・`iteration_vars`・`iteration_ops` は全て空リスト (`ops` に `loop_jagged` が含まれることもない)。テンプレートは `{% if input_format.ok %}` で分岐する。

### 入力形式パース

`.ce.toml` に保存した `input_format_raw` を `SolutionRepository::create` 時に毎回オンザフライでパースし、Tera コンテキストに注入する。パース処理は `usecases/input_format/` モジュールが担う。

#### パイプライン

```
input_format_raw (文字列、複数 pre ブロックは \n\n 区切りで結合済み)
  │
  ▼  前処理
  │    \hspace{...}\vdots → \vdots に正規化 (LaTeX spacing コマンドを除去)
  │    … (U+2026 HORIZONTAL ELLIPSIS) → \ldots に正規化
  │    CDOTS トークンのみで構成される行 (前後 SPACE を無視) → VDOTS として正規化
  │      ※ \ldots を縦区切りとして使う問題 (abc451_e 等) に対応
  │    \n\n で分割してブロック列にする
  │
  ▼  Phase 2 早期検出
  │    ブロック数 == 1 かつブロック[0] が三角行列パターン:
  │      行[0]: Ident 1 個 (size 変数)
  │      行[1]: TriangularRow (同名変数の `Var_{1,col}...(cdots)...Var_{1,bound}`)
  │      行[2]: TriangularRow (first_idx=2) または VDOTS (省略形)
  │      VDOTS を経て最終行: 同名変数の単一要素 (comma 添字、no cdots)
  │      → TriangularMatrix として解析 (ok: true):
  │          triangular = { name (lower), math, var_type (制約推定), bound (normalize_expr 済み) }
  │          vars = [size 変数 VarDecl (is_size: true)]
  │          ops = [ReadLine(size 変数)]
  │      ※ TriangularRow 判定: 同名変数・先頭添字が Num・CDOTS あり
  │      ※ bound: 行[1] の最後の要素の第 2 添字を normalize_expr して取得
  │      ※ var_type: 制約テキストから推定 (デフォルト "int")。GridRow と異なり "str" に固定しない
  │    ブロック数 > 1 かつブロック[1] が数字始まり かつ ブロック[0] に loop marker なし
  │      → 非対応クエリサブ形式 (typical90-L の q_i パターン等)
  │    ブロック数 > 1 かつブロック[0] が単一 Ident トークン かつ loop marker なし
  │      → 簡易 T-testcases 型 → ブロック[1] をスカラー変数リストとしてパース
  │        成功 → testcase_body = [VarDecl, ...]; ブロック[0] の変数は is_size: true に設定
  │        失敗 → testcase_body = [] (テンプレートが TODO を生成)
  │    ※ ブロック[0] に loop marker がある場合は早期検出をスキップ
  │
  ▼  Lexer  (ブロック[0] を行単位でトークン列化)
  │    IDENT       変数名 (大文字・小文字・複合: A, N, ra)
  │    NUM         数字リテラル (1, 2, ...)
  │    SUBSCRIPT   _ (添字開始)
  │    LBRACE/RBRACE  { }
  │    COMMA       ,
  │    PLUS        +
  │    MINUS       -
  │    STAR        *
  │    CDOTS       \ldots \dots \cdots ...
  │    VDOTS       \vdots : ⋮  (正規化済み)
  │    NEWLINE
  │    SPACE
  │
  ▼  Parser  (行パターンマッチ)
  │    スカラー列 / 1D配列(cdots) / vdots → ForLoop
  │    \text{X}_N / \mathrm{X}_N (X は任意文字列) → QueryLine (vdots ブロック内のみ有効)
  │    query_N (大文字小文字不問、N はループ変数) → QueryLine (vdots ブロック内のみ有効)
  │    {query}_N (大文字小文字不問; {\rm Query}_N 等 LaTeX 書式コマンドでラップされた形を含む) → QueryLine (vdots ブロック内のみ有効)
  │      ※ \rm 等の未知 LaTeX コマンドはトークナイザーで無視されるため {query}_N に等価になる
  │      ※ {X}_N 一般ではなく Ident が "query" に一致する場合のみ QueryLine として扱う
  │      ※ 問題文に「クエリ」「テストケース」のどちらが書かれているかは無関係
  │      ※ 上記以外の plain-text 添字 (q_i など) は QueryLine とみなさない
  │    cdots 伴う 1D 配列で添字がすべてアルファベット → Phase 2 → ok: false
  │    1D 固定サイズ (no cdots): 同名 Ident+数値添字が 2 個以上 スペース区切り 連番 → Array1D(size=literal)
  │    2D 固定グリッド行: 同名 Ident_{Num,Num} が 2 個以上 col 添字連番 row 添字固定 → Array2DRow
  │    GridRow (文字グリッド行): 同名 Ident_{2D_left} [同名 Ident_{2D}]* Cdots 同名 Ident_{2D_right}
  │      ※ 先頭要素が 1 個の既存形式 (S_{11} Cdots S_{1W}) も引き続き対象
  │      ※ 先頭に 2D 添字要素が 2 個以上ある拡張形式 (スペースあり: S_{1,1} S_{1,2} \ldots S_{1,W}、スペースなし: S_{1,1}S_{1,2}\ldots S_{1,W}) も対象
  │      ※ cdots 前の追加要素は名前・行インデックス (2D 添字の先頭パーツ) が全要素で一致していること
  │      ※ 追加要素間のスペースは任意 (あり・なし両方対応)
  │    JaggedRow: vdots ボディの最終行の右端要素が `{row_idx, SIZE_VAR_{row_idx}}` 添字を持つ場合
  │      ※ 右端添字の形式: LBRACE + row_idx(Num|Ident) + COMMA + Ident(SIZE_VAR) + SUBSCRIPT + row_idx(同値) + RBRACE
  │      ※ GridRow の `{row_idx, Ident}` (SIZE_VAR が添字なし) とは別パターン
  │      → SIZE_VAR を jagged サイズ変数として抽出
  │
  ▼  多行グルーピング (block_to_ops)
  │    連続する Array2DRow: 同名・row 添字が 1 始まり連番 → VarDecl(dim=2) + ReadLine(dim=2) 1 命令
  │    (テンプレートは `[[T; cols]; rows]` として proconio で一括読み)
  │    vdots ボディ末尾行が JaggedRow → loop_jagged op を生成 (QueryLine より優先)
  │      ボディ = vdots より後に続く行群 (1 行以上)。先行する vdots 前の繰り返し行は無視
  │      ボディの最終行: JaggedRow (cdots 含む、右端添字が `{row_idx, SIZE_VAR_{row_idx}}`)
  │      ボディの残り: JaggedRow 行の JaggedRow より前のトークン + ボディの非最終行すべて → スカラー列
  │        スカラー列には loop-subscript スカラー (loop 変数で添字付き) のみ許容
  │        SIZE_VAR がスカラー列のいずれかの変数名と一致しなければ ok: false
  │      loop bound: vdots 末尾行の row subscript (Ident 部分) から決定。normalize_name 済み
  │
  ▼  Semantic Analysis (ブロック[0])
  │    変数テーブル (dim / size 解決)
  │      VarDecl.size には算術式文字列 (例: `"2*n"`, `"n-1"`) が入ることがある
  │    is_size 計算: VarDecl.size 要素と LoopBegin.end から識別子を抽出して判定
  │      算術式 (`"2*n"`, `"n-1"` 等) は英字部分を抽出して対象変数を特定する
  │      例: `"2*n"` → `n` が is_size=true; `"n-1"` → `n` が is_size=true
  │      loop_jagged の size_var に一致する変数も is_size=true (dim=1 でも同様)
  │        → テンプレートで dim=1, is_size=true → Vec<usize> として生成
  │    is_jagged 計算: loop_jagged の elem_var に一致する変数の is_jagged を true に設定
  │        → テンプレートで dim=1, is_jagged=true → Vec<Vec<T>> として生成
  │    valid_loop_bounds 検証: LoopBegin.end が以下のいずれかならOK、それ以外は ok: false
  │      1. 全桁数字リテラル (例: `"3"`)
  │      2. 宣言済みスカラー変数名 (例: `"n"`, `"q"`)
  │      3. 算術式 — 式中の全ての識別子が宣言済みスカラー変数 (例: `"n-1"`, `"2*n"`, `"2*n-1"`)
  │      ※ loop_jagged の end も同条件で検証
  │    制約テキストから型推定
  │    subscript → loop_var / begin / end 解決 (0-indexed に正規化)
  │    QueryLine vdots ブロック → LoopBegin(end=<loop_bound>) + LoopEnd (body なし)
  │
  ▼  繰り返し本体 (sub-block) 解析 (ブロック[0] に loop marker がある場合のみ)
  │    loop marker の判定: block0 のいずれかの行が QueryLine としてパースされること (parse_line と同一ルール)
  │    ブロック[1..] を順に解析して query_types / query_body / iteration_vars / iteration_ops を構築する
  │    各 sub-block について:
  │      空 sub-block → スキップ
  │      先頭行の先頭トークンが NUM → numbered sub-block (query_types に追加)
  │        type_id = そのトークン
  │        残りトークン (先頭行の残り + 後続行) をスカラー変数リストとして解析
  │          変数名の小文字化・衝突解決はメイン vars と独立して実施
  │          型推定は constraints テキストを用いてメインと同一推定器を適用
  │        パース成功 → QueryTypeDecl { type_id, ok: true, vars }
  │        変数解析失敗 → QueryTypeDecl { type_id, ok: false, vars: [] }
  │      先頭行の先頭トークンが NUM でない → single-format sub-block
  │        前提条件: query_types = [] かつ query_body = [] かつ iteration_vars = [] のとき のみ処理
  │          (いずれかが既に埋まっている場合は当該 sub-block を完全スキップ — ステップ 2 にも進まない)
  │        ステップ 1: スカラー変数リストとして解析を試みる (query_body に格納)
  │          ※ 例: abc334_d の "X" → x: i64 として query_body = [x]
  │          全行を連結してスカラー変数リスト (Scalars のみ; LoopRow は不可) として解析
  │          パース成功 → query_body = [VarDecl, ...]
  │          パース失敗 → ステップ 2 へ
  │        ステップ 2 (スカラーパース失敗時): 完全 InputSpec としてパース (iteration_vars / iteration_ops に格納)
  │          ※ 例: abc456_f の "N K\nA_1 ... A_N"、abc456_e の複数ループ含む本体
  │          当該 sub-block を単独で再帰パース (ブロック分割なし)
  │          パース成功 (ok=true) → iteration_vars = mini_spec.vars, iteration_ops = mini_spec.ops
  │          パース失敗 (ok=false) → iteration_vars = [], iteration_ops = []
  │            (テンプレートは solve にループスタブのみ生成)
  │          iteration_vars / iteration_ops はメイン vars と独立したスコープ
  │          型推定はメインと同一の constraints テキストを使用
  │
  │    すべて空の場合:
  │      query_types = [], query_body = [], iteration_vars = [], iteration_ops = [] のまま。
  │      テンプレートは solve にループスタブ (TODO) を生成する。
  │
  ▼  InputSpec  (vars + ops + query_types + query_body + testcase_body + iteration_vars + iteration_ops)
```

#### 変数名の小文字化規則

- 数学表記の変数名を小文字化してコード変数名 (`name`) とする
  - 例: `N` → `name: "n"`, `math: "N"`
- **衝突時**: 同じコンテキスト内に `N` と `n` が別変数として登場する場合は、大文字の方をそのまま残す
  - 例: `N` と `n` が両方出現 → `name: "N"`, `name: "n"` (小文字化しない)
- 非数値の添字を持つ表記 (`S_X`, `S_Y` など) は現状未対応で、パースは失敗する (`ok: false`)

#### 添字の分類ルール

`A_x` 形式 (単純添字) および `A_{...}` 形式 (ブレース添字) の分類:

| 添字形式                       | 例                       | 分類                   | 結果                                    |
| ------------------------------ | ------------------------ | ---------------------- | --------------------------------------- |
| 単一英字                       | `_i`, `_j`               | ループ変数添字         | `LoopRow` / `Array1D` のインデックス    |
| 単一数値                       | `_1`, `_3`               | 1D 数値添字            | `Array1D` または `Scalars` の要素       |
| `{Num}`                        | `_{1}`, `_{12}`          | 1D 数値添字            | 同上                                    |
| `{Num Ident}` (隣接)           | `_{2N}`, `_{3M}`         | 算術式添字             | `*` を自動挿入 → 構築直後 `"2*N"`, 正規化後 `"2*n"` — 配列サイズ・ループ上限として使用 |
| `{Ident OP Num}` / `{Ident OP Ident}` | `_{N-1}`, `_{N+1}`, `_{M-2}` | 算術式添字 | 演算子を保持 → 構築直後 `"N-1"`, 正規化後 `"n-1"` — 配列サイズ・ループ上限として使用 |
| `{Num OP Ident}` など          | `_{2N-1}`                | 算術式添字 (複合)      | 構築直後 `"2*N-1"`, 正規化後 `"2*n-1"` — 同上 |
| `{Ident Num}` (隣接)           | `_{N2}` など             | 非対応                 | lexer が `N2` を単一 `Ident("N2")` として読むため `*` 挿入は行われない → `ok: false` |
| `{Num,Num}`                    | `_{1,1}`, `_{2,6}`       | 2D numeric 添字        | `Array2DRow` 行・列インデックス         |
| `{Num,Ident}` / `{Ident,Num}`  | `_{1,W}`, `_{H,1}`       | GridRow 範囲式の端点   | `GridRow` 検出 (文字グリッド) で対応済み |
| `{Ident,Ident}` など           | `_{i,j}`, `_{H,W}`       | 非対応 (Phase 2)       | `ok: false`                             |

**算術式添字の構築ルール** (`{...}` ブランチ内):
- `PLUS`(`+`) / `MINUS`(`-`) / `STAR`(`*`) はそのまま式文字列に連結
- 隣接する `NUM IDENT` (演算子なし) → `*` を自動挿入 (`2N` → `"2*N"`)
  - ※ `IDENT NUM` 隣接 (`N2` 等) は lexer が英数字連続を単一 `Ident("N2")` として読むため実際には発生しない
- IDENT は元の大文字・小文字を保持したまま返す。小文字化は後述の名前解決フェーズ (`normalize_expr`) で行われる
  - `normalize_expr`: 式文字列中の各 IDENT に対して個別に `normalize_name` を適用する
  - 衝突回避 (`N` と `n` が共存する場合は大文字を保持) も `normalize_name` 経由で正しく反映される
  - 例: 衝突なし → `"2*N-1"` → `"2*n-1"`, 衝突あり (`N`/`n` 共存) → `"2*N-1"` → `"2*N-1"`
- カンマを含む場合は算術式としてではなく 2D 添字として扱う (上記カンマ添字ルールが優先)

**カンマ添字の文脈依存性**: カンマ添字の解釈はコンテキストによって異なる。

- **`GridRow` (文字グリッド)**: `S_{1,W}...S_{H,W}` のように `cdots` を伴う範囲式として検出される。Ident を含むカンマ添字が許容され、`GridRow` → `[String; N]` として読む
- **`Array2DRow` (固定 2D グリッド)**: `{Num,Num}` のみ対応。`cdots` なしで同名変数の col が連番するパターン
- **それ以外のコンテキスト**: カンマ添字は全て拒否 (`ok: false`)

#### ループのインデックス正規化

添字が 1-origin (`A_1 ... A_N`) であっても、loop は常に 0-indexed に正規化する:

- `loop_begin`: `begin: "0"`, `end: "n"` (大文字の N → 小文字化済み)
- `read_line` 内 VarRef の `index` はループ変数名 (`"i"`, `"j"`, ...)

#### `vars` の形式 (JSON 例)

```json
[
  {
    "name": "n",
    "math": "N",
    "var_type": "int",
    "dim": 0,
    "size": [],
    "is_size": true
  },
  {
    "name": "k",
    "math": "K",
    "var_type": "int",
    "dim": 0,
    "size": [],
    "is_size": false
  },
  {
    "name": "a",
    "math": "A",
    "var_type": "int",
    "dim": 1,
    "size": ["n"],
    "is_size": false
  }
]
```

- `var_type`: `"int"` | `"str"` | `"unknown"`
- `dim`: `0` = スカラー, `1` = 1D 配列, `2` = 2D 固定グリッド
- `size`: dim ごとのサイズ式リスト (小文字化済み変数名またはリテラル数値文字列)
  - dim=0: `[]`
  - dim=1: `["n"]` (可変) または `["3"]` (固定リテラル)
  - dim=2: `["6", "3"]` — `[cols, rows]` の順。両要素とも固定リテラル
- `is_size`: 他の var の `size`、`loop_begin` の `end`、または `loop_jagged` の `size_var` に自分の `name` が現れるなら `true`。dim=0 → `usize`、dim=1 → `Vec<usize>` の型決定に使用する。固定リテラルサイズの場合は `is_size` は持たない (変数ではないため)
- `is_jagged`: `loop_jagged` の `elem_var` に一致する場合 `true` (デフォルト `false`)。dim=1 かつ `is_jagged=true` → `Vec<Vec<T>>`

#### `ops` の形式 (JSON 例)

```json
[
  {
    "tag": "read_line",
    "depth": 0,
    "vars": [
      { "name": "n", "dim": 0 },
      { "name": "k", "dim": 0 }
    ]
  },
  {
    "tag": "read_line",
    "depth": 0,
    "vars": [{ "name": "a", "dim": 1, "size": "k" }]
  }
]
```

ループあり (abc334-F 相当) — Phase 2 以降は多変数ループも `ok: true` で返し、`loop_begin`/`loop_end` を含む ops がそのまま Tera コンテキストに渡される。テンプレートがループコードを生成する。

```json
[
  {
    "tag": "read_line",
    "depth": 0,
    "vars": [{ "name": "n" }, { "name": "k" }]
  },
  {
    "tag": "read_line",
    "depth": 0,
    "vars": [{ "name": "sx" }, { "name": "sy" }]
  },
  {
    "tag": "loop_begin",
    "depth": 0,
    "loop_var": "i",
    "begin": "0",
    "end": "n"
  },
  {
    "tag": "read_line",
    "depth": 1,
    "vars": [
      { "name": "x", "dim": 1, "index": "i" },
      { "name": "y", "dim": 1, "index": "i" }
    ]
  },
  { "tag": "loop_end", "depth": 0 }
]
```

VarRef フィールド:

- `dim == 0` (スカラー): `{"name":"n","dim":0}`
- `dim == 1`, 一括読み (水平 cdots または固定サイズ): `{"name":"a","dim":1,"size":"n"}` または `{"name":"a","dim":1,"size":"3"}` — 1 行を split して配列全体を読む
- `dim == 1`, 要素読み (ループ内): `{"name":"a","dim":1,"index":"i"}` — `a[i]` を 1 つ読む
- `dim == 2` (2D 固定グリッド): `{"name":"a","dim":2}` — サイズは VarDecl.size[0] (cols) と size[1] (rows) から参照。テンプレートは `a: [[T; cols]; rows]` を生成する

ジャギー配列あり (`loop_jagged`) — `loop_begin`/`loop_end` の代わりに `loop_jagged` タグ 1 命令で表現する。`read_line` では表現できないため専用タグを使う。

```json
[
  {
    "tag": "read_line",
    "depth": 0,
    "vars": [{ "name": "n", "dim": 0 }]
  },
  {
    "tag": "loop_jagged",
    "depth": 0,
    "end": "n",
    "scalars": [{ "name": "t", "dim": 0 }],
    "size_var": { "name": "k", "dim": 0 },
    "elem_var": { "name": "a", "dim": 0 }
  },
  {
    "tag": "read_line",
    "depth": 0,
    "vars": [{ "name": "x", "dim": 0 }, { "name": "y", "dim": 0 }]
  }
]
```

`loop_jagged` フィールド:

- `end`: ループ上限 (normalize_name 済みスカラー変数名。宣言済みスカラーであること)
- `scalars`: ボディ内の SIZE_VAR 以外のスカラー VarRef 列 (0 個以上、出現順)
- `size_var`: ジャギーサイズ変数の VarRef (SIZE_VAR。`is_size=true` に設定済み)
- `elem_var`: ジャギー配列変数の VarRef (`is_jagged=true` に設定済み)

テンプレートが生成するコード (abc226_c 形式: `scalars=[t], size_var=k, elem_var=a`):

```rust
let mut t: Vec<i64> = Vec::new();
let mut k: Vec<usize> = Vec::new();
let mut a: Vec<Vec<i64>> = Vec::new();
for _ in 0..n {
    input! { ti: i64, ki: usize, }
    input! { ai: [i64; ki], }
    t.push(ti);
    k.push(ki);
    a.push(ai);
}
```

abc457_b 形式 (`scalars=[], size_var=l, elem_var=a`、末尾 `X Y`):

```rust
let mut l: Vec<usize> = Vec::new();
let mut a: Vec<Vec<i64>> = Vec::new();
for _ in 0..n {
    input! { li: usize, }
    input! { ai: [i64; li], }
    l.push(li);
    a.push(ai);
}
input! { x: i64, y: i64, }
```

proconio の LineSource では `input!` を連続呼び出しすると同一行の残りトークンを続けて消費するため、同行レイアウト (abc457_b、abc226_c) と改行レイアウト (abc446_b) で生成コードは共通になる。

#### `query_types` の形式 (JSON 例)

クエリ型入力 (`\text{query}_Q` 形式) のとき、`blocks[1..]` を解析して構築される。それ以外は空リスト `[]`。

入力例:

```
N Q
\text{query}_1
\vdots
\text{query}_Q

1 x

2 x k

3 l r
```

生成される `query_types`:

```json
[
  {
    "type_id": "1",
    "ok": true,
    "vars": [{ "name": "x", "var_type": "int", "dim": 0, "is_size": false }]
  },
  {
    "type_id": "2",
    "ok": true,
    "vars": [
      { "name": "x", "var_type": "int", "dim": 0, "is_size": false },
      { "name": "k", "var_type": "int", "dim": 0, "is_size": false }
    ]
  },
  {
    "type_id": "3",
    "ok": true,
    "vars": [
      { "name": "l", "var_type": "int", "dim": 0, "is_size": false },
      { "name": "r", "var_type": "int", "dim": 0, "is_size": false }
    ]
  }
]
```

`ok: false` の例 (sub-block が解析できなかった場合):

```json
{ "type_id": "2", "ok": false, "vars": [] }
```

- `type_id`: sub-block 先頭の数字トークン
- `ok`: sub-block の解析成功フラグ (false のときテンプレートが TODO を生成)
- `vars`: このクエリ種別のローカル変数。`dim` は常に 0 (スカラー); `is_size` は常に `false`
- 変数の命名規則・型推定はメイン `vars` と同一ルールを適用するが、スコープは独立

#### `query_body` の形式 (JSON 例)

単一形式クエリ (`query_types = []`) かつ非数値先頭 sub-block が存在するとき、その変数リスト。それ以外は空リスト `[]`。

入力例 (abc334_d 形式):

```
N Q
R_1 \ldots R_N
\text{query}_1
\vdots
\text{query}_Q

X
```

生成される `query_body`:

```json
[{ "name": "x", "math": "X", "var_type": "int", "dim": 0, "is_size": false }]
```

- `dim` は常に 0 (スカラー); `is_size` は常に `false`
- 変数の命名規則・型推定はメイン `vars` と同一ルールを適用するが、スコープは独立

#### `testcase_body` の形式 (JSON 例)

T-testcases 型 (ブロック[0] = 単一スカラー `T`, ブロック[1] = 各テストケースの入力形式) のとき、ブロック[1] をスカラーとして解析した変数リスト。それ以外は空リスト `[]`。

入力例 (abc238-D 形式):

```
T

a s
```

生成される `testcase_body`:

```json
[
  { "name": "a", "math": "a", "var_type": "int", "dim": 0, "is_size": false },
  { "name": "s", "math": "s", "var_type": "str", "dim": 0, "is_size": false }
]
```

生成される `vars`:

```json
[{ "name": "t", "math": "T", "var_type": "int", "dim": 0, "is_size": true }]
```

- `dim` は常に 0 (スカラー)。ブロック[1] がスカラー以外を含む場合は `testcase_body = []` にフォールバック
- ループ変数 (ループ上限) は `vars[0].name` (例: `t`)。テンプレートは `for _ in 0..t { ... }` を生成
- 変数の命名規則・型推定はメイン `vars` と同一ルールを適用するが、スコープは独立

#### `triangular` の形式 (JSON 例)

上三角行列型のとき。それ以外は `null`。

入力例 (abc451_e 形式):

```
N
A_{1, 2} A_{1, 3} \ldots A_{1, N}
A_{2, 3} \ldots A_{2, N}
\vdots
A_{N-1,N}
```

生成される `triangular`:

```json
{ "name": "a", "math": "A", "var_type": "int", "bound": "n" }
```

生成される `vars`:

```json
[{ "name": "n", "math": "N", "var_type": "int", "dim": 0, "size": [], "is_size": true }]
```

生成される `ops`:

```json
[{ "tag": "read_line", "depth": 0, "vars": [{ "name": "n", "dim": 0 }] }]
```

入力例 (abc236_d 形式):

```
N
A_{1, 2} A_{1, 3} A_{1, 4} \cdots A_{1, 2N}
A_{2, 3} A_{2, 4} \cdots A_{2, 2N}
A_{3, 4} \cdots A_{3, 2N}
\vdots
A_{2N-1, 2N}
```

生成される `triangular`:

```json
{ "name": "a", "math": "A", "var_type": "int", "bound": "2*n" }
```

フィールド:

- `name`: 小文字化した変数名
- `math`: 元の表記 (大文字)
- `var_type`: 制約テキストから推定 (デフォルト `"int"`。GridRow と異なり `"str"` に固定しない)
- `bound`: 第 2 添字の上限式 (normalize_expr 済み)。ループ上限 = `{bound}-1`、行 i の長さ = `{bound}-1-_i`

テンプレートが生成する Rust コード (abc451_e 形式):

```rust
fn solve(n: usize, a: Vec<Vec<i64>>) -> String { ... }

fn main() {
    input! { n: usize, }
    let mut a: Vec<Vec<i64>> = Vec::new();
    for _i in 0..n-1 {
        input! { _row: [i64; n-1-_i], }
        a.push(_row);
    }
    print!("{}", solve(n, a));
}
```

テンプレートが生成する Rust コード (abc236_d 形式、bound = `"2*n"`):

```rust
fn main() {
    input! { n: usize, }
    let mut a: Vec<Vec<i64>> = Vec::new();
    for _i in 0..2*n-1 {
        input! { _row: [i64; 2*n-1-_i], }
        a.push(_row);
    }
    print!("{}", solve(n, a));
}
```

- `triangular` の変数はメイン `vars` には含まれない。`solve` 引数・`main` での読み取りはテンプレートが `triangular` フィールドを参照して別途生成する
- `solve` の引数順: `vars` の変数を先に並べ、`triangular.name` を末尾に追加
- `main` での `print!` 呼び出し: `solve({{ vars | map("name") | join(", ") }}, {{ triangular.name }})`

#### `iteration_vars` / `iteration_ops` の形式

ループマーカーがブロック[0] にあり、かつ最初に採用された非数値 sub-block がスカラーパースに失敗した (ループ・配列を含む) 場合に生成される。`query_types`・`query_body`・`testcase_body` のいずれかが非空のときは空リスト。

入力例 (abc456-F 形式: `T\n\mathrm{case}_T\n\nN K\nA_1 A_2 \ldots A_N`):

```
T
\mathrm{case}_1
\vdots
\mathrm{case}_T

N K
A_1 A_2 \ldots A_N
```

生成される `vars` (block[0]):

```json
[{ "name": "t", "math": "T", "var_type": "int", "dim": 0, "size": [], "is_size": true }]
```

生成される `ops` (block[0]):

```json
[
  { "tag": "read_line", "depth": 0, "vars": [{ "name": "t", "dim": 0 }] },
  { "tag": "loop_begin", "depth": 0, "loop_var": "i", "begin": "0", "end": "t" },
  { "tag": "loop_end",   "depth": 0 }
]
```

生成される `iteration_vars` (block[1]):

```json
[
  { "name": "n", "math": "N", "var_type": "int", "dim": 0, "size": [], "is_size": true },
  { "name": "k", "math": "K", "var_type": "int", "dim": 0, "size": [], "is_size": false },
  { "name": "a", "math": "A", "var_type": "int", "dim": 1, "size": ["n"], "is_size": false }
]
```

生成される `iteration_ops` (block[1]):

```json
[
  { "tag": "read_line", "depth": 0, "vars": [{ "name": "n", "dim": 0 }, { "name": "k", "dim": 0 }] },
  { "tag": "read_line", "depth": 0, "vars": [{ "name": "a", "dim": 1, "size": "n" }] }
]
```

- `iteration_vars` / `iteration_ops` の形式はメイン `vars` / `ops` と同一 (VarDecl / InputOp 形式)
- スコープはメイン vars と独立 — 変数名が衝突しても別変数として扱う
- 型推定はメインと同一の constraints テキストを使用
- `iteration_vars` に含まれる変数はメインの `solve` 引数に含めない (for ループ内でローカル宣言)
- ループ上限 (`query_loop_end`) は block[0] の ops から検出 (loop_begin → loop_end が空 body)

#### 2D 固定グリッドの形式 (JSON 例)

`A_{1,1} A_{1,2} A_{1,3} A_{1,4} A_{1,5} A_{1,6}` × 3 行 (abc456-B 形式) のとき、`dim=2` の `VarDecl` 1 件と `ops` 1 命令が生成される。(`...`/`\ldots` は不可: `try_parse_array2d_row` は Cdots を含む行を拒否する)

入力例:

```
A_{1,1} A_{1,2} A_{1,3} A_{1,4} A_{1,5} A_{1,6}
A_{2,1} A_{2,2} A_{2,3} A_{2,4} A_{2,5} A_{2,6}
A_{3,1} A_{3,2} A_{3,3} A_{3,4} A_{3,5} A_{3,6}
```

生成される `vars`:

```json
[{ "name": "a", "math": "A", "var_type": "int", "dim": 2, "size": ["6", "3"], "is_size": false }]
```

生成される `ops`:

```json
[{ "tag": "read_line", "depth": 0, "vars": [{ "name": "a", "dim": 2 }] }]
```

生成される Rust コード (main):

```rust
input! {
    a: [[i64; 6]; 3],
}
```

生成される solve 引数:

```rust
fn solve(a: Vec<Vec<i64>>) -> String { ... }
```

検出ルール:

- 連続する N 行 (N ≥ 2) が全て `Array2DRow { name, row_idx, col_count }` であること
- 全行の `name` が一致すること
- `row_idx` が `"1"` 始まり連番 (`"1"`, `"2"`, ..., `"N"`) であること
- 全行の `col_count` が等しいこと

`Array2DRow` 1 行の検出ルール:

- 行内の全トークンが `Ident_{Num,Num}` パターン (space 区切り)
- 全て同一 `name`, `row_idx` 固定, `col_idx` が `"1"` 始まり連番
- 要素数 ≥ 2

#### 型推定 (制約テキストから)

制約テキストを走査し、以下のヒューリスティックで `vars[*].var_type` を設定する:

| 制約テキストのパターン                                     | 結果             |
| ---------------------------------------------------------- | ---------------- |
| `整数` / `integers` が出現                                 | 対象変数を `int` |
| `\leq` / `<` / `≤` が変数に直接かかる                      | 対象変数を `int` |
| `文字列` / `string` が出現                                 | 対象変数を `str` |
| `英小文字` / `英大文字` / `lowercase` / `uppercase` が出現 | 対象変数を `str` |
| `All input values are integers`                            | 全変数を `int`   |
| (マッチなし)                                               | `unknown`        |

#### Phase 1 対応パターン

実際の AtCoder 問題で確認したパターン:

| パターン                                                                  | 例                                                                             | 確認問題           |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------------------ | ------------------ |
| スカラー列                                                                | `N M K`                                                                        | abc334-A,B         |
| 1D 配列 (水平 cdots, 可変サイズ)                                          | `A_1 A_2 \ldots A_N`                                                           | abc334-C, abc360-C |
| 1D 配列 (水平 cdots, 固定サイズ)                                          | `A_1 \ldots A_3`                                                               | —                  |
| 1D 配列 (no cdots, 固定サイズ)                                            | `A_1 A_2 A_3` (2 個以上・連番)                                                  | —                  |
| 複数配列 (水平)                                                           | `A_1 \ldots A_N` + `W_1 \ldots W_N`                                            | abc360-C           |
| 単独文字列                                                                | `S` (型推定で `str`)                                                           | abc360-A           |
| 1D 配列 (垂直 vdots, 単一変数)                                            | `S_1` / `\vdots` / `S_N`                                                       | abc246-F           |
| `:` 区切り (垂直 vdots 等価)                                              | `S_1` / `:` / `S_H`                                                            | ukuku09-C          |
| 複数変数ループ (`\vdots`)                                                 | `t_1 k_1` / `\vdots` / `t_Q k_Q`                                               | abc242-D           |
| 文字グリッド (2D 添字、先頭1要素)                                         | `S_{11}...S_{1W}` / `:` / `S_{H1}...S_{HW}`                                    | abc151-D, abc176-D |
| 文字グリッド (2D 添字、先頭複数要素・スペースあり)                           | `S_{1,1} S_{1,2} \ldots S_{1,W}` / `\vdots` / `S_{H,1} S_{H,2} \ldots S_{H,W}` | abc453-D           |
| 文字グリッド (2D 添字、先頭複数要素・スペースなし)                           | `S_{1,1}S_{1,2}\ldots S_{1,W}` / `\vdots` / `S_{H,1}S_{H,2}\ldots S_{H,W}`     | abc450-C           |
| 2D 固定グリッド (comma 添字, no cdots)                                    | `A_{1,1} A_{1,2} A_{1,3}` × 3 行 (dots 不可, dim=2, `[[T; 3]; 3]`)             | abc456-B           |
| 1D 配列 (算術式サイズ `{NumIdent}`)                                        | `A_1 A_2 \ldots A_{2N}` → `[i64; 2*n]`                                          | tupc2024-K         |
| 多変数ループ (`\vdots`) + 算術式添字 `{Ident-Num}`                          | `U_1 V_1` / `\vdots` / `U_{N-1} V_{N-1}` → `for _ in 0..n-1`                   | abc448-D           |
| 添字付きスカラー (アルファベット添字)                                     | `A_x A_y`                                                                      | abc246-E           |
| 添字付きスカラー (数値添字・vdots なし)                                   | `r_1 c_1` / `r_2 c_2` (各行独立)                                               | abc176-D           |
| ループマーカー付きクエリ型 (`\text{X}_N` / `\mathrm{X}_N` / `query_N` / `{\rm Query}_N`)、numbered sub-block 自動解析 | `N Q` + `\text{query}_1` / `\vdots` / `\text{query}_Q` + `1 x` / `2 x k` / ... | abc241-D, abc248-D, abc212-D, abc453-G |
| 簡易 T-testcases 型 (ブロック[0]=単一スカラー `T`、ブロック[1]=スカラー変数のみ)                    | `T\n\na s`                                                                        | abc238-D                     |
| 複雑な繰り返し本体 (ループマーカー付き、ブロック[1] にループ・配列含む)                             | `T\n\mathrm{case}_T\n\nN K\nA_1 \ldots A_N`                                      | abc456-F, abc456-E           |
| 上三角行列 (triangular matrix、bound が変数名)                                                       | `N\nA_{1,2}\ldots A_{1,N}\nA_{2,3}\ldots A_{2,N}\n\vdots\nA_{N-1,N}`             | abc451-E                     |
| 上三角行列 (triangular matrix、bound が算術式 `2N`)                                                  | `N\nA_{1,2} A_{1,3} \cdots A_{1,2N}\nA_{2,3}\cdots A_{2,2N}\n\vdots\nA_{2N-1,2N}` | abc236-D                   |
| ジャギー配列 (同行レイアウト、スカラー 1 個)                                                         | `N\nL_1 A_{1,1}\ldots A_{1,L_1}\n\vdots\nL_N A_{N,1}\ldots A_{N,L_N}`             | abc457-B                     |
| ジャギー配列 (改行レイアウト、スカラー 1 個)                                                         | `N M\nL_1\nX_{1,1}\cdots X_{1,L_1}\n\vdots\nL_N\nX_{N,1}\cdots X_{N,L_N}`       | abc446-B                     |
| ジャギー配列 (同行レイアウト、スカラー 2 個以上)                                                     | `N\nT_1 K_1 A_{1,1}\ldots A_{1,K_1}\n\vdots\nT_N K_N A_{N,1}\ldots A_{N,K_N}`   | abc226-C                     |

**前処理**: `\hspace{0.4cm}\vdots` は `\vdots` に正規化する。また `:` のみの行も `\vdots` と等価に扱う (トークナイザーレベルで正規化)。

**単一変数ループのフラット化**: `\vdots` または `:` で囲まれたブロックが「1 行 1 変数」の繰り返しのみで構成される場合、`[T; N]` の一括読み込みに変換する (`flatten_single_var_loops`)。

**複数変数ループのコード生成**: 1 行複数変数のループは `Vec::new()` 宣言 + `for _ in 0..N { input!{...} ... .push(...) }` を生成する。テストハーネスではループ入力は `panic!` を生成するため、サンプルテストは手動で記述する。

**文字グリッド (2D 添字) の検出**: `S_{11}...S_{1W}` や `S_{1,1} S_{1,2} \ldots S_{1,W}` のように `同名変数_{2D_left} [同名変数_{2D}]* Cdots 同名変数_{2D_right}` の形をとる行は「グリッドの 1 行 = 1 つの `String`」として扱う。検出ルール:

- Cdots の前後、および先頭に置かれる追加要素の変数名が全て一致する
- 少なくとも一方の添字ブレースが「2D」であること: 複数トークン (`{1 W}`)、カンマ区切り (`{1,W}`)、または英字を含む長さ ≥ 2 の単一トークン (`{H1}`, `{HW}`, `{iW}`) のいずれか (純数値の多桁トークン `{10}` `{12}` は 1D インデックスとして扱い GridRow 検出から除外する)
- Cdots 前の追加要素は 2D 添字を持ち、行インデックス (先頭パーツ: Num または Ident) が全要素で一致していること
- 追加要素間のスペースは任意 (スペースあり S_{1,1} S_{1,2} もスペースなし S_{1,1}S_{1,2} も両方対応)
- 行全体がこのパターンのみで構成される (前後に他の変数がない)

検出された場合は `RawLine::GridRow` として分類する。行インデックス部分 (後述) を `IntermOp::LoopBegin` の `end` フィールド (ループのサイズ推定) に使用する。行インデックスの抽出ルール: 複数トークン・カンマ区切りの場合は先頭パーツ、英字を含む長さ ≥ 2 の単一トークン (`{H1}`, `{HW}`) の場合は先頭の 1 文字。`var_type` は `Str` として直接付与し、`all_int` 等の制約推定で上書きされない。`flatten_single_var_loops` で `[String; H]` へフラット化される。

#### Phase 1 非対応 → `ok: false` にフォールバック

実際の AtCoder 問題で確認:

| 非対応パターン                                                      | 例                                   | 確認問題    |
| ------------------------------------------------------------------- | ------------------------------------ | ----------- |
| ループマーカーなし plain-text 添字 (`q_i` など、`\text{}`/`\mathrm{}` もなし) | `Q\nq_1\n:\nq_Q`                       | typical90-L |
| ジャギー配列で SIZE_VAR がスカラー列に存在しない                               | `A_{i,1} \ldots A_{i,K_i}` (K_i 未宣言) | —          |
| 繰り返し本体 (iteration_ops) 内のネストループ 2 段以上                        | ループ内にさらにループ                 | —           |
| `{Num,Num}` 以外のカンマ添字 (JaggedRow / GridRow 以外のコンテキスト)        | `A_{i,j}`, `A_{1,N}`                  | —           |
| 2D 固定グリッドで行数 1 またはセル数が 1 行の場合                             | 1 行のみ `A_{1,1}...A_{1,6}`          | —           |

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

`ok=true` のとき: `solve` を純粋な引数関数として生成し、`main` で `input!` → `solve(args...)` と呼ぶ。`triangular` が非空のとき、`solve` 引数末尾に `Vec<Vec<T>>` を追加し、`main` に三角行列読み取りループを生成する。
`ok=false` のとき: フォールバックとして `solve` が `src` を受け取る従来スタイルを生成する。
`solve` 本文は `with_output(|| { ... })` で包まれる。本文内で `out!(answer)` を呼ぶと 1 行出力として `String` に蓄積され、`with_output` が最後にその `String` を返す。`out!(a, b)` は空白区切り、`out!(vec)` / `out!(slice)` / `out!((a, b))` は要素を空白区切りで出力する。

```tera
{% raw %}
trait OutValue { ... }
fn with_output<T>(f: impl FnOnce() -> T) -> String { ... }
macro_rules! out { ... }
{% endraw %}

use proconio::input;

{% if input_format.ok -%}

{# クエリ型ループ検出: LoopBegin の直後が LoopEnd (empty body) = QueryLine 由来 #}
{% set_global query_loop_end = "" -%}
{% for i in range(end=input_format.ops | length) -%}
{% set op = input_format.ops[i] -%}
{% if op.tag == "loop_begin" -%}
{% set next_i = i + 1 -%}
{% if input_format.ops[next_i] is defined and input_format.ops[next_i].tag == "loop_end" -%}
{% set_global query_loop_end = op.end -%}
{% endif -%}
{% endif -%}
{% endfor -%}
fn solve(
    {%- for vd in input_format.vars %}
    {% if vd.dim == 0 -%}
    {{ vd.name }}: {% if vd.is_size %}usize{% elif vd.var_type == "str" %}String{% else %}i64{% endif %},
    {%- elif vd.dim == 1 -%}
    {{ vd.name }}: Vec<{% if vd.var_type == "str" %}String{% else %}i64{% endif %}>,
    {%- endif %}
    {%- endfor %}
    {%- if input_format.triangular %}
    {{ input_format.triangular.name }}: Vec<Vec<{% if input_format.triangular.var_type == "str" %}String{% else %}i64{% endif %}>>,
    {%- endif %}
) -> String {
    with_output(|| {
        {% if query_loop_end != "" -%}
        {% if input_format.query_types | length > 0 -%}
        {# 複数種別クエリ: match dispatch を生成 #}
        for _ in 0..{{ query_loop_end }} {
            input! { query_type: usize, }
            match query_type {
                {% for qt in input_format.query_types -%}
                {{ qt.type_id }} => {
                    {% if qt.ok -%}
                    input! {
                        {% for v in qt.vars -%}
                        {{ v.name }}: {% if v.var_type == "str" %}String{% else %}i64{% endif %},
                        {% endfor -%}
                    }
                    {% endif -%}
                    // TODO: handle query type {{ qt.type_id }}
                    todo!()
                }
                {% endfor -%}
                _ => unreachable!(),
            }
        }
        {% elif input_format.query_body | length > 0 -%}
        {# 単一形式ループ (スカラーのみ sub-block): 変数入力付きループを生成 #}
        for _ in 0..{{ query_loop_end }} {
            input! {
                {% for v in input_format.query_body -%}
                {{ v.name }}: {% if v.var_type == "str" %}String{% else %}i64{% endif %},
                {% endfor -%}
            }
            // TODO: handle query
            todo!()
        }
        {% elif input_format.iteration_ops | length > 0 -%}
        {# 複雑な繰り返し本体 (ループ・配列含む sub-block): iteration_ops をそのままレンダリング #}
        for _ in 0..{{ query_loop_end }} {
            {% for op in input_format.iteration_ops -%}
            {% if op.tag == "read_line" and op.depth == 0 -%}
            input! {
                {% for v in op.vars -%}
                {% if v.dim == 0 -%}
                {% set vd = input_format.iteration_vars | filter(attribute="name", value=v.name) | first -%}
                {{ v.name }}: {% if vd.is_size %}usize{% elif vd.var_type == "str" %}String{% else %}i64{% endif %},
                {% elif v.dim == 1 -%}
                {% if v.size -%}
                {% set vd = input_format.iteration_vars | filter(attribute="name", value=v.name) | first -%}
                {{ v.name }}: [{% if vd.var_type == "str" %}String{% else %}i64{% endif %}; {{ v.size }}],
                {% endif -%}
                {% elif v.dim == 2 -%}
                {% set vd = input_format.iteration_vars | filter(attribute="name", value=v.name) | first -%}
                {{ v.name }}: [[{% if vd.var_type == "str" %}String{% else %}i64{% endif %}; {{ vd.size[0] }}]; {{ vd.size[1] }}],
                {% endif -%}
                {% endfor -%}
            }
            {% elif op.tag == "loop_begin" -%}
            {% set next_index = loop.index0 + 1 -%}
            {% if input_format.iteration_ops[next_index] is defined and input_format.iteration_ops[next_index].tag != "loop_end" -%}
            {% set body_op = input_format.iteration_ops[next_index] -%}
            {% for v in body_op.vars -%}
            {% set vd = input_format.iteration_vars | filter(attribute="name", value=v.name) | first -%}
            let mut {{ v.name }}: Vec<{% if vd.var_type == "str" %}String{% else %}i64{% endif %}> = Vec::new();
            {% endfor -%}
            for _ in 0..{{ op.end }} {
                input! {
            {% else -%}
            for _ in 0..{{ op.end }} {
                // TODO: read loop body
            }
            {% endif -%}
            {% elif op.tag == "read_line" and op.depth > 0 -%}
                    {% for v in op.vars -%}
                    {% if v.index -%}
                    {% set vd = input_format.iteration_vars | filter(attribute="name", value=v.name) | first -%}
                    __tmp_{{ v.name }}: {% if vd.var_type == "str" %}String{% else %}i64{% endif %},
                    {% endif -%}
                    {% endfor -%}
            {% elif op.tag == "loop_end" -%}
            {% set prev_index = loop.index0 - 1 -%}
            {% set body_op = input_format.iteration_ops[prev_index] -%}
            {% if body_op.tag != "loop_begin" -%}
                }
                {% for v in body_op.vars -%}
                {{ v.name }}.push(__tmp_{{ v.name }});
                {% endfor -%}
            }
            {% endif -%}
            {% endif -%}
            {% endfor -%}
            // TODO: solve each iteration
            todo!()
        }
        {% else -%}
        {# ループスタブのみ (sub-block なし / パース失敗): ループスタブを生成 #}
        for _ in 0..{{ query_loop_end }} {
            // TODO: read and handle query
            todo!()
        }
        {% endif -%}
        {% else -%}
        // TODO: write solution and call out!(answer)
        todo!()
        {% endif -%}
    })
}

fn main() {
    {% for op in input_format.ops -%}
    {% if op.tag == "read_line" and op.depth == 0 -%}
    input! {
        {% for v in op.vars -%}
        {% if v.dim == 0 -%}
        {% set vd = input_format.vars | filter(attribute="name", value=v.name) | first -%}
        {{ v.name }}: {% if vd.is_size %}usize{% elif vd.var_type == "str" %}String{% else %}i64{% endif %},
        {% elif v.dim == 1 -%}
        {% if v.size -%}
        {% set vd = input_format.vars | filter(attribute="name", value=v.name) | first -%}
        {{ v.name }}: [{% if vd.var_type == "str" %}String{% else %}i64{% endif %}; {{ v.size }}],
        {% endif -%}
        {% endif -%}
        {% endfor -%}
    }
    {% elif op.tag == "loop_begin" -%}
    {% set next_index = loop.index0 + 1 -%}
    {% set body_op = input_format.ops[next_index] -%}
    {% if body_op.tag == "loop_end" -%}
    {# empty-body loop (QueryLine origin) — handled in solve, skip in main #}
    {% else -%}
    {% for v in body_op.vars -%}
    {% set vd = input_format.vars | filter(attribute="name", value=v.name) | first -%}
    let mut {{ v.name }}: Vec<{% if vd.var_type == "str" %}String{% else %}i64{% endif %}> = Vec::new();
    {% endfor -%}
    for _ in 0..{{ op.end }} {
        input! {
    {% endif -%}
    {% elif op.tag == "read_line" and op.depth > 0 -%}
            {% for v in op.vars -%}
            {% if v.index -%}
            {% set vd = input_format.vars | filter(attribute="name", value=v.name) | first -%}
            __tmp_{{ v.name }}: {% if vd.var_type == "str" %}String{% else %}i64{% endif %},
            {% endif -%}
            {% endfor -%}
    {% elif op.tag == "loop_end" -%}
    {% set prev_index = loop.index0 - 1 -%}
    {% set body_op = input_format.ops[prev_index] -%}
    {% if body_op.tag != "loop_begin" -%}
        }
        {% for v in body_op.vars -%}
        {{ v.name }}.push(__tmp_{{ v.name }});
        {% endfor -%}
    }
    {% endif -%}
    {% endif -%}
    {% endfor -%}
    {% if input_format.triangular -%}
    {# 三角行列: for ループで行ごとに読み取り #}
    {% set tri = input_format.triangular -%}
    {% set tri_type = "String" if tri.var_type == "str" else "i64" -%}
    let mut {{ tri.name }}: Vec<Vec<{{ tri_type }}>> = Vec::new();
    for _i in 0..{{ tri.bound }}-1 {
        input! { _row: [{{ tri_type }}; {{ tri.bound }}-1-_i], }
        {{ tri.name }}.push(_row);
    }
    print!("{}", solve({{ input_format.vars | map(attribute="name") | join(sep=", ") }}, {{ tri.name }}));
    {% else -%}
    print!("{}", solve({{ input_format.vars | map(attribute="name") | join(sep=", ") }}));
    {% endif -%}
}
{% else -%}
fn solve<R: std::io::BufRead>(src: &mut impl proconio::source::Source<R>) -> String {
    // TODO: input_format.ok = false — write input manually
    with_output(|| {
        todo!()
    })
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

**クエリ型テンプレート生成例** (abc241-D 形式、3 種類のクエリ):

```rust
fn solve(n: usize, q: usize) -> String {
    with_output(|| {
        for _ in 0..q {
            input! { query_type: usize, }
            match query_type {
                1 => {
                    input! { x: i64, }
                    // TODO: handle query type 1
                    todo!()
                }
                2 => {
                    input! { x: i64, k: i64, }
                    // TODO: handle query type 2
                    todo!()
                }
                3 => {
                    input! { l: i64, r: i64, }
                    // TODO: handle query type 3
                    todo!()
                }
                _ => unreachable!(),
            }
        }
    })
}

fn main() {
    input! { n: usize, q: usize, }
    print!("{}", solve(n, q));
}
```

sub-block が `ok: false` のクエリ種別は以下のようにフォールバック:

```rust
2 => {
    // TODO: handle query type 2
    todo!()
}
```

**単一形式クエリ例** (abc334_d 形式: `query_types = []`, `query_body = [x]`):

```rust
fn solve(n: usize, q: usize, r: Vec<i64>) -> String {
    with_output(|| {
        for _ in 0..q {
            input! { x: i64, }
            // TODO: handle query
            todo!()
        }
    })
}

fn main() {
    input! { n: usize, q: usize, }
    input! { r: [i64; n], }
    print!("{}", solve(n, q, r));
}
```

**sub-block なし (ループスタブのみ)** (`query_types = []`, `query_body = []`, `iteration_ops = []`):

```rust
fn solve(q: usize) -> String {
    with_output(|| {
        for _ in 0..q {
            // TODO: read and handle query
            todo!()
        }
    })
}
```

**複雑な繰り返し本体 (1D 配列含む)** (`iteration_ops` 非空、abc456-F 形式 `T\n\mathrm{case}_T\n\nN K\nA_1 \ldots A_N`):

```rust
fn solve(t: usize) -> String {
    with_output(|| {
        for _ in 0..t {
            input! { n: usize, k: i64, }
            input! { a: [i64; n], }
            // TODO: solve each iteration
            todo!()
        }
    })
}

fn main() {
    input! { t: usize, }
    print!("{}", solve(t));
}
```

**複雑な繰り返し本体 (複数ループ含む)** (`iteration_ops` 非空、abc456-E 形式):

```rust
fn solve(t: usize) -> String {
    with_output(|| {
        for _ in 0..t {
            input! { n: usize, m: usize, }
            let mut u: Vec<i64> = Vec::new();
            let mut v: Vec<i64> = Vec::new();
            for _ in 0..m {
                input! { __tmp_u: i64, __tmp_v: i64, }
                u.push(__tmp_u);
                v.push(__tmp_v);
            }
            input! { w: i64, }
            let mut s: Vec<String> = Vec::new();
            for _ in 0..n {
                input! { __tmp_s: String, }
                s.push(__tmp_s);
            }
            // TODO: solve each iteration
            todo!()
        }
    })
}

fn main() {
    input! { t: usize, }
    print!("{}", solve(t));
}
```

**型対応表** (テンプレート内の判定ロジック):

| `dim` | `is_size` | `var_type` | Rust 型       |
| ----- | --------- | ---------- | ------------- |
| 0     | true      | any        | `usize`       |
| 0     | false     | `"str"`    | `String`      |
| 0     | false     | other      | `i64`         |
| 1     | false     | `"str"`    | `Vec<String>` |
| 1     | false     | other      | `Vec<i64>`    |

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
