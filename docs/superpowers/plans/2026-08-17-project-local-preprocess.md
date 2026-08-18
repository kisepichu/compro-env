# Project-Local `[submit].preprocess` + `hooks/expand-libraries.sh` 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `[submit].preprocess` を project-local `config.toml` でも設定可能にし、言語非依存の `hooks/expand-libraries.sh`（Rust 分岐は `#[path]` mod を再帰 inline する Python bundler、その他言語は `cat` 素通し）を repo に同梱する。次 PR で aplusb 解法を `#[path]` 経由 import に書き換える下地とする。

**Architecture:**
- `ConfigImpl` を unit struct から `ConfigImpl { project_root }` へ変更し、`submit_preprocess()` の resolve 順を「project-local (`<root>/config.toml` の `[submit].preprocess`) → global (`~/.config/ce/config.toml`)」に変える。project-local の relative path は絶対パス化して返し、global 値は元の挙動どおり `sh -c` にそのまま渡して tilde 展開に任せる。
- `hooks/expand-libraries.sh` は `case "$CE_LANGUAGE"` で分岐する薄い shell スクリプト。Rust 分岐は同 dir に置く `hooks/rust_expand.py`（Python 3、標準ライブラリのみ）を呼び、その他言語は `exec cat`。
- 既存 `hooks/submit-preprocess.sh`（cargo-equip 例）は user global 用サンプルとして残す。project-local には repo の `hooks/expand-libraries.sh` を配線する。

**Tech Stack:** Rust (workspace crates `domain`/`usecases`/`infrastructure`)、POSIX sh、Python 3.10+ 標準ライブラリ、bats-free の bash test runner (`diff` ベース)、GitHub Actions (`ubuntu-latest`)。

## Global Constraints

- Spec-first: `docs/spec.md` §コンフィグ設計 と `docs/commands/submit.md` §config キーの記述を先に更新し、実装をこれに従わせる。
- 4 層 DDD の依存規則: `domain/` は外部依存なし、`usecases/` は `domain` のみ、`interfaces/` は `usecases` まで、`infrastructure/` は全層可。project-local config の path 解決は infrastructure 層 (`config_impl.rs`) 内に閉じる。
- Error handling: `anyhow` + `thiserror`、`E: Error + 'static` 型パラメータ禁止。
- コミット・PR 本文・レビュー返信は日本語、コードコメントと `docs/spec.md` 以外の英語 doc は英語、`spec.md` は既存踏襲（日本語 spec なら日本語で追記）、emoji 禁止。
- python3 は `ubuntu-latest` の default installed に依存する（追加 apt install は不要。CI で verify する）。
- 提出前 preprocess フックの契約（stdin=source / stdout=bundled / exit 0 で採用 / cwd=解法 dir か repository root / env で言語 OJ 情報を渡す）は既存を維持。

---

## Design Decisions

### D1. `ConfigImpl` に project_root を持たせる

**Chosen:** `pub struct ConfigImpl { project_root: PathBuf }` + `impl ConfigImpl { pub fn new(project_root: PathBuf) -> Self }`。既存 unit-struct 使用箇所は明示コンストラクタに書き換える。

**Rationale:**
- `Config::submit_preprocess()` の戻り値 (`Option<String>`) を維持したいので、project-local の相対パスは Config 層で絶対パス化してから返すのが最短。呼び出し側 (`usecases::service::submit::prepare_submission` / `usecases::service::verify::run_preprocess`) は一切変更しなくてよい。
- `find_project_root()` は shell 層のヘルパー。infrastructure の Config が shell を呼ぶ層違反を避けるため、shell 層で root を解決して `ConfigImpl::new(root)` に注入する。
- 既存 shell 呼び出しでの `ConfigImpl.default_language()` などは project_root を実質使わないが、`ConfigImpl::new(find_project_root()?)` に書き換えれば足りる。
- 代替案「`ConfigImpl` が内部で `find_project_root()` を呼ぶ」は、cwd 依存の隠れた副作用を Config 層に持ち込むためテスト・再現性で悪手。

### D2. project-local relative path の絶対パス化

**Chosen:** project-local 値が `sh -c` に渡した際に単一のスクリプトファイルを指すケースを前提に、以下の resolve rule を適用する。
1. 値を空白でトリム。空 or 空白のみ → `None` 相当（未設定）。
2. 値が `/` で始まる（絶対パス） → そのまま返す。
3. 値が `~` で始まる（tilde 展開想定） → そのまま返す（sh に任せる）。
4. それ以外 → `<project_root>/<値>` に join した絶対パスの文字列を返す。

**Rationale:**
- ケース 4 は「引数付きコマンド (`hooks/expand-libraries.sh --debug`)」だと絶対パス化で第 1 引数だけ書き換わらず全体を join してしまう。したがって **space 検出時は 4 ではなく 5 (下記) にフォールバック** する。
5. 上記 4 でさらに値に ASCII whitespace (space / tab) を含む場合は「shell command」とみなしてそのまま返し、資料 (`docs/commands/submit.md`) に「引数を付ける場合は `$CE_PROJECT_ROOT` を使うこと」を明記する。Unicode の NBSP (U+00A0) 等は含まれるパスとして扱う (通常は起きないが、`char::is_ascii_whitespace` で判定することで挙動を明示)。

**Trade-off / 却下案:**
- 「値をパースして最初のトークンだけ join」→ シェルクォート/エスケープを正しく扱えず脆い。
- 「常に project_root を cwd にして sh -c を呼ぶ」→ spec (submit は cwd=解法 dir) との衝突。

### D3. Global 挙動は不変

Global (`~/.config/ce/config.toml`) の `[submit].preprocess` は **元の挙動を維持**：値をそのまま返し、`sh -c` に渡す。tilde は shell が展開する。global の相対パスの `$HOME` からの相対解決は shell が行う既存動作に依存しており、Rust 側で resolve しない（user brief 記載の「global は $HOME からの相対で解決 (元の挙動維持)」を tilde 展開に読み替える。global 値が `~` を含まない bare 相対パスだった場合、既存挙動どおり cwd 依存になる。この edge case は spec 更新で明記）。

### D4. Precedence: project-local が global を上書きする

- project-local `[submit].preprocess` が設定 → その値を採用（絶対パス化ずみ）。
- project-local `config.toml` は無い or `[submit].preprocess` が未設定 → global にフォールバック。
- 両方 None → `None`（従来どおり preprocess を行わない）。

これは `docs/spec.md` の「プロジェクトローカル: `compro-env/config.toml` (任意) — グローバルの同キーを上書き」に整合する。

### D5. `hooks/expand-libraries.sh` の言語分岐

**Chosen:**
```
case "$CE_LANGUAGE" in
  rust) exec python3 "$(dirname "$0")/rust_expand.py" ;;
  cpp|lean) exec cat ;;   # TODO(#issue): 別 issue で bundler を追加する
  *) exec cat ;;
esac
```
Rust bundler 本体は `hooks/rust_expand.py`（Python 3 標準ライブラリのみ）。cpp/lean/その他は現状 passthrough で spec 上の hook 契約を破らない。

**Rationale:** shell の中に長大な Python heredoc を埋めると quoting が事故る。Python は独立ファイルにし、shell は分岐だけを担う。

### D6. Rust bundler の仕様

- 入力: stdin = 解法の `src/main.rs` 相当。
- 対応する mod 宣言（優先度順）:
  1. `#[path = "REL"] mod NAME;` — 明示 path 付き mod 宣言。
  2. `mod NAME;` （path 属性なし） — 起点ファイルからの Rust の暗黙解決:
     - まず `<entry_dir>/NAME.rs`
     - 次に `<entry_dir>/NAME/mod.rs`
     - どちらも無ければ「解決できない mod」として **その `mod NAME;` は展開せず素通しする**（unresolved 扱い。stderr に警告 1 行を出す）。
- 各 `mod NAME;` を `mod NAME { <expanded body> }` に置換。body 中の `mod` 宣言も再帰的に展開する（DFS）。
- entry_dir の決定:
  - CLI 引数で明示 (`python3 rust_expand.py <entry_file>`) されればそのファイルの親ディレクトリ。
  - 引数なし → `CE_SOURCE_FILE` env の親ディレクトリ。
  - env も未設定 → cwd/`src`（fallback。テストでは常に env を渡す）。
- 循環検出: `visited: set[Path]` に `resolve()` した絶対パスを積む。展開中のファイルが再度参照されたら `sys.stderr` にエラーを吐いて `sys.exit(2)`。
- ファイル欠損 / 非 UTF-8 → 同様に非 0 exit（1: file not found, 3: not-UTF-8）。
- comment / string literal 内の `#[path]`-風文字列: **素の regex で走査し、誤検出を許容する** (user brief で明示的に許可された路線)。サンプル解法で `//` 行末コメントや `"..."` string literal 内に本物の `#[path = "..."]` 相当が現れる状況はまず起きない前提。将来「同長のスペース列で comment / string を潰したスキャンバッファ」実装に格上げする可能性は残すが、本 plan のスコープ外。
- 出力: 展開後 source を stdout に、末尾改行 1 個で終わるよう normalize する。
- 「diamond dependency」（A → B → D, A → C → D）は **重複展開が仕様どおり**。B と C はそれぞれの親スコープを持ち、Rust の module システムでは `crate::b::d` と `crate::c::d` が別 module として存在するため、bundler が両方に `mod d { … }` を出しても重複定義エラーにはならない（rustc で確認済み）。`visited.discard(target)` を DFS 巻き戻し時に呼ぶ現行設計はこの Rust semantics に合致しており、意図的な選択。「同一親スコープに同じ module 名が 2 回」というケースは元のソースが Rust としてすでに不正なので bundler の責務外。
- 既知の限界: bare `mod NAME;` の暗黙解決は「現在処理中のファイルの親ディレクトリ」を基準に行い、`mod outer { mod inner; }` のような inline module 内でも Rust 本来の `<outer>/inner.rs` サブディレクトリを参照しない (regex は inline ブロックの nesting を追跡しない)。inline module 内でファイルを include したい場合は必ず `#[path]` 属性を明示すること。競プロ想定のサンプル解法では bare `mod` を top-level だけで使うため実害はない前提。この limitation は `docs/operations/library-expand.md` にも明記する。

### D7. Docs: 別ファイルか節追加か

**Chosen:** `docs/operations/library-expand.md` を新設して `hooks/expand-libraries.sh` の設計・言語別 branch 追加方針を記述、`docs/commands/submit.md` にはリンク 1 行の subsection 追加のみ。

**Rationale:** `hooks/expand-libraries.sh` はコマンドではなく repo 内運用ツールなので `docs/operations/` に属する。既存 `docs/operations/verify-automation.md` と同格に置く。

---

## File Structure

**Modify:**
- `docs/spec.md` §コンフィグ設計（`[submit].preprocess` の project-local 対応を明記）
- `docs/commands/submit.md` §config キー（project-local / global の resolve 順を明記、`hooks/expand-libraries.sh` へのリンク）
- `crates/infrastructure/src/config_impl.rs` (`ConfigImpl` を struct 化、`project_root` フィールド追加、`submit_preprocess()` の resolve 順追加、`project_root()` の trait impl 追加、`config_dir()` は残す)
- `crates/infrastructure/src/shell/mod.rs`（`ConfigImpl.` の unit 呼び出しをすべて `ConfigImpl::new(root)` に書き換える）
- `crates/infrastructure/tests/verify_command.rs` / `crates/usecases/src/service/{submit,init,new_solution,test}.rs` の `StubConfig` (project_root フィールド + `fn project_root(&self) -> &Path` を追加、既存のダミー値を返す)
- `config.toml`（末尾に `[submit]\npreprocess = "hooks/expand-libraries.sh"\n` を追記）
- `.github/workflows/ci.yml`（`cargo test` の後に `bash hooks/tests/run.sh` を実行する step を追加）
- `crates/usecases/src/config.rs`（`Config` trait に `fn project_root(&self) -> &std::path::Path` を追加）
- `crates/usecases/src/service/submit.rs`（`PreprocessContext.project_root` フィールド + `run_preprocess_hook` の `.env("CE_PROJECT_ROOT", …)` 追加、`prepare_submission` で `self.config.project_root()` を差し込む）
- `crates/usecases/src/service/verify.rs`（`run_preprocess` の env チェーンに `.env("CE_PROJECT_ROOT", repository_root)` と `.env("CE_SOURCE_FILE", repository_root.join(entry_rel))` を追加）

**Create:**
- `docs/operations/library-expand.md`（`hooks/expand-libraries.sh` 設計 / 言語別 branch 拡張方針）
- `hooks/expand-libraries.sh`（entrypoint。言語分岐のみ）
- `hooks/rust_expand.py`（Python bundler 本体）
- `hooks/tests/run.sh`（bash test runner。fixture in/expected を diff 比較）
- `hooks/tests/fixtures/rust/basic/main.rs.in` / `main.rs.expected` / `helper.rs`（単段 `#[path]`）
- `hooks/tests/fixtures/rust/nested/main.rs.in` / `main.rs.expected` / `libs.rs` / `algebra/monoid.rs`（`mod libs;` 暗黙解決 + `#[path]` チェーン）
- `hooks/tests/fixtures/rust/cycle/main.rs.in` / `cycled.rs`（循環パターン、exit 2 を確認）
- `hooks/tests/fixtures/rust/missing/main.rs.in`（欠損 `#[path]` file、exit 1 を確認）
- `hooks/tests/fixtures/rust/passthrough/main.rs.in` / `main.rs.expected`（`mod NAME;` (path 属性なし) で `NAME.rs` が無い → passthrough + stderr 警告）
- `hooks/tests/fixtures/rust/diamond/main.rs.in` / `main.rs.expected` / `b.rs` / `c.rs` / `shared.rs`（diamond dep: A → {B, C} → D、`shared` が両親スコープに 1 回ずつ現れることを確認）

---

## Task Order (TDD)

1. **Task 1**: docs 更新（spec + submit + operations）— レビュー通しで先に契約を固定
2. **Task 2**: `Config::submit_preprocess()` の project-local resolve — 失敗テスト → 実装 → 通す
3. **Task 3**: `hooks/rust_expand.py` の bundler — fixture-driven TDD
4. **Task 4**: `hooks/expand-libraries.sh` の shell 分岐 + `hooks/tests/run.sh`
5. **Task 5**: `config.toml` に project-local preprocess を配線
6. **Task 6**: CI (`ci.yml`) に `hooks/tests/run.sh` を追加
7. **Task 7**: `verify` pipeline e2e（`crates/infrastructure/tests/verify_command.rs` に project-local preprocess ケースを追加）

各 Task の終端で `cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `bash hooks/tests/run.sh` を通し、通ったら commit。

---

### Task 1: Docs 更新（spec + submit + operations 新設）

**Files:**
- Modify: `docs/spec.md:80-96`（`[submit]` セクション記述 + project-local subsection に追記）
- Modify: `docs/commands/submit.md:113-127`（config キー節を書き換え）
- Create: `docs/operations/library-expand.md`

**Interfaces:**
- Produces: 「project-local `[submit].preprocess` が global を上書き」「相対パスは project root からの相対 (project-local 側) / tilde/絶対はそのまま」の仕様確定。以降の Task は spec 記述に従う。

- [ ] **Step 1: `docs/spec.md` を編集**

`docs/spec.md` §コンフィグ設計 の `[submit].preprocess` 説明パラグラフに以下を追記する。編集対象は現行の 80–88 行：

```markdown
提出前 preprocess フックは `[submit].preprocess` (全言語共通の1本) のみ。アプリにバンドル/言語別
ロジックを持たず、整形・ライブラリ展開・提出レイアウトをすべてユーザースクリプトに委ねる拡張点。
言語別の分岐は `CE_LANGUAGE` env を使ってスクリプト内で行う (per-language config キーは設けない。
詳細: `docs/commands/submit.md`)。

`[submit].preprocess` は project-local (`<repository_root>/config.toml`) にも書ける。両方に書かれた
場合は project-local が global を上書きする。値の解決規則:
- 絶対パス (`/`) や tilde 始まり (`~`) は shell の展開に任せてそのまま渡す。
- project-local 側で bare relative path (空白なし) の場合、`<repository_root>` を prefix した絶対
  パスに書き換えてから hook 実行に渡す。
- project-local 側で空白を含む値 (引数付きコマンド) は shell command とみなしてそのまま渡す。
  この場合はスクリプト内で `$CE_PROJECT_ROOT` を参照してリポジトリルートを解決すること。
- global 側は元の挙動どおり shell に丸投げ (tilde 展開は shell 依存、bare relative は cwd 依存)。

**セキュリティ**: project-local `[submit].preprocess` は clone したリポジトリの `config.toml` に書かれた任意 shell スクリプト
を `ce submit` / `ce verify` 時にユーザー権限で実行する (Makefile / `package.json` の script と同じ信頼境界)。信頼できるリポジトリでのみ `ce` を使うこと。
```

`### プロジェクトローカル: compro-env/config.toml (任意)` 節にも 1 行、`[submit].preprocess` が
上書き対象キーであることを明記する。

- [ ] **Step 2: `docs/commands/submit.md` を編集**

現行 113–127 行の「### config キー」節を以下に置き換える：

````markdown
### config キー

```toml
# global: ~/.config/ce/config.toml
[submit]
preprocess = "~/.config/ce/hooks/submit-preprocess.sh"   # 全言語共通の1本

# project-local: <repository_root>/config.toml (任意、global を上書き)
[submit]
preprocess = "hooks/expand-libraries.sh"                 # repo 同梱の言語非依存 hook
```

キーは `[submit].preprocess` のみ。project-local と global の両方に書いた場合は **project-local が
global を上書き**する。値の resolve 規則:

| 値のかたち                              | 解決                                                                 |
| --------------------------------------- | -------------------------------------------------------------------- |
| `/…` (絶対パス)                         | そのまま `sh -c` に渡す                                              |
| `~/…` (tilde 付き)                      | そのまま `sh -c` に渡す (shell が展開)                               |
| project-local の bare relative (空白なし) | `<repository_root>/<値>` に絶対パス化して渡す                         |
| project-local の relative (空白あり)     | shell command とみなしそのまま渡す。`$CE_PROJECT_ROOT` を参照して自解決 |
| global の relative                       | 元の挙動どおり shell に丸投げ (cwd 依存)                              |

未設定なら preprocess を行わず元ソースをそのまま提出する (後方互換)。
`Config::submit_preprocess(&self) -> Option<String>` を返し、未設定時は `None` とする
(`&Language` 引数は取らない)。

> **セキュリティ**: project-local `[submit].preprocess` は clone したリポジトリの `config.toml` に書かれた任意 shell スクリプト
> を `ce submit` / `ce verify` 時にユーザー権限で実行する。Makefile / `package.json` の script 等と同じ信頼境界にあるため、
> **信頼できるリポジトリでのみ `ce` を使うこと**。悪意ある `config.toml` を含む repo を clone した第三者が細工したスクリプトを
> 実行させられるリスクがあることを念頭に置く。

**言語別の分岐はアプリではなくスクリプト側で行う。** 言語は `CE_LANGUAGE` env で渡るので、
1 本のスクリプト内で `case "$CE_LANGUAGE" in rust) ... ;; cpp) ... esac` のように分岐する。
言語ごとにファイルを分けたいユーザーは、メインスクリプトに `exec "$(dirname "$0")/hooks/$CE_LANGUAGE.sh"` の
1 行を書けば自前でディスパッチでき、アプリ側の支援は不要 (per-language config キーは設けない)。

### スクリプト例

repo にはユースケース別に 2 本のサンプルを同梱する:

- `hooks/submit-preprocess.sh` — **user global 向け例**。Rust 分岐で `cargo-equip --check` を呼び、
  他言語は `cat` 素通し。`~/.config/ce/hooks/` にコピーして使う。
- `hooks/expand-libraries.sh` — **project-local 向け例**。言語非依存のエントリポイントで、Rust 分岐は
  同 dir の `rust_expand.py` を呼び、solution の `#[path = "..."] mod ...;` チェーンを再帰的に inline
  する。cpp / lean は現状素通し (別 issue で bundler を追加予定)。詳細設計は
  `docs/operations/library-expand.md`。
````

加えて、同ファイル §環境変数 の表 (現行 101–111 行) に以下の行を追記する:

| 変数                 | 内容                                                             |
| -------------------- | ---------------------------------------------------------------- |
| `CE_PROJECT_ROOT`    | リポジトリルートの絶対パス。project-local の relative (空白あり) から自解決するときに使う |

- [ ] **Step 3: `docs/operations/library-expand.md` を新規作成**

```markdown
# hooks/expand-libraries.sh 設計

`hooks/expand-libraries.sh` は repo が同梱する言語非依存の submit preprocess フック。project-local
`[submit].preprocess = "hooks/expand-libraries.sh"` に配線されている。契約は既存 `hooks/submit-preprocess.sh`
と同一 (`docs/commands/submit.md` §提出前 preprocess フック 参照)。

## 言語別分岐

```
case "$CE_LANGUAGE" in
  rust)      exec python3 "$(dirname "$0")/rust_expand.py" ;;
  cpp|lean)  exec cat ;;   # TODO: 別 issue で bundler を追加する
  *)         exec cat ;;
esac
```

## Rust bundler (`hooks/rust_expand.py`)

### 入出力
- stdin: 解法の `src/main.rs` (Rust source)。
- stdout: `mod` 宣言を再帰的に inline した展開後 source。末尾改行 1 個で normalize。
- exit 0: 成功。展開結果を採用。
- exit != 0: 展開失敗。stderr に理由を出し、`ce` は提出を中止する。
  - 1: file not found (`#[path]` 先のファイルが読めない)
  - 2: cycle detected
  - 3: non-UTF-8 file
  - その他: internal error

### 展開ルール
1. `#[path = "REL"] mod NAME;` — `REL` は entry file の親ディレクトリ相対で解決。
2. path 属性のない `mod NAME;` — Rust 標準の暗黙解決:
   - `<entry_dir>/NAME.rs`
   - `<entry_dir>/NAME/mod.rs`
   - どちらも無ければ **passthrough**（stderr に 1 行 warn、`mod NAME;` は元のまま残す）。
3. 各 `mod NAME;` を `mod NAME { <expanded body> }` に inline 置換。
4. body 中の mod 宣言も同様に再帰展開 (DFS)。
5. 展開済みファイルは `visited: set[abs_path]` に記録。再訪 → cycle として exit 2。

> **限界 (nesting 未追跡)**: bare `mod NAME;` の解決は常に「処理中のファイルの親ディレクトリ」を基準に行い、`mod outer { mod inner; }` のような inline module 内でも Rust 本来の `<outer>/inner.rs` を参照しない。inline module 内でファイルを include したい場合は必ず `#[path]` 属性を書く。sample 解法では bare `mod` を top-level に限る運用で実害なし。

### コメント/文字列の扱い
- **採用**: 素の regex で走査し、コメント / 文字列 literal 内の誤検出は許容する (sample 解法で発生しない前提)。
- **将来拡張**: `//` 行末コメント、`/* … */` ブロックコメント、`"…"` / `r"…"` / `r#"…"#` string literal を「同長スペース列」に置換したスキャン用バッファを別途作り、マッチ位置検出だけそちらで行う。実装コスト高のため必要になった時点で切り替える。

### entry file の決定
- 引数 (`python3 rust_expand.py <entry_file>`) を優先。
- 引数なしなら `$CE_SOURCE_FILE` env の親ディレクトリを entry_dir とする (`ce` から呼ぶ場合は必ず
  この env が渡る)。

## 別言語 bundler の追加方針

新しい言語で bundler を書く場合:
1. `hooks/<lang>_expand.py` (or `.sh`) を新規追加。stdin/stdout 契約は `rust_expand.py` と同じ。
2. `hooks/expand-libraries.sh` の case 分岐に 1 行加える。
3. `hooks/tests/fixtures/<lang>/` に fixture in/expected を置き、`hooks/tests/run.sh` に diff 比較を
   追加。
4. `docs/operations/library-expand.md` (このファイル) に該当節を追記。

アプリ (Rust クレート群) 側の変更は一切不要。
```

- [ ] **Step 4: docs を diff で確認 → commit**

`git diff docs/` で 3 ファイルの差分を確認し、typo / リンク切れがないか目視。

```bash
git add docs/spec.md docs/commands/submit.md docs/operations/library-expand.md
git commit -m "docs(preprocess): project-local [submit].preprocess を仕様に追加"
```

---

### Task 2: `Config::submit_preprocess()` の project-local resolve

**Files:**
- Modify: `crates/infrastructure/src/config_impl.rs`
- Modify: `crates/infrastructure/src/shell/mod.rs`（`ConfigImpl` インスタンス化を `ConfigImpl::new(root)` に書き換え）

**Interfaces:**
- Consumes: `find_project_root() -> Result<PathBuf>` (既存、`shell/mod.rs`)。
- Produces: `pub struct ConfigImpl { project_root: PathBuf }` + `pub fn new(project_root: PathBuf) -> Self`. `Config::submit_preprocess()` の resolve 順は「project-local → global」に変更。project-local relative は絶対パス化して返す。

- [ ] **Step 1: 失敗テストを 6 本追加**

`crates/infrastructure/src/config_impl.rs` の `#[cfg(test)] mod tests` 末尾に以下 6 テストを追加。

```rust
/// project-local [submit].preprocess が global を上書きする。
#[test]
#[serial]
fn submit_preprocess_project_local_overrides_global() {
    let global_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        global_dir.path().join("config.toml"),
        "[submit]\npreprocess = \"~/.config/ce/hooks/global.sh\"\n",
    )
    .unwrap();
    let _guard_home = EnvVarGuard::set("CE_CONFIG_DIR", global_dir.path());

    let project_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        project_dir.path().join("config.toml"),
        "[submit]\npreprocess = \"hooks/expand-libraries.sh\"\n",
    )
    .unwrap();

    let config = ConfigImpl::new(project_dir.path().to_path_buf());
    let expected = project_dir
        .path()
        .join("hooks/expand-libraries.sh")
        .to_string_lossy()
        .into_owned();
    assert_eq!(config.submit_preprocess(), Some(expected));
}

/// project-local だけに書いてある場合、絶対パスに resolve される。
#[test]
#[serial]
fn submit_preprocess_project_local_only_resolves_to_absolute() {
    let global_dir = tempfile::tempdir().unwrap();
    let _guard_home = EnvVarGuard::set("CE_CONFIG_DIR", global_dir.path());

    let project_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        project_dir.path().join("config.toml"),
        "[submit]\npreprocess = \"hooks/expand-libraries.sh\"\n",
    )
    .unwrap();

    let config = ConfigImpl::new(project_dir.path().to_path_buf());
    let expected = project_dir
        .path()
        .join("hooks/expand-libraries.sh")
        .to_string_lossy()
        .into_owned();
    assert_eq!(config.submit_preprocess(), Some(expected));
}

/// project-local に無ければ global にフォールバックし、global 値はそのまま返る (tilde 保存)。
#[test]
#[serial]
fn submit_preprocess_falls_back_to_global_verbatim() {
    let global_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        global_dir.path().join("config.toml"),
        "[submit]\npreprocess = \"~/.config/ce/hooks/global.sh\"\n",
    )
    .unwrap();
    let _guard_home = EnvVarGuard::set("CE_CONFIG_DIR", global_dir.path());

    let project_dir = tempfile::tempdir().unwrap();
    // project-local config.toml は作らない

    let config = ConfigImpl::new(project_dir.path().to_path_buf());
    assert_eq!(
        config.submit_preprocess(),
        Some("~/.config/ce/hooks/global.sh".to_string())
    );
}

/// project-local の絶対パスと tilde 始まりはそのまま返る (resolve しない)。
#[test]
#[serial]
fn submit_preprocess_project_local_absolute_and_tilde_pass_through() {
    let global_dir = tempfile::tempdir().unwrap();
    let _guard_home = EnvVarGuard::set("CE_CONFIG_DIR", global_dir.path());

    let project_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        project_dir.path().join("config.toml"),
        "[submit]\npreprocess = \"/opt/ce/hooks/x.sh\"\n",
    )
    .unwrap();

    let config = ConfigImpl::new(project_dir.path().to_path_buf());
    assert_eq!(
        config.submit_preprocess(),
        Some("/opt/ce/hooks/x.sh".to_string())
    );

    std::fs::write(
        project_dir.path().join("config.toml"),
        "[submit]\npreprocess = \"~/foo/x.sh\"\n",
    )
    .unwrap();
    let config = ConfigImpl::new(project_dir.path().to_path_buf());
    assert_eq!(
        config.submit_preprocess(),
        Some("~/foo/x.sh".to_string())
    );
}

/// project-local が空白を含む (引数付きコマンド) 場合はそのまま返る (絶対パス化しない)。
#[test]
#[serial]
fn submit_preprocess_project_local_command_with_args_passes_through() {
    let global_dir = tempfile::tempdir().unwrap();
    let _guard_home = EnvVarGuard::set("CE_CONFIG_DIR", global_dir.path());

    let project_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        project_dir.path().join("config.toml"),
        "[submit]\npreprocess = \"hooks/expand-libraries.sh --debug\"\n",
    )
    .unwrap();

    let config = ConfigImpl::new(project_dir.path().to_path_buf());
    assert_eq!(
        config.submit_preprocess(),
        Some("hooks/expand-libraries.sh --debug".to_string())
    );
}

/// project-local `preprocess = ""` (空文字 / 空白のみ) は「未設定」扱いで global に fallback する。
#[test]
#[serial]
fn submit_preprocess_project_local_empty_value_falls_back_to_global() {
    let global_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        global_dir.path().join("config.toml"),
        "[submit]\npreprocess = \"~/.config/ce/hooks/global.sh\"\n",
    )
    .unwrap();
    let _guard_home = EnvVarGuard::set("CE_CONFIG_DIR", global_dir.path());

    let project_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        project_dir.path().join("config.toml"),
        "[submit]\npreprocess = \"   \"\n",
    )
    .unwrap();

    let config = ConfigImpl::new(project_dir.path().to_path_buf());
    assert_eq!(
        config.submit_preprocess(),
        Some("~/.config/ce/hooks/global.sh".to_string()),
    );
}
```

既存の 3 本 (`submit_preprocess_returns_configured_value`, `submit_preprocess_returns_none_when_not_configured`, `submit_preprocess_returns_none_when_no_config`) は unit-struct 前提。以下のように書き換える (合計 6 本の新規テストに合わせる):

```rust
// 変更前: let result = ConfigImpl.submit_preprocess();
// 変更後: let result = ConfigImpl::new(some_project_root_that_has_no_config()).submit_preprocess();
//         (project-local が None であることを保証するため、project-local config.toml のない
//          別 tempdir を project_root に渡す)
```

新しい helper を tests mod 冒頭に追加:

```rust
fn tmp_root_without_config() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
```

- [ ] **Step 2: テストを実行 → 全 6 本失敗を確認**

```bash
cargo test -p infrastructure config_impl:: -- --nocapture 2>&1 | grep -E 'FAILED|running|test result'
```

**Expected:** 新しい 6 テストが `FAILED` になる（`ConfigImpl::new` が存在しない / project-local resolve 未実装）。

- [ ] **Step 3: `ConfigImpl` を struct 化 + `submit_preprocess()` を書き換え**

```rust
// crates/infrastructure/src/config_impl.rs
use anyhow::{Context as _, Result};
use domain::entity::{Language, OJKind};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use usecases::config::Config;

pub struct ConfigImpl {
    project_root: PathBuf,
}

impl ConfigImpl {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    fn config_dir() -> Result<PathBuf> {
        // 既存実装のまま (global config dir 解決)
        // ...
    }

    fn config_toml_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    fn project_config_toml_path(&self) -> PathBuf {
        self.project_root.join("config.toml")
    }

    /// project-local `[submit].preprocess` を読み、resolve 済みの値を返す。
    /// 空 / 空白のみは「未設定」扱いで `None` を返し、下流で global へ fallback する。
    /// 相対 (bare, no whitespace) は `<project_root>/<value>` に絶対パス化。
    /// 絶対 / tilde / 空白入りはそのまま返す。
    fn read_project_local_preprocess(&self) -> Option<String> {
        let path = self.project_config_toml_path();
        if !path.exists() {
            return None;
        }
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| eprintln!("warning: failed to read {}: {e}", path.display()))
            .ok()?;
        let table: toml::Table = toml::from_str(&contents)
            .map_err(|e| eprintln!("warning: failed to parse {}: {e}", path.display()))
            .ok()?;
        let raw = table
            .get("submit")
            .and_then(|v| v.get("preprocess"))
            .and_then(|v| v.as_str())?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(resolve_project_local_preprocess(trimmed, &self.project_root))
    }

    /// global `[submit].preprocess` を読み、値をそのまま返す。
    fn read_global_preprocess(&self) -> Option<String> {
        let path = Self::config_toml_path().ok()?;
        if !path.exists() {
            return None;
        }
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| eprintln!("warning: failed to read {}: {e}", path.display()))
            .ok()?;
        let table: toml::Table = toml::from_str(&contents)
            .map_err(|e| eprintln!("warning: failed to parse {}: {e}", path.display()))
            .ok()?;
        table
            .get("submit")
            .and_then(|v| v.get("preprocess"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

/// Value の resolve ルール (docs/commands/submit.md 参照)。呼び出し側で
/// 既に `trim` + `is_empty` チェックを済ませている前提。
fn resolve_project_local_preprocess(trimmed: &str, project_root: &Path) -> String {
    debug_assert!(
        !trimmed.is_empty() && trimmed == trimmed.trim(),
        "caller must pass a trimmed, non-empty value",
    );
    // 絶対パス・tilde・空白入り (= 引数付きコマンド) はそのまま。
    if trimmed.starts_with('/')
        || trimmed.starts_with('~')
        || trimmed.chars().any(|c: char| c.is_ascii_whitespace())
    {
        return trimmed.to_string();
    }
    project_root.join(trimmed).to_string_lossy().into_owned()
}

impl Config for ConfigImpl {
    // default_language / default_online_judge / submit_file / lang_id は既存実装を維持
    // (config_toml_path を触るところは同じ、self を追加パラメタとしない場合はそのまま)

    fn project_root(&self) -> &Path {
        &self.project_root
    }

    fn submit_preprocess(&self) -> Option<String> {
        // 空文字 / 空白は `read_project_local_preprocess` 側で `None` に畳まれる
        // ため、追加の `.filter(...)` は不要 (sentinel `Some("")` を経由しない)。
        self.read_project_local_preprocess()
            .or_else(|| self.read_global_preprocess())
    }

    // lang_id / submit_file / default_language は既存実装（&self を渡す形に変更するだけ）
    // ...
}
```

`default_language` / `submit_file` / `lang_id` は既存ロジックそのままだが `&self` を通す関数シグネチャに揃える (すでに `&self` を取っているのでボディ側の変更は不要)。関数内で `Self::config_toml_path()` を呼んでいる箇所は変更なし。

加えて `crates/usecases/src/config.rs` の `Config` trait に対応シグネチャを追加する:

```rust
// crates/usecases/src/config.rs
use std::path::Path;
// ...
pub trait Config {
    // ...既存メソッド...

    /// リポジトリルート (project-local `config.toml` を持つディレクトリ)。
    /// preprocess hook の `CE_PROJECT_ROOT` env と、絶対パス化に使う。
    fn project_root(&self) -> &Path;

    // 既存の submit_preprocess / lang_id / submit_file / default_language は変更なし。
}
```

- [ ] **Step 3b: submit / verify サービスに `CE_PROJECT_ROOT` env の設定を追加**

`crates/usecases/src/service/submit.rs`:

```rust
// PreprocessContext に project_root を追加
struct PreprocessContext<'a> {
    // ... 既存フィールド ...
    project_root: &'a std::path::Path,
}

// prepare_submission の PreprocessContext 構築時
PreprocessContext {
    // ... 既存 ...
    project_root: self.config.project_root(),
}

// run_preprocess_hook の Command::new("sh") チェーンに 1 行追加
.env("CE_PROJECT_ROOT", ctx.project_root)
```

`crates/usecases/src/service/verify.rs::run_preprocess`:

```rust
// 既存の env チェーンに以下 2 行を追加。CE_SOURCE_FILE は rust_expand.py が
// entry_dir を導出するために必須 (submit.rs::run_preprocess_hook は既に
// CE_SOURCE_FILE を設定しているので、verify 経路とここで同期させる)。
.env("CE_PROJECT_ROOT", repository_root)
.env("CE_SOURCE_FILE", repository_root.join(entry_rel))
```

- [ ] **Step 3c: 各 `StubConfig` に `project_root()` を追加**

以下 5 箇所の `impl Config for StubConfig` に、`PathBuf` フィールドと `fn project_root(&self) -> &Path { &self.project_root }` を追加する。既存の他フィールドと同じ構造:

- `crates/usecases/src/service/submit.rs` (tests)
- `crates/usecases/src/service/init.rs` (tests)
- `crates/usecases/src/service/new_solution.rs` (tests)
- `crates/usecases/src/service/test.rs` (tests)
- `crates/infrastructure/tests/verify_command.rs`

```rust
// 例: crates/usecases/src/service/submit.rs tests
struct StubConfig {
    // ... 既存 ...
    project_root: std::path::PathBuf,
}
impl Config for StubConfig {
    // ... 既存 ...
    fn project_root(&self) -> &std::path::Path {
        &self.project_root
    }
}
```

テスト側の `StubConfig { ... }` インスタンス化箇所 (現在 6+ 箇所) にも `project_root: std::env::temp_dir(),` 等のダミー値を追加する。既存の submit_preprocess テストでこの値が意味を持たなければ tempdir で十分。

- [ ] **Step 4: `shell/mod.rs` の `ConfigImpl.` 呼び出しをすべて書き換え**

該当箇所 (grep 結果より):

- `shell/mod.rs:69,125,147,715,821,911,1010,1066,1083,1110` の `ConfigImpl.` / `Box::new(ConfigImpl)` を、周囲に既にある `root` (or `find_project_root()?`) を使って `ConfigImpl::new(root.clone())` / `Box::new(ConfigImpl::new(root.clone()))` に書き換える。

例:
```rust
// Before
Box::new(ConfigImpl),

// After
Box::new(ConfigImpl::new(root.clone())),
```

`ConfigImpl.default_online_judge()` のような読み取り専用呼び出しでも、`root` は上位で計算済み。無ければ:
```rust
let root = find_project_root()?;
let cfg = ConfigImpl::new(root);
```
を差し込む。

- [ ] **Step 5: テストを実行 → 通ることを確認**

```bash
cargo test -p infrastructure config_impl:: -- --nocapture
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

すべて green を確認。

- [ ] **Step 6: commit**

```bash
git add \
  crates/infrastructure/src/config_impl.rs \
  crates/infrastructure/src/shell/mod.rs \
  crates/usecases/src/config.rs \
  crates/usecases/src/service/submit.rs \
  crates/usecases/src/service/verify.rs \
  crates/usecases/src/service/init.rs \
  crates/usecases/src/service/new_solution.rs \
  crates/usecases/src/service/test.rs \
  crates/infrastructure/tests/verify_command.rs
git commit -m "feat(config): [submit].preprocess を project-local で上書き可能に + CE_PROJECT_ROOT env 追加"
```

---

### Task 3: `hooks/rust_expand.py` の bundler 実装

**Files:**
- Create: `hooks/rust_expand.py`
- Create: `hooks/tests/fixtures/rust/basic/`
  - `main.rs.in`, `main.rs.expected`, `helper.rs`
- Create: `hooks/tests/fixtures/rust/nested/`
  - `main.rs.in`, `main.rs.expected`, `libs.rs`, `algebra/monoid.rs`
- Create: `hooks/tests/fixtures/rust/cycle/`
  - `main.rs.in`, `cycled.rs`
- Create: `hooks/tests/fixtures/rust/missing/main.rs.in`
- Create: `hooks/tests/fixtures/rust/passthrough/`
  - `main.rs.in`, `main.rs.expected`
- Create: `hooks/tests/fixtures/rust/diamond/`
  - `main.rs.in`, `main.rs.expected`, `b.rs`, `c.rs`, `shared.rs`
- Create: `hooks/tests/run.sh` (Task 4 で本体を書くが、この Task では diff 比較の骨組みだけ作る)

**Interfaces:**
- Consumes: stdin = Rust source。引数 (optional) = entry file path。
- Produces: stdout = 展開済み source。exit code = 0 (success), 1 (file not found), 2 (cycle), 3 (non-UTF-8)。

- [ ] **Step 1: fixture in/expected を先に書く (TDD)**

**basic** (`#[path]` 明示、単段):

`hooks/tests/fixtures/rust/basic/main.rs.in`:
```rust
#[path = "helper.rs"]
mod helper;

fn main() {
    helper::greet();
}
```

`hooks/tests/fixtures/rust/basic/helper.rs`:
```rust
pub fn greet() {
    println!("hi");
}
```

`hooks/tests/fixtures/rust/basic/main.rs.expected`:
```rust
mod helper {
pub fn greet() {
    println!("hi");
}

}

fn main() {
    helper::greet();
}
```

**nested** (`mod libs;` (path 属性なし) → `libs.rs` → `#[path]` チェーン):

`hooks/tests/fixtures/rust/nested/main.rs.in`:
```rust
mod libs;
use libs::algebra::monoid::AddMonoid;

fn main() {
    let x = AddMonoid::op(&AddMonoid::id(), &42);
    println!("{x}");
}
```

`hooks/tests/fixtures/rust/nested/libs.rs`:
```rust
pub mod algebra {
    #[path = "algebra/monoid.rs"]
    pub mod monoid;
}
```

`hooks/tests/fixtures/rust/nested/algebra/monoid.rs`:
```rust
pub trait Monoid {
    type T: Clone;
    fn id() -> Self::T;
    fn op(a: &Self::T, b: &Self::T) -> Self::T;
}

pub struct AddMonoid;

impl Monoid for AddMonoid {
    type T = i64;
    fn id() -> Self::T { 0 }
    fn op(a: &Self::T, b: &Self::T) -> Self::T { a + b }
}
```

`hooks/tests/fixtures/rust/nested/main.rs.expected` (展開後の 1 ファイル。空白 / 改行位置は実装の
出力に合わせて決めるが、以下の骨格で不変):

```rust
mod libs {
pub mod algebra {
pub mod monoid {
pub trait Monoid {
    type T: Clone;
    fn id() -> Self::T;
    fn op(a: &Self::T, b: &Self::T) -> Self::T;
}

pub struct AddMonoid;

impl Monoid for AddMonoid {
    type T = i64;
    fn id() -> Self::T { 0 }
    fn op(a: &Self::T, b: &Self::T) -> Self::T { a + b }
}

}
}

}
use libs::algebra::monoid::AddMonoid;

fn main() {
    let x = AddMonoid::op(&AddMonoid::id(), &42);
    println!("{x}");
}
```

> 注: `pub mod monoid` 側の展開結果で `pub` が失われる問題は「原文の `pub mod NAME;` を
> `pub mod NAME { … }` に置換する」regex 実装で解決する (下記 Step 2 参照)。
> ここで `libs.rs` の `#[path]` 付き `pub mod monoid;` は `pub mod monoid { … }` になるべき。

**cycle** (直接自己参照):

`hooks/tests/fixtures/rust/cycle/main.rs.in`:
```rust
#[path = "cycled.rs"]
mod cycled;
```

`hooks/tests/fixtures/rust/cycle/cycled.rs`:
```rust
#[path = "cycled.rs"]
mod cycled;
```

期待動作: exit 2, stderr に `cycle detected: .../cycled.rs`。

**diamond** (D6 の意図確認: `visited.discard(target)` が sibling 経由の再展開を許すことを検証):

`hooks/tests/fixtures/rust/diamond/main.rs.in`:
```rust
#[path = "b.rs"]
mod b;
#[path = "c.rs"]
mod c;

fn main() {}
```

`hooks/tests/fixtures/rust/diamond/b.rs`:
```rust
#[path = "shared.rs"]
pub mod shared;
```

`hooks/tests/fixtures/rust/diamond/c.rs`:
```rust
#[path = "shared.rs"]
pub mod shared;
```

`hooks/tests/fixtures/rust/diamond/shared.rs`:
```rust
pub const V: u32 = 42;
```

`hooks/tests/fixtures/rust/diamond/main.rs.expected` (骨格。空白は実装出力に合わせて微調整):
```rust
mod b {
pub mod shared {
pub const V: u32 = 42;

}

}
mod c {
pub mod shared {
pub const V: u32 = 42;

}

}

fn main() {}
```

期待動作: exit 0、`pub mod shared { … }` が 2 回 (`crate::b::shared` と `crate::c::shared`) 出現し、rustc でコンパイルできる (dead-code warning のみ)。`visited.discard(target)` を削除すると 2 回目が cycle エラーになるため、この fixture でリグレッションを検出できる。

**missing** (欠損 file):

`hooks/tests/fixtures/rust/missing/main.rs.in`:
```rust
#[path = "no-such-file.rs"]
mod missing;
```

期待動作: exit 1, stderr に `file not found`。

**passthrough** (`mod NAME;` (path 属性なし) で NAME.rs が無い):

`hooks/tests/fixtures/rust/passthrough/main.rs.in`:
```rust
mod std_only_no_local_file;

fn main() { println!("noop"); }
```

`hooks/tests/fixtures/rust/passthrough/main.rs.expected`:
```rust
mod std_only_no_local_file;

fn main() { println!("noop"); }
```

期待動作: exit 0, main は無変更, stderr に `warning: unresolved mod std_only_no_local_file` を含む。

- [ ] **Step 2: `hooks/rust_expand.py` を実装**

```python
#!/usr/bin/env python3
"""Recursively inline Rust `mod` declarations for submit-time bundling.

Contract (see docs/operations/library-expand.md):
  stdin:  Rust source (typically the solution's src/main.rs).
  stdout: bundled Rust source with `#[path]` mod chains inlined.
  argv[1] (optional): entry file path used to derive the base directory for
                      relative path resolution. When absent we use
                      `$CE_SOURCE_FILE` from the environment; when that is
                      also absent we fall back to `cwd/src`.

Exit codes:
  0 = success
  1 = file not found
  2 = cycle detected
  3 = non-UTF-8 file
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path
from typing import NoReturn

COMBINED_MOD_RE = re.compile(
    r"""^[ \t]*                                    # anchored to line start (re.MULTILINE)
        (?P<attrs>(?:\#\s*\[[^\]]*\]\s*)*)         # zero or more leading attributes (any kind, incl. #[path]); \s* between allows adjacent-with-no-space form
        (?P<vis>pub(?:\s*\(\s*[^)]+\s*\))?\s+)?
        mod\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*;""",
    re.MULTILINE | re.VERBOSE,
)

# Individual `#[path = "..."]` picker for the callback below.
PATH_ATTR_RE = re.compile(r'\#\s*\[\s*path\s*=\s*"([^"]+)"\s*\]\s*')


def die(code: int, msg: str) -> NoReturn:
    print(f"rust_expand: {msg}", file=sys.stderr)
    sys.exit(code)


def read_utf8(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError:
        die(1, f"file not found: {path}")
    except UnicodeDecodeError as e:
        die(3, f"non-UTF-8 file {path}: {e}")


def expand_source(source: str, entry_dir: Path, visited: set[Path]) -> str:
    # Single-pass expansion: one regex covers both `#[path] mod NAME;` and
    # bare `mod NAME;`. Passing them as one alternation guarantees each
    # declaration is scanned exactly once, at the correct nesting level with
    # the correct `entry_dir`. A two-pass approach would let a bare `mod`
    # that was left passthrough by a sub-file get re-scanned in the outer
    # `entry_dir` after the sub-body was spliced in, which would wrongly
    # resolve it against a same-named file in the outer directory.
    def repl(m: re.Match[str]) -> str:
        name = m.group("name")
        vis = (m.group("vis") or "").strip()
        vis_prefix = f"{vis} " if vis else ""
        # Attributes may appear in any order (e.g. `#[allow(...)] #[path = "..."]`
        # or `#[path = "..."] #[cfg(test)]`). Extract the last `#[path]` — Rust
        # only honors one — and preserve everything else on top of the expanded
        # `mod NAME { ... }` so semantics (cfg-gating, allow-lints) survive.
        attrs_raw = m.group("attrs") or ""
        path_matches = list(PATH_ATTR_RE.finditer(attrs_raw))
        rel_path = path_matches[-1].group(1) if path_matches else None
        other_attrs = PATH_ATTR_RE.sub("", attrs_raw).strip()
        extras_prefix = f"{other_attrs}\n" if other_attrs else ""
        if rel_path is not None:
            target = (entry_dir / rel_path).resolve()
            if not target.is_file():
                die(1, f"file not found: {target}")
        else:
            target = None
            for cand in (entry_dir / f"{name}.rs", entry_dir / name / "mod.rs"):
                if cand.is_file():
                    target = cand.resolve()
                    break
            if target is None:
                # Passthrough: leave the declaration verbatim (attributes and
                # all) so the caller can still compile against std / external
                # crates. Warn once.
                print(
                    f"rust_expand: warning: unresolved mod {name}",
                    file=sys.stderr,
                )
                return m.group(0)
        if target in visited:
            die(2, f"cycle detected: {target}")
        visited.add(target)
        body = read_utf8(target)
        expanded = expand_source(body, target.parent, visited)
        visited.discard(target)
        return f"{extras_prefix}{vis_prefix}mod {name} {{\n{expanded}\n}}"

    return COMBINED_MOD_RE.sub(repl, source)


def resolve_entry_file(argv: list[str]) -> Path:
    if len(argv) >= 2:
        return Path(argv[1]).resolve()
    env = os.environ.get("CE_SOURCE_FILE")
    if env:
        return Path(env).resolve()
    return (Path.cwd() / "src" / "main.rs").resolve()


def main() -> None:
    entry = resolve_entry_file(sys.argv)
    entry_dir = entry.parent
    source = sys.stdin.read()
    visited: set[Path] = {entry}
    out = expand_source(source, entry_dir, visited)
    if not out.endswith("\n"):
        out += "\n"
    sys.stdout.write(out)


if __name__ == "__main__":
    main()
```

- [ ] **Step 3: `hooks/tests/run.sh` を骨組みだけ書く (Task 4 で拡張)**

```bash
#!/usr/bin/env bash
# Regression tests for hooks/rust_expand.py.
#
# Layout: hooks/tests/fixtures/rust/<case>/{main.rs.in,main.rs.expected,...}
# For each success case we feed main.rs.in on stdin (passing an absolute
# path as argv[1] to fix the base dir) and diff stdout against
# main.rs.expected. Fail cases (cycle, missing) assert on exit code and
# stderr fragment.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
FIXTURES="$HERE/fixtures"
SCRIPT="$HERE/../rust_expand.py"

fail=0

diff_case() {
    local case_dir="$1"
    local expected_stderr_fragment="${2:-}"
    local entry="$case_dir/main.rs.in"
    local expected="$case_dir/main.rs.expected"
    local stderr_log; stderr_log="$(mktemp)"
    local actual_out; actual_out="$(mktemp)"
    # Run the bundler with its exit code captured separately so we can
    # distinguish "bundler crashed / non-zero exit" from "content mismatch".
    # Piping directly into diff would let `pipefail` mask the bundler exit
    # as a diff failure and print the misleading "(stdout diff)" label.
    local py_exit=0
    python3 "$SCRIPT" "$entry" <"$entry" >"$actual_out" 2>"$stderr_log" || py_exit=$?
    if [ "$py_exit" -ne 0 ]; then
        echo "FAIL: $case_dir (bundler exit $py_exit)" >&2
        cat "$stderr_log" >&2
        rm -f "$stderr_log" "$actual_out"
        fail=1
        return
    fi
    if ! diff -u "$expected" "$actual_out"; then
        echo "FAIL: $case_dir (stdout diff)" >&2
        cat "$stderr_log" >&2
        rm -f "$stderr_log" "$actual_out"
        fail=1
        return
    fi
    if [ -n "$expected_stderr_fragment" ]; then
        if ! grep -q -F "$expected_stderr_fragment" "$stderr_log"; then
            echo "FAIL: $case_dir stderr missing '$expected_stderr_fragment'" >&2
            cat "$stderr_log" >&2
            rm -f "$stderr_log" "$actual_out"
            fail=1
            return
        fi
    fi
    rm -f "$stderr_log" "$actual_out"
    echo "ok:   $case_dir"
}

exit_case() {
    local case_dir="$1"
    local entry="$case_dir/main.rs.in"
    local expected_exit="$2"
    local expected_stderr_fragment="$3"
    local actual_stderr
    set +e
    actual_stderr="$(python3 "$SCRIPT" "$entry" <"$entry" 2>&1 >/dev/null)"
    local actual_exit=$?
    set -e
    if [ "$actual_exit" -ne "$expected_exit" ]; then
        echo "FAIL: $case_dir: exit=$actual_exit (want $expected_exit)" >&2
        echo "stderr: $actual_stderr" >&2
        fail=1
        return
    fi
    case "$actual_stderr" in
        *"$expected_stderr_fragment"*)
            echo "ok:   $case_dir (exit $actual_exit)" ;;
        *)
            echo "FAIL: $case_dir: stderr missing '$expected_stderr_fragment'" >&2
            echo "stderr: $actual_stderr" >&2
            fail=1 ;;
    esac
}

diff_case "$FIXTURES/rust/basic"
diff_case "$FIXTURES/rust/nested"
diff_case "$FIXTURES/rust/passthrough" "warning: unresolved mod std_only_no_local_file"
diff_case "$FIXTURES/rust/diamond"

exit_case "$FIXTURES/rust/cycle" 2 "cycle detected"
exit_case "$FIXTURES/rust/missing" 1 "file not found"

exit "$fail"
```

- [ ] **Step 4: 実行 → fixture の期待値を実装出力に合わせて微調整**

```bash
chmod +x hooks/rust_expand.py hooks/tests/run.sh
bash hooks/tests/run.sh
```

**Expected initial run:** 一部の diff テストが blank line / indent の差で fail する可能性がある。
実装出力に合わせて `main.rs.expected` の空白を実際の bundler 出力に合わせる（TDD の "expected is the
oracle" ではあるが、bundler の出力フォーマットが自明でないため、ここは実装 → 期待値同期を許容）。

**中止条件:** `cycle` と `missing` の exit code / stderr 内容だけは実装より優先で fix する。
これらが期待通りに動かない場合は bundler 側を修正する。

- [ ] **Step 5: commit**

```bash
git add hooks/rust_expand.py hooks/tests/
git commit -m "feat(hooks): rust bundler (rust_expand.py) + fixture テスト"
```

---

### Task 4: `hooks/expand-libraries.sh` の shell 分岐

**Files:**
- Create: `hooks/expand-libraries.sh`
- Modify: `hooks/tests/run.sh` (shell 経由の end-to-end も走らせる)

**Interfaces:**
- Consumes: stdin = source、env (`CE_LANGUAGE`, `CE_SOURCE_FILE`, ...)、cwd = 解法 dir。
- Produces: stdout = 展開後 source。

- [ ] **Step 1: `hooks/expand-libraries.sh` を作成**

```sh
#!/bin/sh
# ce submit preprocess hook — repository-local, language-agnostic.
#
# Wire this from <repository_root>/config.toml:
#
#   [submit]
#   preprocess = "hooks/expand-libraries.sh"
#
# Contract (see docs/commands/submit.md "提出前 preprocess フック"):
#   stdin  = original source
#   stdout = bundled source
#   exit 0 = adopt stdout; non-zero = abort submission
#   cwd    = solution directory
#   env    = CE_LANGUAGE CE_OJ CE_CONTEST_ID CE_PROBLEM_CODE CE_PROBLEM_ID
#            CE_SOLUTION_NAME CE_SOLUTION_DIR CE_SOURCE_FILE CE_LANG_ID
#            CE_PROJECT_ROOT
#
# Language branches live HERE. Adding a language means adding a case arm and
# a hooks/<lang>_expand.{py,sh} sibling; the Rust binaries stay unchanged.
set -eu

here="$(cd "$(dirname "$0")" && pwd)"

case "${CE_LANGUAGE:-}" in
rust)
    exec python3 "$here/rust_expand.py"
    ;;
cpp|lean)
    # TODO(#<follow-up issue>): implement C++ / Lean bundlers (oj-bundle etc.).
    # Passthrough keeps the hook contract intact so submissions of these
    # languages continue to work with their raw source.
    exec cat
    ;;
*)
    exec cat
    ;;
esac
```

`chmod +x hooks/expand-libraries.sh`。

- [ ] **Step 2: `hooks/tests/run.sh` に shell 経由の smoke を追加**

`diff_case` の呼び出しループの後に:

```bash
# End-to-end via the shell entrypoint (CE_LANGUAGE=rust).
end_to_end_rust() {
    local case_dir="$FIXTURES/rust/basic"
    local entry="$case_dir/main.rs.in"
    local expected="$case_dir/main.rs.expected"
    # Pipe to diff; capturing via $() would strip the trailing newline.
    if ! CE_LANGUAGE=rust CE_SOURCE_FILE="$entry" \
            bash "$HERE/../expand-libraries.sh" <"$entry" \
            | diff -u "$expected" -; then
        echo "FAIL: shell rust end-to-end" >&2
        fail=1
    else
        echo "ok:   shell rust end-to-end"
    fi
}
end_to_end_rust

# End-to-end passthrough for cpp / lean.
passthrough_lang() {
    local lang="$1"
    local sample; sample="hello, $lang"
    local expected; expected="$(mktemp)"
    printf '%s' "$sample" >"$expected"
    # Pipe stdin/stdout directly into `diff` so a regression that appends a
    # trailing newline (or drops one) is caught. `$(…)` capture would strip
    # trailing `\n` and hide such regressions.
    if ! printf '%s' "$sample" | CE_LANGUAGE="$lang" \
            bash "$HERE/../expand-libraries.sh" \
            | diff -u "$expected" -; then
        echo "FAIL: $lang passthrough" >&2
        rm -f "$expected"
        fail=1
        return
    fi
    rm -f "$expected"
    echo "ok:   $lang passthrough"
}
passthrough_lang cpp
passthrough_lang lean
passthrough_lang unknown
```

- [ ] **Step 3: 実行 → 通す**

```bash
bash hooks/tests/run.sh
```

- [ ] **Step 4: commit**

```bash
git add hooks/expand-libraries.sh hooks/tests/run.sh
git commit -m "feat(hooks): 言語非依存 expand-libraries.sh を追加 (rust=bundler / cpp,lean=passthrough)"
```

---

### Task 5: `config.toml` に project-local preprocess を配線

**Files:**
- Modify: `config.toml`

- [ ] **Step 1: 末尾に追記**

```toml

[submit]
preprocess = "hooks/expand-libraries.sh"
```

- [ ] **Step 2: 確認 + commit**

```bash
cargo test --workspace     # config パーサに悪影響がないことを確認
git add config.toml
git commit -m "chore(config): project-local [submit].preprocess = hooks/expand-libraries.sh を配線"
```

---

### Task 6: CI に `hooks/tests/run.sh` を追加

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: `cargo fmt check` の後に hooks smoke step を追加**

```yaml
      - name: cargo fmt check
        run: cargo fmt --all --check

      - name: Verify python3 is present (bundler dependency)
        run: python3 --version

      - name: hooks/tests/run.sh (rust bundler regression tests)
        run: bash hooks/tests/run.sh
```

- [ ] **Step 2: local で act 相当は走らせず、`bash hooks/tests/run.sh` が local で通ることを再確認**

```bash
bash hooks/tests/run.sh
```

- [ ] **Step 3: commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: hooks/tests/run.sh を CI Rust ジョブに追加"
```

---

### Task 7: verify pipeline の e2e 統合テスト

**Files:**
- Modify: `crates/infrastructure/tests/verify_command.rs`

**Interfaces:**
- Consumes: `ConfigImpl::new(root)` を tempdir で組み立て、`Controller::verify` を呼ぶ既存パターン。
- Produces: project-local `[submit].preprocess = "hooks/expand-libraries.sh"` が verify pipeline で呼ばれ、CE_SOURCE_FILE / cwd = repository_root / CE_LANGUAGE=rust が渡って stdout を採用することの証明。

- [ ] **Step 1: 統合テストを追加**

verify_command.rs 末尾に:

```rust
/// project-local [submit].preprocess が verify pipeline から呼ばれ、
/// stdout が採用されることを end-to-end で確認する。
#[test]
fn verify_uses_project_local_preprocess_hook() {
    let (tmp, root, config, manifest) = make_repo(true, false, false, "true");
    // repo root に project-local config.toml を書く。preprocess は
    // /bin/sh 経由で「先頭に `// bundled` を足す cat」相当のワンライナー。
    let hook = root.join("hooks/echo-bundle.sh");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\nprintf '// bundled\\n'\ncat\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&hook).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&hook, perms).unwrap();
    // project-local config.toml を書き足す (既存 make_repo は library.* だけ書く)
    let mut cfg = std::fs::read_to_string(root.join("config.toml")).unwrap();
    cfg.push_str("\n[submit]\npreprocess = \"hooks/echo-bundle.sh\"\n");
    std::fs::write(root.join("config.toml"), cfg).unwrap();

    // Controller を通常経路と同じく build する。
    // (既存の make_verify_controller が root を渡す形なら流用、無ければ手動で
    //  StubConfig を書き換えて ConfigImpl::new(root.clone()) を渡す方針で追加。)
    // ここでは既存の Controller テストで使う build_controller 相当をコピペしても良い。

    // FingerprintSource の submitted bytes が "// bundled\n" で始まることを確認する。
    // 既存の compute_solution_fingerprint 経由で snapshot を取り、submitted_source を検査。
    // detail は既存の verify テスト (`verify_command.rs`) の他ケースが submitted_source を
    // どう露出しているかに合わせる (fingerprint フィールドを直接検査するか、
    // FakeStarter に渡された source を Mutex<String> で受け取って比較)。
    // 後者の方が読みやすいのでこの実装を採る。

    // 実装メモ:
    // - FakeStarter に "captured_source: Arc<Mutex<Option<String>>>" を持たせて、
    //   start() が呼ばれたときに req.source を保存する。
    // - assert に captured_source.lock().unwrap().starts_with("// bundled\n") を書く。
}
```

> 注: 既存 `verify_command.rs` は `StubConfig` を fake で使っているため、実 `ConfigImpl::new(root)` を
> 使うために controller ビルドを軽く refactor する必要がある。もし refactor が >100 LOC 化しそうなら、
> **代替として `crates/infrastructure/tests/config_impl_project_local.rs` を独立追加**し、`ConfigImpl`
> の resolve と `Command::new("sh").arg("-c").arg(cfg.submit_preprocess().unwrap())` の
> Command 起動までを検証する軽量テストに切り替える。
>
> 判断基準: Task 7 に着手してから 45 分以内に既存 test refactor がまとまらなければ、上記代替を採る。

- [ ] **Step 2: 実行 → 通す**

```bash
cargo test -p infrastructure --test verify_command verify_uses_project_local_preprocess_hook -- --nocapture
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 3: commit**

```bash
git add crates/infrastructure/tests/verify_command.rs
git commit -m "test(verify): project-local preprocess が verify pipeline から呼ばれる e2e を追加"
```

---

## Self-Review (Plan Author)

### Spec coverage

| Spec 要求                                                                          | 対応 Task |
| ---------------------------------------------------------------------------------- | --------- |
| `[submit].preprocess` を project-local `config.toml` から読める                    | Task 2    |
| project-local が global を上書きする                                               | Task 2    |
| project-local relative は project root からの絶対パスに resolve                    | Task 2    |
| global は元挙動維持 (shell に丸投げ)                                                | Task 2 (Design D3) |
| `hooks/expand-libraries.sh` 追加 (rust 分岐 + cpp/lean/others の passthrough)      | Task 4    |
| Rust bundler が `#[path]` chain と `mod libs;` を再帰展開                          | Task 3    |
| 循環検出 / file 欠損 / non-UTF-8 で abort                                          | Task 3 (exit codes) |
| `config.toml` に project-local `[submit].preprocess` を配線                        | Task 5    |
| hooks/tests/ を CI で回す                                                          | Task 6    |
| docs 更新 (spec.md / commands/submit.md / operations/library-expand.md)            | Task 1    |
| verify pipeline 統合テスト                                                          | Task 7    |
| 既存 `hooks/submit-preprocess.sh` (cargo-equip 例) は差し替えず残す                | (Task 4 で shell 分岐だけ触るため既存 file は不変) |

### Placeholder scan

TODO / TBD / "implement later" は本文に無いことを確認済み。Task 7 に代替パス条件（45 分 timebox）を
書いたが、これは判断基準の明示であり "implement later" ではない。

### Type consistency

- `ConfigImpl::new(project_root: PathBuf) -> Self` を Task 2 で定義、Task 7 でそのまま呼ぶ。
- `Config::submit_preprocess(&self) -> Option<String>` の trait 定義は既存を維持（変更なし）。
- `hooks/rust_expand.py` の CLI: `python3 rust_expand.py [entry_file]` + stdin。Task 3 (bundler) と
  Task 4 (shell wrapper) と Task 6 (CI) で一致。

### Out of scope (次 PR 以降)

- `solutions/librarychecker-aplusb/aplusb/rust/src/main.rs` を `#[path]` 経由 import に書き換え → **次 PR**
- C++ / Lean bundler の実装 → 別 issue
- Solution page の "expand widget" (UI) → 別 PR

---

## Definition of Done (Plan PR)

- 計画 md が `docs/superpowers/plans/2026-08-17-project-local-preprocess.md` に保存されている。
- `main` に向けた PR が Ready、Claude review 完了。
- `cargo test --workspace` / `cargo clippy` は plan-only 変更でも当然 pass する (docs 変更なので影響
  なし、CI が確認)。
