# 仕様書 (WIP)

壁打ちしながら埋める。未決 Q は末尾に。

---

## ディレクトリ構造

```
compro-env/                         ← リポジトリルート
  config.toml                       ← プロジェクトローカル設定 (optional, global を上書き)
  templates/                        ← ce init で使うテンプレート
    rust/
      src/main.rs
      Cargo.toml
    cpp/
      main.cpp
  solutions/
    {contest_id}/                   例: abc334, aoj0000, cf1234a
      testcases/
        {problem_code}/             例: a, b, ex, practice_2 (1文字固定でない)
          1.in
          1.out
      {lang}/                       例: rust, cpp
        Cargo.toml                  ← Rust の場合: contest レベルの workspace
        Cargo.lock
        {problem_code}/
          {solution_name}/          デフォルト: main
            src/main.rs             ← Rust
            Cargo.toml              ← package name = "{problem_code}"
```

- `testcases/` は言語共通。`ce test a --lang cpp` も同じ `testcases/a/` を参照
- テンプレートは `templates/{lang}/` に置き、`ce init` 時にコピー + 文字列置換 (Cargo.toml の `name` 等)。Tera は将来拡張で不要

---

## Cargo 構成 (Rust)

コンテストレベルで Cargo workspace を作る。

```
solutions/abc334/rust/
  Cargo.toml    ← [workspace] members = ["a/main", "a/sol2", "b/main", ...]
  Cargo.lock
  a/main/
    Cargo.toml  ← [package] name = "a"
    src/main.rs
```

- `ce test a` → `cargo test -p a`
- `ce run a` → `cargo run -p a`
- `ce sub a` → `a/main/src/main.rs` を提出
- `SolutionRepository::create()` が `members` を自動更新 (後述)

---

## コンフィグ設計

### グローバル: `~/.config/ce/config.toml`

```toml
[default]
online_judge = "atcoder"

[language.rust]
solution_file = "src/main.rs"
test = "cargo test -p {problem}"
run = "cargo run -p {problem}"
submit_file = "src/main.rs"
submit_preprocess = ""

[language.cpp]
solution_file = "main.cpp"
test = "g++ {file} -o /tmp/ce_bin && echo '{input}' | /tmp/ce_bin"
submit_file = "main.cpp"
```

### プロジェクトローカル: `compro-env/config.toml`

グローバルの同キーを上書き。存在しなくてもよい。

### セッション: `~/.config/ce/session.toml` (グローバル固定)

```toml
[atcoder]
revel_session = "xxxxxxxx"
```

---

## コマンド一覧 (MVP)

### `ce login [oj]`
- `oj` 省略時はデフォルト OJ
- ブラウザ DevTools での `REVEL_SESSION` 取得手順を表示
- stdin でクッキー値を受け取り `~/.config/ce/session.toml` に保存

### `ce init <contest_id_or_url>`
- OJ 判定 (D + C 方式、後述)
- 問題一覧・サンプルを取得
- `testcases/{problem_code}/` にサンプルを保存
- デフォルト言語で全問題に `{lang}/{problem_code}/main/` をテンプレートから生成
- Rust なら contest レベルの workspace `Cargo.toml` を生成
- コンテスト未開始なら開始を待つ

### `ce new <contest_id> <problem_code> [solution_name] [--lang <lang>]`
- 解法フォルダを追加 (`SolutionRepository::create()`)
- `solution_name` 省略時は `main`、`--lang` 省略時はデフォルト言語
- Rust なら workspace `Cargo.toml` の `members` を更新

### `ce test <contest_id> <problem_code> [solution_name] [--lang <lang>]`
- コンフィグのテストコマンドを実行
- `testcases/{problem_code}/` の全サンプルで実行・比較

### `ce sub <contest_id> <problem_code> [solution_name] [--lang <lang>]`
- 提出前処理コマンドを実行
- OJ へ提出

### (将来) リアルタイムコンテストモード
- カレントディレクトリが `solutions/{contest_id}/` 以下なら `contest_id` を自動検出 (A 方式)
- `ce sub a` などの短コマンドが動く

---

## OJ 判定ロジック

```
入力: "abc334"       → プレフィックス "abc"/"arc"/"ahc" → AtCoder
入力: "aoj0000"      → プレフィックス "aoj" → AOJ
入力: "https://atcoder.jp/contests/abc334" → URL パース → AtCoder, id = "abc334"
入力: "xyz999"       → 判定不能 → stdin: "OJ を選んでください [atcoder/...]: "
```

---

## ドメインモデル

```
Contest                             ← Aggregate Root
  id: String                        例: "abc334"
  online_judge: OJKind
  problems: Vec<Problem>

Problem                             ← Entity (Contest 配下)
  id: String                        例: "abc334_a"
  code: String                      例: "a", "ex", "practice_2"
  title: String
  samples: Vec<Sample>

Sample                              ← Value Object
  input: String
  output: String

Solution                            ← Entity (独立 Aggregate)
  contest_id: String
  problem_code: String
  name: String                      例: "main", "sol2"
  language: Language
  path: PathBuf

Session                             ← Value Object
  online_judge: OJKind
  cookie: String
```

`IOSpec` は MVP スコープ外 (将来の入力自動生成機能で追加)。

---

## Repository インターフェース (usecases 層)

```rust
trait ContestRepository {
    fn exists(&self, contest_id: &str) -> Result<bool>;
    fn exists_unstarted(&self, contest_id: &str) -> Result<bool>;
    fn create_unstarted(&self, contest_id: &str) -> Result<()>;
    fn create(&self, contest: &Contest) -> Result<()>;   // テストケース保存含む
    fn get(&self, contest_id: &str) -> Result<Contest>;
}

trait SolutionRepository {
    fn list(&self, contest_id: &str, problem_code: &str) -> Result<Vec<Solution>>;
    fn exists(&self, contest_id: &str, problem_code: &str, name: &str, lang: &Language) -> Result<bool>;
    fn create(&self, solution: &Solution) -> Result<()>;
    // Rust の場合は Cargo.toml members の更新も担う (infrastructure 側で実装)
    fn get_source(&self, solution: &Solution) -> Result<String>;  // submit 用
}
```

**ソースオブトゥルース:** ファイルシステムのディレクトリ構造。
`SolutionRepository::list()` はディレクトリをスキャンして `Solution` を返す。
別途マニフェストファイルは持たない。

---

## アーキテクチャ層構成

giming の 4 層を引き継ぎ、エラー設計を改善。

```
domain/
  entity.rs        Contest, Problem, Sample, Solution, Session, Language, OJKind
  error.rs         (シンプルな trait のみ)

usecases/
  repository/
    contest_repository.rs
    solution_repository.rs
  online_judge.rs  OnlineJudge trait (port)
  config.rs        Config trait (port)
  service/
    login.rs
    init.rs
    new_solution.rs
    test.rs
    submit.rs

interfaces/
  controller/
    input.rs       各コマンドの Input trait

infrastructure/
  repository_impl/
    contest_repository_impl.rs
    solution_repository_impl.rs   ← Cargo.toml members 更新もここ
  online_judge_impl/
    atcoder/
      login.rs, get_problems.rs, submit.rs, ...
  config_impl.rs
  shell/           ← clap + エントリポイント
```

**エラー設計:** giming の `E: Error + 'static` 型パラメータを廃止し `anyhow::Error` に統一。
ドメインエラーは enum で定義し `thiserror` を使用。

---

## 未決 Q リスト

### Q11. `Solution` の `path` フィールド

`Solution.path` は絶対パスか相対パスか?
- 相対パス (プロジェクトルートからの相対) の方が移植性が高い
- でも `SolutionRepository` がルートパスを知っている必要がある

### Q12. `ce test` の出力形式

テスト失敗時にどの程度の情報を出すか?
- `diff` 形式で expected/actual を表示?
- AC/WA/TLE のカラー表示?
- giming の ac ツールには `colors.py` があったが、どの程度 giming の出力に寄せる?
