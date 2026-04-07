# 仕様書 (WIP)

壁打ちしながら埋める。未決 Q は末尾に。

---

## ディレクトリ構造

```
compro-env/                         ← リポジトリルート
  config.toml                       ← プロジェクトローカル設定 (optional, global を上書き)
  templates/
    rust/
      src/main.rs
      Cargo.toml                    ← {problem_code} プレースホルダあり
    cpp/
      main.cpp
  solutions/
    {contest_id}/
      .ce.toml                      ← OJ 情報を保存 (ce init 時に生成)
      testcases/
        {problem_code}/             1文字固定でない (ex, practice_2 等あり)
          1.in  1.out  2.in  2.out
      {lang}/                       rust / cpp / ...
        Cargo.toml                  ← Rust: contest レベル workspace
        Cargo.lock
        {problem_code}/
          {solution_name}/          デフォルト: main
            Cargo.toml              ← name = "{problem_code}-{solution_name}"  ★注意
            src/main.rs
```

### .ce.toml の内容

```toml
online_judge = "atcoder"
contest_id = "abc334"
```

`ce test` / `ce sub` 時に OJ を特定するために必須。プレフィックス判定だけでは `xyz999` 等に対応不可。

### Cargo package name 規則

同一 workspace 内で package name が衝突するため、常に `{problem_code}-{solution_name}` を使う。

| ディレクトリ | package name |
| ------------ | ------------ |
| `a/main/`    | `a-main`     |
| `a/sol2/`    | `a-sol2`     |
| `ex/main/`   | `ex-main`    |

`ce test a` → 内部で `cargo test -p a-main`

---

## Cargo 構成 (Rust)

```
solutions/abc334/rust/
  Cargo.toml    ← [workspace] members = ["a/main", "a/sol2", "b/main", ...]
  Cargo.lock
  a/
    main/
      Cargo.toml  ← [package] name = "a-main"
      src/main.rs
    sol2/
      Cargo.toml  ← [package] name = "a-sol2"
      src/main.rs
  b/
    main/
      Cargo.toml  ← [package] name = "b-main"
      src/main.rs
```

---

## コンフィグ設計

### グローバル: `~/.config/ce/config.toml`

```toml
[default]
online_judge = "atcoder"
language = "rust"

[language.rust]
solution_file = "src/main.rs"
test = "cargo test -p {problem}-{solution}"
run = "cargo run -p {problem}-{solution}"
submit_file = "src/main.rs"
submit_preprocess = ""

[language.rust.atcoder]
lang_id = "5054"

[language.cpp]
solution_file = "main.cpp"
test = "g++ {file} -o /tmp/ce_bin && echo '{input}' | /tmp/ce_bin"
submit_file = "main.cpp"

[language.cpp.atcoder]
lang_id = "5001"
```

### プロジェクトローカル: `compro-env/config.toml` (任意)

グローバルの同キーを上書き。

### セッション: `~/.config/ce/session.toml` (グローバル固定)

```toml
[atcoder]
revel_session = "xxxxxxxx"
```

---

## コマンド一覧 (MVP)

### `ce login [oj]`

詳細: `docs/commands/login.md`

### `ce whoami [oj]`

詳細: `docs/commands/whoami.md`

- セッションを読み `OnlineJudge::whoami(&session)` を呼ぶ
- ユーザー名を表示、セッションなしなら `(not logged in)` を表示して exit 0

### `ce init <contest_id_or_url>`

詳細: `docs/commands/init.md`

### `ce new <contest_id> <problem_code> [solution_name] [--lang <lang>]`

詳細: `docs/commands/new.md`

### `ce test <contest_id> <problem_code> [solution_name] [--lang <lang>]`

詳細: `docs/commands/test.md`

### `ce sub <contest_id> <problem_code> [solution_name] [--lang <lang>]`

詳細: `docs/commands/submit.md`

### (将来) リアルタイムコンテストモード

- cwd が `solutions/{contest_id}/` 以下なら `contest_id` を自動検出
- `ce sub a` などの短コマンドが動く

---

## OJ 判定ロジック

```
"abc334"     → "abc"/"arc"/"ahc" プレフィックス → AtCoder
"aoj0000"    → "aoj" プレフィックス → AOJ (将来)
"https://atcoder.jp/contests/abc334" → URL パース → AtCoder, id="abc334"
"xyz999"     → 不明 → stdin: "OJ を選んでください [atcoder]: "
```

`ce init` 後は `.ce.toml` に保存するため、以降の判定は不要。

---

## ドメインモデル

```
Contest                             ← Aggregate Root
  id: String                        "abc334"
  online_judge: OJKind
  problems: Vec<Problem>

Problem                             ← Entity (Contest 配下)
  id: String                        OJ 固有 ID ("abc334_a" 等)。AtCoder は構築可能だが他 OJ では異なる
  code: String                      ディレクトリ名に使用 ("a", "ex", "practice_2")
  title: String
  samples: Vec<Sample>

Sample                              ← Value Object
  input: String
  output: String

Solution                            ← Entity (独立 Aggregate)
  contest_id: String
  problem_code: String
  name: String                      "main", "sol2"
  language: Language

Session                             ← Value Object
  online_judge: OJKind
  cookie: String                    REVEL_SESSION 等

OJKind                              ← Value Object (enum)
  AtCoder | AOJ | ...

Language                            ← Value Object (enum)
  Rust | Cpp | ...
```

`Solution.path` は `SolutionRepository` がプロジェクトルートを保持し、そこからの相対で導出。
`IOSpec` は MVP スコープ外。

---

## Repository インターフェース (usecases 層)

```rust
trait ContestRepository {
    fn exists(&self, contest_id: &str) -> Result<bool>;
    fn exists_unstarted(&self, contest_id: &str) -> Result<bool>;
    fn create_unstarted(&self, contest_id: &str) -> Result<()>;
    fn create(&self, contest: &Contest) -> Result<()>;
    // ↑ .ce.toml 生成 + testcase ファイル保存を含む
    fn get_oj_kind(&self, contest_id: &str) -> Result<OJKind>;
    fn get_samples(&self, contest_id: &str, problem_code: &str) -> Result<Vec<Sample>>;
    fn list_problem_codes(&self, contest_id: &str) -> Result<Vec<String>>;
}

trait SolutionRepository {
    fn list(&self, contest_id: &str, problem_code: &str) -> Result<Vec<Solution>>;
    fn exists(&self, contest_id: &str, problem_code: &str, name: &str, lang: &Language) -> Result<bool>;
    fn create(&self, solution: &Solution) -> Result<()>;
    // ↑ ディレクトリ + テンプレート展開 + Cargo.toml members 更新を含む
    fn get_source(&self, solution: &Solution) -> Result<String>;
}

trait SessionRepository {
    fn get(&self, oj: OJKind) -> Result<Option<Session>>;
    fn save(&self, session: &Session) -> Result<()>;
}
```

責務の境界:

- `ContestRepository`: コンテストディレクトリ・`.ce.toml`・testcase ファイルを管理
- `SolutionRepository`: 解法ディレクトリ・ソースファイル・Rust workspace を管理
- `SessionRepository`: `~/.config/ce/session.toml` を管理

---

## OnlineJudge インターフェース (usecases 層)

```rust
trait OnlineJudge {
    fn name(&self) -> &str;
    fn whoami(&self, session: &Session) -> Result<String>;
    fn get_problems_detail(&self, contest_id: &str, session: Option<&Session>) -> Result<Vec<Problem>>;
    fn submit(
        &self,
        contest_id: &str,
        problem_id: &str,
        lang_id: &str,
        source: &str,
        session: &Session,
    ) -> Result<SubmitResult>;
    fn wait_for_start(&self, contest_id: &str) -> Result<()>;
}
```

`login(username, password)` は不要 (手動クッキー方式のため削除)。
`get_problems_detail` は公開コンテストなら session 不要 (`Option<&Session>`)。

---

## アーキテクチャ層構成

```
domain/
  entity.rs   Contest, Problem, Sample, Solution, Session, OJKind, Language, SubmitResult

usecases/
  repository/
    contest_repository.rs
    solution_repository.rs
    session_repository.rs
  online_judge.rs
  config.rs
  service/
    login.rs      SessionRepository::save()
    whoami.rs     OnlineJudge::whoami()
    init.rs       OnlineJudge::get_problems_detail() + ContestRepository::create() + SolutionRepository::create()
    new.rs        SolutionRepository::create()
    test.rs       ContestRepository::get_samples() + Config (test command)
    submit.rs     SolutionRepository::get_source() + Config (lang_id) + OnlineJudge::submit()

interfaces/
  controller/
    input.rs   各コマンドの Input trait

infrastructure/
  repository_impl/
    contest_repository_impl.rs
    solution_repository_impl.rs   ← Cargo.toml members 更新含む
    session_repository_impl.rs
  online_judge_impl/
    atcoder/
      get_problems.rs, submit.rs, whoami.rs
  config_impl.rs
  shell/   ← clap エントリポイント
```

**エラー設計**: `anyhow::Error` をデフォルトとし、matchable なドメインエラーは `thiserror` で定義。`E: Error + 'static` 型パラメータは使わない。

---

## 未決 Q リスト

### Q11. `Solution.path` の導出

`SolutionRepository` がプロジェクトルートパスを持ち、`solution.contest_id / lang / problem_code / name` から導出する設計で OK?

### Q12. `ce test` の出力形式

- MVP: シンプルな AC/WA + expected/actual 表示
- 将来: カラー表示、TLE 判定

### Q13. contest_id 省略 (cwd から自動検出) → 将来対応

MVP には含めない。将来のリアルタイムモードで対応。

### Q14. `ce whoami` のエラーハンドリング → 確定

- session 未設定: `(not logged in)` を表示し `Run \`ce login\` to save your session.` を促す。exit 0
- セッション切れ (ユーザー名抽出失敗): `session expired. Run \`ce login\` again.` を表示して exit 1
- AtCoder 接続失敗: エラー内容を表示して exit 1
