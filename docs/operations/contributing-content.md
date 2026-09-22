# コンテンツ追加手順 (ライブラリ / verify 解法)

このリポジトリに **ライブラリ** または **verify 用の解法** を追加する人向けの手順書。

- `ce` ツール本体の開発フロー (`/spec-*`、TDD、`tasks/`) は `CLAUDE.md` を参照。
- 規約の根拠は `docs/superpowers/specs/2026-08-10-library-platform-design.md` (以下「設計文書」)。
  このファイルは手順だけを書き、理由は既存ドキュメントへリンクする。
- verify 自動化と Pages の運用者向け情報は `docs/operations/verify-automation.md` /
  `docs/operations/pages.md`。

**手動でやることはほとんどない。** ファイルを正しい場所に置いて PR を出せば、unit test 実行 (CI)、
候補選択 (5 分 cron)、OJ 提出、サイト再ビルドまで自動で進む。以下の手順はその「置き方」が本体。

## 1. ブランチと PR

1. `main` へ直接コミットしない。ブランチ名は `<type>/<短い要約>`。実運用の type は
   `feat/` `fix/` `refactor/` `docs/` `chore/` (`git log --oneline --merges` 参照)。
   merge するとリモートブランチは自動削除される (`delete_branch_on_merge`)。
2. コミットメッセージは `type(scope): 日本語の要約`。PR タイトル・本文も日本語。
3. 1 PR = 1 論点。ライブラリ追加と解法追加は分けてよいが、
   **ライブラリの move / rename は参照更新と同じ PR に入れる** (設計文書 §4.1)。
4. CI が緑になってからマージする。コンテンツ PR に出る check は `.github/workflows/ci.yml` の 3 job:
   - `CI / Cargo test + clippy + fmt` — `cargo test --all` / `clippy -D warnings` / `fmt --check` /
     `hooks/tests/run.sh` / `ce check` (3 言語のライブラリ検査)
   - `CI / Static site build (root + /compro-env/)` — schema 検証、link チェック、CSP、
     `/` と `/compro-env/` 両 base の build、サイズ summary。
     **入力は fixture (`web/tests/fixtures/site-data.json`) で、リポジトリの実ライブラリではない**
     (`web/scripts/site-build.mjs` の `--fixture` 既定値)。renderer の入力境界を固定するための job なので、
     実データには切り替えない。
   - `CI / Real-content site-data build` — **リポジトリの実ライブラリ / 実解法**に対して
     `ce site-data generate --mode production` を走らせ、生成された site-data で
     `npm run site:build` する。merge 後の `pages.yml` と同じ生成・build の組。
     何を検出するかは 2.5 を参照。

   `push` と `pull_request` の両トリガで起動するため、PR 画面には同名の check が 2 件ずつ (計 6 件) 並ぶ。
   `verify-result-integrity` は head が `automation/verify` の PR 限定なので、コンテンツ PR には出ない。

## 2. ライブラリを 1 本追加する

### 2.1 ソースを置く

置き場所は `config.toml` の `[library.languages.<lang>]` の `root` + `include` で決まる:

| 言語 | root             | include                      |
| ---- | ---------------- | ---------------------------- |
| rust | `libraries/rust` | `**/*.rs`                    |
| cpp  | `libraries/cpp`  | `**/*.hpp`, `**/*.cpp`       |
| lean | `libraries/lean` | `**/*.lean`                  |

```
libraries/rust/algebra/monoid.rs
```

- **ライブラリ ID = リポジトリ相対パス** (設計文書 §2 / §4.1)。
  後から move / rename すると別 ID になり、旧 URL は 404 になる。redirect は生成しない。
- 1 ファイル = 1 ページ。単独でコンパイルできる必要はなく、1 ファイルに複数宣言があってもよい。
- 生 source が 256 KiB を超えると build warning、2 MiB を超えると build error
  (設計文書 §12.11)。`pages.yml` も `CI / Real-content site-data build` も
  `--mode production` で生成するので、hard limit は PR CI の時点で発火する。

### 2.2 単体テストを同じファイルに書く (rust)

`#[cfg(test)] mod tests` を **ライブラリファイル自身に** 書く。別ファイル・別クレートは不要。

```rust
#[cfg(test)]
mod tests {
    // `use super::…` は mod 直下ではなく各 fn の中に置く。
    // module-level `use` があると rust adapter がこのファイルを `partial` と判定する。
    #[test]
    fn add_monoid_identity() {
        use super::{AddMonoid, Monoid};
        assert_eq!(AddMonoid::op(&AddMonoid::id(), &7), 7);
    }
}
```

実例: `libraries/rust/algebra/monoid.rs`。

### 2.3 (任意) 説明の sidecar を置く

ソースファイル名全体に `.md` を付ける (`monoid.rs` → `monoid.rs.md`)。実例: `libraries/rust/algebra/monoid.rs.md`。

frontmatter は `+++` で囲む TOML。許可キーは次の 4 つだけ (設計文書 §5.1)。未知キーは build error。

```toml
+++
title = "Monoid (Rust)"
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

- `title` 省略時は **拡張子込みの basename** (`monoid.rs`)。空文字は build error。
- `publish` 既定 `true`。`false` にすると Web ページを作らない (解析対象には残る)。
- frontmatter なしも可。sidecar 自体なしでもソースページは生成される。
- 対応するソースがない orphan sidecar は build error。
- **Markdown 本文はライブラリページの Documentation セクションになる。** 本文を書けばそのまま
  公開ページに出る。
- **Markdown 本文に `h1` を書いてはいけない。** 見出しは `h2` から始め、level を 2 以上飛ばさない
  (page title が唯一の `h1`。設計文書 §12)。破ると renderer が
  `h1_disallowed_in_markdown` / `heading_level_jump` で build を落とす (2.5)。
- `[[relations]]` はライブラリページの Relations セクションに出る。target は管理対象ライブラリで
  なければならず、自己 relation と同じ `kind` / target の重複は build error (設計文書 §5.1)。
  **公開ページに出るのは公開ライブラリ宛ての relation だけ** で、非公開 target は黙って落ちる。
- `[[dependency_overrides]]` は `action = "add"` だけが適用され、Dependencies に `manual` 付きで
  並ぶ。`remove` / `resolve` / `external` は projection が未対応なので **書くと build error**
  (黙って無視されるより落とす方を選んでいる)。`add` の target は同じ言語の管理対象ライブラリに限る。
- ディレクトリ / 言語 root の説明は `_index.md` に置き、`title` だけを書ける。
  **`_index.md` の本文は site-data のスキーマに載る先が無く、まだどこにも描画されない。**

### 2.4 ローカルで確認する (任意)

CI が同じことをやるので必須ではない。手元で先に見たいとき:

```bash
bash scripts/check-rust-libraries.sh
bash scripts/check-cpp-libraries.sh
bash scripts/check-lean-libraries.sh
```

```bash
cargo run --bin ce -- check
```

3 スクリプトはいずれも言語 root を再帰列挙し、1 ファイルずつ処理する。
**ファイルを置くだけで検査対象になる。** 処理内容と規約は `docs/commands/check.md`
「言語別の check 内容」。必要な toolchain は rust = `rustc`、cpp = PATH 上の `clang++`、
lean = PATH 上の `lean` で、未 install の言語だけ `--language` で外す。

### 2.5 push する

CI が自動でやること:

- `CI / Cargo test + clippy + fmt` の `ce check` → 3 言語すべてのライブラリを検査
  (rust は 2.2 の unit test を実行、cpp は syntax check、lean は elaboration)。
  cpp の `clang++` は runner image 同梱、lean は pin 済み toolchain を install する step がある。
- `CI / Real-content site-data build` → 実リポジトリに対して
  `ce site-data generate --mode production` → `npm run site:build` を走らせる。
  merge 後の `pages.yml` と同じ組なので、**ここが緑なら pages build も通る**。

`Real-content site-data build` が落とすもの:

| 不正 | 落ちる段階 | エラーメッセージの例 |
| --- | --- | --- |
| frontmatter の未知キー | generate (discovery) | ``unknown field `author`, expected one of `title`, `publish`, `relations`, `dependency_overrides` `` |
| frontmatter の malformed TOML | generate (discovery) | `malformed frontmatter in ...: TOML parse error at line 1, column 16` |
| 空の `title` | generate (discovery) | ``` `title` must not be empty (omit the key to inherit the default) ``` |
| orphan sidecar | generate (discovery) | ``discovery rejected 1 problem(s): [orphan_sidecar] sidecar ... has no corresponding source file`` |
| `[verify].libraries` が非公開 / 不在のライブラリを指す | generate (discovery) | ``solution ... verifies library `...` which is not a public discovered library`` |
| `[[relations]]` が不在ライブラリ / 自分自身を指す、同じ `kind` / target の重複 | generate (入力収集) | ``libraries/rust/algebra/monoid.rs.md: relation `port` points at `...`, which is not a managed library (spec §5.1)`` |
| `[[dependency_overrides]]` が `remove` / `resolve` / `external`、または別言語を `add` | generate (入力収集) | ``... dependency override `action = "remove"` is not supported by site-data generation yet; only `add` is applied`` |
| 新規ライブラリが未コミット | generate (projection) | `no git history recorded for published library ...` |
| 生 source が 2 MiB 超 | site:build (renderer) | `SourceRenderError: Source "algebra/huge.rs" is 2101382 bytes; the hard limit is 2097152 bytes.` |
| sidecar 本文の `h1` / 見出し level 飛ばし | site:build (renderer) | `MarkdownRenderError: Documentation must not include a level-1 heading; the page owns the <h1>.` |

CI が **まだ検出しないもの**: 現時点では既知の穴は無い。

### 2.6 cpp / lean の注意

`ce check` はどの言語でもファイルを置くだけで検査する (2.4) が、1 ファイルを単独で処理するため
言語ごとに前提がある。

**cpp**: ヘッダは単独の translation unit としてコンパイルされるので、**self-contained** でなければ
ならない (使う `#include` を自分で書く)。`-Wall -Wextra -Werror` なので警告も失敗になる。
他の cpp ライブラリを参照するときは `libraries/cpp` 相対で書く (`#include "algebra/monoid.hpp"`)。
`clang++` は PATH 上のものを使うため、ローカルと CI でバージョンが異なりうる。
新しい警告でローカルだけ落ちることがある。

**lean**: 各ファイルは toolchain 同梱のモジュールだけを `import` できる。`libraries/lean` に
lakefile が無く `.olean` を作らないので、兄弟ライブラリを `import` すると unknown module で失敗する。
cross-file import が必要になった時点で lakefile の導入から考える。
`-DwarningAsError=true` を付けているので、`sorry` の残った証明は error になる。

## 3. verify 用の解法を 1 本追加する

### 3.1 コンテストを初期化する

Library Checker は「1 問 = 単問コンテスト」として扱い、`contest_id` は `librarychecker-` を冠する
(`docs/online_judges/librarychecker.md`)。

```bash
cargo run --bin ce -- init https://judge.yosupo.jp/problem/aplusb --lang rust
```

`solutions/librarychecker-aplusb/` に `.ce.toml`、`testcases/<problem_code>/`、
`<problem_code>/main/` (テンプレート展開) ができる。

**コミットするのは解法ディレクトリだけ。** `.ce.toml` と `testcases/` は commit しない
(公開対象の選択は解法直下の `ce.toml` の `publish` だけで決まり、コンテストの `.ce.toml` は
影響しない)。ただし `.ce.toml` が存在する場合は discovery が schema 検証のためにパースし、
壊れていれば discovery 全体が失敗する
(`crates/infrastructure/src/library_project/discovery.rs` の `parse_contest_ce_toml`)。

### 3.2 解法ディレクトリを追加する

`main` 以外の名前や別言語が必要なときだけ:

```
ce solution add [OPTIONS] <CONTEST> <PROBLEM> [SOLUTION]
  [SOLUTION]     Solution name (default: main)
  --lang <LANG>
```

```bash
cargo run --bin ce -- solution add librarychecker-aplusb aplusb rust --lang rust
```

詳細: `docs/commands/solution.md`。コンテストが未初期化だと
`contest '<contest_id>' is not initialized.` で止まる (3.1 を先に実行する)。

### 3.3 `ce.toml` を書く

```toml
language = "rust"
test_command = "cargo run --quiet --release"
publish = true
solved_at = "2026-08-12T00:00:00+00:00"
test_timeout_seconds = 600

[verify]
libraries = [
  "libraries/rust/algebra/monoid.rs",
]
language_id = "rust"
```

実在の解法: `solutions/librarychecker-aplusb/aplusb/rust/ce.toml`。

必須:

- `publish = true` — 既定は非公開。公開しない解法は verify 対象にならない。
- `solved_at` — **timezone 付き RFC 3339**。filesystem / Git / OJ 日時への暗黙 fallback はない。
- `test_command` — 実行内容は任意だが、キー自体は verify 解法では必須
  (check + test barrier で実行される)。
  **CWD は解法ディレクトリ** なので、パスは解法ディレクトリ相対で書く
  (`ce test`: `docs/commands/test.md` 手順 3 / `ce verify`:
  `crates/usecases/src/service/verify.rs` の `repository_root.join(&published.root)`)。
- `[verify].libraries` — この解法が保証するライブラリ ID (リポジトリ相対パス) を列挙する。
  推移依存は自動で closure に入るので、直接保証するものだけ書く。

任意:

- `[verify].language_id` — OJ 側の提出言語 ID を上書きしたいときだけ書く。`[verify]` 外には置かない。
- `test_timeout_seconds` — 既定 600 秒。正の整数のみ。

config error になるケース (設計文書 §7.2):

- 非公開解法 (`publish` が true でない) に `[verify]` がある
- `[verify].libraries` が非公開ライブラリを直接指している

### 3.4 ライブラリを取り込む

`src/libs.rs` に `#[path]` を集約し、`main.rs` からは `mod libs;` で使う。

```rust
// src/libs.rs
#[path = "../../../../../libraries/rust/algebra/monoid.rs"]
pub mod monoid;
```

```rust
// src/main.rs
mod libs;
use libs::monoid::{AddMonoid, Monoid};
```

- `#[path]` は文字列リテラルしか受け付けず `env!()` / `concat!()` を展開できないため、
  長い相対パスを 1 ファイルに閉じ込めている。
- **相対パスの基準は「その `#[path]` を書いたファイルのディレクトリ」。** `src/libs.rs` なら `src/` から。
  rustc も `hooks/rust_expand.py` も同じ規則で解決する (bundler は再帰時に inline 先ファイルの
  親ディレクトリを基準にする)。
- この書き方をすると rust adapter が依存を検出し、solution ページの "Depends on" にライブラリが並ぶ。

実例: `solutions/librarychecker-aplusb/aplusb/rust/src/libs.rs` と `src/main.rs`。

### 3.5 提出時の展開は自動

`config.toml` の `[submit].preprocess = "hooks/expand-libraries.sh"` が配線済み。
提出直前に `hooks/rust_expand.py` が `mod` 宣言を再帰 inline して単一ファイルにする。
手で bundle する必要はない。設計と制限: `docs/operations/library-expand.md`。

cpp / lean の bundler は未実装 (passthrough) なので、これらの言語では単一ファイルで完結させる。

### 3.6 ローカルで確認する (任意)

`ce.toml` の `test_command` をそのまま実行する:

```bash
cargo run --bin ce -- test librarychecker-aplusb aplusb rust
```

`ce verify` は実際に OJ へ提出するので、ローカルでは通常実行しない (`docs/commands/verify.md`)。

## 4. マージした後に起きること

**基本的に何もしなくてよい。**

1. マージの `main` push で dispatcher が起動し、変更分類が `source-or-config` なら
   `ce internal pick-candidate` が候補を 1 件選ぶ。5 分ごとの cron でも同じ picker が回る
   (`.github/workflows/verify.yml`)。新規解法は「record が存在しない」ので対象になり、
   ライブラリだけを変えた場合も依存する解法の fingerprint がずれるので対象になる。
   picker は in-flight record を持つ解法も返すので、前回の tick が途中で終わっていれば
   worker が resume する (`docs/operations/verify-automation.md` の
   "Resume: how a stuck attempt gets unstuck")。
2. repository variable `VERIFY_LIVE` が `true` なら、そのまま OJ 提出まで自動で進む。
   未設定 (既定) の場合、tick は走るが OJ に触れず **record も書かない** ので、
   検証を進めたいときだけ 1 回手で叩く:

   ```bash
   gh workflow run verify.yml -f mode=live -f solution=librarychecker-aplusb/aplusb/rust
   ```

   `VERIFY_LIVE` は一度立てれば以降の tick すべてに効く (立て方は
   `docs/operations/verify-automation.md` の G2 手順 8)。
3. terminal record が `automation/verify` に入ると、`pages.yml` が `workflow_run` トリガで
   自動的に再ビルド・再デプロイする。**`gh workflow run pages.yml` を手で叩く必要はない。**
4. "Automation: verification results" PR が自動で作られ、terminal verdict で ready + auto-merge
   になる。merge されると record が `main` に入り、`automation/verify` は自動削除される。
   次の tick が必要になった時点で main の tip から作り直される。
5. 確認先:
   - `main` の `verification/results/<contest_id>/<problem_code>/<solution_name>.json`
     (merge 済みの正本。`automation/verify` は in-flight 中だけ存在する作業ブランチ)
   - 自動生成される "Automation: verification results" PR
   - 公開サイトの該当ライブラリ / 解法ページ

`VERIFY_ACTIVATED` が `true` でないとパイプライン全体が動かない
(`docs/operations/verify-automation.md` human gate G2)。

## 5. つまずきやすい点

- **`#[path]` の相対パス基準を間違える。** 基準は属性を書いたファイルのディレクトリ。
  `src/libs.rs` に集約しているのは、この基準を 1 箇所に固定して `main.rs` を短く保つため。
- **ライブラリを変えると依存解法が Stale になる。** fingerprint 入力に closure のソースが入るため。
  ページには `Source or dependencies changed since the last submission.` が出る。
  live 提出が有効なら次以降の tick で picker が再検証を拾うので待てば消える。dry-run のままだと消えない。
- **fingerprint は preprocess 前の生ソースから計算される** (PR #120)。
  preprocess 後のバイト列は `submitted_source_hash` として別に記録される。
  source を書き換える preprocess hook を足しても fingerprint はずれない。
- **`Unavailable` は永久 dead-letter。** fingerprint drift では復活しないので、
  運用者が record を消す必要がある (`docs/operations/verify-automation.md`)。
  record は merge 済みなら `main`、in-flight 中なら `automation/verify` にある。
- **cpp のヘッダは self-contained でないと `ce check` が落ちる。lean は兄弟ライブラリを `import` できない**
  (2.6 参照)。
- **ローカルのサイトプレビューは必須ではなくなった。** frontmatter と config error は
  `CI / Real-content site-data build` が同じコマンドで検査する (2.5)。
  手元で先に潰したいときや、実際の描画を目で見たいときだけ踏む。analyzer バイナリが必要:

  ```bash
  ./tools/library-analyzers/prepare && ./tools/library-analyzers/build
  cargo run --bin ce -- site-data generate --mode preview
  npm run site:build -- --fixture=target/ce-site-data/site-data.json
  ```

  cold run で LLVM (~700MB) と Lean (~500MB) を落とす。CI 側は
  `pages.yml` / `verify.yml` と共有の analyzer cache に当たるのでこの download は通常発生しない。
  ローカルは作業ツリーが汚れている前提なので `--mode preview` を使う。CI / pages は
  `--mode production` なので、この手順では **サイズ上限だけは落ちない** (2.1)。
  サイズ上限も手元で見たいときは、ツリーを commit 済みにして `--mode production` を渡す。

## 関連

- `CLAUDE.md` — `ce` 本体の開発フロー
- `docs/superpowers/specs/2026-08-10-library-platform-design.md` — コンテンツ規約の正本
- `docs/operations/verify-automation.md` — verify パイプラインの運用
- `docs/operations/pages.md` — Pages デプロイの運用
- `docs/operations/library-expand.md` — submit preprocess bundler
- `docs/commands/check.md` / `docs/commands/solution.md` / `docs/commands/verify.md`
