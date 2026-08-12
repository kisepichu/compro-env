//! Policy tests for the dormant safe-automation workflows (spec §15).
//!
//! Every check here is a static invariant that must hold whenever the
//! workflow files change. The tests parse the YAML themselves rather than
//! trusting GitHub Actions' `paths:` or environment-name matching, since
//! that path is what §15.3 explicitly bypasses (workflows are bypassable;
//! Rust classification isn't).

use std::path::{Path, PathBuf};

use serde_yaml::Value;

const SECRET_ENVIRONMENTS: [&str; 2] = ["oj-library-checker", "verify-state"];
const SHA_PIN_RE_LEN: usize = 40;

// ─── Fixtures ────────────────────────────────────────────────────────────────

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points at crates/infrastructure/.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("workspace root must be two levels up from CARGO_MANIFEST_DIR")
        .to_path_buf()
}

fn workflow_dir() -> PathBuf {
    workspace_root().join(".github").join("workflows")
}

fn load(path: &Path) -> Value {
    let src = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_yaml::from_str::<Value>(&src)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
}

fn load_worker() -> Value {
    load(&workflow_dir().join("verify-worker.yml"))
}

fn load_integrity() -> Value {
    load(&workflow_dir().join("verify-result-integrity.yml"))
}

fn load_ci() -> Value {
    load(&workflow_dir().join("ci.yml"))
}

fn load_pages() -> Value {
    load(&workflow_dir().join("pages.yml"))
}

fn load_dispatcher() -> Value {
    load(&workflow_dir().join("verify.yml"))
}

/// Read `.node-version` (Node patch pin per spec §12.15).
fn node_version_pin() -> String {
    let path = workspace_root().join(".node-version");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
        .trim()
        .to_string()
}

/// Read the Rust channel pin from `rust-toolchain.toml`.
fn rust_toolchain_channel() -> String {
    let path = workspace_root().join("rust-toolchain.toml");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    for line in src.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("channel") {
            // channel = "1.92.0"
            let value = rest
                .trim_start_matches(|c: char| c == '=' || c.is_whitespace())
                .trim_matches('"');
            return value.to_string();
        }
    }
    panic!("rust-toolchain.toml missing channel pin");
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn as_map<'a>(v: &'a Value, label: &str) -> &'a serde_yaml::Mapping {
    v.as_mapping()
        .unwrap_or_else(|| panic!("{label}: expected a mapping, got {v:?}"))
}

fn get<'a>(map: &'a serde_yaml::Mapping, key: &str) -> Option<&'a Value> {
    map.get(&Value::String(key.to_string()))
}

fn jobs(v: &Value) -> &serde_yaml::Mapping {
    let root = as_map(v, "workflow root");
    let jobs = get(root, "jobs").expect("workflow: missing jobs key");
    as_map(jobs, "jobs")
}

fn is_secret_job(job: &Value) -> bool {
    let map = match job.as_mapping() {
        Some(m) => m,
        None => return false,
    };
    match get(map, "environment").and_then(Value::as_str) {
        Some(name) => SECRET_ENVIRONMENTS.contains(&name),
        None => false,
    }
}

fn steps(job: &Value) -> Vec<&Value> {
    let map = match job.as_mapping() {
        Some(m) => m,
        None => return vec![],
    };
    match get(map, "steps").and_then(Value::as_sequence) {
        Some(seq) => seq.iter().collect(),
        None => vec![],
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

/// #1: `pull_request_target` is banned everywhere. It grants an attacker-authored
/// PR access to secrets; §15.4 forbids it outright.
#[test]
fn no_pull_request_target_anywhere() {
    for (label, doc) in [
        ("verify-worker", load_worker()),
        ("verify-result-integrity", load_integrity()),
    ] {
        let on =
            get(as_map(&doc, label), "on").unwrap_or_else(|| panic!("{label}: missing `on:` key"));
        let contains_target = match on {
            Value::Mapping(m) => m.contains_key(Value::String("pull_request_target".into())),
            Value::String(s) => s == "pull_request_target",
            Value::Sequence(seq) => seq
                .iter()
                .any(|v| v.as_str() == Some("pull_request_target")),
            _ => false,
        };
        assert!(
            !contains_target,
            "{label}: `on:` must not contain pull_request_target (spec §15.4)"
        );
    }
}

/// #2: Every job wearing a secret environment must be gated to the `main` branch,
/// unless the entire workflow is `workflow_call`-only (the caller is expected
/// to gate on its side, which the caller-inventory test enforces separately).
#[test]
fn secret_jobs_gated_to_main() {
    for (label, doc) in [
        ("verify-worker", load_worker()),
        ("verify-result-integrity", load_integrity()),
    ] {
        let root = as_map(&doc, label);
        let on = get(root, "on").expect("missing `on:`");
        let workflow_call_only = matches!(on, Value::Mapping(m) if m.len() == 1 && m.contains_key(Value::String("workflow_call".into())));

        for (job_name, job) in jobs(&doc) {
            if !is_secret_job(job) {
                continue;
            }
            let map = as_map(job, "job");
            let if_expr = get(map, "if").and_then(Value::as_str).unwrap_or("");
            let gated = workflow_call_only
                || if_expr.contains("github.ref == 'refs/heads/main'")
                || if_expr.trim() == "false";
            assert!(
                gated,
                "{label}: job {job_name:?} carries a secret environment but is not gated to main (if: {if_expr:?})"
            );
        }
    }
}

/// #3: Any `uses:` referenced from a secret-bearing job must be pinned to a
/// full 40-char SHA. Version-tag pinning is a supply-chain footgun (§15.4).
#[test]
fn third_party_actions_are_sha_pinned() {
    for (label, doc) in [
        ("verify-worker", load_worker()),
        ("verify-result-integrity", load_integrity()),
    ] {
        for (job_name, job) in jobs(&doc) {
            if !is_secret_job(job) {
                continue;
            }
            for (idx, step) in steps(job).iter().enumerate() {
                let uses = match step.as_mapping().and_then(|m| get(m, "uses")) {
                    Some(Value::String(s)) => s.clone(),
                    _ => continue,
                };
                let parts: Vec<&str> = uses.rsplitn(2, '@').collect();
                assert_eq!(
                    parts.len(),
                    2,
                    "{label}: job {job_name:?} step[{idx}]: uses {uses:?} has no @ref"
                );
                let sha = parts[0];
                assert_eq!(
                    sha.len(),
                    SHA_PIN_RE_LEN,
                    "{label}: job {job_name:?} step[{idx}]: uses {uses:?} is not SHA-pinned"
                );
                assert!(
                    sha.chars().all(|c| c.is_ascii_hexdigit()),
                    "{label}: job {job_name:?} step[{idx}]: uses {uses:?} ref is not hex"
                );
            }
        }
    }
}

/// #4: Secret-bearing jobs must declare `permissions:` as a map containing
/// only `contents: read`. Anything else could be abused to escalate.
#[test]
fn secret_jobs_have_minimum_permissions() {
    let disallowed_keys = [
        "packages",
        "pages",
        "id-token",
        "deployments",
        "actions",
        "checks",
        "administration",
        "issues",
        "statuses",
    ];
    for (label, doc) in [
        ("verify-worker", load_worker()),
        ("verify-result-integrity", load_integrity()),
    ] {
        for (job_name, job) in jobs(&doc) {
            if !is_secret_job(job) {
                continue;
            }
            let map = as_map(job, "job");
            let perms = get(map, "permissions")
                .unwrap_or_else(|| panic!("{label}: job {job_name:?} has no permissions block"));
            let perms_map = as_map(perms, "permissions");
            assert_eq!(
                perms_map.len(),
                1,
                "{label}: job {job_name:?} permissions must have exactly one key, got {perms_map:?}"
            );
            let contents = get(perms_map, "contents").and_then(Value::as_str);
            assert_eq!(
                contents,
                Some("read"),
                "{label}: job {job_name:?} permissions.contents must be `read`"
            );
            for key in disallowed_keys {
                assert!(
                    !perms_map.contains_key(Value::String(key.into())),
                    "{label}: job {job_name:?} permissions must not set {key:?}"
                );
            }
        }
    }
}

/// #5: Every `environment:` used anywhere must be one of the two names in the
/// allowlist. A stray environment name would silently opt a job into a
/// different secret store.
#[test]
fn environment_names_are_from_allowlist() {
    for (label, doc) in [
        ("verify-worker", load_worker()),
        ("verify-result-integrity", load_integrity()),
    ] {
        for (job_name, job) in jobs(&doc) {
            let map = as_map(job, "job");
            if let Some(env) = get(map, "environment").and_then(Value::as_str) {
                assert!(
                    SECRET_ENVIRONMENTS.contains(&env),
                    "{label}: job {job_name:?} environment {env:?} is not in the allowlist"
                );
            }
        }
    }
}

/// #6 (plan 062): Secret-bearing jobs must not rebuild `ce`. The OJ / App
/// jobs execute only the pinned `ce` binary artifact — cargo would compile
/// arbitrary post-merge code with secrets in scope.
///
/// Secret jobs may `actions/checkout` a fixed, immutable ref so `ce`'s
/// runtime helpers (`find_project_root`, `SolutionRepository`) can operate.
/// The allowed refs are:
///   - `${{ inputs.after }}`: the immutable SHA `prepare` planned against.
///   - `automation/verify`: the App-managed state branch, which `submit`
///     and `poll` read so `ce internal verify-{start,poll}` can see the
///     record `persist_starting` / `persist_handle` just committed.
/// The `actions/checkout` SHA pin is covered by test #3.
#[test]
fn no_build_or_unpinned_checkout_in_secret_jobs() {
    let banned_cargo_verbs = ["cargo build", "cargo test", "cargo run"];
    let allowed_ref_needles = ["inputs.after", "automation/verify"];
    for (label, doc) in [
        ("verify-worker", load_worker()),
        ("verify-result-integrity", load_integrity()),
    ] {
        for (job_name, job) in jobs(&doc) {
            if !is_secret_job(job) {
                continue;
            }
            for (idx, step) in steps(job).iter().enumerate() {
                let map = match step.as_mapping() {
                    Some(m) => m,
                    None => continue,
                };
                if let Some(uses) = get(map, "uses").and_then(Value::as_str)
                    && uses.starts_with("actions/checkout@")
                {
                    let with = get(map, "with")
                        .and_then(Value::as_mapping)
                        .unwrap_or_else(|| {
                            panic!(
                                "{label}: job {job_name:?} step[{idx}] actions/checkout must set `with:` (ref pin required)"
                            )
                        });
                    let refv = get(with, "ref").and_then(Value::as_str);
                    let matches = refv.is_some_and(|s| {
                        allowed_ref_needles.iter().any(|needle| s.contains(needle))
                    });
                    assert!(
                        matches,
                        "{label}: job {job_name:?} step[{idx}] actions/checkout `ref:` must \
                         be one of `${{{{ inputs.after }}}}` or `automation/verify`, got {refv:?}"
                    );
                }
                if let Some(run) = get(map, "run").and_then(Value::as_str) {
                    for verb in banned_cargo_verbs {
                        assert!(
                            !run.contains(verb),
                            "{label}: job {job_name:?} step[{idx}] runs {run:?} — secret jobs must not invoke {verb}"
                        );
                    }
                }
            }
        }
    }
}

/// #7: No job may carry both OJ and App credentials. In this dormant iteration
/// we assert it by (a) each job has at most one `environment:` and (b) no
/// job-level `env:` references both `secrets.OJ_*` and `secrets.APP_*`.
#[test]
fn oj_and_app_credentials_are_separated() {
    for (label, doc) in [
        ("verify-worker", load_worker()),
        ("verify-result-integrity", load_integrity()),
    ] {
        for (job_name, job) in jobs(&doc) {
            let map = as_map(job, "job");
            // `environment` is a scalar in GH Actions — at most one value per job.
            if let Some(env) = get(map, "environment") {
                assert!(
                    env.as_str().is_some(),
                    "{label}: job {job_name:?} environment must be a scalar name, got {env:?}"
                );
            }
            // env: block — flatten values and check no single job wires both.
            if let Some(env_block) = get(map, "env").and_then(Value::as_mapping) {
                let mut refs_oj = false;
                let mut refs_app = false;
                for (_, v) in env_block {
                    if let Some(s) = v.as_str() {
                        if s.contains("secrets.OJ_") {
                            refs_oj = true;
                        }
                        if s.contains("secrets.APP_") {
                            refs_app = true;
                        }
                    }
                }
                assert!(
                    !(refs_oj && refs_app),
                    "{label}: job {job_name:?} references both OJ_ and APP_ secrets in env: (spec §15.4)"
                );
            }
        }
    }
}

/// #8: The integrity workflow must trigger only on PRs that touch
/// `verification/results/**`. Anything broader could sneak an unreviewed diff
/// through the PR gate.
#[test]
fn result_only_path_restriction_on_integrity_check() {
    let doc = load_integrity();
    let root = as_map(&doc, "verify-result-integrity");
    let on = get(root, "on").expect("missing `on:`");
    let on_map = as_map(on, "on");
    let pr = get(on_map, "pull_request").expect("missing pull_request trigger");
    let pr_map = as_map(pr, "pull_request");
    let paths = get(pr_map, "paths").expect("missing paths");
    let paths_seq = paths.as_sequence().expect("paths must be a sequence");
    let listed: Vec<&str> = paths_seq.iter().filter_map(Value::as_str).collect();
    assert_eq!(
        listed,
        vec!["verification/results/**"],
        "integrity workflow paths must be exactly [verification/results/**]"
    );
}

/// #9.5: `run:` shell strings must never inline a `${{ github.event... }}`
/// expression — those get expanded by Actions before the shell sees them, so
/// a hostile branch name or PR title could inject shell metacharacters. The
/// standard hardening is to pipe the expression through an `env:` block and
/// reference the shell variable instead. `if:` guards are safe because they
/// are evaluated by Actions, not the shell.
#[test]
fn run_steps_use_env_indirection_for_shas() {
    for (label, doc) in [
        ("verify-worker", load_worker()),
        ("verify-result-integrity", load_integrity()),
    ] {
        for (job_name, job) in jobs(&doc) {
            for (idx, step) in steps(job).iter().enumerate() {
                let map = match step.as_mapping() {
                    Some(m) => m,
                    None => continue,
                };
                let run = match get(map, "run").and_then(Value::as_str) {
                    Some(s) => s,
                    None => continue,
                };
                assert!(
                    !run.contains("${{ github.event") && !run.contains("${{github.event"),
                    "{label}: job {job_name:?} step[{idx}] run contains a direct \
                     `${{{{ github.event... }}}}` expansion: {run:?} — route it \
                     through an `env:` block and reference \"$VAR\" from the shell"
                );
            }
        }
    }
}

/// #9.6: The integrity workflow's `integrity` job must be gated to PRs whose
/// head branch is exactly `automation/verify`. Otherwise a human contributor
/// could modify both a result JSON and the `verify-validate-result-pr`
/// classifier logic in the same PR and have the integrity gate certify its
/// own tampering (spec §15.1, §15.3). The bot's App-token writer is the only
/// actor allowed to push to `automation/verify`.
#[test]
fn integrity_workflow_restricted_to_automation_verify_head_ref() {
    let doc = load_integrity();
    let jobs_map = jobs(&doc);
    let job = get(jobs_map, "integrity").expect("missing integrity job");
    let map = as_map(job, "integrity job");
    let if_expr = get(map, "if")
        .and_then(Value::as_str)
        .expect("integrity job missing `if:` guard");
    assert_eq!(
        if_expr, "github.event.pull_request.head.ref == 'automation/verify'",
        "integrity job must be gated to the automation/verify head ref (spec §15.1, §15.3)"
    );
}

/// #9.7 (plan 062): The dispatcher `verify.yml` — where classification now
/// lives (spec §15.3 requires it outside the `verify-heavy` group) — must
/// pipe the resolved `BEFORE` / `AFTER` values through an `env:` block
/// before invoking the shell classifier. Direct `${{ github.event... }}`
/// expansion in `run:` is banned by #9.5; the dispatcher goes further by
/// funneling both values through named step outputs so downstream steps
/// consume them from `${{ steps.resolve.outputs.* }}`.
#[test]
fn verify_dispatcher_wires_before_after_through_env() {
    let doc = load_dispatcher();
    let jobs_map = jobs(&doc);
    let dispatch = get(jobs_map, "dispatch").expect("verify.yml missing dispatch job");
    let dispatch_steps = steps(dispatch);
    let classify_step = dispatch_steps
        .iter()
        .find(|s| {
            s.as_mapping()
                .and_then(|m| get(m, "id").and_then(Value::as_str))
                == Some("classify")
        })
        .expect("dispatch job missing step with id: classify");
    let step_map = as_map(classify_step, "classify step");
    let env_map = get(step_map, "env")
        .and_then(Value::as_mapping)
        .expect("classify step must define an `env:` block wiring BEFORE / AFTER");
    let before = get(env_map, "BEFORE").and_then(Value::as_str);
    let after = get(env_map, "AFTER").and_then(Value::as_str);
    assert!(
        before.is_some_and(|s| s.contains("steps.resolve.outputs.before")),
        "classify step env.BEFORE must derive from steps.resolve.outputs.before (got {before:?})"
    );
    assert!(
        after.is_some_and(|s| s.contains("steps.resolve.outputs.after")),
        "classify step env.AFTER must derive from steps.resolve.outputs.after (got {after:?})"
    );

    // The worker consumes `after` (it needs the plan base SHA); the dispatcher
    // resolves both `before` and `after` for its own classification but only
    // forwards `after`. Assert the worker's `after` input is still
    // `required: true` and `type: string`.
    let worker = load_worker();
    let on = get(as_map(&worker, "verify-worker"), "on").expect("missing `on:`");
    let workflow_call = get(as_map(on, "on"), "workflow_call").expect("missing workflow_call");
    let inputs = get(as_map(workflow_call, "workflow_call"), "inputs")
        .expect("workflow_call missing `inputs:`");
    let inputs_map = as_map(inputs, "inputs");
    for name in ["after"] {
        let input = get(inputs_map, name)
            .unwrap_or_else(|| panic!("workflow_call.inputs missing {name:?}"));
        let input_map = as_map(input, name);
        assert_eq!(
            get(input_map, "required").and_then(Value::as_bool),
            Some(true),
            "workflow_call.inputs.{name} must be required: true"
        );
        assert_eq!(
            get(input_map, "type").and_then(Value::as_str),
            Some("string"),
            "workflow_call.inputs.{name} must be type: string"
        );
    }
}

/// #10 (plan 062): The dispatcher `verify.yml` is the sole caller of
/// `verify-worker.yml`. Nothing else may reach the secret-bearing worker.
#[test]
fn verify_worker_sole_caller_is_dispatcher() {
    let dir = workflow_dir();
    let entries =
        std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("failed to list {}: {e}", dir.display()));
    let mut callers = Vec::new();
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name == "verify-worker.yml" {
            continue;
        }
        if !(name.ends_with(".yml") || name.ends_with(".yaml")) {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        for (idx, line) in content.lines().enumerate() {
            if line.contains(".github/workflows/verify-worker.yml") {
                callers.push((name.to_string(), idx + 1));
            }
        }
    }
    let caller_files: std::collections::BTreeSet<&str> =
        callers.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        caller_files,
        std::collections::BTreeSet::from(["verify.yml"]),
        "verify-worker.yml must be called only by verify.yml; found: {callers:?}"
    );
}

// ─── Plan 061: normal CI policy (spec §15, §12.14, §12.15) ───────────────────

/// Collect every `run:` string across a workflow, tagged with `job/step`.
fn all_run_steps(doc: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (job_name, job) in jobs(doc) {
        let job_label = job_name.as_str().unwrap_or("").to_string();
        for step in steps(job) {
            if let Some(m) = step.as_mapping() {
                if let Some(run) = get(m, "run").and_then(Value::as_str) {
                    out.push((job_label.clone(), run.to_string()));
                }
            }
        }
    }
    out
}

/// #061.1: PR builds must not run on the `dev` base branch (§15 requires
/// `main` as the sole long-lived branch).
#[test]
fn ci_pr_base_is_main_only() {
    let doc = load_ci();
    let root = as_map(&doc, "ci.yml");
    let on = get(root, "on").expect("ci.yml missing `on:`");
    let on_map = as_map(on, "ci on");
    let pr = get(on_map, "pull_request").expect("ci.yml missing pull_request trigger");
    let branches = get(as_map(pr, "pull_request"), "branches")
        .and_then(Value::as_sequence)
        .expect("pull_request.branches must be a sequence");
    let listed: Vec<&str> = branches.iter().filter_map(Value::as_str).collect();
    assert_eq!(
        listed,
        vec!["main"],
        "ci.yml pull_request branches must be exactly [main]"
    );
}

/// #061.2: Every `uses:` in ci.yml is pinned to a 40-char SHA. No version
/// tags, no branch refs.
#[test]
fn ci_actions_are_sha_pinned() {
    let doc = load_ci();
    for (job_name, job) in jobs(&doc) {
        for (idx, step) in steps(job).iter().enumerate() {
            let uses = match step.as_mapping().and_then(|m| get(m, "uses")) {
                Some(Value::String(s)) => s.clone(),
                _ => continue,
            };
            let parts: Vec<&str> = uses.rsplitn(2, '@').collect();
            assert_eq!(
                parts.len(),
                2,
                "ci.yml: job {job_name:?} step[{idx}] uses {uses:?} has no @ref"
            );
            let sha = parts[0];
            assert_eq!(
                sha.len(),
                SHA_PIN_RE_LEN,
                "ci.yml: job {job_name:?} step[{idx}] uses {uses:?} is not SHA-pinned"
            );
            assert!(
                sha.chars().all(|c| c.is_ascii_hexdigit()),
                "ci.yml: job {job_name:?} step[{idx}] uses {uses:?} ref is not hex"
            );
        }
    }
}

/// #061.3: `actions/checkout` in ci.yml uses `fetch-depth: 0` so
/// `library.updated_at` derivation (§15) has full Git history.
#[test]
fn ci_checkout_fetches_full_history() {
    let doc = load_ci();
    let mut saw_checkout = false;
    for (job_name, job) in jobs(&doc) {
        for (idx, step) in steps(job).iter().enumerate() {
            let map = match step.as_mapping() {
                Some(m) => m,
                None => continue,
            };
            let uses = match get(map, "uses").and_then(Value::as_str) {
                Some(u) => u,
                None => continue,
            };
            if !uses.starts_with("actions/checkout@") {
                continue;
            }
            saw_checkout = true;
            let with = get(map, "with")
                .and_then(Value::as_mapping)
                .unwrap_or_else(|| {
                    panic!(
                        "ci.yml: job {job_name:?} step[{idx}] checkout needs `with: fetch-depth: 0`"
                    )
                });
            let depth = get(with, "fetch-depth");
            let ok = matches!(depth, Some(Value::Number(n)) if n.as_i64() == Some(0))
                || matches!(depth, Some(Value::String(s)) if s == "0");
            assert!(
                ok,
                "ci.yml: job {job_name:?} step[{idx}] checkout must set fetch-depth: 0"
            );
        }
    }
    assert!(
        saw_checkout,
        "ci.yml must call actions/checkout at least once"
    );
}

/// #061.4: The Rust setup step must pin `toolchain:` to the same value as
/// `rust-toolchain.toml`.
#[test]
fn ci_rust_toolchain_matches_pin() {
    let doc = load_ci();
    let expected = rust_toolchain_channel();
    let mut found = false;
    for (job_name, job) in jobs(&doc) {
        for (idx, step) in steps(job).iter().enumerate() {
            let map = match step.as_mapping() {
                Some(m) => m,
                None => continue,
            };
            let uses = match get(map, "uses").and_then(Value::as_str) {
                Some(u) => u,
                None => continue,
            };
            if !uses.starts_with("dtolnay/rust-toolchain@") {
                continue;
            }
            found = true;
            let with = get(map, "with")
                .and_then(Value::as_mapping)
                .unwrap_or_else(|| {
                    panic!("ci.yml: job {job_name:?} step[{idx}] rust-toolchain needs `with:`")
                });
            let toolchain = get(with, "toolchain").and_then(Value::as_str);
            assert_eq!(
                toolchain,
                Some(expected.as_str()),
                "ci.yml: job {job_name:?} step[{idx}] rust-toolchain must pin to {expected:?}"
            );
        }
    }
    assert!(found, "ci.yml must install a Rust toolchain");
}

/// #061.5: The Node setup step reads `.node-version` and enables `cache: npm`
/// (spec §12.15 requires exact Node patch pin plus a real cache).
#[test]
fn ci_node_setup_uses_node_version_and_cache() {
    let doc = load_ci();
    let pin = node_version_pin();
    let mut found = false;
    for (job_name, job) in jobs(&doc) {
        for (idx, step) in steps(job).iter().enumerate() {
            let map = match step.as_mapping() {
                Some(m) => m,
                None => continue,
            };
            let uses = match get(map, "uses").and_then(Value::as_str) {
                Some(u) => u,
                None => continue,
            };
            if !uses.starts_with("actions/setup-node@") {
                continue;
            }
            found = true;
            let with = get(map, "with")
                .and_then(Value::as_mapping)
                .unwrap_or_else(|| {
                    panic!("ci.yml: job {job_name:?} step[{idx}] setup-node needs `with:`")
                });
            let node_version_file = get(with, "node-version-file").and_then(Value::as_str);
            assert_eq!(
                node_version_file,
                Some(".node-version"),
                "ci.yml: setup-node must read node-version-file: .node-version"
            );
            let cache = get(with, "cache").and_then(Value::as_str);
            assert_eq!(cache, Some("npm"), "ci.yml: setup-node must set cache: npm");
        }
    }
    assert!(found, "ci.yml must install Node via actions/setup-node");
    // Also sanity check that the pin file has a concrete patch version.
    assert!(
        pin.chars().filter(|c| *c == '.').count() >= 2,
        ".node-version must pin a full patch (got {pin:?})"
    );
}

/// #061.6: The cargo cache step lists the full spec §12.14 paths so restore
/// hits both crate metadata and target artifacts.
#[test]
fn ci_cargo_cache_paths_are_complete() {
    let doc = load_ci();
    let mut ok = false;
    for job in jobs(&doc).values() {
        for step in steps(job) {
            let map = match step.as_mapping() {
                Some(m) => m,
                None => continue,
            };
            let uses = match get(map, "uses").and_then(Value::as_str) {
                Some(u) => u,
                None => continue,
            };
            if !uses.starts_with("actions/cache@") {
                continue;
            }
            let with = get(map, "with")
                .and_then(Value::as_mapping)
                .expect("cache step needs `with:`");
            let path = get(with, "path").and_then(Value::as_str).unwrap_or("");
            for needle in ["~/.cargo/registry", "~/.cargo/git", "target"] {
                assert!(
                    path.contains(needle),
                    "cache path must contain {needle:?}, got {path:?}"
                );
            }
            let key = get(with, "key").and_then(Value::as_str).unwrap_or("");
            assert!(
                key.contains("Cargo.lock"),
                "cache key must include Cargo.lock hash for reproducibility, got {key:?}"
            );
            ok = true;
        }
    }
    assert!(ok, "ci.yml must configure a cargo cache");
}

/// #061.7: The site job invokes `npm ci` (lockfile-only install per §12.15)
/// and `npm run site:build` exactly once (§12.14's single-entry contract).
/// Nothing may invoke `npx --yes` (banned package fetch per §12.15).
#[test]
fn ci_site_job_uses_npm_ci_and_single_site_build() {
    let doc = load_ci();
    let runs = all_run_steps(&doc);
    let mut npm_ci = 0usize;
    let mut site_build = 0usize;
    let mut banned_npx_yes = Vec::new();
    for (job, run) in &runs {
        if run.contains("npm ci") {
            npm_ci += 1;
        }
        // Match `npm run site:build` — bare or with -- args.
        for line in run.lines() {
            let l = line.trim();
            if l.starts_with("npm run site:build") {
                site_build += 1;
            }
        }
        if run.contains("npx --yes") || run.contains("npx -y ") {
            banned_npx_yes.push((job.clone(), run.clone()));
        }
    }
    assert!(npm_ci >= 1, "ci.yml must run `npm ci`");
    assert_eq!(
        site_build, 1,
        "ci.yml must invoke `npm run site:build` exactly once (got {site_build})"
    );
    assert!(
        banned_npx_yes.is_empty(),
        "ci.yml must not use `npx --yes`: {banned_npx_yes:?}"
    );
}

/// #061.8: ci.yml is secretless. No job pulls in a secret environment and no
/// step references `secrets.*`. PR builds live entirely on public data.
#[test]
fn ci_is_secretless() {
    let doc = load_ci();
    for (job_name, job) in jobs(&doc) {
        let map = as_map(job, "job");
        // `environment:` accepts both scalar (`environment: foo`) and mapping
        // (`environment: { name: foo, url: ... }`) shapes — any presence at
        // all opts the job into a protected secret store, so we forbid the
        // key rather than a specific value shape.
        if get(map, "environment").is_some() {
            panic!(
                "ci.yml: job {job_name:?} carries an `environment:` binding — CI must be secretless"
            );
        }
        for (idx, step) in steps(job).iter().enumerate() {
            let smap = match step.as_mapping() {
                Some(m) => m,
                None => continue,
            };
            if let Some(run) = get(smap, "run").and_then(Value::as_str) {
                assert!(
                    !run.contains("secrets."),
                    "ci.yml: job {job_name:?} step[{idx}] references secrets.* in run"
                );
            }
            if let Some(env_block) = get(smap, "env").and_then(Value::as_mapping) {
                for (_k, v) in env_block {
                    if let Some(s) = v.as_str() {
                        assert!(
                            !s.contains("secrets."),
                            "ci.yml: job {job_name:?} step[{idx}] wires secrets.* into env"
                        );
                    }
                }
            }
        }
    }
}

/// #061.9: ci.yml never deploys. Pages upload/deploy actions live in
/// `pages.yml` (Task 3); PR CI must not reach them.
#[test]
fn ci_never_deploys_to_pages() {
    let doc = load_ci();
    let banned_prefixes = [
        "actions/deploy-pages",
        "actions/upload-pages-artifact",
        "actions/configure-pages",
    ];
    for (job_name, job) in jobs(&doc) {
        for (idx, step) in steps(job).iter().enumerate() {
            let uses = match step.as_mapping().and_then(|m| get(m, "uses")) {
                Some(Value::String(s)) => s,
                _ => continue,
            };
            for banned in banned_prefixes {
                assert!(
                    !uses.starts_with(banned),
                    "ci.yml: job {job_name:?} step[{idx}] uses {uses:?} — deploy actions must live in pages.yml only"
                );
            }
        }
        let perms = as_map(job, "job")
            .get(Value::String("permissions".into()))
            .and_then(Value::as_mapping);
        if let Some(perms) = perms {
            for banned_key in ["pages", "id-token", "deployments"] {
                assert!(
                    perms.get(Value::String(banned_key.into())).is_none(),
                    "ci.yml: job {job_name:?} must not request permissions.{banned_key} — deploy lives in pages.yml"
                );
            }
        }
    }
}

// ─── Plan 061 Task 3: Pages workflow policy (spec §15.5) ─────────────────────

/// #061.10: `pages.yml` triggers only on push to main and manual dispatch.
/// PR and schedule triggers are banned so a stray branch cannot spawn a
/// deployment.
#[test]
fn pages_triggers_are_main_push_and_manual_only() {
    let doc = load_pages();
    let root = as_map(&doc, "pages.yml");
    let on = get(root, "on").expect("pages.yml missing `on:`");
    let on_map = as_map(on, "pages on");

    let push = get(on_map, "push")
        .and_then(Value::as_mapping)
        .expect("pages.yml missing push trigger");
    let branches = get(push, "branches")
        .and_then(Value::as_sequence)
        .expect("push.branches must be a sequence");
    let listed: Vec<&str> = branches.iter().filter_map(Value::as_str).collect();
    assert_eq!(
        listed,
        vec!["main"],
        "pages.yml push branches must be exactly [main]"
    );

    assert!(
        get(on_map, "workflow_dispatch").is_some(),
        "pages.yml must accept workflow_dispatch for manual re-publish"
    );

    for banned in [
        "pull_request",
        "pull_request_target",
        "schedule",
        "issue_comment",
    ] {
        assert!(
            get(on_map, banned).is_none(),
            "pages.yml must not use {banned:?} trigger"
        );
    }
}

/// #061.11: The publish workflow uses a fixed `pages-publish` concurrency
/// group with `cancel-in-progress: true` (spec §15.5).
#[test]
fn pages_concurrency_group_is_fixed_and_cancels() {
    let doc = load_pages();
    let root = as_map(&doc, "pages.yml");
    let concurrency = get(root, "concurrency")
        .and_then(Value::as_mapping)
        .expect("pages.yml must set workflow-level concurrency");
    let group = get(concurrency, "group").and_then(Value::as_str);
    assert_eq!(
        group,
        Some("pages-publish"),
        "concurrency group must be `pages-publish`"
    );
    let cancel = get(concurrency, "cancel-in-progress").and_then(Value::as_bool);
    assert_eq!(
        cancel,
        Some(true),
        "pages-publish concurrency must cancel in progress"
    );
}

/// #061.12: The build job only reads from the repository (never writes). Only
/// the deploy job holds `pages: write` and `id-token: write`.
#[test]
fn pages_build_never_deploys_and_deploy_has_minimum_writes() {
    let doc = load_pages();
    let jobs_map = jobs(&doc);
    let build = get(jobs_map, "build").expect("pages.yml missing `build` job");
    let deploy = get(jobs_map, "deploy").expect("pages.yml missing `deploy` job");

    // build has read-only contents (may keep `pages: read` for configure-pages).
    let build_perms = get(as_map(build, "build"), "permissions")
        .and_then(Value::as_mapping)
        .expect("build job must declare permissions");
    let contents = get(build_perms, "contents").and_then(Value::as_str);
    assert_eq!(
        contents,
        Some("read"),
        "build.permissions.contents must be `read`"
    );
    for banned in ["id-token", "deployments"] {
        assert!(
            build_perms.get(Value::String(banned.into())).is_none(),
            "build.permissions must not grant {banned}"
        );
    }
    if let Some(p) = get(build_perms, "pages").and_then(Value::as_str) {
        assert_eq!(p, "read", "build.permissions.pages must be at most `read`");
    }
    // The build job must not upload via `actions/deploy-pages`.
    for step in steps(build) {
        let uses = step
            .as_mapping()
            .and_then(|m| get(m, "uses"))
            .and_then(Value::as_str)
            .unwrap_or("");
        assert!(
            !uses.starts_with("actions/deploy-pages"),
            "build job must not call actions/deploy-pages — that belongs to deploy"
        );
    }

    // deploy holds exactly `pages: write` and `id-token: write`.
    let deploy_perms = get(as_map(deploy, "deploy"), "permissions")
        .and_then(Value::as_mapping)
        .expect("deploy job must declare permissions");
    let pages_write = get(deploy_perms, "pages").and_then(Value::as_str);
    let id_write = get(deploy_perms, "id-token").and_then(Value::as_str);
    assert_eq!(
        pages_write,
        Some("write"),
        "deploy.permissions.pages must be `write`"
    );
    assert_eq!(
        id_write,
        Some("write"),
        "deploy.permissions.id-token must be `write`"
    );
    for banned in ["contents", "deployments", "actions", "checks"] {
        if let Some(v) = deploy_perms
            .get(Value::String(banned.into()))
            .and_then(Value::as_str)
        {
            assert_ne!(v, "write", "deploy.permissions.{banned} must not be write");
        }
    }

    // deploy job binds the `github-pages` environment.
    let deploy_map = as_map(deploy, "deploy");
    let env = get(deploy_map, "environment").expect("deploy must bind environment");
    let env_name = match env {
        Value::String(s) => s.clone(),
        Value::Mapping(m) => m
            .get(Value::String("name".into()))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    };
    assert_eq!(
        env_name, "github-pages",
        "deploy environment must be `github-pages`"
    );
}

/// #061.13: Every third-party action in pages.yml is SHA-pinned. Especially
/// critical because the deploy job holds Pages write.
#[test]
fn pages_actions_are_sha_pinned() {
    let doc = load_pages();
    for (job_name, job) in jobs(&doc) {
        for (idx, step) in steps(job).iter().enumerate() {
            let uses = match step.as_mapping().and_then(|m| get(m, "uses")) {
                Some(Value::String(s)) => s.clone(),
                _ => continue,
            };
            let parts: Vec<&str> = uses.rsplitn(2, '@').collect();
            assert_eq!(
                parts.len(),
                2,
                "pages.yml: job {job_name:?} step[{idx}] uses {uses:?} has no @ref"
            );
            let sha = parts[0];
            assert_eq!(
                sha.len(),
                SHA_PIN_RE_LEN,
                "pages.yml: job {job_name:?} step[{idx}] uses {uses:?} is not SHA-pinned"
            );
            assert!(
                sha.chars().all(|c| c.is_ascii_hexdigit()),
                "pages.yml: job {job_name:?} step[{idx}] uses {uses:?} ref is not hex"
            );
        }
    }
}

/// #061.14: The build job writes a `build-source.json` metadata file
/// carrying the source commit SHA into the artifact so the deploy job can
/// enforce SHA equality (spec §15.5).
#[test]
fn pages_build_emits_source_sha_metadata() {
    let doc = load_pages();
    let jobs_map = jobs(&doc);
    let build = get(jobs_map, "build").expect("missing build job");
    let mut writes_metadata = false;
    for step in steps(build) {
        let map = match step.as_mapping() {
            Some(m) => m,
            None => continue,
        };
        if let Some(run) = get(map, "run").and_then(Value::as_str) {
            if run.contains("build-source.json") && run.contains("source_commit_sha") {
                writes_metadata = true;
            }
        }
    }
    assert!(
        writes_metadata,
        "build job must write a build-source.json capturing source_commit_sha"
    );

    // The build job must also expose the SHA as a job output, so the deploy
    // job can compare it to current main without unpacking the artifact.
    let outputs = get(as_map(build, "build"), "outputs")
        .and_then(Value::as_mapping)
        .expect("build job must expose outputs for the deploy step");
    let source_sha = get(outputs, "source_sha").and_then(Value::as_str);
    assert!(
        source_sha.is_some_and(|s| s.contains("steps.") && s.contains("source_sha")),
        "outputs.source_sha must wire to a step output (got {source_sha:?})"
    );
}

/// #061.15: The deploy job compares the artifact's source SHA to current
/// main HEAD before invoking `actions/deploy-pages`. An old rerun whose
/// source SHA no longer matches main must fail before publishing (§15.5).
#[test]
fn pages_deploy_rejects_stale_reruns() {
    let doc = load_pages();
    let jobs_map = jobs(&doc);
    let deploy = get(jobs_map, "deploy").expect("missing deploy job");
    let step_list = steps(deploy);

    let deploy_idx = step_list
        .iter()
        .position(|s| {
            s.as_mapping()
                .and_then(|m| get(m, "uses"))
                .and_then(Value::as_str)
                .is_some_and(|u| u.starts_with("actions/deploy-pages@"))
        })
        .expect("deploy job must call actions/deploy-pages");

    let mut has_sha_check = false;
    for step in &step_list[..deploy_idx] {
        let map = match step.as_mapping() {
            Some(m) => m,
            None => continue,
        };
        let run = match get(map, "run").and_then(Value::as_str) {
            Some(s) => s,
            None => continue,
        };
        if run.contains("commits/main") && run.contains("ARTIFACT_SHA") {
            has_sha_check = true;
        }
    }
    assert!(
        has_sha_check,
        "deploy job must query commits/main and compare against ARTIFACT_SHA before actions/deploy-pages"
    );
}

/// #061.16: The workflow-level default token permissions are `contents: read`
/// only. No stray write scopes trickle down to jobs that forget to set them.
#[test]
fn pages_default_permissions_are_read_only() {
    let doc = load_pages();
    let root = as_map(&doc, "pages.yml");
    let perms = get(root, "permissions")
        .and_then(Value::as_mapping)
        .expect("pages.yml must declare workflow-level permissions");
    let contents = get(perms, "contents").and_then(Value::as_str);
    assert_eq!(
        contents,
        Some("read"),
        "workflow-level permissions.contents must be `read`"
    );
    for key in perms.keys() {
        let k = key.as_str().unwrap_or("");
        assert_eq!(
            k, "contents",
            "workflow-level permissions must only set `contents`, saw {k:?}"
        );
    }
}

// ─── Plan 062: verify activation policy (spec §15.1–§15.4) ───────────────────

const WORKER_JOB_ORDER: [&str; 6] = [
    "prepare",
    "persist_starting",
    "submit",
    "persist_handle",
    "poll",
    "persist_terminal",
];

const APP_ONLY_JOBS: [&str; 3] = ["persist_starting", "persist_handle", "persist_terminal"];
const OJ_ONLY_JOBS: [&str; 2] = ["submit", "poll"];
const LIVE_ONLY_JOBS: [&str; 4] = ["submit", "persist_handle", "poll", "persist_terminal"];

fn seq_str_values<'a>(v: &'a Value) -> Vec<&'a str> {
    v.as_sequence()
        .map(|s| s.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn needs_of<'a>(job: &'a Value) -> Vec<&'a str> {
    let map = match job.as_mapping() {
        Some(m) => m,
        None => return vec![],
    };
    match get(map, "needs") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(v) => seq_str_values(v),
        None => vec![],
    }
}

/// #062.1: The dispatcher `verify.yml` triggers on push to main, on a
/// 5-minute schedule (dense enough for the 5/10/20/40/80-min → 6h retry
/// budget), and on `workflow_dispatch`. No other trigger is allowed.
#[test]
fn dispatcher_triggers_are_push_schedule_manual() {
    let doc = load_dispatcher();
    let root = as_map(&doc, "verify.yml");
    let on = get(root, "on").expect("verify.yml missing `on:`");
    let on_map = as_map(on, "verify on");

    // Push: main only.
    let push = get(on_map, "push")
        .and_then(Value::as_mapping)
        .expect("verify.yml missing push trigger");
    let push_branches = get(push, "branches")
        .and_then(Value::as_sequence)
        .expect("push.branches must be a sequence");
    let listed: Vec<&str> = push_branches.iter().filter_map(Value::as_str).collect();
    assert_eq!(
        listed,
        vec!["main"],
        "verify.yml push branches must be exactly [main]"
    );

    // Schedule: at least one cron entry, and every entry is 5-minute cadence.
    let schedule = get(on_map, "schedule")
        .and_then(Value::as_sequence)
        .expect("verify.yml missing schedule trigger");
    assert!(
        !schedule.is_empty(),
        "verify.yml schedule must not be empty"
    );
    for entry in schedule {
        let cron = get(as_map(entry, "schedule entry"), "cron")
            .and_then(Value::as_str)
            .expect("schedule entry missing cron");
        assert!(
            cron.starts_with("*/5 "),
            "verify.yml schedule cron {cron:?} must be 5-minute cadence \
             so the 5/10/20/40/80-min retry ladder is honored"
        );
    }

    // Manual dispatch must be present.
    assert!(
        get(on_map, "workflow_dispatch").is_some(),
        "verify.yml must accept workflow_dispatch"
    );

    // No other trigger.
    for banned in [
        "pull_request",
        "pull_request_target",
        "workflow_call",
        "issue_comment",
        "release",
    ] {
        assert!(
            get(on_map, banned).is_none(),
            "verify.yml must not use trigger {banned:?}"
        );
    }
}

/// #062.2: Every job in the dispatcher is gated on the master activation
/// switch `vars.VERIFY_ACTIVATED == 'true'`. Pre-G2 the workflow is fully
/// dormant even though the cron is armed.
#[test]
fn dispatcher_jobs_gated_on_verify_activated() {
    let doc = load_dispatcher();
    for (job_name, job) in jobs(&doc) {
        let map = as_map(job, "job");
        let if_expr = get(map, "if")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("verify.yml job {job_name:?} missing `if:` guard"));
        assert!(
            if_expr.contains("vars.VERIFY_ACTIVATED == 'true'"),
            "verify.yml job {job_name:?} `if:` must gate on \
             `vars.VERIFY_ACTIVATED == 'true'` (got {if_expr:?})"
        );
    }
}

/// #062.3: The dispatcher itself must live OUTSIDE the `verify-heavy`
/// concurrency group (spec §15.3). No workflow-level `concurrency:` and no
/// job-level `concurrency:` on the classify job.
#[test]
fn dispatcher_is_outside_verify_heavy_group() {
    let doc = load_dispatcher();
    let root = as_map(&doc, "verify.yml");
    assert!(
        get(root, "concurrency").is_none(),
        "verify.yml must not set workflow-level concurrency \
         (classification must run outside verify-heavy per §15.3)"
    );
    for (job_name, job) in jobs(&doc) {
        let map = as_map(job, "job");
        if let Some(concurrency) = get(map, "concurrency") {
            // The invoked reusable worker will set the group; the dispatcher
            // job itself must not.
            let name = job_name.as_str().unwrap_or("");
            if name == "worker" {
                continue;
            }
            panic!("verify.yml job {job_name:?} must not declare concurrency: {concurrency:?}");
        }
    }
}

/// #062.4: The dispatcher is secretless — no job carries an `environment:`
/// and no `run:` or `env:` mentions `secrets.*`.
#[test]
fn dispatcher_is_secretless() {
    let doc = load_dispatcher();
    for (job_name, job) in jobs(&doc) {
        let map = as_map(job, "job");
        assert!(
            get(map, "environment").is_none(),
            "verify.yml job {job_name:?} must not bind an environment"
        );
        for (idx, step) in steps(job).iter().enumerate() {
            let smap = match step.as_mapping() {
                Some(m) => m,
                None => continue,
            };
            if let Some(run) = get(smap, "run").and_then(Value::as_str) {
                assert!(
                    !run.contains("secrets."),
                    "verify.yml job {job_name:?} step[{idx}] references secrets.* in run"
                );
            }
            if let Some(env_block) = get(smap, "env").and_then(Value::as_mapping) {
                for (_k, v) in env_block {
                    if let Some(s) = v.as_str() {
                        assert!(
                            !s.contains("secrets."),
                            "verify.yml job {job_name:?} step[{idx}] wires secrets.* into env"
                        );
                    }
                }
            }
        }
    }
}

/// #062.5: The dispatcher's `worker` job is what invokes the worker, gated
/// on `vars.VERIFY_ACTIVATED`, `dispatch.outputs.run_worker == 'true'`, and
/// the `main` branch. `secrets: inherit` is required so the worker can see
/// its two environment-scoped secrets.
#[test]
fn dispatcher_worker_call_is_correctly_gated() {
    let doc = load_dispatcher();
    let jobs_map = jobs(&doc);
    let worker = get(jobs_map, "worker").expect("verify.yml missing worker job");
    let map = as_map(worker, "worker job");

    let uses = get(map, "uses")
        .and_then(Value::as_str)
        .expect("worker job must be a reusable workflow call");
    assert_eq!(
        uses, "./.github/workflows/verify-worker.yml",
        "worker job must invoke ./.github/workflows/verify-worker.yml"
    );

    let if_expr = get(map, "if")
        .and_then(Value::as_str)
        .expect("worker job missing `if:` guard");
    for needle in [
        "vars.VERIFY_ACTIVATED == 'true'",
        "needs.dispatch.outputs.run_worker == 'true'",
        "github.ref == 'refs/heads/main'",
    ] {
        assert!(
            if_expr.contains(needle),
            "worker job `if:` must contain {needle:?} (got {if_expr:?})"
        );
    }

    let secrets = get(map, "secrets").and_then(Value::as_str);
    assert_eq!(
        secrets,
        Some("inherit"),
        "worker job must pass `secrets: inherit`"
    );
}

/// #062.6: The worker's concurrency group is `verify-heavy` with
/// `cancel-in-progress: false` (spec §15.1: 実行中 worker は新しい push で cancel しない).
#[test]
fn worker_uses_verify_heavy_group_without_cancellation() {
    let doc = load_worker();
    let root = as_map(&doc, "verify-worker.yml");
    let concurrency = get(root, "concurrency")
        .and_then(Value::as_mapping)
        .expect("verify-worker.yml must set workflow-level concurrency");
    let group = get(concurrency, "group").and_then(Value::as_str);
    assert_eq!(
        group,
        Some("verify-heavy"),
        "worker concurrency group must be `verify-heavy`"
    );
    let cancel = get(concurrency, "cancel-in-progress").and_then(Value::as_bool);
    assert_eq!(
        cancel,
        Some(false),
        "worker concurrency must NOT cancel in progress"
    );
}

/// #062.7: The six-job chain executes in the exact order
/// `prepare → persist_starting → submit → persist_handle → poll → persist_terminal`,
/// enforced by `needs:` declarations. No job may be missing or reordered.
#[test]
fn worker_job_chain_is_ordered() {
    let doc = load_worker();
    let jobs_map = jobs(&doc);
    for name in WORKER_JOB_ORDER {
        assert!(
            jobs_map.contains_key(Value::String(name.into())),
            "verify-worker.yml missing required job {name:?}"
        );
    }
    for (idx, name) in WORKER_JOB_ORDER.iter().enumerate() {
        let job = get(jobs_map, name).unwrap();
        let needs = needs_of(job);
        if idx == 0 {
            // prepare has no needs; jobs after prepare must transitively
            // depend on it.
            assert!(needs.is_empty(), "job {name:?} must have no needs");
            continue;
        }
        // Each downstream job must depend on `prepare` (for the artifact
        // hashes) and on the immediately preceding job (for ordering).
        let prev = WORKER_JOB_ORDER[idx - 1];
        assert!(
            needs.contains(&"prepare"),
            "job {name:?} needs must include `prepare` (got {needs:?})"
        );
        assert!(
            needs.contains(&prev) || prev == "prepare",
            "job {name:?} needs must include preceding job {prev:?} (got {needs:?})"
        );
    }
}

/// #062.8: The environment bindings partition into App-only vs. OJ-only vs.
/// none. This is spec §15.4's credential separation — no single job carries
/// both credentials, and every downstream job carries exactly one.
#[test]
fn worker_environments_partition_credentials() {
    let doc = load_worker();
    let jobs_map = jobs(&doc);

    // prepare: no environment.
    let prepare = get(jobs_map, "prepare").expect("missing prepare job");
    assert!(
        get(as_map(prepare, "prepare"), "environment").is_none(),
        "prepare job must not bind an environment (it is secretless per §15.4)"
    );

    for name in APP_ONLY_JOBS {
        let job = get(jobs_map, name).unwrap();
        let env = get(as_map(job, name), "environment").and_then(Value::as_str);
        assert_eq!(
            env,
            Some("verify-state"),
            "job {name:?} must bind environment `verify-state`"
        );
    }
    for name in OJ_ONLY_JOBS {
        let job = get(jobs_map, name).unwrap();
        let env = get(as_map(job, name), "environment").and_then(Value::as_str);
        assert_eq!(
            env,
            Some("oj-library-checker"),
            "job {name:?} must bind environment `oj-library-checker`"
        );
    }
}

/// #062.9: `submit`, `persist_handle`, `poll`, `persist_terminal` are gated
/// on `inputs.mode == 'live'`. `dry-run` exercises only prepare and
/// persist_starting (Task 3 CAS+permissions dry run).
#[test]
fn worker_live_only_jobs_are_mode_gated() {
    let doc = load_worker();
    let jobs_map = jobs(&doc);
    for name in LIVE_ONLY_JOBS {
        let job = get(jobs_map, name).unwrap();
        let if_expr = get(as_map(job, name), "if")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("job {name:?} must be gated on inputs.mode"));
        assert!(
            if_expr.contains("inputs.mode == 'live'"),
            "job {name:?} `if:` must contain `inputs.mode == 'live'` (got {if_expr:?})"
        );
    }
}

/// #062.10: Every job that downloads a `verify-*` artifact must re-validate
/// its SHA256 against `needs.prepare.outputs.plan_sha` (and companion outputs
/// for `ce`, `handle`, `terminal`). This is the "secret jobs download only
/// reviewed pinned artifacts" invariant from plan 062 Task 1.
#[test]
fn worker_secret_jobs_validate_artifact_digests() {
    let doc = load_worker();
    let jobs_map = jobs(&doc);
    for name in WORKER_JOB_ORDER.iter().skip(1) {
        let job = get(jobs_map, name).unwrap();
        let mut has_download = false;
        let mut has_plan_sha_check = false;
        for step in steps(job) {
            let map = match step.as_mapping() {
                Some(m) => m,
                None => continue,
            };
            if let Some(uses) = get(map, "uses").and_then(Value::as_str)
                && uses.starts_with("actions/download-artifact@")
            {
                has_download = true;
            }
            if let Some(run) = get(map, "run").and_then(Value::as_str)
                && run.contains("sha256sum")
                && run.contains("EXPECTED_PLAN_SHA")
            {
                has_plan_sha_check = true;
            }
        }
        assert!(
            has_download,
            "job {name:?} must download at least one pinned artifact"
        );
        assert!(
            has_plan_sha_check,
            "job {name:?} must re-validate plan SHA256 against needs.prepare.outputs.plan_sha \
             before running ./ce"
        );
    }
}

/// #062.11: OJ-bearing jobs (`submit`, `poll`) never reference App secrets;
/// App-bearing jobs (`persist_*`) never reference OJ secrets. This is a
/// per-job cross-check on top of the existing #7 test. Secrets can be
/// wired through either `env:` (shell) or `with:` (action inputs, e.g.
/// `private-key: ${{ secrets.VERIFY_APP_PRIVATE_KEY }}` on
/// `actions/create-github-app-token`), so both blocks are walked.
#[test]
fn worker_oj_and_app_secrets_are_disjoint_per_job() {
    fn walk_step_for_forbidden_secret(
        step: &Value,
        job_name: &str,
        forbidden_prefix: &str,
        allowed_kind: &str,
    ) {
        let map = match step.as_mapping() {
            Some(m) => m,
            None => return,
        };
        for field in ["env", "with"] {
            let block = match get(map, field).and_then(Value::as_mapping) {
                Some(b) => b,
                None => continue,
            };
            for (_, v) in block {
                if let Some(s) = v.as_str()
                    && s.contains(forbidden_prefix)
                {
                    panic!(
                        "{allowed_kind}-only job {job_name:?} must not reference \
                         {forbidden_prefix}* (found in `{field}:` value {s:?})"
                    );
                }
            }
        }
    }

    let doc = load_worker();
    let jobs_map = jobs(&doc);
    for name in OJ_ONLY_JOBS {
        let job = get(jobs_map, name).unwrap();
        for step in steps(job) {
            walk_step_for_forbidden_secret(step, name, "secrets.VERIFY_APP_", "OJ");
        }
    }
    for name in APP_ONLY_JOBS {
        let job = get(jobs_map, name).unwrap();
        for step in steps(job) {
            walk_step_for_forbidden_secret(step, name, "secrets.LIBRARYCHECKER_", "App");
        }
    }
}

/// #062.12: App-only jobs must mint the App installation token via
/// `actions/create-github-app-token` (SHA-pinned). The App ID comes from
/// `vars.VERIFY_APP_ID`; the private key from `secrets.VERIFY_APP_PRIVATE_KEY`.
#[test]
fn worker_app_jobs_use_create_github_app_token() {
    let doc = load_worker();
    let jobs_map = jobs(&doc);
    for name in APP_ONLY_JOBS {
        let job = get(jobs_map, name).unwrap();
        let mut found = false;
        for step in steps(job) {
            let map = match step.as_mapping() {
                Some(m) => m,
                None => continue,
            };
            let uses = match get(map, "uses").and_then(Value::as_str) {
                Some(u) => u,
                None => continue,
            };
            if !uses.starts_with("actions/create-github-app-token@") {
                continue;
            }
            found = true;
            let with = get(map, "with")
                .and_then(Value::as_mapping)
                .unwrap_or_else(|| panic!("job {name:?} app-token step must set `with:`"));
            let app_id = get(with, "app-id").and_then(Value::as_str);
            assert_eq!(
                app_id,
                Some("${{ vars.VERIFY_APP_ID }}"),
                "job {name:?} app-token app-id must be vars.VERIFY_APP_ID"
            );
            let key = get(with, "private-key").and_then(Value::as_str);
            assert_eq!(
                key,
                Some("${{ secrets.VERIFY_APP_PRIVATE_KEY }}"),
                "job {name:?} app-token private-key must be secrets.VERIFY_APP_PRIVATE_KEY"
            );
        }
        assert!(
            found,
            "job {name:?} must mint an App token via actions/create-github-app-token"
        );
    }
}

/// #062.13: The `persist_*` jobs must invoke `ce internal verify-persist`
/// with the token passed through the process environment (never on the
/// command line or in job outputs). We assert the shell reads `GH_APP_TOKEN`
/// via `--token-env GH_APP_TOKEN` and the step wires the token through an
/// `env:` block, not through `run: echo $TOKEN`.
#[test]
fn worker_persist_jobs_pass_token_via_env_only() {
    let doc = load_worker();
    let jobs_map = jobs(&doc);
    for name in APP_ONLY_JOBS {
        let job = get(jobs_map, name).unwrap();
        let mut has_persist = false;
        for step in steps(job) {
            let map = match step.as_mapping() {
                Some(m) => m,
                None => continue,
            };
            let run = match get(map, "run").and_then(Value::as_str) {
                Some(s) => s,
                None => continue,
            };
            if !run.contains("verify-persist") {
                continue;
            }
            has_persist = true;
            assert!(
                run.contains("--token-env GH_APP_TOKEN"),
                "persist step in {name:?} must use `--token-env GH_APP_TOKEN` (got {run:?})"
            );
            let env_block = get(map, "env")
                .and_then(Value::as_mapping)
                .unwrap_or_else(|| panic!("persist step in {name:?} must define an `env:` block"));
            let token = get(env_block, "GH_APP_TOKEN").and_then(Value::as_str);
            assert!(
                token.is_some_and(|s| s.contains("app_token") && s.contains(".outputs.token")),
                "persist step in {name:?} env.GH_APP_TOKEN must be wired from the app-token step's \
                 outputs.token (got {token:?})"
            );
        }
        assert!(has_persist, "job {name:?} must call verify-persist");
    }
}

/// #062.14: The worker rejects triggers other than `workflow_call`. The
/// concurrency group and secret environments would be catastrophic under a
/// `push` or `pull_request` trigger.
#[test]
fn worker_is_workflow_call_only() {
    let doc = load_worker();
    let root = as_map(&doc, "verify-worker.yml");
    let on = get(root, "on").expect("worker missing `on:`");
    let on_map = as_map(on, "on");
    assert_eq!(
        on_map.len(),
        1,
        "verify-worker.yml `on:` must contain exactly one trigger"
    );
    assert!(
        get(on_map, "workflow_call").is_some(),
        "verify-worker.yml must trigger only on workflow_call"
    );
}

/// #062.15: The worker exposes `mode` as a `workflow_call` input (accepting
/// `live` or `dry-run`) and `solution` as an optional string. `after` carries
/// the plan base SHA. `before` deliberately does NOT appear on the worker —
/// classification lives in the dispatcher (§15.3) and nothing inside the
/// worker consumes it. `mode` must default to `dry-run` for defense-in-depth.
#[test]
fn worker_declares_mode_and_solution_inputs() {
    let doc = load_worker();
    let root = as_map(&doc, "verify-worker.yml");
    let on = get(root, "on").expect("worker missing `on:`");
    let on_map = as_map(on, "on");
    let wc = get(on_map, "workflow_call").expect("worker missing workflow_call");
    let inputs = get(as_map(wc, "workflow_call"), "inputs")
        .and_then(Value::as_mapping)
        .expect("workflow_call must declare inputs");
    for name in ["after", "mode", "solution"] {
        let input =
            get(inputs, name).unwrap_or_else(|| panic!("workflow_call.inputs missing {name:?}"));
        let m = as_map(input, name);
        let ty = get(m, "type").and_then(Value::as_str);
        assert_eq!(
            ty,
            Some("string"),
            "workflow_call.inputs.{name} must be type: string"
        );
    }

    // `mode` must default to `dry-run` so a caller that forgets to pass it
    // cannot silently exercise the OJ path.
    let mode_map = as_map(
        get(inputs, "mode").expect("workflow_call.inputs.mode missing"),
        "mode",
    );
    let mode_default = get(mode_map, "default").and_then(Value::as_str);
    assert_eq!(
        mode_default,
        Some("dry-run"),
        "workflow_call.inputs.mode default must be `dry-run`"
    );

    // `before` was removed once classification moved to the dispatcher; any
    // future accidental re-add would signal a stale contract.
    assert!(
        get(inputs, "before").is_none(),
        "workflow_call.inputs.before must not exist — classification lives in the dispatcher"
    );
}

/// #062.16: `secret-bearing` jobs must not run `git clone` / `git fetch` /
/// `git checkout` in shell — the only way to materialize source is through
/// the pinned `actions/checkout@<sha>` action targeting `${{ inputs.after }}`
/// (validated by test #6 above).
#[test]
fn worker_secret_jobs_never_git_from_shell() {
    let doc = load_worker();
    let banned_shell_snippets = ["git clone", "git checkout", "git fetch"];
    for (job_name, job) in jobs(&doc) {
        if !is_secret_job(job) {
            continue;
        }
        for (idx, step) in steps(job).iter().enumerate() {
            let map = match step.as_mapping() {
                Some(m) => m,
                None => continue,
            };
            if let Some(run) = get(map, "run").and_then(Value::as_str) {
                for banned in banned_shell_snippets {
                    assert!(
                        !run.contains(banned),
                        "worker: secret job {job_name:?} step[{idx}] runs {banned:?}"
                    );
                }
            }
        }
    }
}
