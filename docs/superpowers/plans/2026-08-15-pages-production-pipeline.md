# Pages Production Pipeline Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the static test fixture in `pages.yml` with a real production pipeline that calls `ce site-data generate`. The live site at https://library.kisen.one/ currently shows hardcoded fixture content; after this plan it will reflect actual library files and verification results.

**Root cause:** `pages.yml` runs `npm run site:build` without first calling `ce site-data generate`. `site-build.mjs` defaults `CE_SITE_DATA_PATH` to `web/tests/fixtures/site-data.json`, a committed test fixture. Additionally, verification records from `automation/verify` have never been merged into `main` because the live verify ran on 2026-08-13 before plan 062 (`verify-pr-set-state` wiring) was merged.

**Architecture:**
- `ce site-data generate` reads library source files from the repo root and verification records from `root/verification/results/` (via `VerificationRepositoryImpl`).
- It requires the Rust adapter binary at `target/library-analyzers/bin/rust-analyzer` (built by `tools/library-analyzers/build`).
- The adapter build chain uses two cargo caches already present in `verify-worker.yml`; `pages.yml` can restore from the same keys without paying the cold-build cost when verify has run recently.
- Verification records live on `automation/verify`; pages overlays them via `git archive` before generating — secretless, no index pollution.

**Tech Stack:** `pages.yml` (GitHub Actions), `ce site-data generate`, existing adapter build toolchain.

## Global Constraints

- **Branch:** `feat/064-pages-production-pipeline`
- **Depends on:** plan 063 merged (current `main` state).
- Secretless: all new steps in `pages.yml` use only public refs and the `GITHUB_TOKEN` already present. No new secrets or environments.
- Do not modify `ce site-data generate` implementation or the web build scripts. Only `pages.yml` changes.
- The test fixture at `web/tests/fixtures/site-data.json` is kept as-is; it is still used by `npm test`.
- `npm run site:build` must receive `--fixture=target/ce-site-data/site-data.json` (the generated path); do not rely on `CE_SITE_DATA_PATH` being set implicitly.

---

### Task 1: Update `pages.yml` to run the production pipeline

**Files:**
- Modify: `.github/workflows/pages.yml`

The `build` job currently goes straight from `npm ci` to `npm run site:build`. Insert the steps below between `npm ci` and `npm run site:build`.

- [ ] **Step 1: Add Rust toolchain and `ce` build**

After the `Install dependencies` step, add:

```yaml
- name: Install Rust toolchain (pinned via rust-toolchain.toml)
  uses: dtolnay/rust-toolchain@4360b52568e2003a75bf9bc1d59f33a8e3fc893c  # stable @ 2026-08
  with:
    toolchain: 1.92.0

- name: Cache cargo registry and build artifacts
  uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9  # v6.1.0
  with:
    path: |
      ~/.cargo/registry
      ~/.cargo/git
      target
    key: ${{ runner.os }}-cargo-verify-${{ hashFiles('**/Cargo.lock') }}
    restore-keys: |
      ${{ runner.os }}-cargo-verify-
      ${{ runner.os }}-cargo-

- name: Build ce
  run: cargo build --release --bin ce
```

Use the **same** cache key as `verify.yml` (`cargo-verify-*`) so warm runs after a recent verify tick skip recompilation.

- [ ] **Step 2: Add adapter prepare/build (with shared cache)**

```yaml
- name: Cache prepared adapter dependencies
  uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9  # v6.1.0
  with:
    path: target/library-analyzers/prepared
    key: ${{ runner.os }}-analyzers-prepared-${{ hashFiles('tools/library-analyzers/dependencies.toml') }}

- name: Cache built analyzer binaries
  uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9  # v6.1.0
  with:
    path: |
      target/library-analyzers/builds
      target/library-analyzers/bin
    key: ${{ runner.os }}-analyzers-builds-${{ hashFiles('tools/library-analyzers/**', 'crates/library-adapter-protocol/**', 'crates/domain/src/adapter_build.rs', 'crates/domain/src/adapter_prepare.rs', 'crates/infrastructure/src/library_adapter/**', 'Cargo.lock', 'rust-toolchain.toml') }}

- name: Install analyzer build dependencies
  run: |
    sudo apt-get update
    sudo apt-get install -y --no-install-recommends cmake ninja-build

- name: Prepare library analyzer dependencies
  run: ./tools/library-analyzers/prepare

- name: Build library analyzer executables
  run: |
    set -euo pipefail
    if ./tools/library-analyzers/build --check; then
      echo "Warm cache: analyzer build already finalized."
      exit 0
    fi
    ./tools/library-analyzers/build
```

Copy the **same** cache keys and `--check` guard from `verify-worker.yml` (lines 160–200) verbatim so cache hits are shared across workflows.

- [ ] **Step 3: Overlay verification records from `automation/verify`**

```yaml
- name: Overlay verification records from automation/verify
  run: |
    set -euo pipefail
    REMOTE_REF=$(git ls-remote origin refs/heads/automation/verify)
    if [ -n "$REMOTE_REF" ]; then
      git fetch --depth=1 origin automation/verify
      git archive FETCH_HEAD verification/results/ | tar -x || true
      echo "Overlaid verification records from automation/verify."
    else
      echo "::warning::automation/verify branch not found; site-data will show no verification results."
    fi
```

Use `git ls-remote` (without `--exit-code`) in a standalone variable assignment. Under `set -euo pipefail`, a variable assignment `VAR=$(cmd)` aborts the job when `cmd` exits non-zero — so a network or auth failure propagates as a hard failure rather than silently falling to the else branch. `git ls-remote` without `--exit-code` exits 0 whether the branch exists or not, returning empty output when the ref is absent; this lets the else branch handle the expected "not yet created" case cleanly. Add `--depth=1` on `git fetch` to avoid pulling the full `automation/verify` history. `git archive … | tar -x` writes `verification/results/**` into the working tree without touching the index or other tracked files. The `|| true` on the archive tolerates an empty or absent `verification/results/` tree on the branch.

- [ ] **Step 4: Run `ce site-data generate` and update the build step**

Add after the overlay:

```yaml
- name: Generate site-data
  run: ./target/release/ce site-data generate --mode preview
```

Use `--mode preview` rather than the default `production`. Production mode calls `git status --porcelain` and exits non-zero if the working tree is dirty; the `tar -x` overlay in Step 3 writes untracked files into `verification/results/`, which always triggers that check. Preview mode skips the clean-tree assertion and is correct for CI contexts where the working tree is intentionally augmented. This writes to `target/ce-site-data/site-data.json` (the command's default output path).

Then update the existing `Build site (production pipeline)` step to pass the generated path:

```yaml
- name: Build site (production pipeline)
  env:
    CE_SITE_BASE: ${{ steps.pages_config.outputs.base_path }}
    CE_SITE_ORIGIN: ${{ steps.pages_config.outputs.origin }}
  run: npm run site:build -- --fixture=target/ce-site-data/site-data.json
```

- [ ] **Step 5: Commit the pages.yml change**

Invoke `/commit` with message:

```text
feat(064): generate real site-data in pages workflow
```

### Task 2: Restore the `automation/verify` → `main` PR flow

The live verify for `aplusb/rust` ran on 2026-08-13 before plan 062's `verify-pr-set-state` wiring was in place, so no automation PR was ever created. The `aplusb/rust` record is `Completed` with matching fingerprint; the picker correctly skips it, meaning `persist_terminal` (which calls `verify-pr-set-state`) will not run again automatically.

**This is an operational step, not a code change.**

- [ ] **Step 1: Force a re-verify of `aplusb/rust`**

Dispatch `verify.yml` manually:

```
mode: live
solution: librarychecker-aplusb/aplusb/rust
```

The dispatcher will bypass the picker (explicit `solution` wins) and invoke the full worker chain, including `persist_terminal` → `verify-pr-set-state`. An automation PR from `automation/verify` → `main` will be created (or its state set to `Ready` + auto-merge if already open).

- [ ] **Step 2: Verify the automation PR is created and auto-merges**

Watch for a PR titled something like `[bot] update verification records` in the repository. Confirm it is set to auto-merge and merges without conflict. After merge, `verification/results/librarychecker-aplusb/aplusb/rust.json` should appear on `main`.

- [ ] **Step 3: Confirm pages reflects verification status**

After the automation PR merges (triggering a Pages rebuild via the `push: main` hook), visit https://library.kisen.one/ and confirm the `aplusb/rust` solution shows `Accepted` verification status rather than the fixture content.

### Task 3: Smoke test and deliver the PR

- [ ] **Step 1: Run the new pages pipeline manually**

From the PR branch dispatch `pages` with `workflow_dispatch`. Confirm:
- The `Generate site-data` step exits 0.
- The built site contains the real library title (from `config.toml [library.site]`) rather than `"compro-env fixture"`.
- Verification results appear (if Task 2 has already completed; otherwise "never verified" badge is acceptable at this stage).

- [ ] **Step 2: Open and drive the PR**

Invoke `/pr --base main`. PR body links this plan.

Invoke `/pr-review` until Copilot returns no new comments. Merge only after CI is green.
