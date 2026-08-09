# ライブラリ・verify・Web 公開機能 設計メモ

- 作成日: 2026-08-02
- 最終更新日: 2026-08-10
- 状態: 設計確定・PR review 待ち

この文書は、壁打ちで合意した設計判断を記録する。
実装計画ではなく、機能境界とデータの意味を定める architecture spec である。

## 1. 目的

このリポジトリに置かれた競技プログラミング用ライブラリ、解法、説明、依存関係、
OJ による verify 結果を静的 Web サイトで閲覧できるようにする。

この機能はこのリポジトリ専用とする。一方、Rust、C++、Lean の構文をアプリ本体へ
直接組み込まず、言語ごとの設定と外部アダプターで扱える言語非依存の構造を
最初から採用する。
MVP の実対応はこの 3 言語とするが、core protocol は対応言語を固定 enum にしない。

## 2. 設計原則

- ライブラリページ 1 つをソースファイル 1 つに対応させる。
- ライブラリ ID はリポジトリルートからの相対パスとする。
- アプリ本体は言語固有の構文やテスト・証明方法を理解しない。
- 言語固有の解析は外部コマンドへ委譲し、共通 JSON へ正規化する。
- 公開対象の決定はアプリ本体が行う。
- 解析アダプターは、アプリ本体が選んだ候補へ情報を付加するだけにする。
- ソース上の依存 `depends_on` と、解法による保証 `verifies` を分離する。
- 通常の check と OJ 提出による verify を分離する。
- Web は静的サイトとし、検索のみブラウザ上で実行する。
- verify の鮮度は Git commit ID ではなく入力内容の fingerprint で判定する。
- main ブランチを公開内容の唯一の正本とする。

## 3. 対象範囲

### 3.1 MVP に含める

- Rust、C++、Lean の 3 言語に対する library check、解析、公開、検索
- 言語・ディレクトリ・ライブラリファイルの発見
- Markdown による説明
- ソースコード表示
- ライブラリ間および解法からライブラリへの依存関係
- 依存元一覧の導出
- 任意ラベル付き関係の基礎表現
- 解法による明示的な verify 登録
- 未実行・stale な verify の OJ 提出、判定追跡、最新結果保存
- MVP での LibraryChecker に対する無人 verify
- ライブラリ、解法、verify 結果の静的 Web 公開
- ファイル単位でまとめた全文・シンボル検索
- `lang:` などの検索フィルター

### 3.2 MVP では扱わない

- 通常テスト、property-based testing、Lean の証明チェック結果の Web 公開
- verify の過去履歴を Web 上で時系列表示する機能
- ライブラリ内の宣言ごとに独立ページを作る機能
- 全言語共通の構文解析ロジックをアプリ本体へ持たせること
- 常駐 Web バックエンドやデータベース
- キャッシュを無視して同一内容を再提出する `--force`
- AtCoder に対する無人 verify とブラウザ提出後の追跡

## 4. ドメイン境界

### 4.1 LibraryFile

言語 config の root / include / exclude で管理対象になった 1 つのソースファイルを表す。
管理対象は公開可否にかかわらず解析する。
include に一致したライブラリは既定で公開し、必要な場合だけ対応する Markdown frontmatter の
`publish = false` で Web 公開を止める。

- `id`: リポジトリ相対パス
- `language`
- `source_path`
- `description_path`
- `published`
- `updated_at`
- `updated_by_commit`
- `symbols`
- `dependencies`
- `relations`
- `analysis_state`
- `diagnostics`

`LibraryFile` は単独でコンパイル可能である必要はない。また、1 ファイルに宣言や定理が
複数含まれていてもよい。宣言は公開ファイルだけ検索やページ内リンクに使う。
公開 `LibraryFile` から 1 対 1 の `LibraryPage` を導出し、非公開ファイルのページは作らない。

公開 library の `updated_at` は、source file と対応する sidecar Markdown をそれぞれ最後に
変更した Git commit の committer date のうち新しい方とする。timezone 付き RFC 3339 へ
正規化し、根拠 commit SHA も保持する。

- author date ではなく committer date を使う。
- source、説明、relation、dependency override の変更を更新として扱う。
- verify result や依存先の変更では、この library 自身の日時を変えない。
- rename 前の履歴は追わず、rename commit を新しい library ID の更新日時とする。
- frontmatter に手動 `updated_at` は設けない。
- 同時刻の sort は library ID の昇順とする。
- production site build では full Git history を必須とし、履歴不足は error とする。
- local preview の未コミット変更は、最終 commit 日時と別の `uncommitted` 表示で表せる。

source path は library identity であり、move / rename は旧 library の削除と新 library の追加として
扱う。Git の rename detection を identity、URL、result 移行には使わない。

- source と sidecar は同じ変更で対応する新 path へ移す。
  旧 path の orphan sidecar を許可しない。
- relations、dependency overrides、`[verify].libraries`、Markdown link は新 ID へ更新する。
- 旧 ID の参照や存在しない参照が残れば config / build error とする。
- 旧 public URL は新しい artifact から消えて static 404 となり、自動 redirect を生成しない。
- public から private への変更や削除でも旧 URL を private path へ redirect しない。
- alias や `redirect_from` は MVP に設けず、必要になった場合に明示 metadata として設計する。

solution の `solved_at` は問題を解いた日時なので引き続き明示入力とする。
Git からは導出しない。

### 4.2 Solution

既存の Contest・Problem 配下にある解法を表す。
MVP では既定を非公開とし、解法の `ce.toml` に `publish = true` がある場合だけ公開する。
公開解法では timezone 付き RFC 3339 形式の `solved_at` を必須とする。filesystem timestamp、
Git commit 日時、OJ 提出日時への暗黙 fallback は行わない。

- コンテスト、問題、OJ
- 解法名、言語
- 解法ルートと提出エントリーファイル
- 公開可否
- `solved_at`
- 依存ライブラリ
- 明示的に verify するライブラリ

solution ID も identity とする。solution directory の move / rename は新しい solution の
追加として扱い、旧 ID の result JSON は同じ変更で削除する。result を新 path へ移して
再利用せず、新 solution の verify 状態は `never` から始める。discovery 済み solution に
対応しない orphan result は schema / build error とする。

### 4.3 Relation

最低限、次の関係を扱う。

```text
LibraryFile --depends_on--> LibraryFile
Solution    --depends_on--> LibraryFile
Solution    --verifies----> LibraryFile
LibraryFile --<任意ラベル>-> LibraryFile
```

`impl`、`proof`、`companion` などはコアの固定概念にせず、任意ラベル付き関係として
表現できるようにする。依存元一覧は `depends_on` を反転してアプリ本体が導出する。

`depends_on` は target が直接宣言する依存だけを表す。adapter は推移依存を edge として
追加せず、core が direct edge の graph から推移閉包を導出する。

- Rust は対象 file が直接参照、import、module 宣言する managed library を返す。
- C++ は対象 file に直接書かれた local `#include` の解決先を返す。
- Lean は対象 module に直接書かれた `import` の解決先を返す。
- solution は solution-owned source 全体から managed library へ出る direct edge の和集合を返す。
- 同じ target への複数参照は 1 edge に正規化する。

adapter が推移依存を direct edge として返すことは protocol の意味上の違反とし、各言語の
contract fixture で検出する。core は言語構文から余分な edge を推測して補正しない。

### 4.4 DependencyAnalysis と SymbolAnalysis

依存解析と symbol 抽出の状態は分けて保持する。

`dependency_analysis.state`:

- `complete`: すべての参照を internal または external へ分類し、unresolved が 0 件。
- `partial`: 信用できる依存はあるが、unresolved が 1 件以上。
- `failed`: 安全な依存集合を取得できず、dependencies は空配列。

`symbol_analysis.state`:

- `complete`: symbol を正常に列挙した。空配列なら本当に 0 件。
- `partial`: 一部の symbol だけを信用できる。
- `failed`: symbol 一覧を取得できず、symbols は空配列。

依存なし、symbol なし、解析失敗を同じ空配列だけで表現しない。
verify と fingerprint を止めるのは dependency analysis の `partial` / `failed` だけとする。
symbol analysis が `partial` / `failed` でも source page を公開し、symbol 欄に warning を出す。
solution result は MVP では symbol analysis を持たない。

直接依存の参照は次の 3 種類へ分類する。

- `internal`: 管理対象の `LibraryFile` へ解決できた参照。公開可否は問わない。
- `external`: 標準ライブラリ、package、Mathlib、システム header などへの参照。
- `unresolved`: 内部・外部を含めて正しく解決できなかった参照。

unresolved は target 内で一意な adapter-defined `key`、人間向け `display`、任意の location を
持つ。`key` は手動 override の照合に使い、Web の表示名には使わない。

内部の direct dependency graph と依存元には `internal` だけを使い、fingerprint closure は
その graph の推移閉包から作る。Web の依存一覧には公開ファイル間の direct edge だけを
出す。
非公開依存がある場合は path や名前を公開 DTO に入れず、必要なら
「非公開依存あり」とだけ表示する。
`external` は正規化データへ保持するが、MVP の Web、検索、依存グラフには表示しない。

diagnostic の共通項目は `severity`、`code`、`message`、任意の source location とする。
`severity` は `info`、`warning`、`error` の 3 種類とし、`code` はアダプター固有文字列を
許可する。

diagnostic message は人間向け説明だけとし、file path を埋め込まない。

- path は必ず構造化 source location へ分離する。
- repository absolute path、その他の absolute path、managed source path、
  solution root-relative path を message に含めることを禁止する。
- core は既知 path と path-like absolute token を検査し、違反を adapter protocol error とする。
- compiler の raw diagnostic をそのまま message へ転記せず、adapter が分解・正規化する。

## 5. リポジトリ内の配置

想定する配置例を示す。

```text
config.toml

libraries/
  rust/
    _index.md
    algebra/
      _index.md
      monoid.rs
      monoid.rs.md

solutions/
  abc999/
    a/
      main/
        ce.toml
        src/main.rs

verification/
  results/
    abc999/
      a/
        main.json

web/
  ...

target/ce-site-data/
  ...                       # Git 管理外の生成物
```

- ソース説明は `monoid.rs.md` のようにソースファイル名全体へ `.md` を付ける。
- 言語・ディレクトリ説明は `_index.md` に置く。
- 解析補正や任意関係は Markdown frontmatter へ置けるようにする。
- verify 登録は解法側の `ce.toml` へ置く。
- 最新の verify 結果はリポジトリ内へ保存する。
- solution ID は `{contest_id}/{problem_code}/{solution_name}` とする。
- 結果は `verification/results/{contest_id}/{problem_code}/{solution_name}.json` に置く。
- Web 用の正規化済みデータと検索 index はビルド生成物とし、Git 管理しない。

frontmatter の概念例:

```toml
+++
title = "Monoid"
publish = true

[[relations]]
kind = "impl"
to = "libraries/rust/algebra/additive_monoid.rs"

[[dependency_overrides]]
action = "add"
to = "libraries/rust/algebra/magma.rs"
reason = "macro-generated dependency"
+++
```

解法側 verify 登録の概念例:

```toml
publish = true
solved_at = "2026-08-02T14:30:00+09:00"
test_command = "./test.sh"
test_timeout_seconds = 600

[verify]
libraries = [
  "libraries/rust/algebra/magma.rs",
  "libraries/rust/algebra/monoid.rs",
]
# Optional per-solution override for an OJ whose language ID varies by contest.
language_id = "rust"
```

### 5.1 Markdown schema

library sidecar の frontmatter は次だけを許可する。

- `title`: 任意。省略時は拡張子を含む source basename。
- `publish`: 任意。既定 `true`。
- `relations`: 任意。
- `dependency_overrides`: 任意。

directory と language root の `_index.md` は `title` だけを許可する。
通常 directory の既定 title は directory basename、language root の既定 title は
language `display_name` とする。directory metadata に `publish`、relation、dependency
override は置かない。

- 公開 descendant がある directory page は `_index.md` の有無にかかわらず生成する。
- 公開 descendant がなくても `_index.md` があれば説明 page を生成する。
- directory page をなくすには公開 descendant と `_index.md` の両方をなくす。
- sidecar がなくても source page を生成する。
- source が存在しない orphan sidecar は build error とする。
- frontmatter は `+++` で囲んだ TOML とし、frontmatter なしも許可する。
- malformed TOML、空 title、用途ごとの未知 key は build error とする。
- `updated_at`、`order`、`tags`、`aliases` は MVP では設けない。

relation の `kind` は `[a-z][a-z0-9_-]*` の token とする。cross-language relation を許可するが、
source と target は両方とも公開 library に限定する。存在しない target、self relation、同じ
kind / target の重複は config error とする。

## 6. 言語解析アダプター

### 6.1 境界

言語ごとに config で外部解析コマンドを指定する。
アダプターは言語に属するライブラリと解法をまとめて解析し、共通 JSON を返す。
アダプターは対象言語につき 1 回起動し、workspace の分割と解析ツールの起動は
アダプター内部へ任せる。

MVP では言語ごとにライブラリ root を 1 つ持つ。複数 package や workspace は単一 root
の下へ配置し、言語固有の workspace 境界はアダプター内部で扱う。

解析コマンドは shell 文字列ではなく argv 配列で指定し、shell を介さず直接起動する。

```toml
[library.languages.rust.analyzer]
command = ["./target/library-analyzers/bin/rust-analyzer"]
timeout_seconds = 600
```

言語 root と管理・公開対象の概念例:

```toml
[library.languages.rust]
display_name = "Rust"
root = "libraries/rust"
include = ["**/*.rs"]
exclude = ["**/generated/**", "**/target/**"]
check_command = "cargo test"
check_timeout_seconds = 600
syntax_highlight = "rust"
expected_toolchains = [
  { name = "rustc", version = "1.92.0" },
  { name = "cargo", version = "1.92.0" },
]

[library.languages.rust.online_judges.librarychecker]
language_id = "rust"
```

ライブラリ設定はリポジトリルートのプロジェクトローカル `config.toml` にだけ置き、
グローバル config とは merge しない。各項目の契約は次とする。

- `[library.languages.<name>]` の `<name>` を言語 ID とする。
- 言語 ID は `[a-z][a-z0-9-]*` に一致する安定 slug とする。
- `display_name` は省略可能とし、省略時は言語 ID を表示名にする。
- `root`、`include`、`analyzer.command` は必須とする。
- `exclude` は省略時に空配列とする。
- `check_command` は省略可能とする。
- `check_timeout_seconds` は省略時 600 秒とする。
- `syntax_highlight` は省略可能とし、省略時は言語 ID を使用する。
- `analyzer.timeout_seconds` は省略時 600 秒とする。
- `expected_toolchains` は production 解析と verify で使う toolchain identity の完全な集合とする。
- 各 toolchain は安定した `name` と完全一致させる opaque `version` を持つ。
- `[library.languages.<id>.online_judges.<oj>].language_id` は、repository 内の
  verify で使う既定の OJ 提出言語 ID とする。
- root と analyzer command の相対パスはリポジトリルート基準とする。
- analyzer の作業ディレクトリはリポジトリルートとする。
- check の作業ディレクトリは言語 root とする。
- check は shell 文字列、analyzer は argv 配列とする。
- `[library]` 以下の未知キーは config error とする。

公開 site の repository 固有 metadata も同じ project-local config に置く。

```toml
[library.site]
title = "compro-env"
description = "Competitive programming libraries and solutions"
language = "en"
repository_url = "https://github.com/owner/compro-env"
```

- production build では 4 項目を必須とし、trim 後の空文字を許可しない。
- `language` は有効な BCP 47 language tag とし、`html[lang]` に使う。
- `repository_url` は userinfo、query、fragment を持たない HTTPS URL に限定する。
- canonical site origin と base path は build environment から渡し、config へ重複保存しない。
- local preview で未設定の場合は warning を出し、
  repository link と canonical metadata を省略できる。
- local preview の一時 fallback は production site-data へ出力しない。

言語 ID は adapter manifest、solution の `language`、URL、検索の `lang:`、site-data で同じ値を
使い、case を暗黙変換しない。`syntax_highlight` と表示名は ID から分離する。
ID rename は URL と fingerprint に影響する破壊的変更とし、MVP では alias / redirect を
持たない。root directory 名と ID の一致は要求しない。

公開 solution と verify solution の language には対応する `[library.languages.<id>]` を
必須とする。private solution は既存の language 設定だけでよい。
公開 solution だけがあり library が 0 件の言語も、空の root と analyzer を設定して扱える。

verify の OJ 提出言語 ID は、解法の `[verify].language_id`、対応する project-local
`[library.languages.<id>.online_judges.<oj>].language_id` の順で解決する。前者は contest ごとに
ID が異なる OJ のための任意 override、後者は repository 内の既定値である。

- verify spec があるのにどちらからも解決できなければ config error とする。
- trim 後の空文字を拒否するが、OJ 固有 ID の case や記号は変更しない。
- global config、OJ API の動的な default、内部 language ID から推測した値へ fallback しない。
- OJ adapter が解決済み ID を確定的に非対応と判定した場合は `unavailable` とする。
- 通常の対話的 `ce submit` は既存の global language / OJ mapping を引き続き使用する。
- 将来 `ce submit --watch` が start / poll lifecycle を再利用しても、通常 submit は
  verify 用 project-local mapping へ暗黙に切り替えない。

toolchain name は `[a-z][a-z0-9._-]*` の一意な token、version は trim 後に非空の
adapter-defined string とする。core は semantic version や compiler family を解釈せず、
observed toolchain を name 順に正規化して expected set と name / version の完全一致を確認する。
OS / CPU target は observed identity の監査情報として保持するが、一致条件には含めない。

- production site-data と `ce verify` は missing、extra、version mismatch を error とする。
- preview site-data は mismatch を warning とし、observed identity を表示して生成を続けられる。
- toolchain version の変更は独立 PR とし、3 言語の fixture、check、analysis diff を確認する。
- Rust は `rust-toolchain.toml` の channel を patch version まで固定する。
- Lean は `lean-toolchain` を exact release に固定し、Lake dependency lockfile を commit する。
- C++ は CI の compiler family / exact version と取得物の checksum または image digest を固定する。
- 各言語の `check_command` は対応 version を検査し、不一致なら check failure とする。

check には `CE_REPOSITORY_ROOT`、`CE_LIBRARY_ROOT`、`CE_LANGUAGE` を環境変数として渡す。

管理対象と公開対象の列挙規則は次とする。

1. root 配下の通常ファイルを列挙する。
2. include のいずれかに一致するファイルを管理候補にする。
3. exclude のいずれかに一致するファイルを管理対象から除外する。
4. 残った全ファイルを解析対象にする。
5. 対応する Markdown の `publish` から Web 公開可否を決める。

glob は root からの `/` 区切り相対パスへ適用する。include は 1 件以上必須、各配列内は
OR、exclude は常に include より優先とする。否定 pattern による再包含は MVP では扱わない。

ディレクトリ symlink は辿らず、ファイル symlink も候補にしない。root が存在しない場合は
config error、一致ファイルが 0 件の場合は warning として空の言語ページを生成可能にする。
frontmatter から exclude 済みファイルを解析対象または公開対象へ戻すことはできない。
`publish = false` は解析除外の指定ではない。

pipe や複数コマンドが必要な場合はリポジトリ内にスクリプトを置き、そのスクリプトを
argv 配列から呼ぶ。shell 文字列との両対応は MVP では行わない。既存の `test_command` と
`submit.preprocess` の shell 文字列契約は変更しない。

アダプター内部ではコンパイラ、LSP、tree-sitter、専用スクリプト、手動メタデータなどを
自由に利用できる。
LSP はアプリ本体が直接統合する共通基盤ではなく、
アダプター実装の選択肢の一つとする。

### 6.2 入力

アプリ本体が先に root / include / exclude から管理対象を確定し、manifest として渡す。
`publish = false` の library も入力に含めるが、adapter に公開可否は渡さない。
solution は `publish = true` の公開対象だけを入力に含める。
アダプターの作業ディレクトリはリポジトリルートとする。UTF-8 の JSON 文書を stdin へ
1 つ書き込み、stdin を閉じる。

`schema_version` は必須の正整数とし、core が現在対応する adapter protocol version を渡す。
MVP の対応 version は `1` だけとする。`libraries` と `solutions` は空配列を許可し、空の
manifest でも adapter identity と observed toolchain を含む正常な response を要求する。

```json
{
  "schema_version": 1,
  "repository_root": ".",
  "language": "rust",
  "libraries": [
    {"path": "libraries/rust/algebra/monoid.rs"}
  ],
  "solutions": [
    {
      "id": "abc999/a/main",
      "root": "solutions/abc999/a/main",
      "entry": "src/main.rs"
    }
  ]
}
```

### 6.3 出力

アダプターは UTF-8 の JSON 文書を stdout へ 1 つだけ出力する。前後の空白は許可するが、
ログや進捗は stdout へ混ぜず stderr へ出す。stderr はアプリ本体が端末または CI ログへ
そのまま流す。

response の `schema_version` は必須で、request と core の対応 version の両方に完全一致する
必要がある。missing、異なる version、非整数は target 単位で回復できない
protocol error とする。

```json
{
  "schema_version": 1,
  "adapter": {
    "name": "example-rust-analyzer",
    "version": "1.0.0",
    "toolchains": [
      {
        "name": "rustc",
        "version": "1.92.0",
        "target": "aarch64-apple-darwin"
      },
      {
        "name": "cargo",
        "version": "1.92.0"
      }
    ]
  },
  "libraries": [
    {
      "path": "libraries/rust/algebra/monoid.rs",
      "dependency_analysis": {
        "state": "complete",
        "dependencies": [
          {
            "kind": "internal",
            "path": "libraries/rust/algebra/magma.rs"
          },
          {
            "kind": "external",
            "name": "std::ops::Add"
          }
        ]
      },
      "symbol_analysis": {
        "state": "complete",
        "symbols": [
          {
            "name": "Monoid",
            "qualified_name": "algebra::Monoid",
            "search_names": ["Monoid"],
            "kind": "trait",
            "location": {
              "path": "libraries/rust/algebra/monoid.rs",
              "start": {"line": 12, "column": 1}
            },
            "signature": "pub trait Monoid ..."
          }
        ]
      },
      "diagnostics": []
    }
  ],
  "solutions": [
    {
      "id": "abc999/a/main",
      "dependency_analysis": {
        "state": "complete",
        "dependencies": [
          {
            "kind": "internal",
            "path": "libraries/rust/algebra/monoid.rs"
          }
        ]
      },
      "diagnostics": []
    }
  ]
}
```

アプリ本体は出力を検証し、手動補正を統合してから依存元を導出する。
対象外ファイルをアダプターが出力しても管理対象や公開対象へ追加しない。

出力の `libraries` と `solutions` は、対応する入力 target をそれぞれちょうど 1 回返す。
missing、extra、duplicate target は adapter protocol error とする。個別 target を解析できない
場合も省略せず、該当 analysis を `state = "failed"` として diagnostics を返す。
library result は dependency analysis と symbol analysis、solution result は dependency
analysis だけを持つ。

`internal` dependency の行き先は同じ言語 manifest 内の library に限定し、solution を
依存先にはできない。adapter は公開可否を決定または出力せず、手動 override と Web 投影は
結果検証後にコアが行う。

`adapter.toolchains` は実際に解析へ使用した toolchain の name、version、任意の target を返す。
name は配列内で一意とし、missing / duplicate / empty field は protocol error とする。
production / verify では config の expected set と一致しなければ解析結果を採用しない。

adapter 出力の dependency は direct reference だけに限定する。library result は対象 file の
direct dependency、solution result は adapter が solution root 内で扱う source 全体から
library へ出る direct dependency の和集合を返す。core は各 target の internal edge を
重複排除してから reverse edge と推移閉包を導出する。

管理対象 source と preprocess 後の提出 source は UTF-8 を必須とする。
symbol、dependency、diagnostic で位置を返す場合は共通の `location` を使う。

```json
{
  "path": "libraries/rust/algebra/monoid.rs",
  "start": {"line": 12, "column": 5},
  "end": {"line": 14, "column": 2}
}
```

- `path` は安全なリポジトリ相対パスとする。
- library result では target 自身の path、solution result では solution root 配下に限定する。
- solution の location path は存在する通常ファイルとし、symlink を許可しない。
- line と column は 1 始まりとする。
- column は Unicode scalar value 単位とし、tab も 1 value と数える。
- `end` は exclusive とする。
- column、end、location 全体は、取得不能な場合に省略できる。
- CRLF は 1 改行として数えるが、source bytes の hash では LF と区別する。
- ファイル外、逆転範囲、target 外 path は adapter protocol error とする。

LSP を利用する adapter は、0 始まりかつ UTF-16 code unit の位置をこの形式へ変換する。
Web の symbol link は `location.start.line` から `#L12` のような行 anchor を生成する。
fingerprint は表示用の encoding や改行変換をせず、元の source bytes から計算する。

symbol の `kind` は言語共通 enum にせず、adapter が決める opaque token とする。

- `kind` は `[a-z][a-z0-9_-]*` に一致する非空文字列とする。
- コアは語彙を列挙または解釈しない。
- `name` は必須の表示名かつ非空とする。
- `qualified_name`、`search_names`、`signature`、`location` は任意とする。
- `search_names` は言語固有の検索 alias の配列とし、表示には使わない。
- `name` は常に検索名へ含め、`qualified_name` があればそれも含める。
- `search_names` の各値は非空かつ Unicode control character を含まないものとする。
- core は各 symbol 内の同一 alias を除去するが、separator の分割や alias の推測は行わない。
- unnamed declaration は adapter が安定した検索可能な表示名を作る。
- 同名 symbol を許可し、qualified name と location で区別する。
- kind、name、qualified name、search names、signature は検索・表示に使うが、
  verify fingerprint には直接含めない。

`trait`、`impl`、`class`、`theorem`、`instance` などをそのまま使用できる。
検索の `kind:` は完全一致 filter とし、利用可能な値は Pagefind filter から動的に表示する。
MVP では異なる kind を共通 group へ強制的にまとめない。

実行結果は次のように扱う。

- exit 0 かつ正常な JSON の場合だけ成功とする。
- exit 0 でも JSON または schema が不正なら adapter error とする。
- request / response の `schema_version` 不一致と対応外 version は version error とする。
- exit nonzero の場合は stdout を解析結果として採用しない。
- 起動失敗と timeout も adapter error とする。
- 特定のファイルや解法だけを解析できない場合は exit 0 とし、対象ごとの
  dependency / symbol analysis を `partial` または `failed` として JSON に含める。
- アダプター全体の結果を信用できない場合だけ exit nonzero とする。

MVP の実行制限と adapter identity は次とする。

- timeout の既定値は言語ごとに 10 分とし、config で上書き可能にする。
- stdout は 64 MiB を上限とし、config では変更できない。
- timeout と stdout 上限超過は言語全体の adapter error とする。
- stderr はアプリ本体で保持・公開せず、端末または CI ログへ直接流す。
- 出力の `adapter.name` と `adapter.version` は必須かつ空文字禁止とする。
- adapter version の形式は SemVer に限定しない。
- 公開データには command argv、adapter identity、schema version を記録する。

adapter version や解析 toolchain identity が変わっただけでは verify を stale にしない。
解析結果によって依存 closure が変わった場合に、その closure と content hash の変化によって
stale とする。expected toolchain mismatch は stale result を作るのでなく、production 解析と
新規 verify の開始を error で止める。

JSON 内のパスは `/` 区切りのリポジトリ相対パスに限定する。絶対パス、`..`、入力候補に
ないファイル、同じファイルの重複出力は拒否する。

アダプターは build cache など Git 管理外のファイルを作成してよいが、追跡対象ファイルを
変更してはいけない。アプリ本体は大きな入出力でも pipe deadlock を起こさないよう、
stdin の書き込みと stdout の読み取りを並行して行う。

### 6.4 AnalysisSnapshot と cache

各言語アダプターは 1 pipeline につき 1 回実行し、正規化した結果を immutable な
`AnalysisSnapshot` として同じ pipeline 内の verify plan、site-data、diagnostics 生成で
再利用する。

- analysis schema version
- 対象 repository revision
- discovery manifest
- 全候補ファイルの content hash
- analyzer command
- adapter identity
- observed toolchain identities
- 対象ごとの解析結果
- snapshot 全体の hash

CI job 間では短期 artifact として渡してよい。
consumer は schema、snapshot hash、repository revision、候補ファイル hash を検証する。
不一致なら利用せず、secret を持たない prepare job で再解析する。
OJ secret job は再解析せず、不整合を検出したら停止する。

コアは MVP では pipeline をまたぐ正規化済み解析結果 cache を持たない。
アダプター内部の Cargo `target/`、C++ build cache、Lake cache などは、追跡対象を
変更しない通常の build cache としてローカルや CI で再利用してよい。

将来 cross-run cache が必要になった場合は、アダプターが実際に参照した入力一覧と
cache key を申告する契約を先に追加する。
候補 source の hash だけで、暗黙に参照する workspace 設定や toolchain を無視した cache は
作らない。

### 6.5 dependency override

手動 dependency override は adapter 出力の schema 検証後、reverse dependency、effective
dependency state、fingerprint closure の計算前に適用する。library では対応する sidecar
Markdown の frontmatter、solution では `ce.toml` に操作の配列として記述する。

override は direct dependency だけを追加、削除、解決する。推移 edge を手動で列挙しない。

```toml
[[dependency_overrides]]
action = "add"
to = "libraries/rust/algebra/magma.rs"
reason = "macro-generated dependency"

[[dependency_overrides]]
action = "remove"
to = "libraries/rust/algebra/false_positive.rs"
reason = "type-only reference reported as dependency"

[[dependency_overrides]]
action = "resolve"
key = "use:crate::algebra::monoid"
to = "libraries/rust/algebra/monoid.rs"
reason = "adapter cannot resolve generated module path"

[[dependency_overrides]]
action = "external"
key = "import:Mathlib.Algebra.Group.Basic"
name = "Mathlib"
reason = "provided by the Lean environment"
```

unresolved dependency は target 内で一意かつ安定した `key`、表示用 `display`、任意の location を
持つ。override は表示文字列でなく key と照合する。

- `reason` は全操作で必須かつ非空とする。
- `add` は同じ言語の管理対象 library を指す。非公開 library も許可する。
- `remove` は adapter が返した internal dependency 1 件と一致する必要がある。
- `resolve` と `external` は unresolved key 1 件と一致する必要がある。
- `resolve` の行き先は同じ言語の管理対象 library とする。
- 一致なし、複数一致、重複操作、既存 edge の再追加は config error とする。
- adapter 改善で不要になった override も一致なしの config error とし、削除を促す。
- dependency analysis の `failed` は override で回復できない。

raw dependency state が `partial` で、すべての unresolved を `resolve` または `external` により
分類できた場合は effective state を `complete` にする。
未解決が残れば `partial` のままとする。
effective dependency graph と fingerprint は override 適用後の結果から作る。
公開 edge が手動追加された場合は Web で `manual` と表示できるが、非公開 target の情報は
公開 DTO に含めない。

### 6.6 Rust adapter profile

Rust adapter は repository 内の独立した `analyzer.command` として Rust で実装する。
Cargo workspace / package / crate root は `cargo metadata --no-deps` から取得し、source syntax は
lockfile で固定した `syn` で解析する。core に Rust parser を組み込まず、正規表現や
rust-analyzer / LSP の availability に依存しない。

Cargo の crate root と通常の module file 規則から managed `.rs` file の module path を作り、
次を direct dependency candidate とする。

- external module declaration の `mod foo;`
- literal path を持つ `#[path = "..."] mod foo;`
- `use crate::...`、`use super::...`、`use self::...`
- 式、型、attribute などに直接書かれた同じ形式の path
- literal path の `include!`

inline `mod foo { ... }` 自体は同じ source file なので edge にせず、その内部にある direct path は
同じ target file の dependency として解析する。alias 付き use は元の path から edge を確定し、
同じ managed target への複数参照は 1 edge にする。`std`、`core`、`alloc` と Cargo の外部 crate は
`external` とする。

glob import、re-export 越しの名前解決、macro が生成する path、非 literal `include!`、通常の
module 規則だけで一意に解決できない path は、推測で internal edge にせず `unresolved` とする。
`cfg` で分岐する direct dependency は見落としを避けて構文上の候補の和集合を取り、候補を
安全に分類できなければ dependency state を `partial` とする。

symbol analysis は module item として現れる `struct`、`enum`、`union`、`trait`、`type`、`fn`、
`const`、`static`、`macro`、`mod`、`impl` などを対象とする。`impl` は対象型と任意の trait から
`impl Trait for Type` のような安定した name / qualified name / search names を作る。
item を生成する macro invocation などで宣言を完全に列挙できない場合は symbol state を
`partial` にするが、それだけを理由に dependency state を下げない。

Rust adapter は type check や test の成功を表明しない。コンパイル、test、lint は言語の
`check_command` に設定した `cargo check` / `cargo test` などが担当する。`lib.rs`、`main.rs`、
`mod.rs` も他の managed source と同じ 1 file / 1 page 候補とし、公開不要なら sidecar の
`publish = false` で隠す。

### 6.7 C++ adapter profile

C++ adapter は repository 内の独立した `analyzer.command` とし、version を固定した
Clang LibTooling を使う。core に C++ parser を組み込まず、include の正規表現抽出や
`clang -M` の推移依存一覧を direct edge として流用しない。

標準 version、include path、define などの共通 compile profile は repository 内の
adapter-owned file に保存し、adapter command と `check_command` の双方から参照する。
core は profile の C++ 固有の意味を解釈しない。OJ との互換確認のため check が GCC を使い、
解析が Clang を使う構成も許可し、両方を observed / expected toolchain に含める。

Clang preprocessor の inclusion callback と SourceManager を使い、include directive の発生元と
実際の解決先を取得する。library target 自身、または solution-owned source から発生した include
だけをその target の direct dependency とし、included file 内から発生した nested include は
最初の target の direct edge に加えない。

include 先は次のように分類する。

- managed source へ解決できる include は `internal`
- system header または明示された外部 package は `external`
- repository 内にあるが managed manifest 外の file は `unresolved`
- 解決不能、曖昧な macro include、compile profile 不足は `unresolved`

条件コンパイルは checked-in compile profile を active configuration とする。inactive branch に
managed source へ解決できる literal include がある場合は安全側の direct edge へ加える。
inactive branch の macro include などを一意に解決できなければ dependency state を `partial` とする。

symbol analysis は AST の spelling location が対象 managed file にある declaration だけを扱う。
namespace、class、struct、union、enum、alias、typedef、concept、function、variable、template、
specialization、macro などを対象にし、implicit declaration、system header 由来 declaration、
include guard macro を除外する。nested declaration は qualified name を持てる。anonymous declaration は
location を使った安定 name を adapter が作る。

AST / semantic 解析だけが失敗しても preprocessor から direct dependency を
安全に確定できた場合は、dependency state を `complete` のまま symbol state だけを
`partial` / `failed` にできる。
コンパイル、test、lint は adapter でなく `check_command` が判定する。

実装時は次の Clang 公式 API を基準にする。

- https://clang.llvm.org/doxygen/classclang_1_1PPCallbacks.html
- https://clang.llvm.org/docs/LibASTMatchersTutorial.html

### 6.8 Lean adapter profile

Lean adapter は repository 内の独立した Lean executable とし、exact `lean-toolchain` の
`lake env` 内で実行する。core に Lean parser を組み込まず、target source を Lean frontend で
処理する。target 自身の symbol を stale `.olean` だけから取得しない。

source header は `Lean.Parser.Module.parseHeader` で解析し、Lake package / library root と
Lean search path を使って import name を実際の `.lean` / `.olean` module へ解決する。
plain `import`、`public import`、`meta import`、`import all` は公開範囲や phase が異なっても、
同じ module への direct dependency edge として扱う。

import 先は次のように分類する。

- managed `.lean` source へ一意に解決できる module は `internal`
- Init、Std、Mathlib、外部 Lake package などは `external`
- missing、複数候補、managed root 内だが manifest 外の module は `unresolved`

implicit prelude は ambient external toolchain として扱う。`open`、`include`、section variable、
namespace の開始は file dependency にしない。header parse が一部成功し、信用できる import と
unresolved を分離できる場合は `partial`、header 全体を信用できない場合は `failed` とする。

body は command を順に parse / macro expand / elaborate する。Lean の command grammar は
import や先行 command によって拡張されるため、built-in command だけを列挙する固定 parser を
adapter 側に再実装しない。import 完了直後の environment と target elaboration 後の environment の
差、command syntax、declaration range / server information を組み合わせ、target source に属する
symbol を抽出する。

symbol は少なくとも次を対象にする。

- `def`、`abbrev`、`opaque`
- `theorem`、`lemma`、`axiom`
- `inductive`、`structure`、`class`、`instance`
- constructor、field
- namespace、notation、syntax、macro

primary declaration と source に明示された constructor / field を優先し、source location を
target へ対応できない内部 recursor などの生成物は除外する。unnamed instance は elaborated type と
location から安定した name / qualified name / search names を作る。custom command が追加した
constant も environment diff と source range を安全に対応できれば generic `declaration` kind で返す。

declaration の kind、name、location を完全に対応できない場合は symbol state を `partial` にする。
body elaboration が失敗しても header の全 direct import を安全に分類できていれば
dependency state は `complete` のままにできる。Lean が返す byte position は target の
UTF-8 source で検証し、共通契約の 1-based line / Unicode scalar column / exclusive end へ
変換する。

Lean adapter は proof や build の成功を表明しない。module build、proof check、lint は
`check_command` に設定した `lake build` などが担当する。

実装時は次の Lean 公式資料を基準にする。

- https://lean-lang.org/doc/reference/latest/Source-Files-and-Modules/
- https://lean-lang.org/doc/reference/latest/Notations-and-Macros/Elaborators/

### 6.9 adapter の配置と build

adapter の source は managed library root の外に置く。MVP の配置は次を基準にする。

```text
tools/library-analyzers/
  build-inputs.toml
  prepare
  build
  protocol/
    analysis-v1.schema.json
    fixtures/
  rust/
  cpp/
  lean/

target/library-analyzers/
  prepared/<dependency-id>/
    manifest.json
    cargo-home/
    lean-toolchain/
    lean-packages/
    clang/
  builds/<build-id>/
    manifest.json
    bin/
      rust-analyzer
      cpp-analyzer
      lean-analyzer
  bin -> builds/<build-id>/bin
  prepare.lock
  build.lock
  build-in-progress
```

共通 protocol の正本は core が所有する Rust の型とし、C++ / Lean adapter が参照できる
JSON Schema をそこから生成する。生成済み schema は repository に commit し、CI で
再生成結果との差分がないことを検証する。adapter ごとの protocol fixture も同じ
directory に置く。

MVP は adapter protocol version `1` だけを実行時に受理し、version negotiation や複数 version の
後方互換処理を持たない。`adapter.name` / `adapter.version` は実装 identity であり、protocol
version の代用にしない。breaking change では schema、core、全 adapter、fixture を同じ変更で
次の version へ更新する。

`tools/library-analyzers/build` は Rust、C++、Lean adapter を安定した順序で staging directory へ
build する。全 executable の protocol handshake に成功した後だけ、build set を
`target/library-analyzers/builds/<build-id>/` へ確定し、`bin` symlink を新しい
`builds/<build-id>/bin` へ atomic に切り替える。language config の `analyzer.command` は
この symlink 配下の完成済み executable を shell を介さず直接起動する。

各 build set の `manifest.json` は protocol version、adapter identity、observed toolchain、
各 executable の cryptographic hash を持つ。

build の同時実行制御と成果物の有効状態は分離する。`build.lock` は同じ worktree で
最大 1 件の build process だけが保持できる OS advisory lock とし、process の正常終了、
失敗、crash で自動解放される。lock を取得できなければ待機しない。
別 build が実行中であることを表示して失敗する。

build process は lock 取得後に `build-in-progress` marker を作り、全 build、handshake、manifest、
symlink 切り替えの成功後だけ marker を除去する。失敗や中断では marker を残す。
lock は process 終了により解放する。次回 build は残った marker を回復対象として
再試行できる。成功後にだけ marker を除去する。stale lock directory の手動削除や
`--force` option は設けない。

`tools/library-analyzers/build` は必要に応じて、OS lock、staging、manifest、atomic publish を扱う
repository-local な小さい Rust build driver の launcher として実装してよい。この driver も
build input digest の対象とする。

`tools/library-analyzers/build-inputs.toml` は build freshness を判定する入力を宣言する。
言語ごとに recursively hash する adapter source directory と、protocol、build script、root の
Cargo manifest / lockfile、toolchain pin、Lake lockfile、C++ compile profile などの追加 file を
リポジトリ相対パスで列挙する。core は language ID を解釈せず、同じ schema と hash 手順を
使う。

input digest は、宣言された directory 配下の通常 file と追加 file のリポジトリ相対パス、file
内容を byte 順の安定した順序で hash して求める。directory symlink と file symlink は入力にせず
error とし、missing input、repository 外 path、重複 input、入力 directory 同士の重なりも
config error とする。新規 file と未 commit の内容変更も digest に含める。

build set manifest は `input_digest`、target platform、build profile、protocol version、handshake
identity / toolchain、executable hash、監査用の Git commit SHA を持つ。`build-id` は Git SHA や
timestamp だけで決めず、これらの正規化済み manifest 内容と executable hash から導出する。
Git SHA は表示と監査に使うが、freshness の代用にしない。

dependency / toolchain artifact の取得と adapter build は分離する。

- `tools/library-analyzers/prepare` は lockfile、revision、checksum で固定された dependency と
  toolchain artifact を取得し、local cache を準備する。
- `prepare` は dependency version を再解決せず、lockfile、manifest、checksum を変更しない。
- `tools/library-analyzers/build` は network access を行わず、準備済み cache だけを使う。
- dependency または toolchain artifact が不足・不一致なら、build は `prepare` を案内して
  marker を残したまま失敗する。
- dependency 更新は独立 PR で pin、lockfile、checksum を明示的に変更し、3 言語 fixture、check、
  analysis diff を確認する。

prepared dependency は global cache に暗黙依存せず、ignore 対象の
`target/library-analyzers/prepared/<dependency-id>/` に content-addressed set として置く。
`dependency-id` は正規化した lockfile、toolchain pin、artifact checksum、target platform から
決定し、timestamp や Git commit SHA だけには依存しない。

`prepare` は一意な staging directory へ取得し、次を検証した後だけ expected dependency ID の
directory として確定する。

- manifest が宣言する dependency、revision、checksum、toolchain identity
- Cargo registry / source、Lean package、Clang artifact の実体
- target platform と prepared set の一致
- expected path 外への symlink、device file、socket などがないこと

remote dependency source は public HTTPS に限定する。SSH / SCP style URL、userinfo を含む URL、
平文 HTTP、credential が必要な registry / repository は MVP で拒否する。Git dependency は
完全な commit hash、download archive と compiler artifact は cryptographic checksum を必須にする。
branch、tag、`latest`、version range だけの可変参照は、lockfile や manifest で immutable な
revision / artifact へ解決されていなければ prepare input として認めない。

local path dependency は repository 内の通常 file / directory に限定し、その relative path と
内容を dependency input digest へ含める。repository 外 path、global install 済み package、
環境変数による未宣言 source override へ fallback しない。

prepare job は OJ credential、GitHub App private key、registry token、SSH agent を受け取らない。
download 後の revision / checksum 検証が終わるまで prepared set へ公開しない。archive 展開では
absolute path、`..`、symlink、hard link、device file、socket を拒否し、展開先を staging set 内へ
限定する。

downloaded dependency の build script や Lake configuration が build 時に実行され得るため、
adapter build job も secret を持たず network access を許可しない。prepare は取得と検証に必要な
tool だけを起動し、downloaded installer script を任意実行しない。

不完全な directory や manifest のない directory は cache hit として扱わない。build は現在の
pin / lock / checksum から expected dependency ID を再計算し、一致する prepared set だけを使う。
CI cache / artifact から復元した場合も manifest、revision、checksum を再検証する。

Cargo home / target、Lake package / build directory、Elan / Lean toolchain、Clang artifact の location は
build driver が repository-local target 配下へ明示する。Lake package config の `packagesDir` と
`buildDir` を使用し、`.lake`、Cargo target、downloaded source などの生成物を
`tools/library-analyzers/**` へ作らない。これにより build output が adapter input digest を変える
循環を防ぐ。

`prepare.lock` は build lock と同じ OS advisory lock 方針で同一 worktree の prepare を
fail-fast させ、process 終了時に自動解放する。既存 prepared set を変更せず、新しい set の
全検証成功後だけ公開する。古い prepared set の自動 cleanup は MVP に含めない。

Lake の workspace / cache directory 契約は次の公式資料を基準にする。

- https://lean-lang.org/doc/reference/latest/Build-Tools-and-Distribution/Lake/
- https://doc.rust-lang.org/cargo/commands/cargo.html
- https://doc.rust-lang.org/cargo/reference/source-replacement.html

`npm run site:build` は `prepare` を暗黙に呼ばず、offline な adapter build から開始する。
fresh environment の CI は secret を持たない準備 step / job で `prepare` を実行し、検証済み cache を
後続の build / site job へ渡す。OJ credential を持つ job は `prepare`、adapter build、analyzer の
いずれも実行しない。

MVP の network-free build / analysis は、prepared-only resolution、ecosystem の offline option、
sanitized process environment で保証する。OS-level socket sandbox や container の
`--network none` は MVP の必須条件にしない。この限界を運用資料へ明記する。
将来 private source や secret を同じ job で扱う場合は、先に OS-level egress sandbox を導入する。

prepare、build、analyzer process は親 process の environment をそのまま継承しない。build driver / core
は phase ごとの固定 allowlist から child environment を組み立てる。

- build / analysis の `PATH` は prepared toolchain directory と必要最小限の system path から作る。
- Cargo、Lake、Lean、Clang の cache / toolchain path と一意な temporary directory を明示する。
- locale、timezone、color 制御など、出力再現性に必要な非 secret 値だけを明示する。
- OJ token、GitHub token / private key、SSH agent、cloud credential、registry credential、proxy、
  user-global tool config を build / analyzer へ渡さない。
- language / adapter config から任意の environment variable 名や値を要求する機能は設けない。
- Cargo は lockfile の変更を拒否する locked mode と offline mode を併用する。
- Lake は prepared package / toolchain だけで解決し、missing dependency を自動取得せず失敗させる。

prepare だけは public HTTPS 取得に必要な proxy と CA certificate path の allowlist を持てる。
それらの値は process memory 以外の manifest、artifact、cache key、stdout / stderr log、error detail へ
保存しない。proxy credential が必要な環境でも、取得 artifact 自体は public source に限定する。

protocol handshake は専用の `--protocol-info` mode を設けず、各 adapter へ通常 protocol の
空 target request を渡す。

```json
{
  "schema_version": 1,
  "repository_root": ".",
  "language": "rust",
  "libraries": [],
  "solutions": []
}
```

response は通常解析と同様に exit 0、UTF-8 JSON 1 文書、対応する schema version、adapter
identity、observed toolchain、空の `libraries` / `solutions` を返す。build script は core と同じ
Rust protocol 型と schema validation を使い、検証済み identity と toolchain を manifest へ
記録する。専用 handshake response schema は作らない。

handshake は executable の起動、toolchain 認識、stdin / stdout protocol だけを確認する
smoke test とする。実 source の dependency / symbol 解析能力は各言語の fixture / contract test で
検証し、空 request の成功だけを adapter 実装完了の根拠にしない。

解析実行時には adapter を暗黙に build しない。`ce site-data generate` と verify は必要な
executable が存在しなければ、`tools/library-analyzers/build` の実行を案内して開始前に
失敗する。marker が残っている場合や、symlink、manifest、executable hash が不整合な場合も
古い executable を使わず、再 build を要求する。adapter build の失敗を古い executable による
解析成功として扱わない。

marker がある場合、`ce site-data generate` と verify は OS lock を非破壊で確認し、取得中なら
「build 実行中」、未取得なら「前回 build 失敗または中断」と区別して停止する。
どちらの場合も同じ build command を案内する。marker や lock の手動削除は要求しない。

`ce site-data generate` と verify は現在の build input digest を同じ汎用処理で再計算し、
manifest と完全一致しなければ全 mode で再 build を要求する。新言語では source directory と
追加 input を `build-inputs.toml` へ加え、core の hash 実装や language 分岐を変更しない。

build 失敗時は新しい set へ切り替えず、既存 build set も削除しない。ただし marker により
解析利用は停止する。古い build set の自動 cleanup は MVP に含めず、ignore 対象の `target/` と
同様に手動 cleanup へ任せる。

`npm run site:build` と production CI は adapter build を最初の step とし、その後に check、
site-data、Astro、Pagefind、最終検証を行う。各 ecosystem の build cache は toolchain lock、
adapter source、依存 lockfile を key に再利用してよいが、完成 executable と
`AnalysisSnapshot` を同一の永続 cache として扱わない。

新しい言語は `tools/library-analyzers/` の新 directory、生成 schema に従う adapter、config の
追加で接続する。core や既存 adapter に language ID 固有の build 分岐を追加しない。

## 7. check と verify

### 7.1 check

check はローカルまたは CI 内で完結する処理である。

- `cargo test`
- property-based testing
- `lake build`
- 証明チェック
- lint など

実行方法は言語ごとの config または既存の解法 `test_command` へ委譲する。アプリ本体は
テストや証明の意味を解釈しない。

- `ce check` は全言語の `check_command` を言語 ID 順に 1 回ずつ実行する。
- `ce check --language <id>` はローカル利用時だけ 1 言語へ限定する。
- `check_command` がない言語は明示的に `skipped` と表示し、失敗にはしない。
- 通常の公開 solution の `test_command` は `ce check` から一括実行しない。
- solution を個別に確認する既存の `ce test` の役割は変えない。
- 複数の check が失敗しても残りを実行し、最後に失敗を集約して exit 1 とする。
- 言語 check の timeout は既定 600 秒とし、`check_timeout_seconds` で上書きできる。
- solution test の timeout は既定 600 秒とし、`test_timeout_seconds` で上書きできる。
- どちらの timeout も正の整数秒だけを許可する。
- stdout と stderr は実行中に端末または CI ログへ逐次転送する。
- timeout は check failure として扱い、残りの対象を引き続き実行する。
- 結果は端末または CI ログへ出す。
- 結果ファイルや Web 公開データへ含めない。
- check 失敗時は CI、公開ビルド、新規 OJ 提出を失敗させる。
- 解法の事前 check が失敗した場合、verify 状態は `never` または `stale` のままとする。

CI の通常 check と公開 build は、言語 filter なしの `ce check` を使う。
filter 付き check の成功だけを repository 全体の公開可否には使わない。

check / test command は既存契約どおり Unix の `sh -c` で起動する。各 command を独立した
process group に置き、timeout 時は group 全体へ `SIGTERM` を送り、5 秒後にも残っていれば
`SIGKILL` を送る。shell だけを終了して compiler や test runner を残さない。
Unix 以外では既存の `test_command` と同様に unsupported とする。

### 7.2 verify

verify は解法を OJ へ提出し、その判定によって明示されたライブラリを検証する
処理である。

verify 用解法は `publish = true` と有効な `solved_at` を持つ公開解法に限定する。
非公開解法に `[verify]` が設定されている場合は config error とする。MVP では、
verify 結果の根拠となる解法コードを必ず Web から閲覧可能にする。
`[verify].libraries` が非公開 library を直接指す場合も config error とする。
verify 用解法では既存の `test_command` を必須とする。
`[verify].language_id` は任意の OJ 提出言語 ID override とし、`[verify]` 外には置かない。

新規提出前には、実行対象に含まれる言語の `check_command` を言語 ID ごとに 1 回実行し、
各 verify 用解法の `test_command` を solution ID 順に実行する。同じ言語の複数解法を
処理しても言語 check は重複実行しない。どちらも失敗を集約し、1 件でも失敗すれば
preprocess、plan 作成、新規提出へ進まない。

保存済みの `Starting`、`AcceptanceUnknown`、handle は、現在の check より前に回復または
terminal まで追跡する。すでに OJ へ送った可能性がある attempt の追跡を、現在の source の
失敗によって止めない。check は新規提出だけを止める。

- `depends_on` から `verifies` を自動生成しない。
- 1 解法が複数ライブラリを verify しても提出は 1 回とする。
- 同一の提出対象を複数登録しても、実行計画で重複排除する。
- 通常の `ce verify` は `never`、`stale`、中断された追跡可能 attempt だけを処理する。
- 現在の入力に対応する rejected 結果は再提出せず、その結果を使って exit 1 とする。
- `--force` は MVP に含めない。
- 1 解法につき verify spec は最大 1 つとし、複数 profile を許可しない。
- verification ID は solution ID と同一にする。
- 別の問題、言語、提出方法は別の solution directory で表現する。

`publish = true` だが `[verify]` を持たない通常 solution は `not_configured` とする。
これは verify の失敗や未実行ではなく、提出対象外であることを表す中立状態である。
`ce verify` は `not_configured` solution を実行対象にしない。

既存 result がある solution から `[verify]` を削除する場合は、同じ変更で result JSON も
削除しなければ config error とする。過去の result を現在も有効に見せたまま
`not_configured` へ移行させない。逆に `[verify]` を追加した時点では `never` となる。

library の代表 verification status は、その library を `[verify].libraries` で直接指定する
全 solution から導出する。dependency closure に含まれただけの solution は evidence にしない。

- direct verifier が 0 件なら `never` とする。
- direct verifier が 1 件以上あり、全件が current accepted の場合だけ `verified` とする。
- それ以外は `rejected > unavailable > stale > never > verified` の優先順位で代表状態を選ぶ。
- `not_configured` は solution 専用であり、library 集約へ入れない。
- list、search filter、トップ集計には代表状態を使う。
- library detail では代表状態に加え、direct verifier ごとの evidence と状態をすべて表示する。
- `verified:true` は、全 direct verifier が current accepted である library にだけ一致する。

## 8. 提出・結果追跡ライフサイクル

提出開始と判定追跡を分離し、`ce verify` と将来の `ce submit --watch` で共有する。

```text
prepare
  -> start_submission
  -> SubmissionStart
       - Trackable { handle }
       - UserActionRequired { url }
       - Unavailable { reason }
```

追跡可能な提出は共通の待機処理へ渡す。

```text
wait_for_result(handle)
  -> Queued
  -> Judging
  -> Completed(JudgeResult)
```

`SubmissionHandle` はプロセスをまたいで resume できるよう永続化可能にする。

- OJ
- submission ID
- submission URL
- OJ 固有の追跡用 locator
- submitted_at

### 8.1 SubmissionPlan

利用者向けの `ce verify` は単一コマンドのままにするが、内部 usecase は secret を使わない
prepare と、OJ secret だけを使う start / poll に分ける。

```text
prepare / plan
  -> persist Starting
  -> start submission
  -> persist handle
  -> poll
  -> persist terminal result
```

prepare は discovery、check、dependency analysis、preprocess、fingerprint 計算を行い、
実際に提出する内容を immutable な `SubmissionPlan` として確定する。

- solution ID
- attempt ID
- 置き換え対象となる直前の result attempt ID。結果がなければ `null`
- OJ、contest、problem、内部 language ID、解決済み OJ 提出言語 ID
- 提出する正確な source bytes または専用 source file
- submitted-source hash
- verify fingerprint
- verifies 対象 library ID
- plan schema version
- plan 全体の hash

start は plan の schema と hash を検証し、plan に固定された source bytes をそのまま送る。
source を再生成せず、analyzer、check、preprocess、shell command を一切実行しない。
poll も保存済み handle から OJ の状態を取得するだけとし、任意コマンドを実行しない。

これにより、fingerprint を計算した source と OJ へ提出した source を一致させる。
prepare 後に working tree や main が変わっても、進行中の attempt は plan に固定された内容を
terminal まで追跡する。現在の repository との違いは次の build で stale として扱う。

利用者向け command は次を基本とする。

```text
ce verify                  # repository 全体の対象
ce verify abc999/a/main    # 1 solution だけへ絞る
```

CI が job 境界で使う plan / start / poll 操作は `ce internal verify-*` 相当の repository 内部
interface とし、一般利用者向けの安定 CLI とはしない。将来の `ce submit --watch` は同じ
start / poll usecase を再利用する。

`ce verify` の流れ:

1. 保存済みの `Starting`、`AcceptanceUnknown`、handle があれば、新規処理より先に resume する。
2. discovery、dependency analysis、fingerprint 計算により新規実行対象を決める。
3. 新規対象に関係する言語 check と solution の `test_command` を実行する。
4. preprocess して immutable plan を作る。
5. 提出前に `Starting` attempt を保存する。
6. 提出を開始する。
7. handle を取得した直後に `Submitted` として保存する。
8. ポーリングして終端判定を取得する。
9. 最新結果を原子的に保存する。

一時的な通信エラー、ポーリングのタイムアウト、OJ の判定結果を区別する。
提出されたか不明な通信切断では、安易に同一内容を再提出しない。

提出要求の送信後、submission ID を受け取る前に通信が切れた場合は、
`AcceptanceUnknown` として attempt を保存する。次回実行時は次の順で扱う。

1. OJ 側から提出を一意に復元できる場合は handle を回復する。
2. OJ 側から未提出を確実に証明できる場合だけ attempt を破棄し、再計画を許可する。
3. それ以外は自動再提出せず、`infrastructure_error`、exit 1 とする。
4. 将来、submission ID の手動関連付けや未提出確認を行う
   修復コマンドを追加できるようにする。

この修復操作は、同一内容を無条件に再提出する `--force` とは分ける。

### 8.2 Starting と受付不明状態の回復

`SubmissionPlan` の作成と `Starting` の永続化を、OJ への接続前後を区別する境界にする。

- plan だけが存在し、`Starting` が存在しない attempt は、提出要求を開始していない。
  fingerprint が変わっていれば安全に破棄して再計画できる。
- `Starting` は、提出要求を送った可能性がある attempt を表す。直接再送してはならない。
- `Starting` の永続化に失敗した場合は、OJ へ一切接続しない。
- start job は、永続化済み `Starting` と plan hash を確認してから OJ へ接続する。

`Starting` には回復と監査に必要な次の情報を保存する。

- attempt ID
- plan hash と submitted-source hash
- OJ、contest、problem、内部 language ID、解決済み OJ 提出言語 ID
- start job が開始された `started_at`

resume で `Starting` または `AcceptanceUnknown` を見つけた場合は、OJ adapter の
`recovery_mode` に従って次の順で処理する。

1. 対応する提出を一意に特定できた場合は、handle を復元して poll する。
2. OJ が未提出を確実に証明できた場合だけ、attempt を破棄して
   現在の入力から再計画する。
3. 一致候補が 0 件または複数件、回復 API が利用不能、
   あるいは確証を得られない場合は、
   `AcceptanceUnknown` として保存し、自動再提出しない。

Actions が実際には POST 前に停止していたとしても、永続状態と OJ から
それを証明できなければ未提出とは扱わない。main の更新や fingerprint の変化も、
この回復手順を迂回する理由にはしない。
handle を回復した attempt は、現在の fingerprint と異なっていても terminal まで追跡する。

`AcceptanceUnknown` がある間は、同じ OJ への新規提出を止め、bot PR を draft のまま残す。
将来の修復操作は、管理者が submission ID を関連付けるか、OJ 上の未提出を確認して
attempt を破棄する操作に限定する。確認なしに再送する `--force` は MVP に設けない。

### 8.3 InfrastructureFailure

main / Web に公開しない運用上の再開状態として `InfrastructureFailure` を持つ。
CI では bot の draft PR にある対象 result JSON 内だけへ保存し、terminal result として
main へ merge しない。

```text
InfrastructureFailure:
  stage: prepare | start | poll
  error_kind: network | rate_limited | service_unavailable |
              credentials_missing | authentication_rejected |
              invalid_response | schema_error | other
  retryable: boolean
  retry_count: integer
  next_retry_at: RFC 3339 timestamp | null
  updated_at: RFC 3339 timestamp
  summary: sanitized string
  plan_hash: optional
  attempt_id: optional
  handle: optional
```

`summary` は allowlist 化した短い説明だけとし、raw response、credential、cookie、token、
request / response header を含めない。公開 repository の draft PR から読まれる前提で扱う。

- network、rate limit、OJ の 5xx は retryable とする。
- credential 不足、認証拒否、schema error、response parse failure は
  operator action required とし、定期的な自動再試行を止める。
- retryable failure は `next_retry_at` より前に OJ へ接続しない。
- operator action required は Actions summary と draft PR summary へ修復理由を表示する。
- current main で fingerprint または plan hash が変わった場合は、未提出の plan を破棄して
  prepare からやり直せる。

同一 command 内では、既定の一時エラー backoff 最大 30 秒と待機上限 15 分を使う。
workflow をまたぐ retry は 5 分から始め、失敗のたびに 10、20、40、80 分と倍増させ、
最終的な上限を 6 時間とする。OJ の `Retry-After` が計算値より長い場合は
そちらを優先する。

start の retry は、要求を未送信または OJ が未受付を確実に返した場合だけ許可する。
受付の可能性が残る通信切断は retry count に入れず、直ちに `AcceptanceUnknown` とする。
未受付を確証した start が 5 回連続で失敗した場合は `retryable = false` へ切り替え、
operator action required とする。

handle 取得済みの poll は新規提出を伴わないため、連続失敗回数だけでは停止しない。
6 時間間隔を上限として自動 retry を継続し、同じ OJ の新規提出は止めたままにする。
OJ への接続成功または判定の進行を確認したら infrastructure failure count を 0 へ戻す。
正常に取得できた pending 判定が 15 分続いたことによる timeout は障害回数に含めない。

提出前または OJ が未受付を確証した start failure では、その attempt を終了する。
同じ入力なら immutable plan を再構築して hash を照合できるが、次の start には新しい
attempt ID を割り当てる。以前の `Starting` を再利用しない。

handle 取得後の failure では attempt ID と handle を維持し、start を呼ばず poll だけを
再開する。修復後に terminal result が得られたら、draft 内の `InfrastructureFailure` を
同じ attempt の terminal result で置き換え、通常の結果 PR lifecycle に戻す。

MVP では OJ ごとに同時進行する提出を最大 1 件とし、提出と判定待ちを直列化する。
未完了 handle が存在する場合は、その追跡を新規提出より先に行う。判定待ちが timeout
した場合は handle を保存してコマンドを終了し、後続の新規提出へ進まない。

ポーリングの状態取得は OJ 実装、待機・backoff・timeout は共通ライフサイクルが担う。
MVP の待機方針は次とする。

- 最初は 2 秒間隔でポーリングする。
- OJ が `Retry-After` を返した場合は、その期間以上待つ。
- pending が続く場合は間隔を徐々に延ばし、最大 15 秒とする。
- 一時的な通信エラーには指数 backoff を適用し、最大 30 秒とする。
- 1 回のコマンドで待つ上限は 15 分とする。
- 15 分を超えた場合は handle を `pending` のまま保存し、次回実行で resume する。
- 認証失敗など再試行で直らないエラーは即座に exit 1 とする。
- ポーリング間隔は config へ公開しない。
- 必要になった場合だけ待機上限を設定可能にする。

## 9. OJ 対応能力

矛盾する boolean の組み合わせを避けるため、提出方式と結果詳細を列挙型で宣言する。

```text
submission_mode:
- unattended_trackable
- interactive_trackable
- interactive_untrackable
- unsupported

result_detail:
- overall_only
- summary_metrics
- testcase_details

recovery_mode:
- exact
- best_effort
- none
```

`ce verify` は `unattended_trackable` を必要とする。`ce submit --watch` は将来
`interactive_trackable` にも対応できる。

`recovery_mode` の意味は次とする。

- `exact`: attempt に対応する handle を一意に回復するか、未提出を確実に証明できる。
- `best_effort`: 提出履歴などから一意な handle を回復できる場合があるが、候補 0 件は
  未提出の証明にならない。
- `none`: handle を失った attempt を OJ 側から回復できない。

MVP では、現在直接提出できる LibraryChecker を `unattended_trackable` として扱う。
回復能力は `best_effort` とし、problem、language、submitted-source hash、受付時刻に対応する
提出履歴から一意に特定できた場合だけ handle を回復する。
履歴に候補がないことだけでは未提出を証明できない。
AtCoder のブラウザ提出は `interactive_untrackable` とし、verify 対象に登録された場合は
`unavailable`、`ce verify` は exit 1 とする。AtCoder の無人 verify と userscript からの
submission ID 受け渡しは将来機能とする。

`unavailable` は、同じ能力設定と入力で再実行しても変わらないことが
確定した場合だけ返す。

- `interactive_untrackable` または `unsupported` な提出方式
- 未対応 OJ
- 対象の OJ、問題、言語、提出方式の組み合わせを確実に扱えない場合

credential 不足、認証失敗、通信障害、rate limit、OJ の 5xx、response parse failure は
`infrastructure_error` とする。これらを公開上の OJ 能力不足として固定しない。

OJ 固有の提出、認証、ポーリング、レスポンス解析は infrastructure 層の Rust 実装へ置く。
言語解析アダプターと異なり、MVP では OJ 処理を外部コマンド化しない。既存の
`OnlineJudge` を肥大化させず、提出開始と判定追跡の port を分けて実装する。

## 10. verify 状態と終了コード

公開上の状態とコマンドの成否を分ける。

| 状態 | 意味 | `ce verify` |
| --- | --- | --- |
| `verified` | 現在の入力に対する成功結果 | exit 0 |
| `rejected` | 現在の入力に対する WA、TLE、CE など | exit 1 |
| `unavailable` | OJ の確定した能力不足により提出・追跡不能 | exit 1 |
| `infrastructure_error` | 通信・解析などの運用上の失敗。次回 resume 対象 | exit 1 |
| `pending` / `judging` | 追跡中または待機タイムアウト。次回 resume 対象 | exit 1 |
| `stale` | 現在の入力と保存結果が一致しない | 実行対象 |
| `never` | 結果が存在しない | 実行対象 |
| `not_configured` | 公開 solution に verify spec がなく提出対象外 | 対象外 |

verify が rejected や unavailable でも、取得できた最新結果は保存して Web に公開する。
解析・schema・整合性エラーでは新しい公開データを作らず、以前の Web を維持する。

`infrastructure_error` は terminal result として main へ merge せず、Web に公開しない。
まだ結果がなければ公開状態は `never`、以前の結果が現在と異なれば
`stale` のままにする。
提出前に未受付を確証できた error では `Starting` を破棄し、Actions summary に失敗を出す。
handle 取得後の error では handle と draft PR を残し、次回 poll で resume する。
再開に必要な分類済み情報は draft PR の `InfrastructureFailure` に保存する。

現在の fingerprint に一致する `rejected` と `unavailable` は自動再実行しない。
source、依存、verify 設定、OJ adapter の名前・version・能力設定のいずれかが変わり、
fingerprint が stale になった場合だけ通常の自動実行対象へ戻す。
変更なしの結果を明示的に再実行する command は MVP に設けない。

## 11. fingerprint と結果保存

fingerprint は最低限、次から作る。

- preprocess 後の実提出ソース
- solution ID
- 明示的に verify するライブラリの ID とソース
- その推移的依存先の ID とソース
- OJ、問題 ID、内部 language ID、解決済み OJ 提出言語 ID
- verify 設定
- OJ 提出・追跡アダプターの名前、バージョン、能力設定
- fingerprint schema version

ライブラリ集合は、`[verify].libraries` の明示対象と解法自身の内部依存を起点にし、
手動 override 適用後の内部依存グラフで推移閉包を取る。
解法が依存するだけのライブラリは stale 判定には影響するが、`verifies` へ追加しない。
推移閉包には非公開 `LibraryFile` も含め、その source hash の変化で stale にする。

closure は循環を許容して相対パス順に正規化する。
solution または closure 内 target の dependency analysis が `partial` / `failed` の場合は、
安全な fingerprint を作れないため新規提出しない。
現在結果を `stale` のままにして exit 1 とする。
symbol analysis の状態は fingerprint 作成を妨げない。

説明 Markdown や Web 表示専用メタデータは含めない。結果 JSON には単一の fingerprint だけで
なく、各入力の content hash も保存し、stale の理由を表示可能にする。

```json
{
  "attempt_id": "opaque-unique-id",
  "replaces_attempt_id": "previous-attempt-id",
  "fingerprint": "sha256:...",
  "language": {
    "id": "rust",
    "oj_language_id": "rust"
  },
  "verified_libraries": [
    "libraries/rust/algebra/monoid.rs"
  ],
  "inputs": {
    "submitted_source": "sha256:...",
    "libraries": {
      "libraries/rust/algebra/magma.rs": "sha256:...",
      "libraries/rust/algebra/monoid.rs": "sha256:..."
    }
  }
}
```

- 解法ごとに最新結果 1 件だけをリポジトリ内へ保存する。
- attempt の開始前に一意な `attempt_id` を割り当て、状態遷移中は変更しない。
- `replaces_attempt_id` には plan 作成時点の最新結果の attempt ID を保存する。
- 新しい結果は既存 JSON を原子的に置き換える。
- 提出途中と終端結果は同じ結果ファイルの `state` で表し、状態遷移ごとに置き換える。
- 過去結果は Git 履歴から参照し、Web では時系列表示しない。
- `source_commit` は追跡情報として保存してよいが、stale 判定には利用しない。
- OJ ごとに取得できない時間・メモリ・ケース別結果は optional とする。
- OJ の生レスポンスを無条件に保存せず、公開可能な固有情報だけを allowlist 化する。
- result は内部 language ID と実際に送信した OJ language ID の両方を保存する。
- result は提出時の direct `verified_libraries` を ID 順で保存する。
- `unavailable` は同じ OJ アダプター能力に対する最新結果として保持する。
- OJ アダプターの名前、バージョン、能力設定が変われば stale となり、再評価する。
- credential、通信、rate limit、5xx、parse failure は `infrastructure_error` とし、
  result JSON の terminal state として main へ保存しない。
- 依存の追加、削除、内容変更をそれぞれ stale 理由として保存・表示する。
- 解析 snapshot ID は追跡情報として保存するが、無関係な解析変更では stale にしない。
- 実行中にソースが変わった場合は、提出時 hash を保存する。
- 次の build で現在の hash と比較し、stale と判定する。

library の move / rename 後も同じ solution ID の result は削除しない。現在の
`[verify].libraries` と保存済み `verified_libraries` の ID 集合が異なるため stale とし、
新 library page ではその solution を stale evidence として表示できる。solution ID 自体の
move / rename では result の owner が変わるため、この再利用は行わない。

### 11.1 判定結果の共通 schema

既知の判定は次の `kind` へ正規化し、OJ の元の判定文字列を `raw` に残す。

```text
accepted
wrong_answer
time_limit_exceeded
memory_limit_exceeded
runtime_error
compile_error
output_limit_exceeded
judge_error
cancelled
other
```

verify 成功として扱うのは `accepted` だけとする。未知の判定は `other` とし、
OJ の値を失わないよう `raw` を必須にする。

時間は非負整数のミリ秒へ正規化し、より細かい値はミリ秒へ切り上げる。メモリは
非負整数の byte とする。取得不能な値は `null` とし、数値 0 とは区別する。

- `max_execution_time_ms` と `max_memory_bytes` は取得不能なら `null` とする。
- ケース詳細全体を取得不能なら `test_cases: null` とする。
- 取得可能だがケースが 0 件なら `test_cases: []` とする。
- ケース ID、時間、メモリも個別に `null` を許可する。
- ケース詳細があれば summary の最大値をケース列から再計算する。
- ケース詳細がなければ OJ の summary 値を採用する。
- OJ 固有項目は公開 allowlist を通した `extra` にだけ保存できる。

## 12. 静的 Web

通常ページはビルド時に HTML を生成し、検索だけブラウザ上の JavaScript で実行する。

MVP の Web 実装は Astro の静的生成と Pagefind の静的検索を採用する。Rust 側は
version 付きの正規化 JSON 生成までを担当し、Astro は完成済みデータの表示だけを行う。
Astro の build 後に Pagefind が生成済み HTML を index 化する。

Rust 側に公開専用の `site-schema` crate を置き、公開 DTO を schema の唯一の正とする。
domain model から公開 DTO へ変換し、site-data JSON、JSON Schema、TypeScript 型を生成する。
Web 都合の型を domain entity へ混ぜない。

- JSON Schema と TypeScript 型は build 時に生成し、手書きで二重管理しない。
- Astro build は入力 JSON を schema 検証する。
- breaking change では `schema_version` を上げる。
- Astro が未対応 version を読んだ場合は build を失敗させる。
- 同一 commit から build するため、複数 schema version の後方互換は持たない。
- デザイン fixture も同じ schema で検証する。
- 非公開 library の path、source、symbol、diagnostic を公開 DTO に入れない。
- 非公開依存による stale 理由は path を除いた共通メッセージへ変換する。
- solution diagnostic の公開 location は entry source 内だけに限定する。
- 非 entry solution file の location は内部 snapshot に保持するが、公開 DTO では location を
  `null` にし、表示対象外 file にあることを示す共通 reason だけを持たせる。
- 公開 library diagnostic が private dependency を指す場合も location と target 情報を除去する。

説明 Markdown とソースコードは Astro の build 時に HTML 化する。ブラウザ上で
runtime syntax highlight は行わない。

説明 Markdown の pipeline は次とする。

1. Rust 側で frontmatter を解析し、本文と構造化 metadata を分離する。
2. Astro 側で Markdown を parse し、GFM の表や task list を扱う。
3. Markdown 内の raw HTML は MVP では無効にする。
4. HTML は allowlist 方式で sanitize する。
5. 見出しには `doc-` prefix 付きの安定 ID を付ける。

page title を唯一の `h1` とするため、sidecar と `_index.md` の Markdown には
次の見出し契約を適用する。

- Markdown の ATX / Setext `h1` を禁止し、見つけた場合は build error とする。
- 見出しを使う場合は最初を `h2` とし、
  現在 level から 2 以上深くなる飛び越しを禁止する。
- Markdown の `h2` から `h6` は level を変更せず、そのまま HTML へ変換する。
- code fence 内の `#` や Setext 風文字列は見出し検証の対象外とする。
- 同名見出しには document order の suffix を付け、すべての ID を `doc-*` namespace に置く。
- paragraph だけで見出しがない本文も許可する。

見出し anchor は dependency の slug 実装へ委譲せず、次の repository 固有規則で生成する。

1. heading inline node から表示 plain text を抽出する。
2. exact UTF-8 text は Unicode normalization せず SHA-256 へ入力する。
3. ASCII `A-Z` を lowercase にし、`a-z0-9` の連続列を `-` で結んだ hint を作る。
4. hint は 48 byte で切って末尾の `-` を除き、空なら `h` とする。
5. digest の先頭 10 hex character を使い、`doc-{hint}-{digest}` とする。
6. 同一 document 内で同じ ID が重複する場合は 2 件目から `-2`、`-3` を付ける。

日本語だけの heading も `doc-h-{digest}` となり、platform、locale、Unicode library version に
依存しない。heading text の変更は permalink の変更として扱う。

固定の `Documentation` 見出しは追加せず、Markdown body を page header の後へ置く。
library detail では本文を非 landmark の `<div id="documentation">` で囲み、
in-page navigation の移動先とする。body が空の場合は wrapper と navigation item を生成しない。

ソースコードは Astro の Shiki ベースの code renderer で build 時に highlight する。
transformer で各行へ `id="L1"`、`id="L2"` のような安定 ID と permalink を付ける。
ソース本文は常に text として扱い、raw HTML として挿入しない。

`[library.languages.<name>].syntax_highlight` には Shiki の言語 ID を指定できる。
省略時は compro-env の言語 ID をそのまま試す。未知の言語や Markdown code fence は
plain text へ fallback して warning を出し、公開 build 自体は継続する。
`doc-*` は説明見出し、`L*` はソース行にだけ使い、anchor namespace を衝突させない。

```text
正規化済み JSON
  -> Astro による HTML 生成
       - トップ
       - 言語
       - ディレクトリ
       - ライブラリ
       - 解法一覧
       - コンテスト
       - 問題
       - 解法
  -> Pagefind による search index 生成
       - 検索ページでクライアント検索
```

### 12.0 デザイン handoff

視覚デザインは、route、ページ構造、状態、検索属性が確定してから
外部デザイン工程へ渡す。
確定済みの構造資料は
[Library Web semantic structure handoff](2026-08-10-library-web-structure-handoff.md) とする。
その時点で、次をまとめた構造資料と代表 fixture site を作る。

- route 一覧とページ間遷移
- ページごとの semantic HTML / component tree
- 各 component の入力データと表示状態
- verified、stale、rejected、unavailable、解析失敗などの代表 fixture
- 空一覧、長い依存一覧、長いソース、モバイル幅などの edge case
- 検索 query、filter、結果、行アンカーの interaction contract
- `data-pagefind-*`、source line ID、permalink など変更してはいけない属性
- accessibility 上必要な landmark、見出し階層、label、keyboard 操作

ユーザーが構造資料を外部デザインツールへ渡し、戻ってきたデザインを
Astro component と CSS へ統合する。
デザイン統合ではデータ意味、route、検索属性、行アンカーを変更しない。

### 12.1 URL と base path

route は次を基本形とする。

```text
/                                      # トップ
/search/                               # 検索
/libraries/                            # 公開ライブラリの言語一覧
/libraries/{lang}/                     # 言語
/libraries/{lang}/{directory...}/      # ディレクトリ
/libraries/{lang}/{source-path...}/    # ライブラリ
/solutions/                            # 公開解法一覧
/solutions/{contest}/                  # コンテスト
/solutions/{contest}/{problem}/        # 問題
/solutions/{contest}/{problem}/{name}/ # 解法
/404.html                              # static not-found page
```

生成ページは directory-style とし、canonical URL は末尾 `/` ありに統一する。
ライブラリ URL には言語 root からの相対ソースパスを使用し、拡張子、大文字・小文字を
維持する。パスの各 segment を UTF-8 で個別に percent-encode し、階層を表す `/` は
encode しない。内部の library ID は引き続きリポジトリ相対パスとし、公開 URL と分離する。
library / solution ID の move や rename では旧 route を生成せず、旧 URL は static 404 とする。
Git history から redirect を推測しない。

Astro の `site` と `base` は build 環境から渡す。ローカルや root 配信では `/`、
GitHub Project Pages では `/compro-env/` を base とする。内部リンク、canonical URL、
ソース行 permalink、asset、Pagefind bundle の URL は共通 helper から生成する。
root 始まりの文字列を各 component へ直書きせず、Pagefind の検索結果 URL も同じ base 配下へ
正規化する。

CI では少なくとも base `/` と `/compro-env/` の両方で static build と内部 link check を行う。

`/libraries/` は共通 navigation の Libraries link が指す常設 page とする。

- `h1` `Libraries` と言語 card の `ul` を持つ。
- 言語 card はトップ page と同じ component を再利用する。
- 公開 library がある、または language root に `_index.md` がある言語だけを掲載する。
- private library だけで `_index.md` もない言語は page を作らず、一覧からも存在を隠す。
- 言語は language ID の UTF-8 byte 順で並べる。
- 対象言語が 0 件でも `/libraries/` は生成し、empty-state message を表示する。
- `/libraries/` は Pagefind index から除外する。

解法階層では、公開 solution が 1 件以上存在する page だけを生成する。

- `/solutions/` は公開 solution 全体への入口とする。
- contest page は、その contest に公開 solution がある problem だけを列挙する。
- problem page は同じ問題の公開 solution を `solved_at` 降順、solution ID 昇順で並べる。
- contest / problem の表示名は既存 metadata を使い、取得できなければ ID を表示する。
- contest ID は OJ 間で衝突しない既存の namespace 済み ID を使う。
- 一覧、contest、problem page は Pagefind の検索対象外とし、solution detail だけを index 化する。
- 共通 navigation から `Libraries`、`Solutions`、`Search` へ移動可能にする。

存在しない route、削除済み page、非公開化された detail は static `404.html` で扱う。

- 404 も共通 header、global search、footer を使う。
- main は唯一の `h1` `Page not found`、短い説明、recovery navigation を持つ。
- recovery navigation は Home、Libraries、Solutions、Search へ link する。
- breadcrumb は `Home > Page not found` だけとする。
- requested path を表示するためだけの JavaScript は追加しない。
- 自動 redirect や類似 page の推測を行わない。
- `noindex` と Pagefind index 除外を設定する。
- root と project base の両 build で同じ `404.html` の asset / internal link を検証する。
- build / schema error の公開 error page は作らず、deploy を失敗させて既存 site を維持する。

### 12.2 共通 semantic layout

全ページで次の landmark と順序を維持する。

```text
body
|- skip link -> #main-content
|- header
|  |- site title / home link
|  |- primary navigation
|  |  |- Libraries
|  |  |- Solutions
|  |  `- Search
|  `- global search form
|- main#main-content
|  |- breadcrumb navigation
|  |- page header
|  |  |- page ごとに唯一の h1
|  |  |- summary
|  |  `- status
|  `- page-specific content
`- footer
   |- repository link
   `- build source commit SHA
```

- landmark は `header`、`nav`、`main`、`footer` を使う。
- primary navigation は list とし、現在位置へ `aria-current` を付ける。
- global search は label 付きの `GET` form とし、`/search/?q=...` へ遷移する。
- JavaScript は検索結果の取得と描画を拡張するが、form の通常遷移を壊さない。
- 各 page の `h1` は 1 件だけとし、section は見出し階層を飛ばさない。
- breadcrumb はトップ以外で表示し、現在 page は link にしない。
- モバイルでも semantic HTML は変えず、navigation と search form は CSS で折り返す。
- 必須の hamburger menu JavaScript は設けない。
- skip link とすべての操作要素に視認可能な keyboard focus を持たせる。
- デザイン統合で class と非意味的 wrapper は追加できるが、landmark、見出し順、form field、
  `aria-*` の意味は維持する。
- navigation、breadcrumb、footer は `data-pagefind-ignore` で検索本文から除外する。

### 12.3 トップページ

トップ page は次の section 順とする。

```text
page header
|- h1
`- repository summary

status overview
languages
recently updated libraries
recently solved solutions
attention required
```

- status overview は `dl` とし、公開 library 数、公開 solution 数、verify 状態別件数を表示する。
- 集計、recent、attention は公開対象だけから計算し、
  非公開 library の存在や状態を漏らさない。
- languages は `ul` とし、各言語を `article` の card として表す。
- 言語 card は表示名、言語 ID、公開 library 数、verify 状態別件数を持つ。
- recently updated libraries は `updated_at` 降順で最大 10 件表示する。
- recently solved solutions は `solved_at` 降順で最大 10 件表示する。
- 同時刻の tie-break は既定の library ID / solution ID 昇順を使う。
- 各 recent item は `ul` 内の `article` とし、detail page への link と日時を持つ。
- attention required は stale、rejected、unavailable、公開 library の解析失敗を最大 10 件表示する。
- verify 状態の残件は対応する filter 付き検索 URL へ link する。
- 検索 filter がない解析失敗は総数だけを併記し、表示中の detail link 以外を捏造しない。
- 件数 0 の section も heading を維持し、短い empty-state message を表示する。
- global search が header にあるため、トップ専用の重複した search form は置かない。
- トップ page 全体を Pagefind index から除外する。

### 12.4 言語・ディレクトリページ

言語 page と directory page は、同じ component contract の階層 browser として扱う。

```text
page header
|- h1
|- language ID または language-root-relative path
`- public verify status overview

overview
`- _index.md body

child directories
`- direct child directory list

library files
`- direct public library list
```

- breadcrumb で libraries、言語、親 directory、現在 page の階層を示す。
- `_index.md` がなければ overview section 自体を生成しない。
- `_index.md` がある空 directory は page を生成し、library files section に
  「公開 library はありません」という empty-state message を表示する。
- child directory item は title、root 相対 path、公開 descendant 数、verify 状態集計を持つ。
- library item は title、file name、updated_at、verify 状態を持つ。
- child directories と library files は別の見出し付き `section` と `ul` にする。
- 一覧は直下の項目だけを含め、descendant library を再帰的に展開しない。
- child directory は path 順、library file は path 順の既定 stable sort を使う。
- 集計と一覧には公開 library だけを含める。
- client-side の sort / filter は MVP では設けない。
- 言語・directory page 全体を Pagefind index から除外する。

### 12.5 ライブラリページ

library detail は tab へ分けず、次の順で単一の `article` にする。

```text
article
|- page header
|  |- h1
|  |- language / relative path / updated_at
|  `- verify / analysis status
|- in-page navigation
|- documentation
|- symbols
|- source
|- dependencies
|  |- depends on
|  `- used by
|- relations
|- verification evidence
`- diagnostics
```

- section ID は `documentation`、`symbols`、`source`、`dependencies`、`relations`、
  `verification`、`diagnostics` に固定する。
- in-page navigation は `nav` とし、実際に存在する section だけへ link する。
- in-page navigation 自体は `data-pagefind-ignore` とする。
- sidecar body がなければ documentation block を省略する。
- documentation は固定見出しを加えず、`div#documentation` 内へ Markdown body を描画する。
- symbol analysis が complete かつ空なら、symbols section に empty-state message を表示する。
- symbol analysis が partial / failed でも symbols section を維持し、warning と
  diagnostics section への link を表示する。
- source section は常に生成し、全行に安定した `L*` anchor を持たせる。
- source section の HTML contract は次とする。

```html
<section id="source" aria-labelledby="source-heading">
  <h2 id="source-heading">Source</h2>
  <div class="source-toolbar" data-pagefind-ignore>...</div>
  <pre class="source-code" tabindex="0"
       aria-labelledby="source-heading"><code data-language="rust">
    <span id="L1" class="source-line" data-line="1"><a
      class="source-line-number" href="#L1" aria-label="Line 1"
      data-pagefind-ignore>1</a><span class="source-line-content">...</span></span>
  </code></pre>
</section>
```

- `L*` は page 内で一意な 1-based line anchor とする。
- source line number と source text は別 element とし、line number は text 選択対象外にする。
- Shiki token element は `source-line-content` の内側だけに生成する。
- 空行にも `source-line` を生成し、line element 間に改行を保持する。
- `pre` は keyboard focus 可能な横 scroll region とし、MVP では長い行を折り返さない。
- source toolbar と line number は検索対象外、`source-line-content` は検索対象とする。
- toolbar は language、relative path、build source commit 上の repository link を持つ。
- repository link は表示中 source と同じ commit SHA と path を指す。
- source text は必ず escape し、highlight token 以外の HTML を挿入しない。
- 行範囲選択、wrap 切り替え、copy button は MVP に含めない。
- デザイン統合では `L*`、line permalink、line number と source text の分離を維持する。
- depends on と used by は direct edge だけを表し、公開 library への link だけを列挙する。
- 非公開依存は名前、path、件数を出さず、「非公開依存を含みます」とだけ表示する。
- external dependency は MVP では表示しない。
- relation は kind と公開 target library link を表示する。
- verification evidence は solution ごとの最新状態、solution link、OJ link、判定日時を表示する。
- verification evidence は `[verify].libraries` で直接指定した solution だけを列挙する。
- stale の場合は verification evidence 内に公開可能な stale reason を表示する。
- diagnostics は severity 順とし、location があれば library 自身の source line へ link する。
- tab UI は MVP で使わず、視覚デザインが sticky navigation を加えても全 section を HTML に残す。
- detail `article` を Pagefind body とし、page header の値は filter / metadata として登録する。

### 12.6 解法 browse ページ

solution browse は solutions、contest、problem の 3 段階とする。

```text
/solutions/
`- contest cards

/solutions/{contest}/
`- problem list

/solutions/{contest}/{problem}/
`- solution list
```

- solutions root の contest card は OJ、contest 名、公開 problem 数、公開 solution 数、
  最新 solved_at を持つ。
- contest は最新 solved_at 降順、contest ID 昇順で並べる。
- contest page header は contest 名、OJ、公式 contest link を持つ。
- problem item は problem 名と code、公開 solution 数、最新 solved_at を持つ。
- problem は既存 metadata の順序を優先し、なければ problem code の UTF-8 byte 順にする。
- problem page header は contest、problem、OJ への link を持つ。
- solution item は solution 名、言語、solved_at、verify 状態、直接依存 library 数を持つ。
- solution は solved_at 降順、solution ID 昇順で並べる。
- すべての item と集計は公開 solution だけから作る。
- 各一覧は見出し付き `section` の `ul > li > article` とする。
- 公開 solution がない階層は page 自体を生成しない。
- solutions、contest、problem page を Pagefind index から除外する。

### 12.7 解法ページ

solution detail は library detail と同じ単一 article と source component を使う。

```text
article
|- page header
|  |- h1
|  |- contest / problem / OJ
|  |- language / solved_at
|  `- verification status
|- in-page navigation
|- source
|- libraries
|  |- verifies
|  `- depends on
|- verification result
`- diagnostics
```

- contest、problem、OJ は内部 browse page と公式 page の適切な link を持つ。
- source section は library detail と同じ `L*`、line permalink、Shiki HTML contract を使う。
- source toolbar に `Repository source` と明記し、build source commit 上の file へ link する。
- preprocess があれば、OJ 上の提出 source そのものではないことを source toolbar に表示する。
- verifies と direct depends on は別の小見出しと list で表示する。
- 非公開 dependency は library detail と同じ非公開情報を漏らさない共通表示へ変換する。
- `not_configured` では verifies と verification result を省略し、header に中立状態を表示する。
- `never` では verification result を維持し、まだ提出結果がないという empty state を表示する。
- result summary は verdict、judged_at、実行時間、memory、OJ link を `dl` で表示する。
- testcase detail が存在する場合だけ caption と column heading を持つ `table` を表示する。
- stale reason は verification result 内に表示する。
- solution dependency analysis が partial / failed なら diagnostics を残し、
  location を solution source line へ link する。
- diagnostic が entry source 以外を指す場合は path / line / column を表示せず、
  `Location is in a non-displayed solution file` という共通表示にする。
- detail article を Pagefind body とし、solution 名、problem 名、language、status を metadata にする。
- in-page navigation と verification UI は全文検索本文から除外する。

MVP で表示する source は解法の `ce.toml` が指定する entry file とする。
preprocess 後の実提出 source 本体は result JSON や Git 履歴へ保存せず、Web にも掲載しない。

- preprocess がある場合は、表示中なのが repository source であることを明記する。
- verify 結果には submitted-source hash、fingerprint、OJ submission URL を表示する。
- stale の場合は現在の source hash と提出時 hash の差を表示する。
- entry source 本文は検索対象にできる。
- 将来の提出 source artifact は既定で検索対象外とする。
- 「提出 source そのもの」と誤認させる表現は使わない。

current な結果では、entry source と依存 closure から同じ提出物を再生成できることを
fingerprint で保証する。将来、実提出 source の閲覧が必要になった場合は、result JSON とは
別の content-addressed artifact として追加する。

### 12.8 status component

verification と analysis は別の状態軸として表示し、1 つの badge へ合成しない。

```html
<span class="status-badge" data-status="verified">
  <svg aria-hidden="true">...</svg>
  <span>Verified</span>
</span>
```

公開 verification status の label:

- `verified`: `Verified`
- `rejected`: `Rejected`
- `unavailable`: `Unavailable`
- `stale`: `Stale`
- `never`: `Never verified`
- `not_configured`: `Verification not configured`。solution だけに使う。

dependency / symbol analysis status の label:

- `complete`: `Analysis complete`
- `partial`: `Analysis partial`
- `failed`: `Analysis failed`

- badge は色だけに依存せず、decorative icon と完全な text label を常に含める。
- `data-status` は CSS、design fixture、test が共有する固定 contract とする。
- 静的な badge へ `role="alert"` や live region を付けない。
- list card では badge だけを表示する。
- detail では stale、rejected、unavailable、partial、failed に必要な status callout を併記する。
- callout は状態の対象、公開可能な理由、影響、次に確認する内部または OJ link を
  文章で示す。
- dependency analysis と symbol analysis は同じ外観を使えても、対象名を text で明示する。
- timestamp は RFC 3339 の `datetime` を持つ `<time>` として描画する。
- callout には非公開情報を除去した公開 DTO の message だけを渡す。

### 12.9 detail list components

testcase detail 以外の構造化一覧は table にせず、共通 list component とする。

```text
symbols
`- section > ul > li
   |- kind
   |- name / qualified name / signature
   `- source location link

dependencies / used by / relations
`- section > ul > li
   |- target title
   |- language / path
   `- relation kind / manual marker

verification evidence
`- section > ul > li > article
   |- solution link / status
   `- judged time / OJ link

diagnostics
`- section > ul > li
   |- severity / adapter code
   `- message / source location link
```

- component 自身は heading level を固定せず、caller が page 階層に合う `h2` / `h3` を渡す。
- optional value がない場合は空 label や反復する dash を出さず、その field 自体を省略する。
- 0 件では空の `ul` を生成せず、complete / unavailable / failed を区別した message を表示する。
- symbol name、qualified name、signature は code text として検索対象にする。
- dependency UI、verification UI、diagnostics は全文検索本文から除外する。
- location は共通 source-location component を使い、
  同じ page の `L*` または対象 detail へ link する。
- kind と severity は色だけでなく text label を必須にする。
- 長い signature、path、diagnostic message は container を越えず折り返せるようにする。
- visual card への変更は許可するが、`ul > li` と field の意味順は維持する。
- testcase detail だけは列比較を優先し、caption と heading を持つ table とする。

### 12.10 document head と crawler metadata

production の全 HTML に page title、description、canonical URL、基本 Open Graph metadata を
生成する。

- Home の document title は site title とする。
- その他の page は `{page title} | {site title}` とする。
- description Markdown がある detail は、rendered plain text の空白を畳み、
  先頭 160 Unicode scalar value までを description 候補にする。
- page 固有 description が空なら site description へ fallback する。
- canonical URL は build environment の site origin、base、既定 route helper から生成する。
- Open Graph は title、description、canonical URL、site name、`website` type を持つ。
- OG image、RSS、JSON-LD は MVP に含めない。
- `/search/` と `404.html` は `noindex`、Home、browse、detail page は index 可能にする。
- `sitemap.xml` は 404、search、非公開 page を除く canonical public page だけを含める。
- `robots.txt` は sitemap の canonical URL を示す。
- repository source link は `repository_url`、build source commit SHA、repository path から生成する。

### 12.11 公開 source の size 境界

source 本文、検索、line permalink の意味を一致させるため、公開 source を途中で truncate、
分割表示、virtualize しない。公開する file は全行を同じ HTML に生成する。

- highlight 前の raw source が 256 KiB を超えた場合は build warning とする。
- 2 MiB を超えた場合は production build error とする。
- byte 数は Git に保存された raw bytes から計算する。
- `publish = false` の source は Web size 境界に数えないが、analysis 対象には残す。
- 超過時は source file の分割、exclude、`publish = false` の選択肢を error に示す。
- frontmatter から上限を個別解除する機能は MVP に含めない。
- CI summary に observed toolchain identity、最大 raw source、最大 generated HTML、
  site artifact、Pagefind bundle size を記録する。
- site 全体の hard limit は実データの計測後に決め、MVP の初期値を推測で固定しない。
- size warning / error があっても、公開した source の検索と `L*` contract は変更しない。

### 12.12 browser security boundary

GitHub Pages では custom response header に依存せず、全 HTML の resource より前に
meta Content Security Policy を生成する。

```text
default-src 'none';
script-src 'self' 'wasm-unsafe-eval';
style-src 'self' 'unsafe-inline';
img-src 'self' data:;
font-src 'self';
connect-src 'self';
worker-src 'self' blob:;
form-action 'self';
base-uri 'none';
object-src 'none';
frame-src 'none';
```

- JavaScript と CSS file は site artifact から self-host する。
- inline script、inline event handler、JavaScript `eval`、`javascript:` URL を禁止する。
- `wasm-unsafe-eval` は Pagefind WebAssembly、`worker-src blob:` は Pagefind worker のためだけに使う。
- `style-src 'unsafe-inline'` は build 済み Shiki token style のために許可し、script へは許可しない。
- CDN、external font、analytics、iframe は MVP に含めない。
- Pagefind の highlighted excerpt は属性なしの `mark` element だけを許可して sanitize する。
- Pagefind の title、metadata、query、filter value は HTML として解釈せず text node にする。
- search result URL は configured base 配下の生成済み public detail URL と fragment だけを許可する。
- external link は同じ tab を既定とする。`target="_blank"` を使う場合は
  `rel="noopener noreferrer"` を必須とする。
- meta CSP で指定できない `frame-ancestors` は、header を設定できる hosting へ移行した場合に
  `frame-ancestors 'none'` として追加する。
- CSP を HTTP header へ移した場合も、許可範囲を広げず同じ policy を正とする。

実装時の参照:

- https://pagefind.app/docs/hosting/
- https://pagefind.app/docs/api/
- https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/CSP

### 12.13 Markdown link

Markdown の relative link は生成後の page URL ではなく、Markdown file の repository 上の
directory を基準に解決する。

- 公開 managed source またはその sidecar は library detail URL へ変換する。
- `_index.md` または生成対象の directory は directory page URL へ変換する。
- managed target 以外の repository file は build source commit 上の repository URL へ変換する。
- private target、存在しない target、repository 外へ出る target は build error とする。
- fragment は target page の生成済み `doc-*` または `L*` と一致することを build 時に検証する。
- project base を壊す `/` 始まりの root-relative link は禁止する。
- external link は `https`、`http`、`mailto` scheme だけを許可する。
- `http` link は許可するが build warning を出す。
- その他の scheme、protocol-relative URL、制御文字を含む URL は build error とする。
- Markdown image syntax は未対応の config error とし、image asset pipeline を設けない。
- raw HTML が無効なため、`img` element を説明へ直接書くこともできない。
- 画像対応は予定せず、必要性が生じた場合だけ
  別の公開・形式・size policy として設計し直す。

### 12.14 site build command boundary

Rust CLI は正規化済み site-data 生成までを担当し、Node / Astro / Pagefind process を
直接起動しない。

```text
ce check
ce site-data generate --mode production
Astro build / exact-search-index generation
Pagefind indexing
schema / link / artifact validation
```

`ce site-data generate`:

- discovery、analysis、公開 DTO 変換、schema 検証済み JSON 出力を担当する。
- 既定 output は repository の `target/ce-site-data/` とする。
- staging directory へ生成し、全 target 成功後だけ output を atomic に置き換える。
- `--mode preview` は Git history、site metadata、source size の production requirement を
  warning にできる。
- `--mode production` は既定の full history、metadata、size、privacy contract を厳格に検証する。
- Node executable、package manager、Astro、Pagefind を起動しない。

Web workspace の `package.json` script:

- `npm run site:build`: adapter build、`ce check`、production site-data、Astro、Pagefind、
  最終検証を順に実行する。
- `npm run site:preview`: production 相当の artifact を作り、local static server で配信する。
- `npm run site:dev`: preview site-data と Astro dev server を使う。
- dev で current Pagefind index がない場合は古い index を使わず、検索 unavailable を明示する。

local と CI の正式な full build entrypoint は `npm run site:build` とする。
CI は lockfile に基づく `npm ci` 後に同じ script を実行する。これにより Web toolchain の
変更を Rust CLI interface から分離する。

### 12.15 Web toolchain reproducibility

MVP の Node major line は Node 24 LTS とする。`LTS` のような可変 alias は build 入力にせず、
repository root の `.node-version` に patch version まで記録する。最初の実装時点で利用する
Node 24 の patch version を選び、local と CI は同じファイルを参照する。

- `package.json` の `engines.node` は `24.x` とし、選択した Node に同梱される npm major も
  `engines.npm` で固定する。
- npm を唯一の package manager とし、`package-lock.json` を commit する。
- local の再現 build と CI install は `npm ci` を使う。
- Astro、Pagefind、Shiki などの direct dependency は `^` や `~` を使わない完全一致 version とする。
- transitive dependency は `package-lock.json` で固定する。
- build script は repository の `node_modules/.bin` に解決される command だけを呼ぶ。
- package を都度取得する `npx --yes`、未導入 package を暗黙に取得する `npx`、
  CDN 読み込みを禁止する。

Node patch、npm major、Web dependency、lockfile の更新は、それぞれ独立に review 可能な PR とする。
通常の Node patch 更新は保守変更として扱うが、Node major 更新は Astro、Pagefind、Shiki、
CI runtime、CSP の互換性を明示的に確認する。dependency update PR は自動 merge せず、
production site build に加えて root base と project base、検索、CSP、内部 link の
fixture test をすべて通す。

## 13. 検索

検索結果はライブラリファイル単位でまとめる。
該当シンボルや一致行はカード内に表示する。
Pagefind の JavaScript API を使い、ユーザー query を独自 parser で全文検索語と filter に
分ける。ユーザー向けの `lang:` は内部の `code_lang` filter へ変換し、HTML の言語属性と
区別する。
シンボル結果からソースの行アンカーへ移動できるようにする。

```text
monoid
monoid lang:cpp
monoid lang:rust kind:trait
path:algebra verified:true
path:"data structures/fenwick tree.cpp"
status:stale type:library
```

MVP の query は空白区切り token、`"..."` の phrase、quoted filter value を扱う。

- filter key は `lang`、`kind`、`path`、`verified`、`status`、`type` とする。
- filter は `key:bare-value` または `key:"quoted value"` とする。
- bare value は次の空白までとし、最初の key separator より後の `:` は
  value の通常文字とする。
- quote は token 先頭の phrase または `key:` 直後の quoted value にだけ使える。
- quoted text 内の escape は `\"` と `\\` だけを許可する。
- 同じ key の複数指定は OR、異なる key 間は AND とする。
- filter 以外の語と phrase は Pagefind の全文検索へ渡す。
- filter だけの query も許可する。
- `verified:true` は `status:verified` と同義とする。
- `verified:false` は `not_configured` を含む verified 以外の全状態とする。
- status は `verified`、`rejected`、`unavailable`、`stale`、`never`、
  `not-configured` を受け付ける。
- `not-configured` は内部の `not_configured` に対応する。
- type は `library` または `solution` を受け付ける。
- 未知 key の `foo:bar` は error にせず、quote でまとめた部分も含めて全文検索へ渡す。
- 既知 key の空 value、無効な boolean / enum、閉じていない quote、不正な escape、
  quote 後に同じ token の文字が続く入力は query error とする。
- negation、parenthesis、明示的な `OR` operator は MVP では扱わない。
- filter key と照合用 value は lowercase へ正規化する。
- URL の `q` は `URLSearchParams` 相当の処理で一度だけ decode し、query parser は
  percent decoding を行わない。
- 元 query は `/search/?q=...` に保持し、reload、履歴移動、URL 共有に対応する。

`path:` 用 filter には、言語 root 相対パスを lowercase 化した各 segment と累積 prefix を
登録する。例えば `algebra/monoid.rs` は `algebra`、`monoid.rs`、
`algebra/monoid.rs` の 3 value を持つ。実際の表示 path の大文字・小文字は変更しない。

solution の `path:` には entry file path でなく、安定した solution ID を使う。

- `abc999/a/main` は `abc999`、`a`、`main`、`abc999/a`、`abc999/a/main` を持つ。
- `path:` 単独は library と solution の両方へ適用する。
- `type:solution path:abc999/a` のように type filter と組み合わせられる。
- `src/main.rs` など solution 内部の entry path は登録しない。
- template や entry file の変更だけでは solution の path filter value を変えない。
- 表示用 ID / path は元の case を維持し、filter 照合値だけ lowercase にする。
- private library / solution の path value は検索 index へ出力しない。

順位の基本方針:

1. シンボル名・ファイル名の完全一致
2. シンボル名の部分一致
3. タイトル・説明 Markdown
4. パス・カテゴリ
5. ソース本文

MVP の全文検索 index はメタデータ、説明、ソース本文、全 code language を
まとめて 1 つにする。
Pagefind 自身の検索語別 chunk と遅延取得を利用し、言語別・source 別 bundle は作らない。
`lang:cpp` は共通 index の `code_lang` filter として適用する。

検索対象はライブラリ詳細と公開解法詳細を基本とする。
トップ、言語、ディレクトリ、検索結果の各ページは重複を避けるため index しない。
title、file name、symbol name を高 weight、
説明 Markdown を中 weight、ソース本文を低 weight とし、navigation や検索ノイズになる表示は
`data-pagefind-ignore` で除外する。

異なる公開 page が同じ title、basename、symbol name、search alias を持つことは正常系とし、
重複名を build error にしない。page の識別は常に canonical page ID で行う。

検索 UI は Pagefind の page-level result を 1 ファイル 1 card として表示する。
result data の `anchors` と `locations` を使い、一致した symbol や `L*` source line を
card 内の sub-result として組み立てる。

CI では Pagefind bundle、exact match index のサイズと代表 query の転送量を記録する。
実データで問題が確認された場合だけ、言語別 index を生成し Pagefind の index merge で
横断検索する構成を検討する。分割 threshold は実測なしに固定しない。

### 13.1 exact match index

Pagefind の relevance score は全文検索の順位に使い、ファイル名とシンボル名の
完全一致だけは補助的な静的 `exact-search-index.json` で保証する。Astro build は
Rust が出力した公開 DTO だけからこの index を生成し、Git には commit しない。

各 record は次だけを持つ。

- schema version
- canonical page ID、base 対応の detail URL、type、title、language、status、表示 path
- 公開 page の title、拡張子込み source basename、最後の拡張子だけを除いた basename
- 公開シンボルの name、qualified name、search names、kind、生成済み fragment
- `lang`、`kind`、`path`、`verified`、`status`、`type` と同じ filter value

説明 Markdown、source 本文、diagnostic、依存、非公開 page / symbol / path は含めない。
生成後に公開 DTO の page ID 集合と一致することを検証し、検索 page 以外からは
読み込まない。

full-text 部分が bare token 1 個または phrase 1 個の query だけ exact lookup を行う。
page の 3 種の名前と symbol の全検索名を lookup alias とする。filter を除いた query text と
alias の両方へ locale 非依存の Unicode lowercase だけを適用して比較し、Unicode normalization、
separator 分割、記号置換は行わない。filter 条件を満たす record だけを採用し、記号だけの
名前も tokenization せず exact key として扱う。

例えば `monoid.rs` は `monoid.rs` と `monoid` の両方を持つ。`foo.test.cpp` から自動生成する
拡張子なし alias は `foo.test` だけであり、`foo` まで推測しない。Rust の `::`、Lean の
namespace、Lean の notation / operator などから別名が必要な場合は adapter が
`search_names` を返す。

検索結果の merge 順は次とする。

1. file / symbol exact match を canonical page ID 単位にまとめる
2. exact match 間は canonical page ID の UTF-8 byte 順にする
3. Pagefind の relevance 順 result を続ける
4. canonical page ID が既出の Pagefind result は除く
5. union 完成後に 20 card 単位で paginate する

exact index だけで見つかった記号シンボルも通常の file card として表示し、一致シンボルの
fragment を sub-result にする。完全一致以外の部分一致順位は Pagefind の weight に任せ、
client 側で全 result を独自 score に並べ直さない。

同じ exact alias が複数 page に存在する場合は該当する全 page を exact match 群へ入れる。
20 page を超える exact match も省略せず、exact match 群だけで複数 page に paginate できる。
同じ page で title、file、symbol が複数一致しても card は 1 件だけとし、一致理由と
sub-result をその card に集約する。

### 13.2 検索ページの semantic structure

header の global search form を検索 page でも唯一の form とし、main 内に重複 form を作らない。
input は URL の `q` を復元する。

```text
page header
`- h1 Search

parsed filters
search status / error
result summary
ordered result list
`- li
   `- article
      |- h2 / detail link
      |- type / language / status / path
      |- match reasons
      |- excerpt
      `- matched symbols / source lines
pagination
```

- 正本 URL は `/search/?q=...&page=...` とし、reload、履歴移動、共有で同じ状態を復元する。
- 空 query では全件を表示せず、query grammar と代表例を表示する。
- filter-only query は通常の検索として実行する。
- loading は `role="status"`、query error は `role="alert"`、件数更新は
  `aria-live="polite"` とする。
- query error、0 result、Pagefind load failure を別々の状態と message にする。
- result は rank 順の `ol` とし、1 page 20 card とする。
- 1 card は 1 library file または 1 solution file を表す。
- card は title に加え、type、language、verification status、完全な公開 path を常に表示する。
- 一致理由は `Title match`、`File match`、`Symbol match` の重複なし text label とする。
- card 内は heading、metadata、match reasons、excerpt、sub-result の DOM 順を維持する。
- sub-result は exact symbol match、その他の symbol / source line match の順とする。
- exact symbol 同士は location ありを先に start line 順、同じ位置または location なしでは
  kind、name の UTF-8 byte 順にする。
- card 内の symbol / source line sub-result は最大 5 件とし、detail の該当 anchor へ link する。
- 5 件を超える場合は `ほか N 件` と detail page への link を表示する。
- pagination は Previous、Next、現在 page を表示し、すべての link で `q` を維持する。
- 不正、0、範囲外の `page` は query error にせず 1 へ canonicalize する。
- `<noscript>` は検索に JavaScript が必要な旨と Libraries / Solutions の browse link を持つ。
- 検索 page 自身、parsed filter UI、status message、pagination を Pagefind index から除外する。

## 14. 公開ビルドの失敗方針

次は致命的エラーとして公開を止める。

- アダプターコマンド失敗
- 共通 JSON schema 違反
- リポジトリ外または対象外への不正パス
- 存在しないライブラリを指す `verifies`
- 重複 ID
- 不正な手動 override

未解決の外部 import や依存循環は diagnostics として保存し、必ずしも公開を止めない。
依存グラフ処理は循環があっても停止しないことを前提とする。

生成時には次を守る。

- Markdown を sanitize し、ソースコードを HTML escape する。
- Markdown の raw HTML を無効にし、生成 HTML は allowlist 方式で sanitize する。
- ソース行 anchor は `L*`、説明見出し anchor は `doc-*` の namespace に分ける。
- セッション、cookie、Authorization header を結果型へ入れない。
- 配列、依存関係、ページ一覧を安定順で sort する。
- 一時ディレクトリへ生成し、成功後に公開成果物を置き換える。
- 正規化 JSON に `schema_version` を持たせる。

安定 sort は locale、OS、filesystem の列挙順に依存させない。

- language は language ID の UTF-8 byte 昇順。
- directory page は子 directory、library file の順に分け、各 group を root 相対 path 順。
- dependency と reverse dependency は library ID 順。
- relation は kind、target library ID の順。
- location 付き symbol は開始位置、kind、qualified name、name の順。
- location なし symbol は location 付きの後で kind、qualified name、name の順。
- diagnostic は error、warning、info の順とし、同 severity は location、code、message の順。
- library の verify solution は solution ID 順。
- recent library は updated_at 降順、library ID 昇順。
- recent solution は solved_at 降順、solution ID 昇順。
- fingerprint closure は library ID 順。
- JSON の object key と array も対応する規則で正規化する。

path と ID は Unicode normalization、locale-aware sort、case folding を行わず、Git に保存された
UTF-8 byte 列を基準にする。将来 display-only の並び替えを追加しても、ID、fingerprint、
正規化 JSON の順序には影響させない。

## 15. ブランチと CI

`main` を唯一の正本とし、用途は短命 branch 名で表す。

```text
main
|- docs/<topic>
|- feat/<task-number>-<topic>
|- library/<topic>
`- solution/<contest>
```

application code は既存 repository の task / branch convention に合わせて
`feat/<task-number>-<topic>` とする。architecture spec や implementation plan だけの変更は
`docs/<topic>`、library content は `library/<topic>`、contest 中の解法は
`solution/<contest>` を使う。

長期の `dev`、`develop`、`library`、`solution` branch は基本的に持たない。コンテスト中の
`solution/<contest>` は作業中しばらく保持してよいが、完了後に main へ PR を出す。

大型機能でも一時的な integration branch は作らず、独立して build / test できる小さい PR に
分割する。各 implementation branch は直前 PR の merge 後に最新 main から作り、未完成機能は
設定や workflow へ接続するまで休眠状態にして main を常に build 可能に保つ。

この機能では最初に設計資料だけの PR を main へ出し、AI review と人間の確認を merge 前に
完了する。設計 PR の review 完了後に Superpowers 形式の sub-project implementation plan を
作る。各実装 PR は task 内の TDD commit を `/commit`、main 向け PR を `/pr`、review 対応を
`/pr-review` の順で行い、review 完了後も merge は人間の判断まで待つ。

推奨 CI フロー:

1. 通常 PR では資格情報を使わず、check、解析、schema 検証、静的サイト build を行う。
2. main へのマージ後、レビュー済みコードへだけ OJ 資格情報を渡して verify する。
3. CI は `verification/results/` を更新する bot PR を main へ作る。
4. 結果 PR の検証・マージ後、main から静的サイトを公開する。

site-data を生成する checkout は library の `updated_at` を導出できる full Git history を持つ。

Web ホスティングに生成物専用ブランチが必要でも、そのブランチをソースや verify 結果の
正本にはしない。

MVP の GitHub Pages 公開は Actions の Pages artifact を使い、`gh-pages` などの生成物 branch は
作らない。公開元は常に main の repository source と最新 verify result とする。

### 15.1 verify bot PR の lifecycle

CI 上の提出途中 state は、最大 1 本の `automation/verify` branch と draft PR へ保存する。
PR merge 後に branch を削除し、次回は同名 branch を main から作り直してよい。

1. worker は main に対して check、解析、fingerprint 計算を行う。
2. 新規提出対象があれば結果 JSON を `Starting` にし、draft PR へ commit / push する。
3. remote branch への保存成功後にだけ OJ へ POST する。
4. `Trackable(handle)` を得た直後に結果 JSON を置き換え、commit / push する。
5. poll 中の重要な状態遷移も同じ branch へ保存する。
6. terminal 結果を保存したら PR を ready にする。
7. timeout や一時的障害では draft のまま残し、次回 worker が resume する。

変更分類 job は concurrency group の外で実行する。
非 result 変更、定期実行、手動実行から起動する heavy verify job だけが
同じ concurrency group を使う。
同時に実行する verify worker は 1 つとし、実行中 worker は新しい push で cancel しない。
pending heavy job は最新の 1 件だけ残し、古い未開始 job は置き換えてよい。
result-only push は heavy job を作らないため、待機中の実質的な verify を置き換えない。

worker は新規提出前に既存 draft PR を探し、保存済み pending handle の追跡を優先する。
定期実行は timeout 後の resume を目的に含める。schedule が遅延・停止する可能性を考慮し、
同じ resume を `workflow_dispatch` から手動実行できるようにする。

draft PR に `InfrastructureFailure` がある場合も先に確認する。retryable なら
`next_retry_at` 以後だけ再開し、operator action required なら自動 OJ job を起動しない。
start 前の failure では現在の main から plan を再構築して整合性を確認し、新しい attempt ID で
開始する。poll failure では保存済み handle だけを OJ job へ渡す。

bot PR の base である main が進んでも、terminal 結果は最新の観測として保存する。
現在の fingerprint と異なる場合は結果を捨てず、merge 後の公開状態を `stale` とする。

古い attempt が新しい結果を上書きしないよう、結果更新は compare-and-swap とする。
PR の `replaces_attempt_id` と現在の main にある result の `attempt_id` が一致する場合だけ
置き換える。plan 作成時に結果がなかった場合は、main に結果がない場合だけ置ける。
一致しなければ自動マージせず、既存結果との競合として再調整する。

提出済み handle は、現在の main で stale になっても terminal まで追跡する。
同じ内容の二重提出は行わない。

workflow が作った commit や PR でも通常の PR 検証を確実に起動するため、bot は
`GITHUB_TOKEN` ではなく最小権限の GitHub App installation token を使う。

### 15.2 terminal 結果の自動マージ

bot PR は `accepted` だけでなく、OJ が返した rejected verdict と `unavailable` も
terminal な最新観測として自動マージする。rejected には WA、TLE、CE、RE、`judge_error`、
`cancelled`、`other` など accepted 以外の終端 verdict を含む。

- `accepted`: 自動マージする。
- rejected verdict: 自動マージする。
- `unavailable`: 自動マージする。
- `Starting`、queued、judging: draft のまま追跡する。
- `AcceptanceUnknown`: draft のまま回復または手動判断を待つ。
- schema 違反、不正パス、アダプター完全性エラー: マージしない。

`ce verify` 自体は rejected と unavailable で exit 1 を返す。CI controller はこの終了状態を
観測結果として受け取り、結果保存と PR 更新を続ける。bot PR の必須 check は verdict の成否
ではなく、結果 schema、fingerprint、状態遷移、変更範囲の完全性を検証する。
したがって rejected / unavailable の結果 PR でも完全性 check は成功できる。
verify 不成功は Actions summary と PR summary へ明示し、Web にも最新状態として公開する。

### 15.3 result-only push の分類

verify workflow は main のすべての push で起動するが、最初の資格情報を使わない job で
変更パスを分類する。GitHub event の `paths-ignore` だけには依存しない。

- `verification/results/**` だけの push では、check、解析、OJ verify job を起動しない。
- それ以外の変更を 1 件以上含む push では、通常の verify pipeline を実行する。
- `schedule` と `workflow_dispatch` は差分に関係なく resume と current state 確認を行う。
- site publish workflow は result-only push でも実行する。
- bot PR の完全性 check は、変更先が `verification/results/**` だけであることを検証する。

push event の `before` と `after` の間にある全変更パスを取得する。
GitHub の path filter にあるファイル数制限の影響を受けない形で分類する。
分類 job は OJ 資格情報と bot token を受け取らない。

### 15.4 資格情報の分離

資格情報は用途別の GitHub Environment へ分け、同じ job に OJ secret と repository write
credential を同時に渡さない。

`oj-library-checker` environment:

- OJ の認証情報だけを保存する。
- job の `GITHUB_TOKEN` は `contents: read` とする。
- repository への書き込み権限を与えない。
- secret は job 全体ではなく submit / poll の必要な step にだけ渡す。

`verify-state` environment:

- GitHub App ID と private key だけを保存する。
- App はこの repository だけへ install する。
- repository permission は `Contents: read and write`、`Pull requests: read and write`、
  `Metadata: read` だけに絞る。
- Actions、Checks、Workflows、Administration、Pages、Deployments、organization permission は
  与えない。
- App ID は environment variable、private key は environment secret とする。
- installation token は state writer job 内で都度発行し、対象をこの repository 1 件、
  permission を App の上記 allowlist へ再度縮小する。
- installation token は 1 時間以内の同じ job でだけ使い、job 間で再利用しない。
- OJ の認証情報へアクセスさせない。

両 environment は許可 branch を main だけにする。自動運用を維持するため required reviewer は
設定せず、通常 PR の review と main branch protection を秘密情報利用前の承認境界とする。
`workflow_dispatch` も main 上の workflow に限る。PR や feature branch、
`pull_request_target` から secret-bearing job を呼ばない。

secret-bearing job では third-party action を極力使わず、`actions/create-github-app-token` を含む
必要な action は commit SHA で固定する。App token は必要な API step の environment variable
だけへ渡し、checkout の Git credential、Git config、job output、artifact、cache、ログへ保存しない。
token の prefix、長さ、内部形式を検査しない。raw OJ response、cookie、token を artifact、
result JSON、ログへ保存しない。

state writer は secret なしの job で作った plan と変更候補を token 発行前に検証し、
各 remote write 直前にも base SHA、plan digest、attempt の CAS、変更 path、JSON schema を
再検証する。変更 path は意図した `verification/results/**` の通常 JSON file に限定し、
symlink、workflow、source、config の変更を拒否する。branch commit と PR 操作には token を
永続 Git credential にせず、GitHub API を使う。

CI の state 保存と OJ 操作は次の job 境界に分ける。

```text
prepare immutable plan     # secret なし、任意 command はここだけ
  -> persist Starting      # verify-state
  -> OJ submit plan        # oj-library-checker、任意 command を実行しない
  -> persist handle        # verify-state
  -> OJ poll handle        # oj-library-checker、任意 command を実行しない
  -> persist terminal      # verify-state
```

GitHub App の作成、対象 repository だけへの installation、2 environment の作成、App ID / private key と
OJ credential の登録、branch protection / auto-merge の有効化は人間による初回 setup とする。
workflow 実装がこの段階へ到達した時点で、必要な画面操作、正確な permission、
secret / variable 名、動作確認、key rotation / revoke 手順の checklist を提示し、
完了確認後に secret-bearing test へ進む。
setup 完了前は local fixture と secret なし workflow policy test まで実施できる。

submit job が OJ 受付後に停止し、handle を次 job へ渡せなかった場合も、remote branch 上の
`Starting` から `AcceptanceUnknown` の回復手順へ入る。poll job の開始前には handle が remote
branch へ保存済みでなければならない。

start job は `Starting` の remote branch への保存を確認するまで OJ へ接続しない。
`Starting` が残った場合は、後続 worker が OJ の回復能力に従って handle の一意な復元または
未提出の確証を試みる。どちらも得られなければ `AcceptanceUnknown` とし、同じ OJ の queue を
止めたまま手動修復を待つ。

### 15.5 静的サイトの公開

PR と main で同じ site build pipeline を使い、公開操作だけを main に限定する。

```text
PR
  -> ce check
  -> analysis / schema validation
  -> Astro build / Pagefind indexing
  -> link and fixture validation

main push
  -> same build pipeline with full Git history
  -> upload temporary GitHub Pages artifact
  -> deploy artifact to github-pages environment
```

- PR は site build を検証するが Pages へ deploy しない。
- main の result-only push も最新 result を含む site を再生成して deploy する。
- build job は `contents: read` だけを持ち、Pages の書き込み権限を持たない。
- deploy job だけが `pages: write` と `id-token: write` を持つ。
- deploy job は main から生成された artifact だけを受け取り、repository command を実行しない。
- deploy には main だけを許可する `github-pages` environment を使う。
- Pages artifact、site-data、Pagefind index は一時生成物であり、Git へ commit しない。
- deploy が失敗した場合は既存の公開 site を維持し、生成途中の内容へ切り替えない。
- Pages 関連を含むすべての Actions は既決定どおり commit SHA で固定する。

main の publish workflow 全体を固定の `pages-publish` concurrency group に入れ、
`cancel-in-progress: true` とする。新しい main push は古い build または deploy を中止し、
最新 run だけを残す。PR の site build はこの group を使わず、main の公開を中止させない。

artifact には build 対象の source commit SHA と site schema version を metadata として含める。
deploy 直前に source SHA が現在の main HEAD と一致することを GitHub API から確認し、
一致しない artifact は公開しない。これにより、古い push workflow の再実行も site を
巻き戻せないようにする。

手動公開は過去 run の artifact を再 deploy せず、workflow 開始時点の最新 main を
checkout して新しい artifact を作る。最新 build が失敗した場合は、古い workflow を
復活させず、現在公開中の site を維持する。site の footer または build metadata から
公開元 commit SHA を確認可能にする。

GitHub Pages の custom Actions workflow が提供する artifact upload と deploy の境界を使う。
実装時の参照は次とする。

- https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages

## 16. 既存コードへの入れ方

現在の `crates/domain/src/entity.rs` は大きいため、新しい型を同ファイルへ追加し続けない。
新機能は責務ごとのモジュールとして追加する。

```text
domain/
  library.rs
  verification.rs
  solution.rs
  online_judge.rs
  input_format.rs
```

ただし全面的なリファクタリングを先行タスクにはしない。
新機能が触る既存型だけを必要に応じて移し、無関係な整理は行わない。

既存の `OnlineJudge::submit -> SubmitOutcome` は提出開始までしか表現しないため、提出準備、
提出開始、handle 永続化、判定追跡を再利用可能なユースケースへ分離する必要がある。

### 16.1 MVP の言語対応と実装順

MVP の実対応言語は Rust、C++、Lean の 3 つとする。最初の generic core / public DTO / Web の
vertical slice から 3 言語すべての fixture を使い、特定言語だけで成立する schema や UI を
作らない。

実装順は次とする。

1. generic discovery、adapter protocol、analysis state、public DTO、Web を fixture adapter で通す
2. Rust adapter を接続して end-to-end contract を固める
3. C++ adapter を同じ contract へ接続する
4. Lean adapter を同じ contract へ接続する
5. 3 言語を同時に含む repository fixture で check、解析、site、検索を検証する

各言語は独立した `analyzer.command` を持つ。実装 code や schema client library の共有は許すが、
core に language ID の `match` を追加したり、1 つの analyzer process 内で language ID による
巨大な分岐を作ったりしない。

3 言語それぞれの MVP acceptance は次とする。

- root / include / exclude による source discovery が動く
- 言語固有の `check_command` が動く
- dependency と symbol を共通 adapter protocol へ出力する
- repository が採用した構成の内部 dependency を安全に解決する
- 解決不能な dependency は空集合の `complete` でなく `partial` / `failed` とする
- よく使う declaration の symbol と source location を抽出し、未対応構文は symbol state で表す
- source page、syntax highlight、filter、全文検索、exact symbol search が動く

dependency analysis が `partial` / `failed` の target は設計どおり新規 verify を止める。
symbol analysis の `partial` / `failed` は source page、check、dependency-complete な verify を
止めない。

solution template、submit preprocess、OJ language mapping は library language 対応とは別の
有効化層とする。3 言語すべての library 作成、check、公開、検索を MVP に含める一方、
OJ が対応しない言語の提出を MVP 完了条件にはしない。その solution は verify spec の有無と
OJ capability に応じて `not_configured` または `unavailable` になる。

## 17. テスト方針

- Rust、C++、Lean 各アダプターの fixture / contract test
- 3 言語混在 fixture が共通 discovery / DTO / Web / search pipeline を通る test
- core / Web に言語 ID 固有の分岐を追加せず 3 adapter を差し替えられる architecture test
- adapter toolchain name / version / optional target と重複 name の protocol test
- expected / observed toolchain set の missing、extra、version mismatch test
- production site-data と verify が toolchain mismatch を拒否する test
- preview site-data が mismatch を warning にして observed identity を保持する test
- target OS / CPU 差だけでは expected toolchain comparison を失敗させない test
- toolchain identity だけの変更が fingerprint を stale にしない test
- Rust patch channel、Lean exact release / lockfile、C++ CI artifact pin の policy test
- toolchain update PR 相当で 3 言語 fixture、check、analysis diff を実行する workflow test
- library / solution target の missing、extra、duplicate を拒否する contract test
- 個別解析失敗 target が省略されず `failed` で返る contract test
- dependency / symbol state を独立に検証する contract test
- Rust の直接参照 / import / module 宣言だけを direct dependency にする fixture test
- Rust adapter が Cargo metadata から crate root と module file を解決する fixture test
- Rust の external / inline / path module、use alias、literal include の dependency fixture test
- Rust の glob、re-export、macro path、non-literal include、ambiguous module を partial にする test
- Rust の cfg dependency 候補を安全側の和集合にする fixture test
- Rust の module item と安定した impl name / search names / location の symbol fixture test
- Rust の item macro が symbol だけを partial にし、dependency state と分離される test
- Rust の lib / main / mod source が通常 page 候補となり publish=false で非公開になる test
- C++ の直接 local include、Lean の直接 import だけを direct dependency にする fixture test
- C++ adapter と check が checked-in compile profile を共有する fixture test
- C++ inclusion callback が includer ごとの direct edge を返し、nested include を混ぜない test
- C++ の managed / system / repository-unmanaged / unresolved include 分類 test
- C++ の inactive literal include の安全側 edge と macro include partial test
- C++ が clang -M の推移依存を direct edge として採用しない contract test
- C++ の source-owned declaration、qualified / anonymous name、location の symbol fixture test
- C++ の implicit / system declaration と include guard macro を symbol から除く test
- C++ の AST failure が安全な dependency state と symbol failure に分離される test
- C++ の Clang analyzer と GCC check を別 toolchain identity として検証する test
- Lean の plain / public / meta import と import all を同じ direct edge とする fixture test
- Lean が Lake の root / search path から managed、external、missing、multiple、
  manifest 外の module を分類する test
- Lean の implicit prelude、open、include、section variable、namespace を dependency にしない test
- Lean の header parse が信用できる import を残した partial と、全体を信用できない failed の test
- Lean の command grammar 拡張を含む source を順次 parse / macro expand / elaborate する test
- Lean の declaration、constructor、field、unnamed instance の安定した symbol name / location test
- Lean の target source location を持たない内部 recursor を symbol から除く test
- Lean の custom command が追加した declaration を安全な場合だけ generic kind で返す test
- Lean の body elaboration failure が安全な dependency state と symbol partial に分離される test
- Lean の byte position を Unicode scalar column と exclusive end へ変換する test
- Lean が target symbol を stale `.olean` だけから取得しない test
- Lean adapter の解析結果と `lake build` による check 成否を分離する test
- adapter source が managed library discovery に含まれない test
- Rust の protocol type から生成した checked-in JSON Schema に差分がない test
- 3 adapter が共通 protocol fixture を受理し、共通 invalid fixture を拒否する test
- request / response の schema version が欠落、不正、不一致、未対応の場合の protocol error test
- adapter name / version の変更を protocol version compatibility として解釈しない test
- protocol の breaking fixture を core、3 adapter、schema の同時 version update で移行する test
- adapter build が 3 言語を安定順で処理し、固定した target path に executable を配置する test
- 3 adapter の build / protocol handshake 成功後だけ bin symlink を一括で切り替える test
- 各 adapter が通常 protocol の空 target request を受理し、identity、toolchain、空 result を返す test
- handshake response を通常解析と同じ Rust protocol 型 / schema validator で拒否・受理する test
- adapter 専用 handshake mode / response schema が存在しないことを確認する contract test
- build set manifest の protocol、identity、toolchain、executable hash 検証 test
- build input の相対 path と file byte を安定順で hash する reproducibility test
- adapter source の追加、削除、未 commit 変更が input digest を変える test
- toolchain pin、lockfile、compile profile、protocol、build script の変更が digest を変える test
- build input の missing、repository 外、symlink、重複、directory overlap を拒否する test
- Git commit SHA だけが同じでも input digest mismatch なら adapter 利用を拒否する test
- build-id が正規化 manifest と executable hash から決まり、timestamp に依存しない test
- 新言語の source / additional input 宣言を core の language 分岐なしで hash する test
- prepare が pin / lock / checksum を変更せず、固定済み dependency だけを取得する test
- dependency ID が lock / pin / checksum / target platform から安定して導出される test
- prepare が staging の全検証後だけ content-addressed set を公開する test
- partial / manifest 欠落 prepared directory を cache hit として扱わない test
- Cargo / Lean / Clang prepared artifact の revision / checksum / platform mismatch を拒否する test
- CI cache 復元後に prepared manifest と artifact を再検証する test
- prepared set 内の repository 外 symlink、device file、socket を拒否する test
- prepare が HTTPS 以外、userinfo 付き URL、credential 必須 source を拒否する test
- Git dependency の可変 ref と checksum のない archive / compiler artifact を拒否する test
- lock / manifest で immutable artifact へ解決された registry version を受理する test
- repository 内 local path dependency を digest に含め、repository 外 path / global fallback を拒否する test
- archive の absolute / parent path、symlink、hard link、device file、socket entry を拒否する test
- prepare / build job に OJ、GitHub App、registry、SSH credential がない policy test
- checksum 検証前の download と partial archive を prepared set へ公開しない test
- build が downloaded dependency code を secretless / network-free で実行する policy test
- Cargo / Lake / Clang の生成物を tools source tree 外へ配置する test
- prepared / build output が adapter source input digest を変えない test
- prepare の同時実行が OS lock で fail-fast し、失敗時に既存 set を変更しない test
- 古い prepared set を自動削除しない test
- offline adapter build が network access を試みない test
- dependency cache の missing / mismatch が prepare command を案内して build を失敗させる test
- site:build が prepare を暗黙に呼ばず、準備済み cache から offline build する test
- CI の prepare が secretless で、OJ secret job が prepare / build / analyzer を起動しない policy test
- dependency update PR 相当で 3 言語 fixture、check、analysis diff を実行する test
- build / analyzer child process が親 environment を継承せず固定 allowlist だけを受ける test
- credential / proxy / user-global config が build / analyzer environment に入らない test
- adapter config が任意 environment variable を要求できない schema test
- Cargo locked / offline と Lake prepared-only の missing dependency failure test
- prepare だけが proxy / CA path を受け取り、値を manifest / log / cache key に残さない test
- direct network probe を行う fixture が OS sandbox なしでは遮断されない限界を
  運用文書に示す test
- private source / secret 同居前に OS-level egress sandbox を要求する architecture test
- 同じ worktree の同時 build が OS lock により fail-fast する test
- build process の正常終了、失敗、crash で OS lock が解放される test
- build failure / interruption 後は marker だけが残り、次回 build が手動 cleanup なしで
  再試行する test
- site-data / verify が lock 状態から実行中と前回失敗を区別する test
- adapter build failure / interruption で marker が残り、古い executable を使用しない test
- partial build set や manifest / symlink / executable hash 不整合を拒否する test
- build failure が既存 build set を削除または current として再公開しない test
- site-data と verify が executable 欠落時に build command を案内し、暗黙 build しない test
- fixture adapter を追加しても core に language ID 固有分岐を必要としない architecture test
- adapter が推移 dependency を direct edge として返さない contract test
- solution-owned source 全体の direct library edge を重複排除して集約する test
- core が direct graph から reverse edge と循環を許す推移閉包を導出する test
- dependency override が direct edge だけへ作用する test
- symbol extraction failure が verify と fingerprint を止めない test
- dependency `partial` / `failed` が新規提出を止める test
- `ce check` が全言語を安定順で実行し、省略された check を `skipped` とする test
- `ce check --language` が指定言語だけを実行する test
- `ce check` が通常の公開 solution の `test_command` を一括実行しない test
- 複数言語の check 失敗を最後に集約する test
- check / test の既定 timeout と個別上書きの test
- timeout 時に shell 配下の process group 全体を終了する test
- timeout 後も残りの check を実行し、失敗を集約する test
- check / test の stdout と stderr を逐次転送する test
- verify 用解法に `test_command` がない場合の config error test
- 複数 verify 解法で同じ言語 check を 1 回だけ実行する test
- verify の言語 check と solution test の失敗が新規 plan 作成を止める test
- 現在の check が失敗しても保存済み handle を先に terminal まで追跡する test
- UTF-8 source、1-based line、Unicode scalar column、exclusive end の location test
- language ID、display name、case mismatch、公開 solution の language 参照 test
- library move / rename を新 ID とし、旧 sidecar / relation / verify target / link を拒否する test
- library ID の変更が同一 content でも fingerprint を stale にする test
- renamed library page が旧 route / redirect を生成せず、新 route だけを生成する test
- public から private または削除した library の旧 route が 404 になる test
- library rename 後も同じ solution result を保持し、新 target の stale evidence にする test
- solution ID の move / rename で旧 result 削除を要求し、新 solution を never にする test
- discovery 済み solution に対応しない orphan result を拒否する test
- verify の solution override、project-local OJ language mapping、未設定 error の test
- verify が global config、OJ default、内部 language ID 推測へ fallback しない test
- OJ language mapping の変更が fingerprint を stale にする test
- plan と result が内部 language ID と OJ language ID を区別して保存する test
- OJ adapter が解決済み language ID を非対応と判定した場合の unavailable test
- 通常 submit が既存 global mapping を使い、verify mapping と混ざらない test
- verify 未設定の公開 solution が not_configured となり、ce verify で skip される test
- verify spec の追加で not_configured から never へ移る test
- verify spec 削除時に既存 result も同じ変更で削除することを要求する test
- library の direct verifier 0 件と全 accepted の代表 status test
- rejected / unavailable / stale / never が混在する library status precedence test
- dependency closure に含まれただけの solution を library evidence から除外する test
- verified:true が全 direct verifier accepted の library だけに一致する test
- LSP の UTF-16 location 変換と CRLF source の境界 test
- target 外 path、symlink、範囲外、逆転 range を拒否する contract test
- diagnostic message 内の absolute / managed / solution-relative path を拒否する contract test
- symbol kind token、必須 name、同名 symbol の contract test
- 不正 JSON、schema version、リポジトリ外パスの拒否
- 依存循環、未解決依存、手動 override
- add / remove / resolve / external override の適用順と stale override の error test
- sidecar / `_index.md` の用途別 frontmatter key と default title の test
- orphan sidecar、空 title、未知 key、不正 relation を拒否する test
- `_index.md` だけの空 directory page と sidecar なし source page の生成 test
- 全 unresolved の手動分類で dependency state が complete になる test
- dependency failed を override で回復できない test
- 非公開 library を解析と fingerprint closure に含め、公開 DTO から除外するテスト
- 非公開 library の path が Web の依存・diagnostic・stale 理由へ漏れないテスト
- solution 非 entry file の diagnostic location が公開 DTO で null と共通 reason になる test
- 非 entry diagnostic を内部 snapshot と dependency / fingerprint では維持する test
- private dependency を指す public library diagnostic の target 情報を除去する test
- fingerprint と stale 理由
- 提出、ポーリング、タイムアウト、中断、resume の状態遷移
- 同じ解法を複数ライブラリへ登録した場合の提出重複排除
- OJ ごとの optional result detail
- unavailable と rejected の終了コード
- 恒常的な能力不足だけを unavailable とし、一時的な OJ 障害を infrastructure error とする test
- infrastructure error が main の terminal result を置き換えず、Web 状態も変えない test
- handle 取得前の確定した未受付では Starting を破棄できる test
- handle 取得後の infrastructure error が handle を残して poll 再開対象になる test
- InfrastructureFailure が draft PR だけに残り、secret や raw response を含まない test
- retryable failure が next_retry_at より前に OJ へ接続しない test
- workflow 間 retry が 5、10、20、40、80 分から最大 6 時間へ増える test
- OJ の Retry-After が計算済み next_retry_at より長い場合に優先される test
- 受付不明の start failure が retry されず AcceptanceUnknown になる test
- 未受付を確証した start failure が 5 回続くと operator action required になる test
- poll failure が 6 時間間隔を上限に同じ handle で継続される test
- OJ 接続成功または判定進行で infrastructure failure count が reset される test
- 正常な pending 判定の 15 分 timeout が infrastructure failure count に入らない test
- operator action required が定期 OJ job を起動せず、手動修復理由を表示する test
- start 再試行時は plan hash を検証し、新しい attempt ID を割り当てる test
- poll 再試行時は attempt ID と handle を維持し、start を呼ばない test
- current rejected / unavailable を自動再実行せず、fingerprint 変更後だけ対象に戻す test
- CI 中断後に draft bot PR の Starting / handle から resume する統合テスト
- plan だけの attempt は OJ へ未接続であり、安全に破棄して再計画できるテスト
- `Starting` の永続化失敗時に OJ へ接続しないテスト
- `Starting` から一意な handle を回復し、同じ attempt として poll するテスト
- `best_effort` 回復の候補 0 件または複数件を `AcceptanceUnknown` とするテスト
- 未提出を確実に証明できる OJ だけが `Starting` を破棄して再計画できるテスト
- `AcceptanceUnknown` が同じ OJ の新規提出を止めるテスト
- main 更新や fingerprint 変更が `Starting` の回復を迂回しないテスト
- main 更新後に完了した古い fingerprint の結果を保存し、stale と判定するテスト
- `replaces_attempt_id` が一致しない結果で main の新しい attempt を上書きしないテスト
- accepted、rejected、unavailable の有効な結果 PR が merge 可能になるテスト
- pending、AcceptanceUnknown、不正結果が merge 可能にならないテスト
- result-only main push が OJ job を起動せず、site publish は起動する workflow test
- PR の site build が Pages deploy job を起動しない workflow test
- main の Pages build job が `contents: read` 以外の書き込み権限を持たない policy test
- Pages deploy job だけが `pages: write` と `id-token: write` を持つ policy test
- Pages deploy artifact に symlink や repository 外の file が含まれない test
- Pages 生成物が Git の変更として残らない test
- 新しい main push が古い `pages-publish` run を中止する workflow test
- PR の site build が `pages-publish` concurrency group を使わない workflow test
- deploy 前に artifact の source SHA と現在の main HEAD の一致を検証する test
- 古い push workflow の再実行が Pages deploy を拒否される test
- 手動公開が過去 artifact でなく最新 main から build する test
- result-only push が pending heavy verify job を concurrency queue から追い出さないテスト
- 300 ファイルを超える差分でも result-only 判定を誤らないテスト
- OJ secret job が repository write credential を持たない workflow policy test
- bot job が OJ secret を持たず、GitHub App permission が allowlist 内である policy test
- App installation token が repository / permission を縮小し、job 間で保存されない policy test
- App token が Git credential、output、artifact、cache、log に渡らない workflow policy test
- state writer が remote write 直前に base、plan、CAS、path、schema を再検証する test
- state writer が results 外、symlink、workflow、source、config の変更を拒否する test
- PR / feature branch から secret-bearing job が起動しない workflow test
- plan の source hash と実際の submit body が常に一致するテスト
- 解法ページが entry source と submitted-source hash を区別する表示 test
- solution detail の section 順と library detail との source component 共通化 test
- not_configured、never、stale の solution detail 表示分岐 test
- verifies と depends on の分離、および非公開 dependency の非漏洩 test
- Web の depends on / used by が direct edge だけを表示する test
- preprocess 有無による Repository source 表示と注意書き test
- testcase detail table の caption、column heading、optional 表示 test
- solution diagnostic location が entry source line へ link する test
- solution article の Pagefind metadata と verification UI 除外 test
- verification と dependency / symbol analysis が別々の status badge になる test
- status badge の固定 data-status、text label、aria-hidden icon の test
- static badge が alert / live region でなく、必要な detail だけ callout を持つ test
- status timestamp が RFC 3339 datetime の time element になる test
- status callout へ非公開 dependency や diagnostic の情報が渡らない test
- symbol / dependency / relation / verification / diagnostic list の semantic structure test
- detail list が optional field を省略し、0 件を状態別 empty message にする test
- symbol text だけを検索対象にし、dependency / verification / diagnostic UI を除外する test
- detail list の location が共通 component から正しい `L*` または detail URL を生成する test
- testcase detail だけが caption / heading 付き table を使う test
- preprocess 後 source 本体が result JSON と検索 index に含まれない test
- OJ secret job が analyzer、check、preprocess、任意 command を起動しない policy test
- plan 作成後の working tree 変更が進行中 attempt の submit body を変えないテスト
- snapshot の schema、hash、repository revision、候補 hash 不一致を拒否するテスト
- 同じ pipeline 内の verify plan と site-data が同一 snapshot を参照するテスト
- 検索順位と `lang:` フィルター
- query phrase、同一 filter の OR、異なる filter の AND、filter-only query の test
- status:not-configured と verified:false の公開 solution 検索 test
- unknown key の全文検索 fallback と malformed known filter の error test
- path segment / prefix filter と search URL round-trip の test
- library path と solution ID の共通 path filter test
- solution entry file 変更が path filter value を変えない test
- type:solution と path prefix の組み合わせ、および private path 非出力 test
- 検索結果をファイル単位にまとめ、symbol / source line の sub-result を生成する処理
- search page が header の form だけを使い、q と page を URL から復元する test
- 空 query、filter-only、query error、0 件、Pagefind load failure の UI state test
- phrase と bare / quoted filter value、value 内 colon、quote / backslash escape の parser test
- 空 filter、未閉鎖 quote、不正 escape、quote 後の文字を query error にする parser test
- unknown key を quoted 部分も含めて全文検索へ渡し、percent decode を重ねない test
- exact index が公開 DTO だけから生成され、private path / symbol / diagnostic を含まない test
- file / symbol の大文字小文字を無視した完全一致と、記号だけの symbol exact lookup test
- title、full basename、last-extension-stripped basename の page alias test
- symbol name、qualified name、adapter-provided search name の exact alias test
- core が search name の control character を拒否し、重複を除去して separator を推測しない test
- exact alias が lowercase 以外の Unicode normalization と記号置換を行わない test
- exact match が filter 適用後に先頭へ並び、page ID 順で安定する test
- exact / Pagefind result を page ID で重複排除し、union 後に paginate する test
- 同じ title / basename / symbol を複数 page と同一 page 内で許可する test
- 同じ exact alias の全 page を返し、exact match だけで 20 件超を paginate する test
- search card が type、language、status、完全な公開 path と重複なし match reason を持つ test
- exact symbol sub-result の location / line / kind / name 安定順 test
- sub-result 5 件超で `ほか N 件` の detail link を表示する test
- 複数 full-text token と filter-only query が exact lookup を使わない test
- search result の 20 card pagination、rank 順、5 sub-result 上限の test
- pagination が q を維持し、不正または範囲外 page を 1 へ canonicalize する test
- search status の role / aria-live と noscript browse link の test
- 全 page に skip link、landmark、primary navigation、唯一の h1 がある test
- トップの集計と一覧が公開対象だけから作られ、非公開情報を含まない test
- トップの recent 10 件制限、日時順、ID tie-break の test
- トップの attention 10 件制限と verify filter link の test
- トップの各空 section が heading と empty-state message を維持する test
- トップに global search と重複する search form がなく、Pagefind index 対象外である test
- 言語 / directory page が同じ component contract と breadcrumb 階層を使う test
- 言語 / directory page が直下の子 directory と library だけを別 section に列挙する test
- child directory の公開 descendant 数と verify 集計が非公開 library を含まない test
- `_index.md` がない overview の省略と、空 directory の empty-state 表示 test
- 言語 / directory page の path stable sort と Pagefind index 除外の test
- library detail の section 順、固定 ID、存在 section だけを指す in-page navigation test
- sidecar body なし、symbol 0 件、symbol partial / failed の表示状態 test
- library detail が source section と全 `L*` anchor を常に生成する test
- source HTML が `pre/code`、line number、line content、Shiki token の包含契約を守る test
- 空行を含む全行の 1-based `L*` anchor と permalink の test
- source toolbar / line number の検索除外と line content の検索対象化 test
- source repository link が build commit SHA と repository path を指す test
- production build の必須 site metadata、BCP 47、HTTPS repository URL validation test
- local preview の site metadata warning と repository / canonical link 省略 test
- page title、description fallback、160 Unicode scalar truncation の test
- canonical / Open Graph URL が root と project base の helper を使う test
- sitemap / robots が公開 canonical page だけを含み、search / 404 を除く test
- 256 KiB source warning と 2 MiB production error の byte 境界 test
- local preview と production における large source の挙動差 test
- publish false source が Web size 判定外でも analysis 対象に残る test
- source を truncate / virtualize せず全行の検索 text と anchor を生成する test
- CI summary の observed toolchain / 最大 source / HTML / artifact / Pagefind size test
- 全 HTML の CSP meta が resource より前にあり、既定 policy と一致する test
- inline script / handler、eval、javascript URL、third-party resource を拒否する test
- Pagefind excerpt が属性なし mark だけを許し、metadata / query を text 描画する test
- search result URL が base 配下の生成済み detail / fragment だけを指す test
- Pagefind search が wasm / worker CSP 下で動作する browser integration test
- external target blank link が noopener noreferrer を必須とする test
- Markdown relative link の source / sidecar / directory / repository file rewrite test
- private、missing、repository 外、root-relative Markdown link を拒否する test
- Markdown fragment が生成済み doc / line anchor と一致することを検証する test
- Markdown external scheme allowlist と HTTP warning test
- Markdown image syntax と raw img element を拒否する test
- site-data generate が Node / Astro / Pagefind を起動せず DTO だけを atomic 生成する test
- preview / production mode の requirement 差と既定 output path test
- site:build の adapter build、check、site-data、Astro、Pagefind、最終検証の順序 test
- site:dev が古い Pagefind index を使わず unavailable を表示する test
- CI が npm ci と site:build の共通 entrypoint を使う workflow test
- local と CI が同じ `.node-version` の exact Node 24 patch を使う test
- package.json の Node / npm major、direct dependency の exact version、lockfile commit を検証する test
- site build が未導入 package の取得、CDN、lockfile を変更する install を行わない test
- Web toolchain 更新時に root / project base、検索、CSP、内部 link fixture を実行する workflow test
- source text の HTML escape と highlight token allowlist の test
- 非公開依存の名前、path、件数が dependency section に漏れない test
- verification evidence の solution / OJ link、判定日時、stale reason の test
- diagnostic location が source line anchor へ link する test
- library detail の article だけが Pagefind body になる test
- breadcrumb の現在 page が link でなく、navigation の現在位置に aria-current がある test
- global search form が base path を考慮した GET `/search/?q=...` を生成する test
- keyboard focus の CSS が無効化されず、操作要素が keyboard で到達可能な test
- navigation、breadcrumb、footer が Pagefind 本文から除外される test
- Pagefind bundle、exact index size と代表 query の転送量の計測
- Markdown の raw HTML 無効化、sanitize、GFM 表示
- Markdown の h1、先頭 h2、見出し level 飛び越しを検証する test
- Markdown 見出し level を変更せず、`doc-*` と同名 suffix を安定生成する test
- ASCII、Unicode、記号だけ、48 byte 超、同名 heading の anchor fixture test
- locale、Unicode normalization form、platform が heading digest を変えない test
- 見出しなし本文と空本文の documentation wrapper / navigation item test
- syntax highlight の既知・未知言語 fallback と安定した行アンカー
- base `/` と `/compro-env/` での build、内部 link、Pagefind bundle path
- URL path segment の percent-encode と canonical trailing slash
- libraries root の言語 card、公開条件、安定順、empty-state、Pagefind 除外 test
- root / project base の 404 asset、internal link、noindex、Pagefind 除外 test
- 削除・非公開化された detail が redirect されず static 404 になる test
- 公開 solution だけから solution / contest / problem の一覧階層を生成する test
- 公開 solution が 0 件の contest / problem page を生成しない test
- problem page の solved_at 降順と solution ID tie-break の test
- contest / problem metadata 欠落時に ID へ fallback する test
- solution 一覧階層を検索対象外、detail page を検索対象にする test
- solutions root の公開件数、最新 solved_at、contest 並び順 test
- contest page の metadata link、problem metadata 順と code fallback 順の test
- problem page の solution metadata、直接依存数、安定 sort の test
- solution browse の `section > ul > li > article` 構造と Pagefind index 除外 test
- source / sidecar の Git committer date から library updated_at を導出する test
- shallow history の production build 拒否と uncommitted local preview の test
- fixture リポジトリから静的公開データを作る統合テスト
- macOS / Linux 相当の列挙順差、Unicode、case 差で正規化順が変わらない test

## 18. 決定事項サマリー（重要度順）

### 優先度 1: verify 可能な OJ と追跡境界

決定済み:

- MVP の無人 verify は LibraryChecker を対象とする。
- AtCoder は `interactive_untrackable` とし、MVP の verify では `unavailable` とする。
- 将来 `unattended_trackable` な OJ が増えた場合は、
  同じライフサイクルへ追加できるようにする。
- OJ 固有の提出・認証・追跡処理は infrastructure 層の Rust 実装へ置く。
- MVP では OJ 処理を外部コマンド化しない。
- 提出受付が不明な場合は `AcceptanceUnknown` とし、自動再提出しない。
- OJ 側から提出を一意に復元できる場合だけ handle を回復する。
- OJ 側から未提出を確実に証明できる場合だけ attempt を破棄して再計画する。
- `Starting` 以後は main や fingerprint が変化しても、回復手順を迂回して再提出しない。
- OJ adapter は `exact`、`best_effort`、`none` の回復能力を宣言する。
- LibraryChecker は `best_effort` とし、候補 0 件を未提出の証明には使わない。
- `AcceptanceUnknown` がある間は、同じ OJ への新規提出を止める。
- unavailable は確定した能力不足だけに使い、一時的な障害は infrastructure error とする。
- infrastructure error は main / Web の terminal result にせず、追跡情報を保って再開する。
- 運用エラーは sanitize 済み `InfrastructureFailure` として draft PR 内だけへ保存する。
- transient failure は next_retry_at 後に再試行し、設定・認証・schema failure は手動修復を待つ。
- start の再試行は新しい attempt ID、poll の再試行は既存 attempt と handle を使う。
- workflow 間 retry は 5 分から最大 6 時間まで指数的に延ばし、Retry-After を優先する。
- 未受付 start の自動 retry は 5 回までとし、受付不明では一度も retry しない。
- handle 取得後の poll は自動停止せず、同じ OJ の新規提出を止めたまま継続する。
- OJ ごとに同時進行する提出を最大 1 件とする。
- 未完了 handle があれば、新規提出より先に追跡を再開する。
- 判定待ちが timeout した場合は、後続の新規提出へ進まない。
- 共通ライフサイクルが backoff と 15 分の待機上限を管理する。
- OJ の `Retry-After` を優先し、timeout 後は handle を保存して次回 resume する。

### 優先度 2: 言語アダプターの実行契約

決定済み:

- MVP の実対応は Rust、C++、Lean の 3 adapter とする。
- 最初から 3 言語 fixture を共通 core / DTO / Web pipeline へ通す。
- solution template、submit preprocess、OJ 対応は library language 対応と分離する。
- Rust adapter は Cargo metadata と syn を使い、core や rust-analyzer に parser 責務を置かない。
- Rust の曖昧な import / macro / cfg は推測せず dependency または symbol の partial で表す。
- C++ adapter は pinned Clang の preprocessor callback と AST を使い、direct include と
  declaration を独立に抽出する。
- C++ の compile profile は adapter / check で共有し、GCC check と Clang analysis の併用を許す。
- Lean adapter は exact `lean-toolchain` の `lake env` で動かし、header は `parseHeader`、
  body は frontend の順次 elaboration で解析する。
- Lean の import は direct dependency とし、custom command や生成 declaration の曖昧さは
  core の特別扱いでなく symbol state の `partial` で表す。
- adapter source は `tools/library-analyzers/`、完成 executable は ignore 対象の
  `target/library-analyzers/bin/` に置く。
- protocol の正本は core の Rust 型とし、生成した JSON Schema と fixture を commit する。
- request / response は同じ protocol version を必須とし、MVP は version 1 だけを厳格に受理する。
- adapter identity と protocol version を分離し、version negotiation や複数 version 対応は行わない。
- breaking change は core、3 adapter、schema、fixture を同じ変更で次 version へ移行する。
- adapter は明示的に事前 build し、解析時の暗黙 build は行わない。
- 3 adapter は staging で build / handshake し、全成功後だけ atomic な bin symlink で一括公開する。
- handshake は通常 protocol の空 target request と同じ response schema を使う。
- handshake は起動と protocol の smoke test に限定し、実解析能力は言語 fixture で検証する。
- build set manifest は protocol、identity、toolchain、executable hash を持つ。
- `build-inputs.toml` が adapter source、protocol、build script、lock / toolchain input を宣言する。
- input digest は安定した relative path / content hash とし、新規 file と未 commit 変更を含める。
- build-id は manifest と executable hash から導出し、Git SHA と timestamp だけに依存しない。
- site-data と verify は現在の input digest が build manifest と一致しなければ再 build を要求する。
- network を伴う `prepare` と network-free な adapter build を分離する。
- prepare は固定済み dependency / toolchain artifact だけを取得し、lock や checksum を変更しない。
- prepared dependency は target 配下の content-addressed set とし、global cache に暗黙依存しない。
- dependency ID は lock / pin / checksum / target platform から導出する。
- remote source は public HTTPS と immutable revision / checksum に限定する。
- private / credentialed source、可変 ref だけの dependency、repository 外 local path を拒否する。
- local dependency は repository 内に限定し、内容を dependency digest へ含める。
- archive は staging 内へ安全に展開し、検証完了前の artifact を公開しない。
- prepare は staging の全検証後だけ set を公開し、partial / manifest 欠落 set を利用しない。
- Cargo / Lake / Clang の cache と生成物を adapter source tree の外へ置く。
- CI cache 復元後も manifest、revision、checksum、platform を再検証する。
- prepare は worktree 単位の OS lock で同時実行を fail-fast させる。
- 古い prepared set の自動 cleanup は MVP に含めない。
- site:build は prepare を暗黙実行せず、cache 不足時は実行方法を案内して失敗する。
- CI の prepare は secretless とし、OJ secret job は prepare / build / analyzer を実行しない。
- build / analysis は prepared-only、offline option、sanitized environment で network-free とする。
- child environment は固定 allowlist から構築し、credential、proxy、user-global config を渡さない。
- prepare だけは proxy / CA path を許可できるが、値を manifest、artifact、log に保存しない。
- adapter config から任意 environment variable を要求する機能は設けない。
- OS-level egress sandbox は MVP 必須にせず、private source / secret を同居させる前に導入する。
- worktree ごとの OS advisory lock は同時 build を fail-fast させ、process 終了時に自動解放する。
- 成果物無効 marker は失敗後も残し、次回 build 成功時にだけ除去する。
- lock / marker の手動削除や `--force` は設けない。
- build 中断 marker または manifest / symlink / hash 不整合があれば古い adapter を使用しない。
- 古い build set の自動 cleanup は MVP に含めない。
- full site build は 3 adapter の build を最初に行い、欠落時は実行方法を明示して失敗する。
- 新言語 adapter の追加を core の language ID 固有分岐から独立させる。
- 解析コマンドは argv 配列で指定し、shell を介さず直接起動する。
- shell 処理が必要な場合は、ユーザーが用意したスクリプトを argv から呼ぶ。
- MVP では shell 文字列との両対応を行わない。
- 作業ディレクトリはリポジトリルートとする。
- stdin は UTF-8 JSON 1 文書、stdout は UTF-8 JSON 1 文書とする。
- 人間向けログは stderr へ出し、stdout へ混ぜない。
- exit 0 と schema 検証成功の両方を成功条件とする。
- パスは安全なリポジトリ相対パスに限定する。
- アダプターは対象言語につき 1 回起動する。
- workspace の分割は言語固有の処理としてアダプター内部へ任せる。
- target 単位の失敗は JSON、アダプター全体の失敗はプロセス終了状態で表す。
- timeout の既定値は 10 分、stdout 上限は固定 64 MiB とする。
- timeout だけ config で上書き可能にする。
- `adapter.name` と `adapter.version` を必須にする。
- adapter は解析に使用した toolchain name / version と任意 target を返す。
- production / verify は project-local `expected_toolchains` と observed set の完全一致を要求する。
- preview は mismatch を warning にし、OS / CPU target は監査用で一致条件に含めない。
- 依存を `internal`、`external`、`unresolved` へ分類する。
- adapter は target の direct dependency だけを返し、推移 dependency は core が導出する。
- solution result は solution-owned source 全体から library へ出る direct edge の和集合とする。
- `unresolved` があって残りを信用できる場合は `partial` とする。
- 安全な依存集合を取得できない場合は dependency analysis を `failed` とする。
- dependency state と symbol state を分け、verify は dependency state だけを見る。
- symbol state が `partial` / `failed` でも source page と verify を維持する。
- diagnostic severity は `info`、`warning`、`error` とする。
- MVP の Web には外部依存を表示しない。
- 正規化済み `AnalysisSnapshot` は同じ pipeline 内でだけ再利用する。
- core は MVP で cross-run analysis cache を持たず、adapter 内部の build cache だけを許可する。
- 入出力は library と solution の配列を分け、各入力 target に結果をちょうど 1 件要求する。
- missing、extra、duplicate target は protocol error とする。
- 個別解析不能 target は省略せず `failed` と diagnostics を返す。
- internal dependency は同じ言語 manifest 内の library だけを指せる。
- adapter は公開可否を決定または出力しない。
- diagnostic message は path-free とし、path を構造化 location へ分離する。
- 管理 source と提出 source は UTF-8 必須とする。
- source location は 1-based line / Unicode scalar column、exclusive end へ正規化する。
- location path は library 自身または solution root 配下の通常ファイルに限定する。
- symbol kind は形式だけを検証する adapter-defined token とし、コアに固定語彙を持たせない。
- symbol の表示名と qualified name は検索対象とし、言語固有の別名は adapter の
  `search_names` で受け取る。
- core は symbol 検索名の separator を解釈せず、空文字と control character を拒否して
  重複を除く。
- `kind:` は完全一致とし、利用可能な値を検索 index から動的に取得する。

### 優先度 3: config と公開対象の schema

決定済み:

- 言語ごとのライブラリ root は MVP では 1 つとする。
- 複数 workspace は root 配下へ置き、アダプター内部で扱う。
- include に一致したライブラリは既定で公開する。
- root / include / exclude で残った library は公開可否にかかわらず解析する。
- `publish = false` は Web 投影だけを止め、解析と fingerprint からは除外しない。
- 解法は `ce.toml` の `publish = true` による明示 opt-in だけを公開する。
- 公開解法では timezone 付き RFC 3339 の `solved_at` を必須にする。
- `solved_at` に filesystem、Git、OJ データからの暗黙 fallback を使わない。
- verify 用解法では `publish = true` と有効な `solved_at` を必須にする。
- 非公開解法に `[verify]` がある場合は config error とする。
- verify 未設定の公開 solution は not_configured とし、ce verify の対象にしない。
- verify spec 削除時は保存済み result も同じ変更で削除する。
- include と exclude で管理・解析対象を決め、その後 frontmatter で公開可否を決める。
- exclude は常に優先し、MVP では否定 pattern による再包含を扱わない。
- symlink は管理対象にせず、root 不在は error、候補 0 件は warning とする。
- ライブラリ設定はプロジェクトローカル `config.toml` の `[library.languages]` へ置く。
- root、include、analyzer command を必須とし、check command は省略可能とする。
- Rust / Lean は ecosystem の exact version file、C++ CI compiler は checksum / digest で固定する。
- toolchain update は独立 PR で 3 言語 check / analysis fixture を検証する。
- analyzer はリポジトリルート、check は言語 root を作業ディレクトリとする。
- `[library]` 以下の未知キーは config error とする。
- `[library.site]` に title、description、language、repository URL を置く。
- verify 用 OJ language ID は project-local の language / OJ mapping に置き、solution 単位で
  override できる。
- verify は global config や OJ default へ fallback せず、未解決なら config error とする。
- production では site metadata を必須とし、origin / base は build environment から渡す。
- `verifies` が非公開 library を直接指す場合は config error とする。
- language ID は lowercase ASCII slug とし、表示名と syntax highlight 名から分離する。
- 公開 solution の language は `[library.languages]` に存在する必要がある。
- dependency override は typed operation 配列とし、全操作に reason を必須とする。
- dependency override は direct edge だけへ作用し、推移 edge を列挙しない。
- stale、重複、対象不一致 override は config error とする。
- partial の全 unresolved を分類できた場合だけ effective state を complete にできる。
- dependency failed は手動 override で回復できない。
- sidecar は title / publish / relations / dependency overrides だけを許可する。
- library path の move / rename は新 identity とし、旧参照を同じ変更で更新する。
- Git rename から alias / redirect を推測せず、public から private / delete でも旧 URL は 404 とする。
- `_index.md` は title だけを許可する。
- 公開 descendant または `_index.md` があれば directory page を生成する。
- relation は公開 library 間なら cross-language を許可する。
- `ce check` は全言語の `check_command` を安定順で実行し、通常 solution は一括 test しない。
- ローカル用の `--language` filter は許可するが、CI と公開 build は全言語を check する。
- `check_command` がない言語は `skipped` とし、check 結果は保存も Web 公開もしない。
- check / test の timeout は既定 600 秒とし、それぞれ config で上書き可能にする。
- timeout 時は command の process group 全体を終了し、通常の check failure として集約する。

### 優先度 4: verify spec と結果 schema

決定済み:

- 1 解法につき verify spec は最大 1 つとする。
- `[verify].language_id` は project-local language / OJ mapping を上書きする任意値とする。
- verification ID は solution ID と同一にする。
- 複数ライブラリの verify は `[verify].libraries` の配列で表現する。
- solution ID は `{contest_id}/{problem_code}/{solution_name}` とする。
- 結果ファイルは solution ID の階層を `verification/results/` 配下へ写す。
- solution ID の move / rename では旧 result を削除し、新 solution は never から始める。
- library rename では solution result を維持し、保存済み target ID との差で stale にする。
- OJ は contest metadata から取得し、OJ 間の ID 衝突は contest ID 側で namespace 化する。
- 判定は既知の共通 enum と OJ の `raw` 文字列を持つ。
- verify 成功は `accepted` だけとする。
- 時間は切り上げ整数 ms、メモリは byte、取得不能は `null` とする。
- ケース詳細全体の取得不能は `null`、取得可能な 0 件は空配列とする。
- fingerprint は明示 verify 対象と解法依存の和集合から内部依存の推移閉包を取る。
- fingerprint、SubmissionPlan、result は内部 language ID と解決済み OJ language ID を持つ。
- closure 内 dependency state の `partial` / `failed` は新規提出を止める。
- symbol state の `partial` / `failed` は verify を止めない。
- verify 用解法では `test_command` を必須とする。
- 新規提出前に対象言語の check を 1 回ずつ、対象解法の test を 1 回ずつ実行する。
- check / test の失敗は集約して報告し、新規提出へ進まない。
- 保存済み attempt の回復と追跡は check より先に行い、現在の check 失敗では止めない。
- verify solution の `test_timeout_seconds` は省略時 600 秒とする。
- current rejected / unavailable は fingerprint が変わるまで自動再実行しない。
- 変更なしの rejected / unavailable を明示的に再実行する機能は MVP に含めない。
- not_configured は公開 solution だけの中立状態とし、library には使わない。
- library は全 direct verifier が current accepted の場合だけ verified とする。
- library の混在 status は rejected、unavailable、stale、never、verified の順で代表化する。
- dependency closure に含まれただけの solution は library evidence に数えない。

### 優先度 5: Web 実装技術

決定済み:

- Astro で通常ページを静的生成する。
- Pagefind で生成済み HTML を静的検索 index にする。
- Rust 側は正規化 JSON 生成までを担当し、Astro は表示だけを行う。
- 視覚デザイン前に semantic HTML 構造、状態 fixture、interaction contract をまとめる。
- 外部デザイン統合時も route、検索属性、行アンカー、データ意味を維持する。
- Rust の `site-schema` crate が公開 DTO を所有する。
- JSON Schema と TypeScript 型を公開 DTO から build 時に生成する。
- breaking change では `schema_version` を上げ、Web は未対応 version を拒否する。
- 説明 Markdown は GFM 対応とし、raw HTML を無効にして sanitize する。
- 説明 Markdown は h1 を禁止し、h2 から始まる page 全体の見出し階層を検証する。
- 固定 Documentation 見出しは加えず、Markdown heading level をそのまま描画する。
- `doc-*` anchor は ASCII hint と exact UTF-8 heading text の短い SHA-256 から生成する。
- ソースは Shiki で build 時に highlight し、各行へ `L*` anchor を付ける。
- 言語ごとに任意の `syntax_highlight` を指定でき、未知の言語は plain text へ fallback する。
- canonical URL は directory-style の末尾 `/` ありとする。
- library / solution rename の旧 route と自動 redirect は生成しない。
- `/libraries/` は公開可能な言語 page を列挙する常設 browse route とする。
- static 404 は共通 shell と recovery navigation を持ち、redirect せず noindex とする。
- solution 一覧、contest、problem、detail の browse route を生成する。
- 公開 solution が存在する階層だけを生成し、一覧 page 自体は検索対象外とする。
- solution browse は公開対象だけを contest、problem、solution の 3 段階で集計する。
- 全 page は共通 landmark、skip link、primary navigation、global search、breadcrumb を持つ。
- page ごとの h1 は 1 件に限定し、navigation と footer を検索本文から除外する。
- トップは公開状態集計、言語、recent library / solution、attention を安定順で表示する。
- 言語 / directory は直下の子 directory と公開 library を分けて表示する階層 browser とする。
- library detail は tab を使わない単一 article とし、固定 ID の section を上から並べる。
- source viewer は行ごとの `L*` permalink と検索可能な text を持つ固定 HTML contract にする。
- solution detail は同じ article / source contract を使い、library 関係と verify result を表示する。
- verification と dependency / symbol analysis は別々の text 付き status component で表示する。
- detail の構造化情報は共通 list component とし、testcase detail だけを table にする。
- URL は各 path segment を個別に percent-encode し、内部 ID とは分離する。
- Astro の `site` / `base` は build 環境から渡し、すべての URL を共通 helper で生成する。
- CI は root 配信と GitHub Project Pages の `/compro-env/` 配信の両方を検証する。
- Pagefind index は MVP では全 code language、説明、ソースを含む 1 bundle とする。
- 公開 file / symbol 完全一致は公開 DTO 由来の `exact-search-index.json` で順位を保証する。
- 同じ検索名を持つ page をすべて許可し、canonical page ID ごとの card で区別する。
- search card は type、language、status、完全な公開 path、一致理由を常に表示する。
- sub-result は exact symbol を優先して最大 5 件とし、超過件数から detail へ link する。
- page の exact alias は title、full basename、最後の拡張子だけを除いた basename とする。
- symbol の exact alias は name、qualified name、adapter-provided search names とする。
- exact match を先頭へ置き、Pagefind result と page ID で重複排除してから paginate する。
- 記号だけの symbol は Pagefind の tokenization に依存せず exact lookup できる。
- `lang:` は共通 index の `code_lang` filter とし、Pagefind の組み込み chunk を利用する。
- 言語別 index は実測で問題が確認された場合だけ検討する。
- 検索 query は phrase と既知 filter を扱い、未知 key は全文検索語として保持する。
- 同一 filter key は OR、異なる key は AND とする。
- `path:` は言語 root 相対 path の segment と累積 prefix に一致させる。
- solution の `path:` は entry file でなく solution ID の segment / prefix に一致させる。
- search page は header の単一 form、URL 同期、20 件 pagination、file 単位 card を使う。
- 解法ページは repository の entry source を表示する。
- preprocess 後 source は保存・掲載せず、結果には hash と OJ link だけを持たせる。
- library updated_at は source / sidecar の最新 Git committer date から導出する。
- production site build は full Git history を必須とする。
- production site build は title、description、language、repository URL を必須とする。
- 全 page に title / description / canonical / basic Open Graph metadata を生成する。
- search / 404 は noindex とし、公開 page だけの sitemap と robots を生成する。
- 公開 source は 256 KiB で warning、2 MiB で production error とし、途中省略しない。
- site 全体の hard size limit は初期実測後に決める。
- meta CSP は self-hosted asset と Pagefind の WASM / worker だけを許可する。
- search excerpt は mark だけを許し、metadata と query は text として描画する。
- search は phrase と bare / quoted filter value を扱い、quoted value 内は quote と backslash だけを
  escape できる。
- URL layer だけが query を percent-decode し、search parser は decode を繰り返さない。
- Markdown relative link は repository path から公開 route へ変換し、target を build 時に検証する。
- Markdown image と image asset pipeline は対象外とする。
- Rust CLI は site-data までを生成し、Astro / Pagefind は Web package script が起動する。
- local / CI の full build entrypoint は `npm run site:build` に統一する。
- MVP の Node major は 24 LTS とし、`.node-version` で patch まで local / CI 共通に固定する。
- npm と committed lockfile を使い、direct dependency は完全一致 version にする。
- Web toolchain update は自動 merge せず、base path、検索、CSP、link fixture をすべて検証する。
- GitHub Pages は main の build から一時 artifact を deploy し、生成物 branch を持たない。
- PR は production と同じ site build を検証するが deploy しない。
- result-only main push でも最新 result を含む site を再生成して deploy する。
- publish workflow は固定 concurrency group で古い run を中止し、最新 main だけを deploy する。
- artifact の source SHA と deploy 時の main HEAD が一致しなければ公開しない。

Web 実装技術の主要判断は決定済みとする。

### 優先度 6: CI による結果更新

決定済み:

- verify の作業状態は最大 1 本の `automation/verify` draft PR へ保存する。
- OJ へ POST する前に `Starting`、handle 取得直後に `Trackable` を remote branch へ保存する。
- timeout 時は draft PR を残し、schedule または手動実行で resume する。
- verify worker は同時に 1 つだけ実行し、実行中 worker は新しい main push で cancel しない。
- pending heavy verify job は最新の 1 件だけを残す。
- result-only push の分類 job は heavy verify の concurrency group に入れない。
- terminal 結果は現在の fingerprint と異なっても保存し、公開状態を stale とする。
- 結果更新は `attempt_id` と `replaces_attempt_id` による compare-and-swap とする。
- stale になった提出も terminal まで追跡し、同じ提出を自動でやり直さない。
- bot はこの repository だけに install した最小権限の GitHub App token を使用する。
- App permission は Contents と Pull requests の read/write、Metadata read だけとする。
- App token は state writer job ごとに repository / permission を縮小して発行し、永続化しない。
- accepted、rejected、unavailable は有効な terminal 観測として自動マージする。
- pending と `AcceptanceUnknown` は draft のまま残す。
- bot PR の必須 check は verdict ではなく結果の完全性を判定する。
- verify workflow は main push の全変更を軽量 job で分類する。
- result-only push は OJ job を skip し、site publish だけを実行する。
- schedule / manual 実行は差分に関係なく pending state を resume する。
- OJ credential と GitHub App credential は `oj-library-checker` と `verify-state` の
  別 environment / job に分ける。
- 両 environment は main だけを許可し、required reviewer は設定しない。
- OJ job は repository へ書き込めず、bot job は OJ secret を参照できない。
- secret-bearing job で使う action は commit SHA へ固定する。
- secret なしの prepare で immutable `SubmissionPlan` と提出 source を確定する。
- OJ secret job は plan の検証、提出、poll だけを行い、任意 command を起動しない。
- 利用者向け `ce verify` は各内部段階を一連で実行する単一 command とする。
- Pages build と deploy を分け、deploy job だけに `pages: write` と `id-token: write` を与える。
- Pages deploy には main だけを許可する `github-pages` environment を使う。
- site-data、Pagefind index、Pages artifact は Git へ commit しない。
- PR build は main の Pages concurrency group と分離する。
- 過去 workflow の再実行では deploy せず、手動公開は最新 main から build し直す。

CI による結果更新の主要判断は決定済みとする。

### 優先度 7: 将来拡張

- 同じ external adapter contract による追加言語対応。
- AtCoder のブラウザ提出から submission ID を得る userscript / CLI 連携。
- 任意関係ラベルの標準語彙。
- 宣言単位の permalink や詳細ページ。
- 依存グラフの可視化。
- check 結果の公開。
- verify 履歴の Web 表示。
- より高度な検索構文。
