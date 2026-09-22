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
2. コミットメッセージは `type(scope): 日本語の要約`。PR タイトル・本文も日本語。
3. 1 PR = 1 論点。ライブラリ追加と解法追加は分けてよいが、
   **ライブラリの move / rename は参照更新と同じ PR に入れる** (設計文書 §4.1)。
4. CI が緑になってからマージする。コンテンツ PR に出る check は `.github/workflows/ci.yml` の 2 job:
   - `CI / Cargo test + clippy + fmt` — `cargo test --all` / `clippy -D warnings` / `fmt --check` /
     `hooks/tests/run.sh` / `ce check --language rust`
   - `CI / Static site build (root + /compro-env/)` — schema 検証、link チェック、CSP、
     `/` と `/compro-env/` 両 base の build、サイズ summary。
     **入力は fixture (`web/tests/fixtures/site-data.json`) で、リポジトリの実ライブラリではない**
     (`web/scripts/site-build.mjs` の `--fixture` 既定値)。

   `push` と `pull_request` の両トリガで起動するため、PR 画面には同名の check が 2 件ずつ (計 4 件) 並ぶ。
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
- 生 source が 256 KiB を超えると build warning、2 MiB を超えると production build error (設計文書 §12.11)。

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
- **Markdown 本文に `h1` を書くと build error。** 見出しは `h2` から始め、level を 2 以上飛ばさない
  (page title が唯一の `h1`。設計文書 §12)。
- ディレクトリ / 言語 root の説明は `_index.md` に置き、`title` だけを書ける。

### 2.4 ローカルで確認する (任意)

CI が同じことをやるので必須ではない。手元で先に見たいとき:

```bash
bash scripts/check-rust-libraries.sh
```

```bash
cargo run --bin ce -- check --language rust
```

`scripts/check-rust-libraries.sh` は `libraries/rust/**/*.rs` を 1 ファイルずつ
`rustc --edition 2024 --test` でコンパイル・実行する。**ファイルを置くだけでテストが走る。**

### 2.5 push する

CI が自動でやること:

- `CI / Cargo test + clippy + fmt` の `ce check --language rust` → 2.2 の unit test を実行

CI が **やらないこと**: `CI / Static site build` は fixture に対して web パイプラインを検証するだけで、
追加したライブラリの frontmatter・`h1` 禁止・サイズ境界は検査しない。
これらが実データで検査されるのは merge 後の `pages.yml` (`ce site-data generate` →
`npm run site:build --fixture=target/ce-site-data/site-data.json`) なので、
frontmatter を壊すと **merge 後に pages build が失敗する**。
事前に確認したい場合は 5 章「つまずきやすい点」のローカルプレビュー手順を踏む。

### 2.6 cpp / lean の注意

**cpp / lean は現状 `check_command` がファイルをハードコードしており、新規ライブラリが検査されない。**
cpp は `libraries/cpp/algebra/monoid.hpp` 単体指定、lean は `lake build`。追加したファイルは
`ce check` で検査されないので、当面は手元でコンパイル・テストを確認する。
追跡: [issue #122](https://github.com/kisepichu/compro-env/issues/122)。

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

実在の解法: `solutions/librarychecker-aplusb/aplusb/rust/ce.toml` (ただし `test_command` は
3.6 の注意を参照)。

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

> 注意: 現在の `solutions/librarychecker-aplusb/aplusb/rust/ce.toml` の `test_command` は
> リポジトリルート相対の `--manifest-path` を持つため、上記の CWD 規則に反しており
> `ce test` / `ce verify` のどちらからも `manifest path ... does not exist` で失敗する。
> 新しい解法ではこれを真似しないこと。

## 4. マージした後に起きること

**基本的に何もしなくてよい。**

1. マージの `main` push で dispatcher が起動し、変更分類が `source-or-config` なら
   `ce internal pick-candidate` が候補を 1 件選ぶ。5 分ごとの cron でも同じ picker が回る
   (`.github/workflows/verify.yml`)。新規解法は「record が存在しない」ので対象になり、
   ライブラリだけを変えた場合も依存する解法の fingerprint がずれるので対象になる。
2. repository variable `VERIFY_LIVE` が `true` なら、そのまま OJ 提出まで自動で進む。
   未設定 (既定) の場合は dry-run で `persist_starting` まで進んで止まるので、
   そのときだけ 1 回手で叩く:

   ```bash
   gh workflow run verify.yml -f mode=live -f solution=librarychecker-aplusb/aplusb/rust
   ```

   `VERIFY_LIVE` は一度立てれば以降の tick すべてに効く (立て方は
   `docs/operations/verify-automation.md` の G2 手順 8)。
3. terminal record が `automation/verify` に入ると、`pages.yml` が `workflow_run` トリガで
   自動的に再ビルド・再デプロイする。**`gh workflow run pages.yml` を手で叩く必要はない。**
4. 確認先:
   - `automation/verify` ブランチの
     `verification/results/<contest_id>/<problem_code>/<solution_name>.json`
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
  運用者が overlay record を消す必要がある (`docs/operations/verify-automation.md`)。
- **cpp / lean のライブラリは `ce check` で検査されない** ([issue #122](https://github.com/kisepichu/compro-env/issues/122))。
- **ローカルでサイトをプレビューする**のは、frontmatter や `h1` の違反を merge 前に検出する唯一の方法
  (CI は fixture しか見ない。2.5 参照)。analyzer バイナリが必要:

  ```bash
  ./tools/library-analyzers/prepare && ./tools/library-analyzers/build
  cargo run --bin ce -- site-data generate --mode preview
  npm run site:build -- --fixture=target/ce-site-data/site-data.json
  ```

  cold run で LLVM (~700MB) と Lean (~500MB) を落とす。sidecar を足さない・単純な追加だけなら
  省いてよい。

## 関連

- `CLAUDE.md` — `ce` 本体の開発フロー
- `docs/superpowers/specs/2026-08-10-library-platform-design.md` — コンテンツ規約の正本
- `docs/operations/verify-automation.md` — verify パイプラインの運用
- `docs/operations/pages.md` — Pages デプロイの運用
- `docs/operations/library-expand.md` — submit preprocess bundler
- `docs/commands/check.md` / `docs/commands/solution.md` / `docs/commands/verify.md`
