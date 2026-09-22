# Issue #106 — C++/Lean adapter env プラミング Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `handshake_adapter` と `build_analysis`（`ce site-data generate` の analyze 経路）に per-language env を配線し、`config.toml` の `[library.languages.cpp]` / `[library.languages.lean]` を再有効化して、site-data に 3 言語すべてが乗るようにする。

**Architecture:** 現状 `LibraryAdapterRunner::analyze` はランナ構築時の env に固定されており、`LanguageBuildPlan::handshake_environment` は宣言だけで誰も読まない。`.analyze()` のシグネチャに env パラメータを追加してハンドシェイクと解析の両方で per-language env を渡す（Rust は共有 sanitized env、C++ も同じ、Lean は `CE_LEAN_ROOT` と `<lean_root>/lib` を `LD_LIBRARY_PATH`、`<lean_root>/bin` を `PATH` に混ぜる）。Lean 側の handshake で `lean` が `PATH` に見えず落ちる問題は `build_lean_env` に `<lean_root>/bin` の `PATH` prepend を追加して解消する。`build_analysis` は `<analyzer_root>/prepared/<dep_id>` を突き止めて同じ env ヘルパを解析ランナにも適用する。

**Tech Stack:** Rust 1.92.0, `usecases::library_adapter::LibraryAdapterRunner`, `infrastructure::library_adapter::{build,language_plans,process,lean_toolchain}`.

## Global Constraints

- 依存レイヤ規則: `domain → usecases → interfaces → infrastructure`。`LibraryAdapterRunner` は `usecases` にある。トレイト拡張時に外側の型を持ち込まない。
- エラーハンドリング: `anyhow` + `thiserror`。既存パターンに合わせる。
- コードコメントは英語、コミット・PR 本文・レビュー返信は日本語、emoji 禁止。
- 段階的 fallback は禁止（例: ambient PATH 上の `lean` に落ちてはいけない）。env プラミングが失敗したら hard error。
- Spec §§6.7–6.9 の toolchain pin（clang 22.1.0 / lean 4.30.0）は変更しない。
- config.toml の block を uncomment する際は現状のコメントブロックと同じキー・順序で復元する（`analyzer.command` は既存の相対パス）。

---

## Files touched (map)

- Modify: `crates/usecases/src/library_adapter.rs` — `analyze` に env パラメータを追加。
- Modify: `crates/infrastructure/src/library_adapter/process.rs` — `ProcessLibraryAdapterRunner` から env フィールドを削除し `analyze` の env 引数を使う。
- Modify: `crates/infrastructure/src/library_adapter/build.rs` — `handshake_adapter` に env 引数を追加し、`run_one_plan` が `plan.handshake_environment` を渡す。
- Modify: `crates/infrastructure/src/library_adapter/language_plans.rs` — `build_lean_env` に `<lean_root>/bin` の PATH prepend を追加。`analyze_language_env(prepared_root, platform, language)` を新設。
- Modify: `crates/infrastructure/src/library_analyzer_impl.rs` — `ProcessLibraryAnalyzer::new` に per-language env マップを受け取らせ、`analyze_all` が per-lang env を `runner.analyze` に渡す。
- Modify: `crates/infrastructure/src/shell/mod.rs` — `build_analysis` と `site-data generate` の runner 構築を per-language env に切り替え。
- Modify: `crates/infrastructure/src/bin/library-adapter-build.rs` — 新しい `ProcessLibraryAdapterRunner::new` シグネチャに追随。
- Modify: `crates/infrastructure/tests/adapter_process.rs` / `adapter_build.rs` / `rust_adapter_handshake.rs` / `cpp_adapter_handshake.rs` / `lean_adapter_handshake.rs` — `.analyze()` 呼び出し側と `ProcessLibraryAdapterRunner::new` の呼び出しを新 API に更新。cpp / lean のハンドシェイクテストは production env helper 経由で env を組み立てる。
- Modify: `config.toml` — `[library.languages.cpp]` / `[library.languages.lean]` ブロックと `[library.languages.<lang>.online_judges.librarychecker]` を uncomment。冒頭のコメント塊を削除。
- Modify: `docs/spec.md` / `docs/commands/site-data.md` — cpp/lean の deferred 記述が残っていれば更新（現状 spec には該当なし、site-data.md も無し。config.toml のコメント削除のみが実質。ただし念のため差分確認）。

---

### Task 1: env プラミング — trait と handshake_adapter に per-language env を通す

**Files:**
- Modify: `crates/usecases/src/library_adapter.rs`
- Modify: `crates/infrastructure/src/library_adapter/process.rs`
- Modify: `crates/infrastructure/src/library_adapter/build.rs`
- Modify: `crates/infrastructure/src/library_adapter/language_plans.rs`
- Modify: `crates/infrastructure/src/library_analyzer_impl.rs`
- Modify: `crates/infrastructure/src/shell/mod.rs`
- Modify: `crates/infrastructure/src/bin/library-adapter-build.rs`
- Modify: `crates/infrastructure/tests/adapter_process.rs`
- Modify: `crates/infrastructure/tests/adapter_build.rs`
- Modify: `crates/infrastructure/tests/rust_adapter_handshake.rs`
- Modify: `crates/infrastructure/tests/cpp_adapter_handshake.rs`
- Modify: `crates/infrastructure/tests/lean_adapter_handshake.rs`

**Interfaces:**
- Produces:
  - `LibraryAdapterRunner::analyze(&self, executable: &Path, request: &AnalysisRequest, timeout: Duration, environment: &BTreeMap<String, String>) -> Result<AnalysisResponse, AdapterRunError>` — env が per-call に。
  - `ProcessLibraryAdapterRunner::new(working_directory: PathBuf) -> Self` — env 引数を削除。
  - `handshake_adapter(runner, executable, language, timeout, environment) -> Result<AdapterIdentity, BuildError>` — env 引数追加。
  - `build_lean_env(lean_root, env)` — `PATH` に `<lean_root>/bin` を prepend する挙動を追加。
  - `analyze_language_env(prepared_root: &Path, platform: &TargetPlatform, language: &str) -> Result<BTreeMap<String, String>, LeanToolchainError>` — 解析経路が per-language env を作る公開ヘルパ。cpp / rust は `sanitized_language_env()` そのまま、lean は `build_lean_env` を適用。
  - `ProcessLibraryAnalyzer::new(runner, config, envs: BTreeMap<LanguageId, BTreeMap<String, String>>)`。

- [ ] **Step 1.1: `LibraryAdapterRunner` トレイトを更新**

`crates/usecases/src/library_adapter.rs`:

```rust
use std::collections::BTreeMap;

pub trait LibraryAdapterRunner {
    fn analyze(
        &self,
        executable: &Path,
        request: &AnalysisRequest,
        timeout: Duration,
        environment: &BTreeMap<String, String>,
    ) -> Result<AnalysisResponse, AdapterRunError>;
}
```

- [ ] **Step 1.2: `ProcessLibraryAdapterRunner` から env フィールドを外す**

`crates/infrastructure/src/library_adapter/process.rs`:

- struct から `environment: BTreeMap<String, String>` を削除。
- `new(working_directory: PathBuf)` に絞る。`with_extra_args` / `with_stdout_limit_bytes` / `with_stderr_tail_bytes` は残す。
- `analyze` は引数の `environment` を使って `cmd.env_clear()` 後に流し込む。

- [ ] **Step 1.3: `handshake_adapter` に env を通す**

`crates/infrastructure/src/library_adapter/build.rs`:

```rust
pub fn handshake_adapter(
    runner: &dyn LibraryAdapterRunner,
    executable: &Path,
    language: &str,
    timeout: Duration,
    environment: &BTreeMap<String, String>,
) -> Result<AdapterIdentity, BuildError> {
    let request = empty_handshake_request(language);
    let response = runner
        .analyze(executable, &request, timeout, environment)
        .map_err(|source| BuildError::HandshakeRun { language: language.into(), source })?;
    // ... 変更なし
}
```

`run_one_plan` の呼び出し側を `handshake_adapter(runner, &staged, &plan.language, request.handshake_timeout, &plan.handshake_environment)` に更新。

- [ ] **Step 1.4: `build_lean_env` が `<lean_root>/bin` を PATH の先頭に置く**

`crates/infrastructure/src/library_adapter/language_plans.rs`:

```rust
fn build_lean_env(lean_root: &Path, mut env: BTreeMap<String, String>) -> BTreeMap<String, String> {
    let root_str = lean_root.to_string_lossy().into_owned();
    let bin_dir = lean_root.join("bin").to_string_lossy().into_owned();
    let lib_dir = lean_root.join("lib").to_string_lossy().into_owned();
    env.insert("CE_LEAN_ROOT".into(), root_str);
    let path_entry = env.entry("PATH".into()).or_default();
    if path_entry.is_empty() {
        *path_entry = bin_dir;
    } else {
        *path_entry = format!("{bin_dir}:{path_entry}");
    }
    let ld_entry = env.entry("LD_LIBRARY_PATH".into()).or_default();
    if ld_entry.is_empty() {
        *ld_entry = lib_dir;
    } else {
        *ld_entry = format!("{lib_dir}:{ld_entry}");
    }
    env
}
```

- [ ] **Step 1.5: `ProcessLibraryAdapterRunner::new` の呼び出しを一斉に env なしに更新**

以下の呼び出し箇所を `ProcessLibraryAdapterRunner::new(working_directory)` に変更（env は `.analyze()` に渡す方に移す）:

- `crates/infrastructure/src/bin/library-adapter-build.rs:225-226`
- `crates/infrastructure/src/shell/mod.rs:323`（Task 2 で二度触るので現段階では暫定的に `sanitized_language_env()` を握って `analyze` 側で使う wiring を先に）
- `crates/infrastructure/src/shell/mod.rs:1134`（同上）
- `crates/infrastructure/tests/adapter_process.rs`（複数箇所）
- `crates/infrastructure/tests/adapter_build.rs`（`runner_with` を env なしに）
- `crates/infrastructure/tests/rust_adapter_handshake.rs:107`
- `crates/infrastructure/tests/cpp_adapter_handshake.rs:101`
- `crates/infrastructure/tests/lean_adapter_handshake.rs:103`

各テスト内で `.analyze(&bin, &req, timeout)` 呼び出しに env 引数を追加。テストローカルの env をそのまま渡す形に。

- [ ] **Step 1.6: `library_analyzer_impl` を per-language env に対応**

`crates/infrastructure/src/library_analyzer_impl.rs`:

- `ProcessLibraryAnalyzer` に `envs: BTreeMap<LanguageId, BTreeMap<String, String>>` を追加。
- `new(runner, config, envs)`。
- `analyze_all` 内で `let env = self.envs.get(language_id).ok_or_else(|| anyhow!("no analyzer env for {}", language_id.as_str()))?;` として `.analyze` に渡す。

- [ ] **Step 1.7: adapter_build テストの `handshake_adapter` に沿った修正**

`crates/infrastructure/tests/adapter_build.rs` は `handshake_adapter` を直接呼んでいないが、`build_adapters` 経由で走る `run_one_plan` が `plan.handshake_environment` を使うようになる。既存の `plan_that_copies` / `plans_by_language` は既に `handshake_environment` を埋めているので、そのままで OK。ランナ構築の env は不要になるので `runner_with` は env 引数を落として `ProcessLibraryAdapterRunner::new(cwd)` にする。

- [ ] **Step 1.8: 部分ビルド確認**

```bash
cargo build --workspace
```

期待: すべてコンパイル通る。

- [ ] **Step 1.9: ユニット/統合テスト実行**

```bash
cargo test --workspace
```

期待: 全部 pass。`cpp_adapter_handshake` / `lean_adapter_handshake` は gate 未設定なら `println!` して return（Task 2 で本気の env を組む）。

- [ ] **Step 1.10: clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

期待: warnings 無し。

- [ ] **Step 1.11: commit**

```bash
git add crates/usecases/src/library_adapter.rs \
        crates/infrastructure/src/library_adapter/{process.rs,build.rs,language_plans.rs} \
        crates/infrastructure/src/library_analyzer_impl.rs \
        crates/infrastructure/src/bin/library-adapter-build.rs \
        crates/infrastructure/src/shell/mod.rs \
        crates/infrastructure/tests/
git commit -m "$(printf 'refactor(library-adapter): analyze/handshake に per-language env を通す\n\n- LibraryAdapterRunner::analyze の env をランナ構築時の固定値から呼び出しごとの引数に切り替え\n- handshake_adapter に environment 引数を追加し LanguageBuildPlan.handshake_environment を配線\n- build_lean_env が PATH に <lean_root>/bin を prepend するよう修正 (issue #106)\n- ProcessLibraryAnalyzer が per-language env マップを受け取るよう更新\n')"
```

---

### Task 2: analyze パスに per-language env を配線し、handshake テストを production 経路に寄せる

**Files:**
- Modify: `crates/infrastructure/src/library_adapter/language_plans.rs`
- Modify: `crates/infrastructure/src/shell/mod.rs`
- Modify: `crates/infrastructure/tests/cpp_adapter_handshake.rs`
- Modify: `crates/infrastructure/tests/lean_adapter_handshake.rs`
- Modify: `crates/infrastructure/tests/rust_adapter_handshake.rs`

**Interfaces:**
- Consumes: Task 1 が出した `analyze_language_env`, `ProcessLibraryAnalyzer::new(runner, config, envs)`。
- Produces:
  - `build_analysis` が `sanitized_language_env()` 単一 env の代わりに `BTreeMap<LanguageId, BTreeMap<String, String>>` を組んで analyzer に渡す。
  - handshake テストが production env helper（`analyze_language_env` / `build_lean_env`）を経由して env を組む。

- [ ] **Step 2.1: `analyze_language_env` を実装**

`crates/infrastructure/src/library_adapter/language_plans.rs`:

```rust
/// Build the analyze-time env for one language, mirroring what
/// `handshake_environment` on the corresponding `LanguageBuildPlan` receives.
/// The caller supplies the resolved `<analyzer_root>/prepared/<dep-id>/`
/// directory; `select_*_toolchain` / `locate_prepared_*_root` reproduce the
/// same layout checks that gate the build path.
pub fn analyze_language_env(
    prepared_root: &Path,
    platform: &TargetPlatform,
    language: &str,
) -> Result<BTreeMap<String, String>, LeanToolchainError> {
    let base = sanitized_language_env();
    match language {
        LEAN_LANGUAGE => {
            // Reuse the same locator the build plan uses so a lean_root
            // layout drift shows up identically in both paths.
            let lean_root = locate_prepared_lean_root_from_root(prepared_root, platform)?;
            Ok(build_lean_env(&lean_root, base))
        }
        _ => Ok(base),
    }
}
```

`locate_prepared_lean_root_from_root(prepared_root: &Path, platform)` は `lean_toolchain.rs` に薄いラッパを追加（`locate_prepared_lean_root` 本体を `prepared_root: &Path` 版に refactor し、既存の `locate_prepared_lean_root(prepared_set, ...)` は `&prepared_set.root` を渡す薄いラッパに）。C++ 側は現時点で env 拡張が不要（`sanitized_language_env` の LD_LIBRARY_PATH で足りる）だが対称性を保つため `cpp_toolchain.rs` にも同じラッパを足しておく（`analyze_language_env` からは呼ばないが、将来の cpp env 拡張のフックとして）。→ 実装コスト対策で今回はまず lean のみに閉じる。

- [ ] **Step 2.2: `build_analysis` が per-language env を組む**

`crates/infrastructure/src/shell/mod.rs`:

`build_analysis` 内で:

```rust
let analyzer_root = root.join("target").join("library-analyzers");
let platform = TargetPlatform {
    os: std::env::consts::OS.into(),
    arch: std::env::consts::ARCH.into(),
};
let prepared_root = discover_prepared_root(&analyzer_root)?; // helper
let mut envs: BTreeMap<LanguageId, BTreeMap<String, String>> = BTreeMap::new();
for language_id in config.languages.keys() {
    let env = crate::library_adapter::language_plans::analyze_language_env(
        &prepared_root,
        &platform,
        language_id.as_str(),
    )
    .map_err(|e| anyhow!("failed to build analyze env for {}: {e}", language_id.as_str()))?;
    envs.insert(language_id.clone(), env);
}
let runner = ProcessLibraryAdapterRunner::new(root.to_path_buf());
let analyzer = ProcessLibraryAnalyzer::new(runner, config.clone(), envs);
```

`discover_prepared_root` は `<analyzer_root>/prepared/` を読み `staging-` 接頭辞を除いた唯一のエントリを返す関数（`shell/mod.rs` 内 private）:

```rust
fn discover_prepared_root(analyzer_root: &Path) -> Result<PathBuf> {
    let prepared_dir = analyzer_root.join("prepared");
    let entries: Vec<_> = std::fs::read_dir(&prepared_dir)
        .map_err(|e| anyhow!("failed to read {}: {e}", prepared_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            !s.starts_with("staging-")
        })
        .collect();
    if entries.len() != 1 {
        bail!(
            "expected exactly one prepared set under {}, found {}",
            prepared_dir.display(),
            entries.len()
        );
    }
    Ok(entries[0].path())
}
```

`site-data generate` 側の runner 構築（shell/mod.rs:323）は analyzer 経由でしか使われないので `build_analysis` に集約できるならこちらに寄せる。現状 site-data generate はここでランナを組んで analyzer に渡している — Task 2 の一環で `build_analysis` から `(manifest, snapshot, analyzer_result)` 相当を返し、site-data 側でも同じヘルパを共有させる。

- [ ] **Step 2.3: handshake テストの env を production helper に切り替え**

`crates/infrastructure/tests/lean_adapter_handshake.rs`:

`sanitized_env()` 定義を差し替え、`CE_RUN_LEAN_HANDSHAKE=1` が立っているときの env を:

```rust
use infrastructure::library_adapter::language_plans::analyze_language_env;
use domain::adapter_build::TargetPlatform;

fn lean_analyze_env() -> BTreeMap<String, String> {
    let analyzer_root = workspace_root().join("target").join("library-analyzers");
    let prepared_root = discover_prepared_root(&analyzer_root);
    let platform = TargetPlatform {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
    };
    analyze_language_env(&prepared_root, &platform, "lean")
        .expect("lean analyze env")
}

fn discover_prepared_root(analyzer_root: &Path) -> PathBuf {
    let prepared_dir = analyzer_root.join("prepared");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&prepared_dir)
        .unwrap_or_else(|e| panic!("read {} failed: {e}", prepared_dir.display()))
        .filter_map(|e| e.ok())
        .filter(|e| !e.file_name().to_string_lossy().starts_with("staging-"))
        .map(|e| e.path())
        .collect();
    entries.sort();
    entries.pop().expect("prepared set present")
}
```

`.analyze(&bin, &req, timeout, &lean_analyze_env())` として呼び出す。

`cpp_adapter_handshake.rs` は `analyze_language_env(..., "cpp")` で `sanitized_language_env` を得るだけなので、同じ形にそろえる。`rust_adapter_handshake.rs` も同様。

- [ ] **Step 2.4: 部分ビルド + workspace test**

```bash
cargo build --workspace
cargo test --workspace
```

期待: 全部 pass（cpp / lean gate はデフォルトでスキップ）。

- [ ] **Step 2.5: clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 2.6: commit**

```bash
git add crates/infrastructure/src/library_adapter/language_plans.rs \
        crates/infrastructure/src/library_adapter/lean_toolchain.rs \
        crates/infrastructure/src/shell/mod.rs \
        crates/infrastructure/tests/{rust,cpp,lean}_adapter_handshake.rs
git commit -m "$(printf 'feat(library-adapter): analyze パスに per-language env を配線\n\n- build_analysis が analyze_language_env でハンドシェイクと同じ env を組む\n- discover_prepared_root で <analyzer_root>/prepared/<dep-id> を解決\n- {cpp,lean,rust}_adapter_handshake が production env helper 経由で env を構築 (issue #106)\n')"
```

---

### Task 3: config.toml uncomment + docs 反映 + workspace 全体検証

**Files:**
- Modify: `config.toml`
- Modify: `docs/spec.md`（cpp/lean deferred の記述があれば削る。現状 grep で該当なし → no-op の可能性）
- Modify: `docs/commands/site-data.md`（同上）

**Interfaces:**
- Consumes: Task 1 / Task 2 の env プラミング。
- Produces: `config.toml` の `[library.languages.cpp]` / `[library.languages.lean]` ブロック復活、`[library.languages.<lang>.online_judges.librarychecker]` の binding も復活。

- [ ] **Step 3.1: config.toml のコメント塊を削除しブロックを uncomment**

`config.toml:28-71` の C++ / Lean deferred コメント塊を削除し、以下を有効化:

```toml
[library.languages.cpp]
display_name = "C++"
root = "libraries/cpp"
include = ["**/*.hpp", "**/*.cpp"]
exclude = []
check_command = "clang++ -std=c++20 -Wall -Wextra -Werror -fsyntax-only libraries/cpp/algebra/monoid.hpp"
check_timeout_seconds = 600
syntax_highlight = "cpp"
entry_file = "main.cpp"
expected_toolchains = [
  { name = "clang", version = "22.1.0" },
]

[library.languages.cpp.analyzer]
command = ["./target/library-analyzers/bin/cpp-analyzer"]
timeout_seconds = 600

[library.languages.cpp.online_judges.librarychecker]
language_id = "cpp"

[library.languages.lean]
display_name = "Lean"
root = "libraries/lean"
include = ["**/*.lean"]
exclude = []
check_command = "lake build"
check_timeout_seconds = 900
syntax_highlight = "lean"
entry_file = "Main.lean"
expected_toolchains = [
  { name = "lean", version = "4.30.0" },
]

[library.languages.lean.analyzer]
command = ["./target/library-analyzers/bin/lean-analyzer"]
timeout_seconds = 900

[library.languages.lean.online_judges.librarychecker]
language_id = "lean"
```

- [ ] **Step 3.2: docs で deferred / Rust-only の残滓を確認**

```bash
grep -RIn "temporarily disabled\|adapter env plumbing\|Rust-only" docs
```

無ければ変更なし。あれば削除。

- [ ] **Step 3.3: workspace 全体で最終検証**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

期待: 全部 pass、warning ゼロ。

- [ ] **Step 3.4: `ce site-data generate --mode preview` の smoke**

事前条件: `<repo>/target/library-analyzers/bin/{rust,cpp,lean}-analyzer` が存在すること。

```bash
cd /home/kise/repos/compro-env
tools/library-analyzers/prepare
tools/library-analyzers/build
ce site-data generate --output /tmp/site-data-preview --mode preview
jq '.languages | map(.id)' /tmp/site-data-preview/site-data.json
```

期待: `["cpp","lean","rust"]`。

**実行不能な場合の代替検証:**

この smoke は `libraries/cpp`（LLVM 22.1.0 の実タブ）と `libraries/lean`（Lean 4.30.0 ~150 MB tarball）の実物 prepare + build が要る。ネットワーク・時間コスト上、開発環境で必ずしも走らせられない。その場合は次の 2 点で代替する:

1. `cargo test -p infrastructure --test cpp_adapter_handshake -- --ignored` に相当する gate `CE_RUN_CPP_HANDSHAKE=1` / `CE_RUN_LEAN_HANDSHAKE=1` が env プラミングを通した runner で pass することを最低限確認する（開発環境が Lean/LLVM を持たない場合はスキップ）。
2. `ce site-data generate --mode preview` が cpp / lean を含んで動くには adapter 実行が必要という制約は仕様どおり。手元では走らせられない旨を PR 本文で明記し、CI もしくは main への push 後の実機で必ず確認する運用にする。

- [ ] **Step 3.5: commit**

```bash
git add config.toml docs
git commit -m "$(printf 'feat(config): C++ / Lean adapter を再有効化 (issue #106)\n\n- [library.languages.cpp] / [library.languages.lean] ブロックと librarychecker binding を uncomment\n- adapter env プラミング完了に伴い temporarily disabled コメントを削除\n')"
```

- [ ] **Step 3.6: push + PR**

```bash
git push -u origin feat/106-cpp-lean-adapter-env
```

`skill://pr` に従い PR を作成する。PR タイトル例: `feat(library-adapter): C++/Lean adapter env プラミングと再有効化 (#106)`。本文で以下を含める:

- 何が変わったか (trait 変更 / handshake_environment 配線 / build_lean_env の PATH 追加 / config.toml uncomment)
- 手元で走らせた検証 (cargo test / clippy)
- `ce site-data generate` smoke の実行有無・実行できていない場合はその理由と main への push 後の追跡タスク
- Closes #106

- [ ] **Step 3.7: `skill://pr-review claude` で Claude レビューをかける**

指摘があれば返信・修正・resolve までループ。

---

## Self-Review

1. **Spec coverage:**
   - Issue #106 の Root cause / What needs to happen 全 5 項目に対応する task がある（trait 拡張は Task 1、`build_analysis` per-lang env は Task 2、config.toml uncomment は Task 3、handshake tests refresh は Task 2、live smoke は Task 3.4 で明示）。

2. **Placeholder scan:** TBD / TODO なし。すべて具体的なコード断片・コマンドを併記。Step 3.4 の代替検証は「実行不能なとき」の明示的な条件・落とし所を書いた。

3. **Type consistency:**
   - `analyze_language_env(prepared_root: &Path, platform: &TargetPlatform, language: &str) -> Result<BTreeMap<String, String>, LeanToolchainError>` — Task 1/2 で一貫。
   - `LibraryAdapterRunner::analyze(..., environment: &BTreeMap<String, String>)` — Task 1 で導入、Task 2 でも同じ。
   - `ProcessLibraryAnalyzer::new(runner, config, envs)` — Task 1 で導入、Task 2 で使用。

以上。
