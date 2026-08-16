# site-data current fingerprint recomputation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** issue #104 に従い `ce site-data generate` で保存済み verification record の `Verified` が `Stale` に落ちないよう、per-solution の *current* fingerprint を再計算して `PublicProjectionInput.current_fingerprints` に流し込む。

**Architecture:** `verify` パイプライン (`crates/usecases/src/service/verify.rs::build_plan_context`) が使う `verification_closure` + `calculate_fingerprint` を site-data 生成でも共有する。共有のために `hash_verify_config` と capabilities 変換ヘルパを `verification::fingerprint` に公開する。`GenerateSiteData` に `starters: &'a StarterRegistry` を足し、`SubmissionStarter::descriptor()` から `AdapterIdentity` を組み立てる (プリプロセスは呼ばない — site-data はオフライン。保存済み record の `submitted_source_hash` は生ファイル hash と一致することを確認済み)。

**Tech Stack:** Rust 4-layer DDD (`domain` / `usecases` / `interfaces` / `infrastructure`)。cargo workspace。fingerprint payload は既存の canonical JSON。

## Global Constraints

- コメントは英語、コミット/PR/レビュー返信は日本語、絵文字禁止。
- `E: Error + 'static` は禁止。`anyhow` / `thiserror` に統一。
- インナー層はアウター層を import しない: `usecases` が `infrastructure` に依存してはならない。
- fingerprint payload の順序変更・スキーマ変更は禁止 (既存 record と互換維持)。今回変更するのは *呼び出し側* だけ。
- `docs/spec.md` §11 (fingerprint) / §10 (status) と `docs/commands/site-data.md` の仕様と実装の整合を必ず取る。plan 052 参照は削除する。

---

## Signature Reference (grounding)

- `crates/usecases/src/verification/fingerprint.rs`
  - `pub fn verification_closure(solution_id: &SolutionId, explicit_verify_libraries: &BTreeSet<LibraryId>, snapshot: &AnalysisSnapshot) -> Result<BTreeSet<LibraryId>, FingerprintError>` (L122-)
  - `pub fn calculate_fingerprint(material: &FingerprintMaterial) -> Result<VerifyFingerprint, FingerprintError>` (L227-)
  - `pub struct FingerprintMaterial { solution_id, submitted_source, verified_libraries, dependency_library_sources, binding, adapter, verify_config_hash }` (L96-)
  - `pub struct AdapterIdentity { name: String, version: String, capabilities: SubmissionCapabilities }` (L65-)
  - `pub struct OjBinding { oj, problem_id, language_id, oj_language_id }` (L57-)
  - `pub struct FingerprintSource { path: String, bytes: Vec<u8> }` (L74-)
- `crates/usecases/src/service/verify.rs`
  - private `fn hash_verify_config(verify: &VerifySpec) -> ContentHash` (L987-999) — libraries (sorted) と oj_language_id を canonical JSON SHA-256。
  - private `fn map_submission_mode / map_result_detail / map_recovery_mode` (L962-985) — Port→Domain の capabilities 変換。
- `crates/usecases/src/submission_lifecycle.rs`
  - private `fn capabilities_of_starter(starter: &dyn SubmissionStarter) -> SubmissionCapabilities` (L1123-1153) — descriptor から Domain capabilities 生成。
- `crates/usecases/src/submission.rs`
  - `pub struct StarterRegistry { pub fn get(&self, oj: &OJKind) -> Result<&dyn SubmissionStarter> }` (L471-499)
  - `pub struct SubmissionAdapterDescriptor { name, version, submission_mode, result_detail, recovery_mode }` (L69-75)
- `crates/usecases/src/verification/status.rs`
  - `pub fn classify_solution_status(verify_spec: Option<&VerifySpec>, current_fingerprint: Result<&VerifyFingerprint, &FingerprintError>, saved: Option<&VerificationRecord>) -> VerificationStatus` (L49-82) — Err(_) は Stale/Never にフォールバック。
- `crates/usecases/src/site_data_generator.rs`
  - `pub struct GenerateSiteData<'a> { … }` (L32-45)
  - `pub fn generate_site_data(spec: &GenerateSiteData<'_>) -> Result<SiteData>` (L49-158) — step 7 (L115-120) が現状の空 map。
- `crates/usecases/src/site_data.rs`
  - `pub struct PublicProjectionInput<'a> { current_fingerprints: &'a BTreeMap<SolutionId, Result<VerifyFingerprint, FingerprintError>>, … }` (L88-103)
  - `fn classify_all_solutions` (L349-370) — `None` → sentinel `UnknownSolution` を作って Err path、`Some(Ok/Err)` はそのまま渡す。
- `crates/interfaces/src/controller.rs::site_data_generate` (L124-166) — 現在は `starters` を受け取らない。
- `crates/infrastructure/src/shell/mod.rs` — `SiteData::Generate` 分岐 (L288-374) と `submission_impl/registry.rs::build_starter_registry()` (L11-22) は既存。

---

## File Structure

- Modify: `crates/usecases/src/verification/fingerprint.rs` — `hash_verify_config` と `capabilities_from_descriptor` を pub にする。
- Modify: `crates/usecases/src/verification/mod.rs` — re-export 追加。
- Modify: `crates/usecases/src/service/verify.rs` — private `hash_verify_config` と private capabilities converter を上記 pub API に置き換え、重複コードを削除。
- Modify: `crates/usecases/src/submission_lifecycle.rs` — private `capabilities_of_starter` を pub `verification::fingerprint::capabilities_from_descriptor` 経由に差し替え。
- Modify: `crates/usecases/src/site_data_generator.rs`
  - `GenerateSiteData` に `pub starters: &'a StarterRegistry` を追加。
  - step 7 で `compute_current_fingerprints(...)` を呼ぶ実装を追加。
  - docstring から "plan 052" 参照を削除、新挙動を反映。
- Modify: `crates/interfaces/src/controller.rs::site_data_generate` — `starters: &StarterRegistry` を引数に追加、`GenerateSiteData` に渡す。
- Modify: `crates/infrastructure/src/shell/mod.rs` — `SiteData::Generate` 分岐で `controller.starter_registry()` 相当を取得して渡す。すでに `interfaces::Controller` は `service.starter_registry()` を保持しているので、`controller.site_data_generate(...)` に追加引数を回すのみ。
- Modify: `crates/infrastructure/tests/site_data_generate.rs` — 既存 2 テストは新フィールド `starters: &StarterRegistry::new()` を渡すだけ。新規テストで fingerprint 再計算 (`Ok` insert / `Verified`) と source 変更検知 (`Stale`) を検証。
- Modify: `docs/commands/site-data.md` — Status 節を書き換え、plan 052 参照を削除。
- Create: `docs/superpowers/plans/2026-08-17-104-current-fingerprints.md` (本ドキュメント)。

---

### Task 1: 共有ヘルパを `verification::fingerprint` に集約

**Files:**
- Modify: `crates/usecases/src/verification/fingerprint.rs`
- Modify: `crates/usecases/src/verification/mod.rs`
- Modify: `crates/usecases/src/service/verify.rs`
- Modify: `crates/usecases/src/submission_lifecycle.rs`

**Interfaces:**
- Produces:
  - `pub fn hash_verify_config(verify: &domain::solution::VerifySpec) -> domain::verification::ContentHash`
  - `pub fn capabilities_from_descriptor(descriptor: &crate::submission::SubmissionAdapterDescriptor) -> domain::online_judge::SubmissionCapabilities`
- Consumes (呼び側): `service/verify.rs::build_plan_context`, `submission_lifecycle.rs::capabilities_of_starter`

- [ ] **Step 1-1: `verification/fingerprint.rs` に pub helper を追加**

`hash_verify_config` は `service/verify.rs:987-999` の実装を移植する。動作は完全に等価:

```rust
// canonical JSON: {"libraries": sorted, "oj_language_id": ...} を SHA-256。
pub fn hash_verify_config(verify: &domain::solution::VerifySpec) -> ContentHash {
    let mut libs: Vec<String> = verify.libraries.iter().map(|l| l.to_string()).collect();
    libs.sort();
    let json = serde_json::json!({
        "libraries": libs,
        "oj_language_id": verify.oj_language_id,
    });
    let text = serde_json::to_string(&json).expect("serializes");
    let hex = sha256_hex(text.as_bytes());
    ContentHash::parse(&hex).expect("static hash")
}
```

`capabilities_from_descriptor` は `submission_lifecycle.rs:1123-1153` の変換ロジックを移植 (`SubmissionAdapterDescriptor` を直接受け取る形):

```rust
pub fn capabilities_from_descriptor(
    descriptor: &crate::submission::SubmissionAdapterDescriptor,
) -> SubmissionCapabilities { /* Port→Domain match */ }
```

- [ ] **Step 1-2: `verification/mod.rs` で re-export**

```rust
pub use fingerprint::{
    FINGERPRINT_SCHEMA_VERSION, FingerprintError, FingerprintMaterial, FingerprintSource,
    OjBinding, calculate_fingerprint, capabilities_from_descriptor, hash_verify_config,
    verification_closure,
};
```

- [ ] **Step 1-3: `service/verify.rs` の重複を削除**

- 冒頭 use を `hash_verify_config` と `capabilities_from_descriptor` を含めるよう追加。
- 900-917 の `let adapter = AdapterIdentity { ... capabilities: SubmissionCapabilities { ... } }` を `capabilities_from_descriptor(&d)` の呼び出しに置き換える。
- 987-999 の private `hash_verify_config` と 962-985 の private `map_*` を削除。

- [ ] **Step 1-4: `submission_lifecycle.rs::capabilities_of_starter` を薄いラッパに**

```rust
fn capabilities_of_starter(starter: &dyn SubmissionStarter) -> domain::online_judge::SubmissionCapabilities {
    crate::verification::fingerprint::capabilities_from_descriptor(&starter.descriptor())
}
```

- [ ] **Step 1-5: `cargo test -p usecases` で既存 fingerprint / verify 単体テストが通ることを確認**

```bash
cargo test -p usecases -- verification::fingerprint
cargo test -p usecases -- service::verify
```

Expected: 既存の pass 状態が維持される。

- [ ] **Step 1-6: `git add` して WIP commit**

```bash
git add crates/usecases/src/verification/fingerprint.rs \
        crates/usecases/src/verification/mod.rs \
        crates/usecases/src/service/verify.rs \
        crates/usecases/src/submission_lifecycle.rs
git commit -m "refactor(usecases): fingerprint 補助関数を公開して verify/site-data で共有"
```

---

### Task 2: `GenerateSiteData` に fingerprint 再計算を組み込む

**Files:**
- Modify: `crates/usecases/src/site_data_generator.rs`
- Test (integration): `crates/infrastructure/tests/site_data_generate.rs`

**Interfaces:**
- Consumes: Task 1 の `hash_verify_config`, `capabilities_from_descriptor`。`verification_closure`, `calculate_fingerprint`, `FingerprintMaterial`, `FingerprintSource`, `AdapterIdentity`, `OjBinding` は既に pub。
- Produces: `GenerateSiteData::starters: &'a StarterRegistry`, 新 private `fn compute_current_fingerprints(...) -> BTreeMap<SolutionId, Result<VerifyFingerprint, FingerprintError>>`

- [ ] **Step 2-1: 失敗テストを書く (source-changed → Stale, unchanged → Verified)**

`crates/infrastructure/tests/site_data_generate.rs` に新テストを追加。`FakeAnalyzer` を用意し、solution 1 個 + library 1 個。starter は `interfaces` 越しではなく test 内で `StubStarter` を定義:

```rust
struct StubStarter;
impl SubmissionStarter for StubStarter { /* descriptor = librarychecker/1.0.0/UnattendedTrackable/TestcaseDetails/BestEffort */ }

// (A) verify record.fingerprint == recomputed → status Verified
// (B) 別の source_hash になるよう source を書き換えて再計算 → Stale
```

record を先に verify pipeline と同じ材料で作って保存し、site-data 再計算で match を確認する形が確実。

具体的手順:
1. FakeAnalyzer が snapshot に solution + library を Complete 状態で返す。
2. LibraryProjectConfig, DiscoveryManifest を組む。
3. `verify_config_hash = hash_verify_config(&verify_spec)`, `binding`, `adapter` (StubStarter.descriptor()) を組んで `calculate_fingerprint` を呼び、期待 fingerprint を得る。
4. その fingerprint を CompletedState に載せた `VerificationRecord` を `verifications` 経由で提供 (fake 実装)。
5. `generate_site_data(&spec)` を呼び、`data.solutions[0].verification_status == Verified` を assert。
6. 同じセットアップで source ファイルの中身だけを 1 byte 変えて再度呼び、`Stale` に flip することを assert。

- [ ] **Step 2-2: `cargo test -p infrastructure -- site_data_generate` を実行して fail することを確認**

Expected: 新テストがコンパイル/実行に失敗 (`GenerateSiteData` に `starters` フィールドがない or verification 判定が Stale)。

- [ ] **Step 2-3: `GenerateSiteData` を拡張**

```rust
pub struct GenerateSiteData<'a> {
    // ... 既存
    pub starters: &'a crate::submission::StarterRegistry,
    pub mode: BuildMode,
}
```

- [ ] **Step 2-4: `compute_current_fingerprints` を実装**

`generate_site_data` の step 6 と step 8 の間に挿入。関数シグネチャ:

```rust
fn compute_current_fingerprints(
    manifest: &DiscoveryManifest,
    snapshot: &AnalysisSnapshot,
    starters: &StarterRegistry,
    library_sources: &BTreeMap<LibraryId, Vec<u8>>,
    solution_sources: &BTreeMap<SolutionId, Vec<u8>>,
) -> BTreeMap<SolutionId, Result<VerifyFingerprint, FingerprintError>>
```

処理:

1. `manifest.libraries` から `id -> source_path` の索引を作る (dependency_library_sources の path を埋めるため)。
2. 各 solution について:
   - `verify` が None なら map に入れない (classifier が NotConfigured を返す)。
   - `OJKind::detect(id.contest_id())` が None なら `FingerprintError::UnknownSolution` (sentinel) を Err で入れる。
   - `starters.get(&oj)` が Err なら同じく sentinel を Err で入れる (starter 未登録 = 判定不能 → Stale)。
   - `explicit: BTreeSet<LibraryId>` を `verify.libraries` から組む。
   - `verification_closure(...)` を呼び、`Err(e)` はそのまま Err として insert して次へ。
   - closure 内 library ごとに `library_sources` から bytes を取り、`FingerprintSource { path: <manifest 参照>, bytes }` を組む。ソース bytes が無ければ `FingerprintError::MissingLibrarySource(id)` を Err insert。
   - `solution_sources.get(id)` が None → `FingerprintError::MissingSolutionSource(id)` を Err insert。あれば `FingerprintSource { path: <root>/<entry>, bytes }`。
   - `adapter = AdapterIdentity { name, version, capabilities: capabilities_from_descriptor(&starter.descriptor()) }` (name/version は descriptor から)。
   - `binding = OjBinding { oj: oj.as_str().to_string(), problem_id: id.problem_code().to_string(), language_id: solution.language.clone(), oj_language_id: verify.oj_language_id.clone() }`。
   - `verify_config_hash = hash_verify_config(verify)`。
   - `verified_libraries = verify.libraries.iter().cloned().collect::<BTreeSet<_>>()`。
   - `FingerprintMaterial` を組んで `calculate_fingerprint(&material)` を呼ぶ。
   - 結果 (Ok/Err) をそのまま map に insert。

**プリプロセスは呼ばない** (site-data はオフライン。verify pipeline が preprocess hook を使う設定でも record と site-data 側でズレる可能性は spec §11 の再検証で解消する運用)。

- [ ] **Step 2-5: `generate_site_data` step 7 を書き換え**

- 現状 L115-120 のプレースホルダを削除。
- 上記 `compute_current_fingerprints(...)` の返り値を `current_fingerprints` に束縛。
- コメントを新実装に合わせて英語で書き直す (plan 052 参照を削除)。

- [ ] **Step 2-6: 既存 2 テスト (`end_to_end_pipeline_writes_projected_site_data`, `production_mode_rejects_uncommitted_tree`) に `starters: &StarterRegistry::new()` を追記**

solutions が空なので empty registry で十分。

- [ ] **Step 2-7: `cargo test -p infrastructure -- site_data_generate` が pass することを確認**

- [ ] **Step 2-8: WIP commit**

```bash
git add crates/usecases/src/site_data_generator.rs \
        crates/infrastructure/tests/site_data_generate.rs
git commit -m "feat(usecases): site-data 生成で current fingerprint を再計算"
```

---

### Task 3: controller / shell の配線と docs

**Files:**
- Modify: `crates/interfaces/src/controller.rs`
- Modify: `crates/infrastructure/src/shell/mod.rs`
- Modify: `docs/commands/site-data.md`
- Modify: `crates/usecases/src/site_data_generator.rs` (docstring)

- [ ] **Step 3-1: controller.rs を拡張**

`Controller::site_data_generate` に引数を足さない代わりに、`self.service.starter_registry()` から取得してそのまま `spec.starters` に渡す。これで shell 側のシグネチャ変更は最小になる。

- [ ] **Step 3-2: shell/mod.rs は変更不要 (controller 内解決) を確認**

`build_controller()` が既に starter registry を保持していれば OK。テスト時のダミー controller は既に `StarterRegistry::new()` を渡している。

- [ ] **Step 3-3: docs 更新**

- `docs/commands/site-data.md` の Status 節を書き換え: 「Current fingerprint recomputation for stale detection is now wired via `verification_closure` + `calculate_fingerprint` reusing the verify pipeline's helpers.」的な英語 (docs は英語ベースなのでそのまま英語で更新)。
- `plan 052` の言及を削除。
- `crates/usecases/src/site_data_generator.rs` の module docstring / step コメントも同様に更新。

- [ ] **Step 3-4: `cargo test --workspace` を実行**

```bash
cargo test --workspace
```

Expected: 全 pass。

- [ ] **Step 3-5: `cargo clippy --workspace --all-targets -- -D warnings`**

Expected: warnings ゼロ。

- [ ] **Step 3-6: smoke test `ce site-data generate --mode preview`**

```bash
cargo run -p compro-env -- site-data generate --mode preview --output target/ce-site-data
python3 -c 'import json,sys; d=json.load(open("target/ce-site-data/site-data.json")); s=[x for x in d["solutions"] if x["solution_id"]=="librarychecker-aplusb/aplusb/rust"][0]; print(s["verification_status"])'
```

Expected 出力: `verified`。

- [ ] **Step 3-7: 一括 commit + push + PR + Claude レビュー**

```bash
git add -A
git commit -m "docs: site-data の fingerprint 再計算に合わせて仕様を更新"
git push -u origin fix/104-site-data-fingerprints
```

skill://pr で Ready PR を作成し、skill://pr-review claude でレビューを依頼して対応する。

---

## Self-Review

**Spec coverage:**
- 仕様 §11 fingerprint schema → 既存の `calculate_fingerprint` を再利用するため schema 変更なし。
- 仕様 §10 status classification → `classify_solution_status` の `Ok(fp)/Err(e)` 動作をそのまま使用。
- `docs/commands/site-data.md` Status 節 → Task 3 で更新。
- `site_data_generator.rs` module docstring / step 7 → Task 2/3 で更新。

**Placeholder scan:** 「TODO」「あとで」「将来」なし。

**Type consistency:**
- `AdapterIdentity` は fingerprint 用 (name/version/capabilities) と site_schema DTO 用 (language/name/version) が同名別型。ambiguity を避けるため site_data_generator.rs では `crate::verification::fingerprint::AdapterIdentity` の完全パス or use alias で参照。
- `StarterRegistry` を `GenerateSiteData` に持つと `Default` 不能な借用参照になる — 既存 pattern (analyzer, verifications, git_history) と同じく `&'a` 参照でよい。
- `capabilities_from_descriptor` の入力は `&SubmissionAdapterDescriptor` (借用) にして所有権を握らない。
