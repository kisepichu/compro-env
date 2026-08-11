//! Problem metadata helpers for LibraryChecker.

use anyhow::{Context, Result};
use serde::Deserialize;

const REST_BASE: &str = "https://v3.api.judge.yosupo.jp";
const STORAGE_BASE: &str = "https://storage.googleapis.com/v2-prod-library-checker-data-public";

pub(super) fn problem_info_url(name: &str) -> String {
    format!("{REST_BASE}/problems/{name}")
}

pub(super) fn info_toml_url(name: &str, overall_version: &str) -> String {
    format!("{STORAGE_BASE}/v4/files/{name}/{overall_version}/{name}/info.toml")
}

pub(super) fn example_in_url(name: &str, testcases_version: &str, idx: usize) -> String {
    // Example files are zero-padded to two digits (example_00, …, example_10, …).
    format!("{STORAGE_BASE}/v4/examples/{name}/{testcases_version}/in/example_{idx:02}.in")
}

pub(super) fn example_out_url(name: &str, testcases_version: &str, idx: usize) -> String {
    format!("{STORAGE_BASE}/v4/examples/{name}/{testcases_version}/out/example_{idx:02}.out")
}

pub(super) fn task_md_url(name: &str, overall_version: &str) -> String {
    format!("{STORAGE_BASE}/v4/files/{name}/{overall_version}/{name}/task.md")
}

/// Strips the "librarychecker-" namespace prefix to recover the bare problem name.
pub(super) fn bare_problem_name(contest_id: &str) -> &str {
    contest_id
        .strip_prefix("librarychecker-")
        .unwrap_or(contest_id)
}

/// Extracts the input format from a task.md statement source.
///
/// The statement has a `## @{keyword.input}` heading followed by a fenced code block
/// holding the layout (e.g. `$A$ $B$`). We strip `$` so the result matches the
/// `$`-free format the input parser expects (e.g. `A B`, `N\nA_1 \dots A_N`).
/// Returns None if no input block is found or it is empty.
pub(super) fn extract_input_format(task_md: &str) -> Option<String> {
    let block = fenced_block_after(task_md, "@{keyword.input}")?;
    let cleaned = block.replace('$', "");
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Extracts the constraints section from task.md, resolving `@{param.NAME}`
/// placeholders against the `[params]` table in info.toml and stripping `$`.
/// Returns None if no constraints section is found.
pub(super) fn extract_constraints(task_md: &str, info_toml: &str) -> Option<String> {
    let section = section_after_heading(task_md, "@{keyword.constraints}")?;
    let resolved = resolve_params(section.trim(), info_toml).replace('$', "");
    let trimmed = resolved.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Returns the content of the first fenced ```` ``` ```` block that appears after a
/// line containing `heading_marker`.
fn fenced_block_after(md: &str, heading_marker: &str) -> Option<String> {
    let after_heading = &md[md.find(heading_marker)? + heading_marker.len()..];
    let fence_start = after_heading.find("```")?;
    let after_open = &after_heading[fence_start + 3..];
    // Skip to the end of the opening fence line (handles ```text etc.).
    let body_start = after_open.find('\n')? + 1;
    let body = &after_open[body_start..];
    let fence_end = body.find("```")?;
    Some(body[..fence_end].to_string())
}

/// Returns the text from just after the line containing `heading_marker` up to the
/// next `##` heading (or end of document).
fn section_after_heading(md: &str, heading_marker: &str) -> Option<String> {
    let pos = md.find(heading_marker)?;
    let after = &md[pos + heading_marker.len()..];
    // Skip the rest of the heading line.
    let nl = after.find('\n')?;
    let body = &after[nl + 1..];
    let end = body.find("\n##").unwrap_or(body.len());
    Some(body[..end].to_string())
}

/// Replaces `@{param.NAME}` placeholders with their values from the info.toml
/// `[params]` table. Unknown placeholders are left untouched.
fn resolve_params(text: &str, info_toml: &str) -> String {
    let params = match toml::from_str::<toml::Table>(info_toml) {
        Ok(t) => t,
        Err(_) => return text.to_string(),
    };
    let Some(params) = params.get("params").and_then(|v| v.as_table()) else {
        return text.to_string();
    };
    let mut out = text.to_string();
    for (name, value) in params {
        let needle = format!("@{{param.{name}}}");
        if out.contains(&needle) {
            let replacement = match value {
                toml::Value::Integer(i) => i.to_string(),
                toml::Value::Float(f) => f.to_string(),
                toml::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            out = out.replace(&needle, &replacement);
        }
    }
    out
}

pub(super) struct ProblemInfo {
    pub(super) title: String,
    pub(super) overall_version: String,
    pub(super) testcases_version: String,
}

#[derive(Deserialize)]
struct ProblemInfoResponse {
    title: String,
    overall_version: String,
    testcases_version: String,
}

pub(super) fn parse_problem_info(json: &str) -> Result<ProblemInfo> {
    let r: ProblemInfoResponse =
        serde_json::from_str(json).context("failed to parse problem info response")?;
    Ok(ProblemInfo {
        title: r.title,
        overall_version: r.overall_version,
        testcases_version: r.testcases_version,
    })
}

/// Counts examples from info.toml: the `[[tests]]` entry named `example.in` has a
/// `number` field giving the example count. Returns 0 if absent.
pub(super) fn count_examples(info_toml: &str) -> usize {
    let table: toml::Table = match toml::from_str(info_toml) {
        Ok(t) => t,
        Err(_) => return 0,
    };
    table
        .get("tests")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .find(|entry| entry.get("name").and_then(|n| n.as_str()) == Some("example.in"))
        .and_then(|entry| entry.get("number"))
        .and_then(|n| n.as_integer())
        .map(|n| n.max(0) as usize)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_builders_match_frontend_layout() {
        assert_eq!(
            problem_info_url("aplusb"),
            "https://v3.api.judge.yosupo.jp/problems/aplusb"
        );
        assert_eq!(
            info_toml_url("aplusb", "OV"),
            "https://storage.googleapis.com/v2-prod-library-checker-data-public/v4/files/aplusb/OV/aplusb/info.toml"
        );
        assert_eq!(
            example_in_url("aplusb", "TCV", 0),
            "https://storage.googleapis.com/v2-prod-library-checker-data-public/v4/examples/aplusb/TCV/in/example_00.in"
        );
        assert_eq!(
            example_out_url("aplusb", "TCV", 1),
            "https://storage.googleapis.com/v2-prod-library-checker-data-public/v4/examples/aplusb/TCV/out/example_01.out"
        );
        // Two-digit zero padding: idx >= 10 must not become a 3-digit "example_010".
        assert_eq!(
            example_in_url("aplusb", "TCV", 10),
            "https://storage.googleapis.com/v2-prod-library-checker-data-public/v4/examples/aplusb/TCV/in/example_10.in"
        );
    }

    #[test]
    fn count_examples_reads_example_in_number() {
        let info = r#"
[[tests]]
    name = "example.in"
    number = 2
[[tests]]
    name = "random.cpp"
    number = 10
"#;
        assert_eq!(count_examples(info), 2);
    }

    #[test]
    fn count_examples_absent_is_zero() {
        let info = "[[tests]]\n    name = \"random.cpp\"\n    number = 10\n";
        assert_eq!(count_examples(info), 0);
        assert_eq!(count_examples("not valid toml ["), 0);
    }

    #[test]
    fn parse_problem_info_extracts_versions_and_title() {
        let json = r#"{"title":"A + B","source_url":"https://x","time_limit":2,
            "version":"V","overall_version":"OV","testcases_version":"TCV"}"#;
        let info = parse_problem_info(json).expect("should parse");
        assert_eq!(info.title, "A + B");
        assert_eq!(info.overall_version, "OV");
        assert_eq!(info.testcases_version, "TCV");
    }

    #[test]
    fn bare_problem_name_strips_namespace() {
        assert_eq!(bare_problem_name("librarychecker-aplusb"), "aplusb");
        // Already-bare names (or other shapes) pass through unchanged.
        assert_eq!(bare_problem_name("aplusb"), "aplusb");
    }

    #[test]
    fn extract_input_format_strips_dollars() {
        let task = "## @{keyword.input}\n\n\n```\n$A$ $B$\n```\n\n## @{keyword.output}\n";
        assert_eq!(extract_input_format(task).as_deref(), Some("A B"));
    }

    #[test]
    fn extract_input_format_multiline_array() {
        let task = "## @{keyword.input}\n\n```\n$N$\n$A_1$ $A_2$ $\\dots$ $A_N$\n```\n## @{keyword.output}\n";
        assert_eq!(
            extract_input_format(task).as_deref(),
            Some("N\nA_1 A_2 \\dots A_N")
        );
    }

    #[test]
    fn extract_input_format_absent_is_none() {
        assert_eq!(extract_input_format("no input section here"), None);
    }

    #[test]
    fn extract_constraints_resolves_params_and_strips_dollars() {
        let task = "## @{keyword.constraints}\n\n- $0 \\leq A, B \\leq @{param.A_AND_B_MAX}$\n\n## @{keyword.input}\n";
        let info = "[params]\n    A_AND_B_MAX = 1_000_000_000\n";
        assert_eq!(
            extract_constraints(task, info).as_deref(),
            Some("- 0 \\leq A, B \\leq 1000000000")
        );
    }

    #[test]
    fn extract_constraints_absent_is_none() {
        assert_eq!(extract_constraints("nothing", "").as_deref(), None);
    }
}
