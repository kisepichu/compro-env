# Fingerprint raw-source Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Verify pipeline と site-data generator が同じ solution に対して同じ fingerprint を計算するように、`calculate_fingerprint` の hash 入力を **raw source bytes** (`[submit].preprocess` 適用前) に統一する。これにより preprocess を有効にした解法でも `Stale` badge が誤って出なくなる。

**Architecture:** `crates/usecases/src/verification/fingerprint.rs` の `FingerprintMaterial.submitted_source` を `raw_source` にリネームし、canonical JSON payload の key も `submitted_source_hash` → `raw_source_hash` に変更する。`crates/usecases/src/service/verify.rs::build_plan_context` は preprocess 前の `entry_bytes_raw` を fingerprint 用に、preprocess 後の `entry_bytes_post` を OJ 送信用 (`PlanBuildContext.submitted_source`) に分けて保持する。`crates/usecases/src/site_data_generator.rs::fingerprint_for_solution` は元々 preprocess しないので、変数名を `raw_source` に揃えるだけ。record schema の `submitted_source_hash` (OJ 送信 bytes の archival) はそのまま。

**Tech Stack:** Rust 4-layer DDD (`domain` / `usecases` / `interfaces` / `infrastructure`)。cargo workspace。fingerprint payload は既存の canonical JSON。preprocess は Unix でのみ実行。

## Global Constraints

- fingerprint hash 入力は **raw source bytes** (preprocess 前) に統一する。
- record schema の `submitted_source_hash` field と `PlanContext.submitted_source_hash` は変更しない (OJ 送信 bytes の archival として維持)。
- `SubmissionPlanBody.submitted_source_bytes` / `submitted_source_hash` は preprocess 後の bytes を保持する (OJ 送信の "wire content")。
- JSON payload の key rename に伴い、fingerprint schema version (`FINGERPRINT_SCHEMA_VERSION = 1`) は据え置きで、旧レコードは自動的に current fingerprint 再計算で mismatch し `Stale` になる。既存 record は 1 件だけ (`librarychecker-aplusb/aplusb/rust`) で、preprocess 導入後は既に Stale なので実害は同じ。マージ後に live verify を 1 回走らせて record を刷新する (PR 本文に明記する)。
- 実装、コメント、docs 追記は英語。spec.md / superpowers spec の既存日本語記述はそのまま日本語で維持。コミット・PR 本文・レビュー返信は日本語、emoji 禁止。
- 4-layer DDD の依存規則を守る (usecases が infrastructure を import しない)。

---

## File Structure

- **Modify** `crates/usecases/src/verification/fingerprint.rs`
  - `FingerprintMaterial.submitted_source: FingerprintSource` → `raw_source: FingerprintSource`
  - `calculate_fingerprint`: `material.raw_source.hash()`, JSON key `"raw_source_hash"`
  - Doc comment: fingerprint は raw source bytes を hash 対象にすることを明記
  - `tests` モジュール内の `material()` helper と各テストが使う field 名 `submitted_source:` → `raw_source:`、local var `submitted:` → `raw:`

- **Modify** `crates/usecases/src/service/verify.rs`
  - `build_plan_context`: `entry_bytes_raw` を preprocess 前に保持、preprocess 後を `entry_bytes_post` に rename
  - `raw_source = FingerprintSource { path: entry_rel.clone(), bytes: entry_bytes_raw }` を新規に構築 (material に渡す)
  - `submitted_source = FingerprintSource { path: entry_rel.clone(), bytes: entry_bytes_post }` は変更なし (`PlanBuildContext.submitted_source` に渡す。OJ 送信 bytes)
  - `FingerprintMaterial { ..., raw_source, ... }` に変更

- **Modify** `crates/usecases/src/site_data_generator.rs`
  - `fingerprint_for_solution`: 既存 local var `submitted_source` を `raw_source` に rename (意味は変わらない — site-data は preprocess しないので `solution_sources.get()` の返す生 bytes がそのまま raw source)
  - `FingerprintMaterial { ..., raw_source, ... }` に変更

- **Modify** `crates/infrastructure/tests/site_data_generate.rs`
  - `compute_expected_fingerprint` 内の `FingerprintMaterial` 構築部分 (line 722-738 付近) の field 名 `submitted_source:` → `raw_source:`

- **Modify** `crates/infrastructure/tests/verify_command.rs`
  - `verify_uses_project_local_preprocess_hook` の末尾に regression assertion を追加:
    persisted record の `fingerprint` と、raw entry 生 bytes から再計算した fingerprint が一致することを確認

- **Modify** `docs/commands/site-data.md`
  - 68-70 行目の "Preprocess hooks are not invoked ... legitimately read as Stale" 段落を差し替え

- **Modify** `docs/superpowers/specs/2026-08-10-library-platform-design.md`
  - 1493 行目 "preprocess 後の実提出ソース" → "生ソース bytes (`[submit].preprocess` 適用前)"
  - 1516-1535 の result JSON example (`inputs.submitted_source`) は archival 表現なので触らない

---

## Task 1: Rename `FingerprintMaterial.submitted_source` → `raw_source` (module local)

**Files:**
- Modify: `crates/usecases/src/verification/fingerprint.rs`
- Test: `crates/usecases/src/verification/fingerprint.rs` (同ファイル内の `#[cfg(test)]` mod)

**Interfaces:**
- Consumes: なし (最上流の rename)
- Produces:
  - `FingerprintMaterial { solution_id, raw_source, verified_libraries, dependency_library_sources, binding, adapter, verify_config_hash }`
  - `calculate_fingerprint(material)` は payload key `"raw_source_hash"` を使う。他の key はそのまま

- [ ] **Step 1: `FingerprintMaterial` の field 名を `raw_source` に変更**

`crates/usecases/src/verification/fingerprint.rs` の該当箇所:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintMaterial {
    pub solution_id: SolutionId,
    /// Raw solution source bytes as they exist on disk, before any
    /// `[submit].preprocess` transformation. Hashing the raw file (not the
    /// OJ-bound "wire content") keeps site-data — which is offline and
    /// never runs preprocess — able to reproduce the fingerprint that the
    /// verify pipeline stored on the record.
    pub raw_source: FingerprintSource,
    pub verified_libraries: BTreeSet<LibraryId>,
    pub dependency_library_sources: BTreeMap<LibraryId, FingerprintSource>,
    pub binding: OjBinding,
    pub adapter: AdapterIdentity,
    pub verify_config_hash: ContentHash,
}
```

- [ ] **Step 2: `calculate_fingerprint` 内の payload key を `"raw_source_hash"` に変更**

`crates/usecases/src/verification/fingerprint.rs` の `calculate_fingerprint`:

```rust
pub fn calculate_fingerprint(
    material: &FingerprintMaterial,
) -> Result<VerifyFingerprint, FingerprintError> {
    let raw_source_hash = material.raw_source.hash();
    let mut library_hashes: BTreeMap<String, String> = BTreeMap::new();
    for (id, source) in &material.dependency_library_sources {
        library_hashes.insert(id.to_string(), source.hash().to_string());
    }
    for lib in &material.verified_libraries {
        if !material.dependency_library_sources.contains_key(lib) {
            return Err(FingerprintError::MissingLibrarySource(lib.clone()));
        }
    }

    let payload = serde_json::json!({
        "schema_version": FINGERPRINT_SCHEMA_VERSION,
        "solution_id": material.solution_id.as_str(),
        "raw_source_hash": raw_source_hash.as_str(),
        "verified_libraries": material
            .verified_libraries
            .iter()
            .map(|l| l.as_str().to_string())
            .collect::<Vec<_>>(),
        "library_hashes": library_hashes,
        "verify_config_hash": material.verify_config_hash.as_str(),
        "binding": {
            "oj": material.binding.oj,
            "problem_id": material.binding.problem_id,
            "language_id": material.binding.language_id.as_str(),
            "oj_language_id": material.binding.oj_language_id,
        },
        "adapter": {
            "name": material.adapter.name,
            "version": material.adapter.version,
            "capabilities": material.adapter.capabilities,
        },
    });
    let bytes = canonical_json(&payload);
    let hex = sha256_hex(&bytes);
    Ok(VerifyFingerprint::parse(&hex).expect("sha256_hex emits a valid fingerprint string"))
}
```

- [ ] **Step 3: `tests` mod の `material()` helper と関連 test の field 名を追随**

`crates/usecases/src/verification/fingerprint.rs` の `tests` モジュール、`material()` helper のパラメータ名を `raw:` に、struct 構築の field 名を `raw_source:` に変える。以降の呼び出し側 (`fingerprint_uses_source_bytes_without_newline_normalization` など) は helper 経由なので影響なし。

```rust
    fn material(
        raw: FingerprintSource,
        verified: Vec<&str>,
        deps: Vec<FingerprintSource>,
    ) -> FingerprintMaterial {
        let mut verified_libs: BTreeSet<LibraryId> = BTreeSet::new();
        for v in verified {
            verified_libs.insert(LibraryId::parse(v).unwrap());
        }
        let mut sources = BTreeMap::new();
        for s in deps {
            sources.insert(LibraryId::parse(&s.path).unwrap(), s);
        }
        FingerprintMaterial {
            solution_id: SolutionId::parse("abc999/a/main").unwrap(),
            raw_source: raw,
            verified_libraries: verified_libs,
            dependency_library_sources: sources,
            binding: binding(),
            adapter: adapter(),
            verify_config_hash: config_hash(),
        }
    }
```

- [ ] **Step 4: unit test 実行**

Run: `cargo test -p usecases --lib verification::fingerprint`
Expected: 全 test PASS (assert_eq!/assert_ne! は helper 経由で入力が対称なので fingerprint 値が変わっても壊れない)。

- [ ] **Step 5: この時点ではまだ他 caller をコンパイル通していない**

`cargo build -p usecases --lib` は Task 2/3 完了まで壊れているので実行しない。もし Step 4 でユニット test が単独で通ったら、この rename は完結。

- [ ] **Step 6: Commit**

```bash
git add crates/usecases/src/verification/fingerprint.rs
git commit -m "refactor(verify): FingerprintMaterial.submitted_source を raw_source にリネーム

fingerprint hash 入力を \"preprocess 後の submission bytes\" から
\"raw source bytes (preprocess 適用前)\" に一本化するリファクタ。
canonical JSON payload の key も submitted_source_hash → raw_source_hash に変更。
site-data (offline / preprocess 実行しない) と verify pipeline が同じ
fingerprint を再現できるようにする第一歩。"
```

---

## Task 2: `build_plan_context` で raw_source を分離して material に渡す

**Files:**
- Modify: `crates/usecases/src/service/verify.rs:815-948`

**Interfaces:**
- Consumes: `FingerprintMaterial { ..., raw_source, ... }` (Task 1)
- Produces: 引き続き `PlanBuildContext.submitted_source` は preprocess 後の bytes (OJ 送信用)。fingerprint は raw source bytes を hash 対象とする

- [ ] **Step 1: preprocess 前後を明示的な variable で分ける**

`crates/usecases/src/service/verify.rs::build_plan_context` の該当ブロック (現行 line 860-932 相当) を次のように書き換える:

```rust
    // Read the solution's entry file. `entry_bytes_raw` participates in
    // the fingerprint below; `entry_bytes_post` is the wire content sent
    // to the OJ (identical to the raw bytes when no preprocess hook is
    // configured). Splitting them keeps site-data — which is offline and
    // never runs preprocess — able to recompute the same fingerprint that
    // the verify pipeline stored on the record.
    let mut entry_rel = published.root.clone();
    if !entry_rel.ends_with('/') {
        entry_rel.push('/');
    }
    entry_rel.push_str(&published.entry);
    let entry_bytes_raw = std::fs::read(repository_root.join(&entry_rel))
        .map_err(|e| format!("failed to read solution entry {entry_rel}: {e}"))?;

    // Run the global preprocess hook if configured (Unix only). The hook
    // never influences the fingerprint — its output is the wire content
    // only.
    #[cfg(unix)]
    let entry_bytes_post = match submit_preprocess {
        Some(cmd) if !cmd.trim().is_empty() => {
            match run_preprocess(
                cmd,
                &entry_bytes_raw,
                solution_id,
                &published.language,
                &oj,
                repository_root,
                &entry_rel,
            ) {
                Ok(out) => out,
                Err(e) => return Err(format!("preprocess hook failed: {e}")),
            }
        }
        _ => entry_bytes_raw.clone(),
    };
    #[cfg(not(unix))]
    let entry_bytes_post = {
        let _ = submit_preprocess;
        entry_bytes_raw.clone()
    };

    let raw_source = FingerprintSource {
        path: entry_rel.clone(),
        bytes: entry_bytes_raw,
    };
    let submitted_source = FingerprintSource {
        path: entry_rel.clone(),
        bytes: entry_bytes_post,
    };
```

- [ ] **Step 2: `FingerprintMaterial` の初期化を新 field 名に合わせる**

同関数末尾の material 構築 (現行 line 924-932 相当):

```rust
    let material = FingerprintMaterial {
        solution_id: solution_id.clone(),
        raw_source,
        verified_libraries,
        dependency_library_sources,
        binding: ojb,
        adapter,
        verify_config_hash,
    };
```

`PlanBuildContext.submitted_source` にはそのまま `submitted_source` を返す (変更なし):

```rust
    Ok(PlanBuildContext {
        oj,
        binding,
        submitted_source,
        verify_libraries,
        fingerprint,
    })
```

- [ ] **Step 3: build 確認**

Run: `cargo build -p usecases --lib`
Expected: `site_data_generator.rs` の rename 未対応で 1 箇所 error のみ (raw_source field 名 mismatch)。それ以外の error は Step 1/2 の書き換え漏れを示すので直す。

- [ ] **Step 4: Commit**

```bash
git add crates/usecases/src/service/verify.rs
git commit -m "refactor(verify): build_plan_context を raw_source と wire content で分離

entry_bytes_raw (preprocess 前) を fingerprint 計算に、entry_bytes_post
(preprocess 後) を OJ 送信 body に、それぞれ別 variable で保持する。
FingerprintMaterial には raw_source を渡し、PlanBuildContext.submitted_source
(OJ 送信 bytes) は preprocess 後のまま。"
```

---

## Task 3: `site_data_generator::fingerprint_for_solution` の local var rename

**Files:**
- Modify: `crates/usecases/src/site_data_generator.rs:338-374`

**Interfaces:**
- Consumes: `FingerprintMaterial { ..., raw_source, ... }` (Task 1)
- Produces: 変更なし (semantic は元々 raw source を渡していた)

- [ ] **Step 1: local var と material 初期化を rename**

`crates/usecases/src/site_data_generator.rs::fingerprint_for_solution` の該当ブロック:

```rust
    let entry_path = solution_entry_path(sol);
    let entry_bytes = solution_sources
        .get(&sol.id)
        .ok_or_else(|| FingerprintError::MissingSolutionSource(sol.id.clone()))?
        .clone();
    // site-data is offline and never runs preprocess, so the working-tree
    // bytes are already the raw source that the verify pipeline hashed.
    let raw_source = FingerprintSource {
        path: entry_path,
        bytes: entry_bytes,
    };
```

material 初期化:

```rust
    let material = FingerprintMaterial {
        solution_id: sol.id.clone(),
        raw_source,
        verified_libraries,
        dependency_library_sources,
        binding,
        adapter,
        verify_config_hash,
    };
    calculate_fingerprint(&material)
```

- [ ] **Step 2: build 確認**

Run: `cargo build -p usecases --lib`
Expected: exit 0 (usecases crate 内の rename 完了)。

- [ ] **Step 3: Commit**

```bash
git add crates/usecases/src/site_data_generator.rs
git commit -m "refactor(site-data): fingerprint_for_solution の submitted_source を raw_source にリネーム

意味は変わらない (site-data は preprocess を実行しないので working tree の
bytes = raw source)。FingerprintMaterial の field 名変更 (Task 1) に追随。"
```

---

## Task 4: infrastructure テストの `FingerprintMaterial` 構築を追随

**Files:**
- Modify: `crates/infrastructure/tests/site_data_generate.rs:701-740`

**Interfaces:**
- Consumes: `FingerprintMaterial { ..., raw_source, ... }` (Task 1)

- [ ] **Step 1: `compute_expected_fingerprint` の material 構築を rename**

`crates/infrastructure/tests/site_data_generate.rs` の該当箇所 (line 722-738):

```rust
        let material = FingerprintMaterial {
            solution_id: sol.id.clone(),
            raw_source: FingerprintSource {
                path: format!("{}/{}", sol.root, sol.entry),
                bytes: entry_bytes.to_vec(),
            },
            verified_libraries: verify.libraries.iter().cloned().collect(),
            dependency_library_sources: dep_sources,
            binding: OjBinding {
                oj: OJKind::LibraryChecker.as_str().to_string(),
                problem_id: sol.id.problem_code().to_string(),
                language_id: sol.language.clone(),
                oj_language_id: verify.oj_language_id.clone(),
            },
            adapter,
            verify_config_hash: hash_verify_config(verify),
        };
```

- [ ] **Step 2: test 実行**

Run: `cargo test -p infrastructure --test site_data_generate`
Expected: 全 test PASS。fingerprint 値は verify pipeline 側と一致するので既存 assertion は継続して通る。

- [ ] **Step 3: Commit**

```bash
git add crates/infrastructure/tests/site_data_generate.rs
git commit -m "test(site-data): FingerprintMaterial 構築を raw_source に追随

fingerprint schema の field 名変更 (raw_source) に合わせて既存 test の
material 構築を更新。fingerprint 値の期待値は変わらない
(site-data は元々 raw source を渡していた)。"
```

---

## Task 5: 回帰テスト — preprocess ありでも stored fingerprint が raw source と一致

**Files:**
- Modify: `crates/infrastructure/tests/verify_command.rs:1419-1567` (既存 `verify_uses_project_local_preprocess_hook` を拡張、または隣接した新 test を追加)

**Interfaces:**
- Consumes: `calculate_fingerprint`, `FingerprintMaterial`, `FingerprintSource`, `AdapterIdentity`, `OjBinding`, `hash_verify_config`, `capabilities_from_descriptor` (すべて既に `pub` — Task 1 で変更なし)

- [ ] **Step 1: 失敗する assertion を最初に書く**

`crates/infrastructure/tests/verify_command.rs` の `verify_uses_project_local_preprocess_hook` 末尾 (`drop(tmp);` の直前) に次を追加:

```rust
    // Regression: fingerprint は preprocess 前の raw source bytes を hash する。
    // preprocess hook が source bytes を書き換えても、record の fingerprint は
    // repository の生 entry 内容から再計算した値と一致し、site-data の
    // fingerprint_for_solution も同じ値を出せる (これが Stale 誤判定の直接原因)。
    let record = infrastructure::verification_repository_impl::VerificationRepositoryImpl::new(
        root.clone(),
    )
    .load(&lc_id())
    .unwrap()
    .expect("record persisted for the preprocessed solution");

    // raw file bytes を実ディスクから読み、closure library と合わせて
    // FingerprintMaterial を組み立てる。CapturingStarter の descriptor を
    // 再利用して adapter identity を揃える。
    let solution = manifest
        .solutions
        .iter()
        .find(|s| s.id == lc_id())
        .expect("librarychecker solution present in manifest");
    let verify = solution
        .verify
        .as_ref()
        .expect("librarychecker solution has [verify] block");
    let entry_path = format!("{}/{}", solution.root, solution.entry);
    let entry_bytes = std::fs::read(root.join(&entry_path)).expect("read raw entry bytes");

    let mut dep_sources: BTreeMap<LibraryId, FingerprintSource> = BTreeMap::new();
    for lib in &verify.libraries {
        let lib_path = manifest
            .libraries
            .iter()
            .find(|l| &l.id == lib)
            .map(|l| l.source_path.clone())
            .expect("verify library present in manifest");
        let bytes = std::fs::read(root.join(&lib_path)).expect("read library bytes");
        dep_sources.insert(
            lib.clone(),
            FingerprintSource {
                path: lib_path,
                bytes,
            },
        );
    }

    let descriptor = SubmissionAdapterDescriptor {
        name: "capture-lc".into(),
        version: "1".into(),
        submission_mode: SubmissionMode::UnattendedTrackable,
        result_detail: ResultDetailLevel::TestcaseDetails,
        recovery_mode: RecoveryMode::BestEffort,
    };
    let material = FingerprintMaterial {
        solution_id: lc_id(),
        raw_source: FingerprintSource {
            path: entry_path,
            bytes: entry_bytes,
        },
        verified_libraries: verify.libraries.iter().cloned().collect(),
        dependency_library_sources: dep_sources,
        binding: OjBinding {
            oj: OJKind::LibraryChecker.as_str().to_string(),
            problem_id: lc_id().problem_code().to_string(),
            language_id: solution.language.clone(),
            oj_language_id: verify.oj_language_id.clone(),
        },
        adapter: AdapterIdentity {
            name: descriptor.name.clone(),
            version: descriptor.version.clone(),
            capabilities: capabilities_from_descriptor(&descriptor),
        },
        verify_config_hash: hash_verify_config(verify),
    };
    let recomputed = calculate_fingerprint(&material).expect("fingerprint recomputes");
    assert_eq!(
        record.fingerprint, recomputed,
        "stored fingerprint must equal fingerprint recomputed from raw source \
         bytes; preprocess hook must not enter the hash input"
    );
```

必要な use 節を先頭にまとめて追加 (既存のものと重複しないように merge する):

```rust
use domain::library::LibraryId;
use interfaces::submission::SubmissionAdapterDescriptor;
use usecases::verification::fingerprint::{
    AdapterIdentity, FingerprintMaterial, FingerprintSource, OjBinding,
    calculate_fingerprint, capabilities_from_descriptor, hash_verify_config,
};
use usecases::verification_repository::VerificationRepository;
```

(既存の `use` と混ぜて重複が出ないよう、テスト作成時に file 全体の use を確認する。実際の crate path は `crates/interfaces/src/submission.rs` や既存の import と揃える。)

- [ ] **Step 2: test 単体を実行**

Run: `cargo test -p infrastructure --test verify_command verify_uses_project_local_preprocess_hook`
Expected: PASS。

- [ ] **Step 3: 回帰性の確認 (red-green)**

`crates/usecases/src/verification/fingerprint.rs::calculate_fingerprint` の `material.raw_source.hash()` を `material.dependency_library_sources.values().next().map(|s| s.hash()).unwrap_or_else(|| material.raw_source.hash())` のような偽実装に一時的に差し替えて実行し、追加した assertion が FAIL することを確認。確認後 revert。

Run (偽実装): `cargo test -p infrastructure --test verify_command verify_uses_project_local_preprocess_hook`
Expected: FAIL (recomputed fingerprint mismatch)。

Revert 後:
Run: `cargo test -p infrastructure --test verify_command verify_uses_project_local_preprocess_hook`
Expected: PASS。

- [ ] **Step 4: Commit**

```bash
git add crates/infrastructure/tests/verify_command.rs
git commit -m "test(verify): preprocess ありでも record.fingerprint が raw source と一致する回帰テスト

verify_uses_project_local_preprocess_hook の末尾で、persisted record の
fingerprint と repository 上の raw entry bytes から再計算した fingerprint が
一致することを確認する。preprocess hook が hash 入力に混入していたら FAIL
する — これが librarychecker-aplusb/aplusb/rust の Stale 誤判定バグの直接
回帰テスト。"
```

---

## Task 6: docs 更新

**Files:**
- Modify: `docs/commands/site-data.md:68-70`
- Modify: `docs/superpowers/specs/2026-08-10-library-platform-design.md:1493`

**Interfaces:**
- なし (docs のみ)

- [ ] **Step 1: `docs/commands/site-data.md` の該当段落を差し替え**

`docs/commands/site-data.md:68-70` の既存段落:

```
Preprocess hooks are not invoked during recomputation — site-data is
offline — so records persisted with a source-mutating `[submit].preprocess`
hook can legitimately read as `Stale` until the source is re-verified.
```

を次に差し替える:

```
Preprocess hooks are intentionally never part of the fingerprint. Both
the verify pipeline and site-data hash the *raw* on-disk source bytes
(preprocess-free), so a source-mutating `[submit].preprocess` hook does
not shift the fingerprint away from what site-data recomputes.
```

- [ ] **Step 2: spec §11 の fingerprint 入力を差し替え**

`docs/superpowers/specs/2026-08-10-library-platform-design.md:1493` の既存行:

```
- preprocess 後の実提出ソース
```

を次に差し替える:

```
- 解法の生ソース (`[submit].preprocess` 適用前の on-disk bytes)
```

result JSON example (line 1528-1534) の `inputs.submitted_source` は record の archival 表現なので触らない。

- [ ] **Step 3: Commit**

```bash
git add docs/commands/site-data.md docs/superpowers/specs/2026-08-10-library-platform-design.md
git commit -m "docs: fingerprint 入力を raw source に統一 (site-data と verify の対称性)

- site-data.md: preprocess が Stale を招く既知制限を削除し、raw source を
  両側で hash する統一仕様を明記
- spec §11: fingerprint 入力を \"preprocess 後の実提出ソース\" から
  \"生ソース (preprocess 適用前)\" に修正
"
```

---

## Task 7: 全体検証

**Files:**
- なし (verification 実行のみ)

**Interfaces:**
- なし

- [ ] **Step 1: workspace 全 test を実行**

Run: `cargo test --workspace`
Expected: 全 test PASS。fingerprint 値変更に伴う fixture 更新は最小に留める。以下の fixture / test は `submitted_source_hash` 文字列を含むが、これは **record schema の field 名** (record 内 archival hash) であって fingerprint JSON payload の key ではないため、値も key も変更しない (触ると誤修正になる):
  - `crates/domain/tests/fixtures/verification/accepted.json`
  - `crates/domain/tests/fixtures/verification/rejected.json`
  - `crates/infrastructure/tests/fixtures/library-platform/verification/accepted.json`
  - `crates/infrastructure/tests/fixtures/verification/accepted.json`
  - `crates/infrastructure/tests/fixtures/verification/stale-attempt.json`

もし fingerprint 値を hardcode しているテストがあれば actual に差し替える (値は前後で異なる:`calculate_fingerprint` の payload key が変わるため)。

- [ ] **Step 2: workspace clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: warnings なし、exit 0。

- [ ] **Step 3: hooks 単体 test**

Run: `bash hooks/tests/run.sh`
Expected: PASS (影響なし想定)。

- [ ] **Step 4: 追加コミット (もし fixture 修正があれば)**

必要なら:

```bash
git add <fixture files>
git commit -m "test: fingerprint 値の期待値を更新 (raw_source_hash payload key rename に追随)"
```

修正が無ければこのステップは skip。

---

## PR 手順 (実装完了後)

skill://finishing-a-development-branch と skill://pr に従って `develop` (または該当リリースブランチ) 向けに PR を作成する。PR body に次を必ず含める (日本語、emoji 禁止):

- 概要: fingerprint hash 入力を raw source bytes に統一 (site-data と verify pipeline で対称性を回復)
- 変更対象: `FingerprintMaterial.raw_source` rename、JSON payload key `raw_source_hash` rename、`build_plan_context` の raw/post 分離、docs 更新
- 影響: 既存 record は new key で計算した値と mismatch し `Stale` になる。該当 record は `librarychecker-aplusb/aplusb/rust` の 1 件のみ (preprocess 導入で既に Stale)。
- **マージ後アクション (必須)**: `gh workflow run verify.yml -f mode=live -f solution=librarychecker-aplusb/aplusb/rust` を叩いて record を刷新すること (orchestrator 側で実行)

PR 作成後 skill://pr-review claude で Claude レビューを回す。5 巡以上 nit のみで空転する兆候があれば `ask` で確認する。critical でない指摘は blockquote で理由付きの push back OK。
