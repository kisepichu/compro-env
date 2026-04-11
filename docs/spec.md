# 仕様書 (WIP)

壁打ちしながら埋める。未決 Q は末尾に。

---

## ディレクトリ構造

```
compro-env/                         ← リポジトリルート
  config.toml                       ← プロジェクトローカル設定 (optional, global を上書き)
  templates/
    rust/                           ← 解法ディレクトリのテンプレート (ユーザーが言語を追加可能)
      Cargo.toml.tera               ← [package] name = "{{problem.code}}-{{solution.name}}"
      src/main.rs
  solutions/
    {contest_id}/
      .ce.toml                      ← [アプリ管理] OJ 情報を保存 (ce init 時に生成、以降上書きしない)
      testcases/                    ← [アプリ管理] ce init が生成・管理。ユーザーは直接編集しない
        {problem_code}/             1文字固定でない (ex, practice_2 等あり)
          1.in  1.out  2.in  2.out
      {problem_code}/
        {solution_name}/            ← [ユーザー作業領域] templates/{lang}/ を展開したもの。以降はユーザーが自由に編集
          Cargo.toml                ← templates/rust/Cargo.toml.tera から展開
          src/main.rs
```

**領域の区別**:
- `.ce.toml` と `testcases/` は `ce` が管理するファイル。ユーザーは直接編集しない。
- `{problem_code}/{solution_name}/` 以下がユーザーの作業領域。`ce init` / `ce solution add` 時に `templates/{lang}/` を展開して初期化され、以降はユーザーが自由に編集する。

### .ce.toml の内容

```toml
online_judge = "atcoder"
contest_id = "abc334"

[[problems]]
code = "a"
title = "Product"

[[problems]]
code = "b"
title = "ABC"
```

`ce test` / `ce sub` 時に OJ を特定するために必須。プレフィックス判定だけでは `xyz999` 等に対応不可。
`problems` は `ce solution add` 等で問題コード一覧を参照するために保存する。
`ce init` 時に生成し、以降は上書きしない。samples は testcases/ にファイルとして保存するため `.ce.toml` には含まない。

---

## コンフィグ設計

### グローバル: `~/.config/ce/config.toml`

```toml
[default]
online_judge = "atcoder"
language = "rust"

[language.rust]
solution_file = "src/main.rs"
test = "cargo run --manifest-path {dir}/Cargo.toml < {input_file}"
submit_preprocess = ""

[language.rust.atcoder]
lang_id = "5054"
```

`{dir}` は解法ディレクトリの絶対パス、`{input_file}` はテストケース入力ファイルの絶対パス。  
言語はユーザーが自由に追加できる。`templates/{lang}/` ディレクトリを追加するだけで `ce` がその言語名を認識する。`[language.{name}]` セクションはテスト・提出コマンドの設定に使用する (省略した場合はデフォルト設定のみ)。

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

### `ce logout [oj]`

詳細: `docs/commands/logout.md`

- `SessionRepository::delete(oj)` を呼んでセッションを削除する
- 削除できた場合: `Logged out from {oj}.` を表示して exit 0
- セッションがなかった場合: `Already logged out.` を表示して exit 0

### `ce init <contest_id_or_url> [--lang <lang>]`

詳細: `docs/commands/init.md`

### `ce solution <subcommand>`

詳細: `docs/commands/solution.md`

サブコマンド: `add` (将来: `rename` 等)

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
"abc334"     → "abc"/"arc"/"agc"/"ahc" プレフィックス → AtCoder
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

Language                            ← Value Object (String の newtype)
  templates/{lang}/ ディレクトリ名がそのまま言語名になる。固定 enum ではない。
  検証: templates/{lang}/ が存在するかで判断する。
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
    // .ce.toml 生成 (problems 含む、samples は除く) + testcase ファイル保存
    fn get_oj_kind(&self, contest_id: &str) -> Result<OJKind>;
    // .ce.toml から OJKind を読み取る
    fn get_samples(&self, contest_id: &str, problem_code: &str) -> Result<Vec<Sample>>;
    fn list_problem_codes(&self, contest_id: &str) -> Result<Vec<String>>;
    // testcases/ 以下のディレクトリ名から問題コード一覧を返す
}

trait SolutionRepository {
    fn list(&self, contest_id: &str, problem_code: &str) -> Result<Vec<Solution>>;
    fn exists(
        &self,
        contest_id: &str,
        problem_code: &str,
        name: &str,
        lang: &Language,
    ) -> Result<bool>;
    fn create(&self, solution: &Solution) -> Result<()>;
    // templates/{lang}/ を solutions/{contest_id}/{problem_code}/{solution_name}/ に展開
    // 既存ファイルはスキップ
    fn get_source(&self, solution: &Solution) -> Result<String>;
}

trait SessionRepository {
    fn get(&self, oj: &OJKind) -> Result<Option<Session>>;
    fn save(&self, session: &Session) -> Result<()>;
    fn delete(&self, oj: &OJKind) -> Result<bool>;
    // bool: true = deleted, false = was not present
}
```

責務の境界:

- `ContestRepository`: コンテストディレクトリ・`.ce.toml`・testcase ファイルを管理
- `SolutionRepository`: 解法ディレクトリ・ソースファイルを管理 (テンプレート展開含む)
- `SessionRepository`: `~/.config/ce/session.toml` を管理

---

## OnlineJudge インターフェース (usecases 層)

```rust
/// コンテストページ 1 回のフェッチで取れるメタ情報。
struct ContestMeta {
    /// コンテスト開始時刻。取得できない場合は None。
    start_time: Option<DateTime<Utc>>,
    /// ナビバードロップダウンから取れた (problem_code, problem_id) ペア。
    /// 現行コンテストでは空 Vec。空なら get_problems_detail 側で {contest_id}_{code} と推定する。
    /// ABC/ARC 同時開催の旧コンテストでは arc103_a 等の実際の ID が入る。
    problem_id_hints: Vec<(String, String)>,
}

trait OnlineJudge {
    fn name(&self) -> &str;
    fn whoami(&self, session: &Session) -> Result<String>;
    /// コンテストページを 1 回フェッチして開始時刻と problem_id ヒントを返す。
    fn get_contest_meta(&self, contest_id: &str) -> Result<ContestMeta>;
    /// tasks_print ページを 1 回フェッチして全問題詳細を返す。
    /// problem_id_hints が空なら {contest_id}_{code} と推定する。
    fn get_problems_detail(
        &self,
        contest_id: &str,
        session: Option<&Session>,
        problem_id_hints: &[(String, String)],
    ) -> Result<Vec<Problem>>;
    fn submit(
        &self,
        contest_id: &str,
        problem_id: &str,
        lang_id: &str,
        source: &str,
        session: &Session,
    ) -> Result<SubmitResult>;
}
```

`login(username, password)` は不要 (手動クッキー方式のため削除)。  
`get_problems_detail` は公開コンテストなら session 不要 (`Option<&Session>`)。  
コンテスト開始待機ロジック (ポーリング・カウントダウン表示) は `usecases/service/init.rs` に実装し、`get_contest_meta` で取得した時刻をもとに制御する。OJ 固有ロジックは含まない。  
通常の `ce init` (コンテスト開始後) は `get_contest_meta` + `get_problems_detail` の **2 リクエスト**のみ。

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
    init.rs       OnlineJudge::get_contest_meta() + 待機ループ + OnlineJudge::get_problems_detail()
                  + ContestRepository::create() + SolutionRepository::create() × N
    solution/
      add.rs      ContestRepository::exists() + SolutionRepository::create()
    test.rs       ContestRepository::get_samples() + Config (test command)
    submit.rs     SolutionRepository::get_source() + Config (lang_id) + OnlineJudge::submit()

interfaces/
  controller/
    input.rs   各コマンドの Input trait

infrastructure/
  repository_impl/
    contest_repository_impl.rs
    solution_repository_impl.rs   ← テンプレート展開含む
    session_repository_impl.rs
  online_judge_impl/
    atcoder.rs
  config_impl.rs
  shell/   ← clap エントリポイント
```

**エラー設計**: `anyhow::Error` をデフォルトとし、matchable なドメインエラーは `thiserror` で定義。`E: Error + 'static` 型パラメータは使わない。

---

## 未決 Q リスト

### Q11. `Solution.path` の導出

`SolutionRepository` がプロジェクトルートパスを持ち、`solution.contest_id / problem_code / name` から導出する設計で OK?

### Q12. `ce test` の出力形式

- MVP: シンプルな AC/WA + expected/actual 表示
- 将来: カラー表示、TLE 判定

### Q13. contest_id 省略 (cwd から自動検出) → 将来対応

MVP には含めない。将来のリアルタイムモードで対応。

### Q14. `ce whoami` のエラーハンドリング → 確定

- session 未設定: `(not logged in)` を表示し `Run \`ce login\` to save your session.` を促す。exit 0
- セッション切れ (ユーザー名抽出失敗): `session expired. Run \`ce login\` again.` を表示して exit 1
- AtCoder 接続失敗: エラー内容を表示して exit 1
