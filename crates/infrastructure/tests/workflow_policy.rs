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

/// #6: Secret-bearing jobs must not check out source or build with cargo.
/// The whole point of the split is that only the classifier / integrity
/// jobs touch code (§15.3).
#[test]
fn no_checkout_or_build_in_secret_jobs() {
    let banned_cargo_verbs = ["cargo build", "cargo test", "cargo run"];
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
                if let Some(uses) = get(map, "uses").and_then(Value::as_str) {
                    assert!(
                        !uses.starts_with("actions/checkout"),
                        "{label}: job {job_name:?} step[{idx}] uses {uses:?} — secret jobs must not checkout"
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

/// #9.7: `verify-worker.yml` is `workflow_call`-only and its classify step
/// references `$BEFORE` / `$AFTER`. Both variables must be wired to
/// `workflow_call` inputs so plan 061 can enable the step without immediately
/// failing on empty revisions. The inputs must be `required: true` and
/// `type: string`, and the classify step must pipe them through an `env:`
/// block using `${{ inputs.before }}` / `${{ inputs.after }}` (see #9.5 —
/// direct expression expansion is banned).
#[test]
fn verify_worker_defines_before_after_inputs_for_classify() {
    let doc = load_worker();
    let root = as_map(&doc, "verify-worker");
    let on = get(root, "on").expect("missing `on:`");
    let on_map = as_map(on, "on");
    let workflow_call = get(on_map, "workflow_call").expect("missing workflow_call trigger");
    let wc_map = as_map(workflow_call, "workflow_call");
    let inputs = get(wc_map, "inputs").expect("workflow_call missing `inputs:`");
    let inputs_map = as_map(inputs, "inputs");

    for name in ["before", "after"] {
        let input = get(inputs_map, name)
            .unwrap_or_else(|| panic!("workflow_call.inputs missing {name:?}"));
        let input_map = as_map(input, name);
        let required = get(input_map, "required").and_then(Value::as_bool);
        assert_eq!(
            required,
            Some(true),
            "workflow_call.inputs.{name} must be required: true"
        );
        let ty = get(input_map, "type").and_then(Value::as_str);
        assert_eq!(
            ty,
            Some("string"),
            "workflow_call.inputs.{name} must be type: string"
        );
    }

    // Classify step must carry an `env:` block feeding BEFORE / AFTER from
    // `inputs.*`. Without this the `$BEFORE` / `$AFTER` shell expansions
    // would be empty when plan 061 removes the `if: false` guard.
    let jobs_map = jobs(&doc);
    let classify_job = get(jobs_map, "classify").expect("missing classify job");
    let classify_steps = steps(classify_job);
    let classify_step = classify_steps
        .iter()
        .find(|s| {
            s.as_mapping()
                .and_then(|m| get(m, "id").and_then(Value::as_str))
                == Some("classify")
        })
        .expect("classify job missing step with id: classify");
    let step_map = as_map(classify_step, "classify step");
    let env_map = get(step_map, "env")
        .and_then(Value::as_mapping)
        .expect("classify step must define an `env:` block wiring BEFORE / AFTER");
    let before = get(env_map, "BEFORE").and_then(Value::as_str);
    let after = get(env_map, "AFTER").and_then(Value::as_str);
    assert_eq!(
        before,
        Some("${{ inputs.before }}"),
        "classify step env.BEFORE must be `${{{{ inputs.before }}}}`"
    );
    assert_eq!(
        after,
        Some("${{ inputs.after }}"),
        "classify step env.AFTER must be `${{{{ inputs.after }}}}`"
    );
}

/// #10: `verify-worker.yml` is dormant. No other workflow may `uses:` it.
#[test]
fn verify_worker_has_no_caller() {
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
                callers.push(format!("{}:{}", path.display(), idx + 1));
            }
        }
    }
    assert!(
        callers.is_empty(),
        "verify-worker.yml must have no callers; found: {callers:?}"
    );
}
