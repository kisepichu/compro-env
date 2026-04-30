use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use chrono::{DateTime, Utc};
use domain::entity::{Problem, Session};
use usecases::online_judge::{ContestMeta, OnlineJudge};

pub struct AtCoder {
    client: reqwest::blocking::Client,
}

impl AtCoder {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder().build()?,
        })
    }
}

impl OnlineJudge for AtCoder {
    fn name(&self) -> &str {
        "atcoder"
    }

    fn whoami(&self, session: &Session) -> Result<String> {
        // Set the REVEL_SESSION cookie before making the request.
        let cookie = format!("REVEL_SESSION={}", session.cookie);
        self.client
            .get("https://atcoder.jp/home")
            .header(reqwest::header::COOKIE, cookie)
            .send()?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!(e))
            .and_then(|resp| resp.text().map_err(|e| anyhow::anyhow!(e)))
            .and_then(|body| {
                parse_username_from_html(&body)
                    .ok_or_else(|| anyhow::anyhow!("session expired. Run `ce login` again."))
            })
    }

    fn get_contest_meta(&self, contest_id: &str) -> Result<ContestMeta> {
        let url = format!("https://atcoder.jp/contests/{}", contest_id);
        let body = self
            .client
            .get(&url)
            .send()
            .map_err(|e| anyhow::anyhow!("failed to fetch contest page for {contest_id}: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("contest page returned error for {contest_id}: {e}"))?
            .text()
            .map_err(|e| anyhow::anyhow!("failed to read contest page for {contest_id}: {e}"))?;
        Ok(ContestMeta {
            start_time: parse_start_time_from_html(&body),
            problem_id_hints: vec![],
        })
    }

    fn get_problems_detail(
        &self,
        contest_id: &str,
        session: Option<&Session>,
        problem_id_hints: &[(String, String)],
    ) -> Result<Vec<Problem>> {
        let url = format!("https://atcoder.jp/contests/{}/tasks_print", contest_id);
        let mut req = self.client.get(&url);
        if let Some(session) = session {
            req = req.header(
                reqwest::header::COOKIE,
                format!("REVEL_SESSION={}", session.cookie),
            );
        }
        let resp = req.send()?.error_for_status()?;
        // AtCoder redirects unauthenticated requests to /login (still 200).
        let final_url = resp.url().clone();
        let html = resp.text()?;
        if final_url.path().contains("/login") {
            return Err(domain::error::CeError::NotLoggedIn {
                oj: "atcoder".into(),
            }
            .into());
        }
        let problems = parse_tasks_print_from_html(&html, contest_id, problem_id_hints);
        Ok(problems)
    }

    fn build_submit_url(
        &self,
        contest_id: &str,
        problem_id: &str,
        lang_id: &str,
        source: &str,
    ) -> String {
        // Encode {lang_id, source} as URL-safe base64 JSON and embed in the fragment.
        // The Tampermonkey userscript reads this fragment and auto-fills the submit form.
        // See docs/userscript.md for the full protocol.
        let payload = serde_json::json!({
            "lang_id": lang_id,
            "source": source,
        })
        .to_string();
        let fragment = format!("ce={}", URL_SAFE.encode(payload.as_bytes()));

        // Build the URL via reqwest::Url so that contest_id and problem_id are
        // percent-encoded, producing a well-formed URL even if they contain
        // URL-reserved characters.
        let mut url = reqwest::Url::parse("https://atcoder.jp/").expect("base URL is valid");
        url.path_segments_mut()
            .expect("base URL is cannot-be-a-base")
            .push("contests")
            .push(contest_id)
            .push("submit");
        url.query_pairs_mut()
            .append_pair("taskScreenName", problem_id);
        url.set_fragment(Some(&fragment));
        url.to_string()
    }
}

fn parse_start_time_from_html(html: &str) -> Option<DateTime<Utc>> {
    // AtCoder may render the time tag with multiple classes, e.g.
    // `class="fixtime fixtime-full"` or `class="fixtime-full"`.
    // Scan all <time ...> tags and pick the first one whose attributes contain "fixtime-full".
    let mut search_from = 0;
    while let Some(rel) = html[search_from..].find("<time") {
        let tag_start = search_from + rel;
        let tag_end = tag_start + html[tag_start..].find('>')?;
        let start_tag = &html[tag_start..=tag_end];
        if start_tag.contains("fixtime-full") {
            let after = &html[tag_end + 1..];
            let end = after.find("</time>")?;
            let text = &after[..end];
            return chrono::DateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%z")
                .ok()
                .map(|dt| dt.with_timezone(&Utc));
        }
        search_from = tag_end + 1;
    }
    None
}

fn parse_tasks_print_from_html(
    html: &str,
    contest_id: &str,
    problem_id_hints: &[(String, String)],
) -> Vec<domain::entity::Problem> {
    let marker = "<span class=\"h2\">";
    let chunks: Vec<&str> = html.split(marker).collect();
    let mut problems = Vec::new();

    // Skip the first chunk (content before the first problem)
    for chunk in chunks.iter().skip(1) {
        // Extract heading: full text up to </span> (e.g. "A - Christmas Present")
        let heading = match chunk.find("</span>") {
            Some(end) => chunk[..end].trim().to_string(),
            None => continue,
        };

        // Split "A - Title" into code "a" and title "Title".
        // If no separator exists, treat the whole heading as both.
        let (problem_code, title) = match heading.split_once(" - ") {
            Some((code, t)) => (code.trim().to_lowercase(), t.trim().to_string()),
            None => (heading.trim().to_lowercase(), heading.clone()),
        };

        // Extract samples from this chunk
        let inputs = extract_pre_texts_after(chunk, "<h3>Sample Input");
        let outputs = extract_pre_texts_after(chunk, "<h3>Sample Output");
        let samples = inputs
            .into_iter()
            .zip(outputs)
            .map(|(input, output)| domain::entity::Sample { input, output })
            .collect();

        // Extract input_format_raw
        let input_format_raw =
            extract_section_pre_blocks(chunk, &["<h3>入力</h3>", "<h3>Input</h3>"]);

        // Extract constraints_raw
        let constraints_raw =
            extract_section_text(chunk, &["<h3>制約</h3>", "<h3>Constraints</h3>"]);

        // Determine problem_id: look up in hints or infer
        let problem_id = problem_id_hints
            .iter()
            .find(|(code, _)| code == &problem_code)
            .map(|(_, id)| id.clone())
            .unwrap_or_else(|| format!("{}_{}", contest_id, problem_code));

        problems.push(domain::entity::Problem {
            id: problem_id,
            code: problem_code,
            title,
            samples,
            input_format_raw,
            constraints_raw,
        });
    }

    problems
}

/// Find all occurrences of `heading_marker` and for each, extract the text of
/// the next `<pre>...</pre>` block. Inline HTML tags are stripped and HTML
/// entities are decoded so the result is plain text suitable for testcase files.
fn extract_pre_texts_after(html: &str, heading_marker: &str) -> Vec<String> {
    let mut results = Vec::new();
    let mut search_from = 0;

    while let Some(pos) = html[search_from..].find(heading_marker) {
        let abs_pos = search_from + pos + heading_marker.len();
        // Find the next <pre> after this heading
        if let Some(pre_pos) = html[abs_pos..].find("<pre>") {
            let content_start = abs_pos + pre_pos + "<pre>".len();
            if let Some(end_pos) = html[content_start..].find("</pre>") {
                let raw = &html[content_start..content_start + end_pos];
                results.push(decode_pre_content(raw));
                search_from = content_start + end_pos;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    results
}

/// Find the first matching section heading and collect all `<pre>` blocks until the next `<h3>`.
/// Returns `Some(joined)` if at least one block is found, or `None`.
fn extract_section_pre_blocks(html: &str, headings: &[&str]) -> Option<String> {
    // Find the first matching heading
    let (heading_end, heading_len) = headings
        .iter()
        .find_map(|h| html.find(h).map(|pos| (pos, h.len())))?;

    let after_heading = &html[heading_end + heading_len..];

    // Determine the end of this section: the next <h3> tag
    let section = match after_heading.find("<h3>") {
        Some(next_h3) => &after_heading[..next_h3],
        None => after_heading,
    };

    // Extract all <pre>...</pre> blocks from section
    let mut blocks = Vec::new();
    let mut search_from = 0;
    while let Some(pre_pos) = section[search_from..].find("<pre>") {
        let content_start = search_from + pre_pos + "<pre>".len();
        if let Some(end_pos) = section[content_start..].find("</pre>") {
            let raw = &section[content_start..content_start + end_pos];
            blocks.push(decode_pre_content(raw));
            search_from = content_start + end_pos + "</pre>".len();
        } else {
            break;
        }
    }

    if blocks.is_empty() {
        None
    } else {
        // Join blocks with "\n\n". Each block may already end with "\n",
        // so insert only "\n" between blocks to produce a clean "\n\n" boundary.
        let mut result = String::new();
        for (i, block) in blocks.iter().enumerate() {
            if i > 0 {
                if result.ends_with('\n') {
                    result.push('\n');
                } else {
                    result.push_str("\n\n");
                }
            }
            result.push_str(block);
        }
        Some(result)
    }
}

/// Find the first matching section heading and extract the text content (HTML tags stripped)
/// until the next `<h3>`. Returns `Some(text)` if non-empty, or `None`.
fn extract_section_text(html: &str, headings: &[&str]) -> Option<String> {
    let (heading_end, heading_len) = headings
        .iter()
        .find_map(|h| html.find(h).map(|pos| (pos, h.len())))?;

    let after_heading = &html[heading_end + heading_len..];

    let section = match after_heading.find("<h3>") {
        Some(next_h3) => &after_heading[..next_h3],
        None => after_heading,
    };

    // Strip HTML tags (reuse the same tag-stripping logic as decode_pre_content)
    let stripped = decode_pre_content(section);
    let text = stripped.trim().to_string();

    if text.is_empty() { None } else { Some(text) }
}

/// Strip inline HTML tags and decode common HTML entities from `<pre>` content.
fn decode_pre_content(s: &str) -> String {
    // Strip inline tags (e.g. <var>, <sub>, <sup>, <b>, etc.)
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            // consume until '>'
            for c2 in chars.by_ref() {
                if c2 == '>' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    // Decode HTML entities. Loop until stable so that double-encoded sequences
    // like `&amp;lt;` are fully decoded to `<`.
    loop {
        let decoded = out
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ");
        if decoded == out {
            return out;
        }
        out = decoded;
    }
}

fn parse_username_from_html(html: &str) -> Option<String> {
    // AtCoder injects the logged-in username as a JavaScript variable near the
    // top of every page: `var userScreenName = "alice";` when logged in, or
    // `var userScreenName = "";` when not. This is more reliable than scraping
    // the navbar HTML, which changed structure over time.
    let marker = "var userScreenName = \"";
    let pos = html.find(marker)?;
    let after = &html[pos + marker.len()..];
    let end = after.find('"')?;
    let username = &after[..end];
    if username.is_empty() {
        None
    } else {
        Some(username.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Simulate the logged-in AtCoder page.
    /// AtCoder injects `var userScreenName = "alice";` in the <head> when
    /// logged in. The page body includes ranking user links that must not
    /// shadow the logged-in user.
    fn logged_in_html(username: &str) -> String {
        format!(
            r#"<!DOCTYPE html>
<html>
<head>
<script>
var userScreenName = "{username}";
</script>
</head>
<body>
<div class="container">
  <table>
    <tr><td><a href="/users/highly_ranked_user" class="username"><span class="user-red">highly_ranked_user</span></a></td></tr>
    <tr><td><a href="/users/another_ranked_user" class="username"><span class="user-red">another_ranked_user</span></a></td></tr>
  </table>
</div>
</body>
</html>"#,
            username = username
        )
    }

    /// Simulate the unauthenticated AtCoder page.
    /// `var userScreenName = "";` (empty string) when not logged in.
    /// Page body still has user links from rankings.
    fn not_logged_in_html() -> String {
        r#"<!DOCTYPE html>
<html>
<head>
<script>
var userScreenName = "";
</script>
</head>
<body>
<div class="container">
  <table>
    <tr><td><a href="/users/highly_ranked_user" class="username"><span class="user-red">highly_ranked_user</span></a></td></tr>
    <tr><td><a href="/users/another_ranked_user" class="username"><span class="user-red">another_ranked_user</span></a></td></tr>
  </table>
</div>
</body>
</html>"#
            .to_string()
    }

    fn contest_page_html_with_start_time() -> String {
        r#"<!DOCTYPE html>
<html>
<head><title>AtCoder Beginner Contest 334</title></head>
<body>
<div class="panel">
  <small><span class="label label-default">Contest Duration</span>
    <a href="/contests/abc334"><time class="fixtime-full">2023-12-23 21:00:00+0900</time></a>
    -
    <a href="/contests/abc334"><time class="fixtime-full">2023-12-23 23:00:00+0900</time></a>
  </small>
</div>
</body>
</html>"#
            .to_string()
    }

    fn contest_page_html_without_start_time() -> String {
        r#"<!DOCTYPE html>
<html>
<head><title>AtCoder Beginner Contest 334</title></head>
<body>
<div class="panel">
  <p>No time information available.</p>
</div>
</body>
</html>"#
            .to_string()
    }

    #[test]
    #[serial]
    fn parse_start_time_returns_datetime_when_present() {
        use chrono::TimeZone;
        let html = contest_page_html_with_start_time();
        let result = parse_start_time_from_html(&html);
        // 2023-12-23 21:00:00+0900 == 2023-12-23 12:00:00 UTC
        let expected = chrono::Utc
            .with_ymd_and_hms(2023, 12, 23, 12, 0, 0)
            .unwrap();
        assert_eq!(result, Some(expected));
    }

    #[test]
    #[serial]
    fn parse_start_time_returns_none_when_absent() {
        let html = contest_page_html_without_start_time();
        let result = parse_start_time_from_html(&html);
        assert_eq!(result, None);
    }

    #[test]
    #[serial]
    fn parse_username_returns_username_when_logged_in() {
        // The fixture includes ranking user links that must not shadow the
        // logged-in user extracted from userScreenName.
        let html = logged_in_html("kisepichu");
        let result = parse_username_from_html(&html);
        assert_eq!(result, Some("kisepichu".to_string()));
    }

    #[test]
    #[serial]
    fn parse_username_returns_none_when_not_logged_in() {
        let html = not_logged_in_html();
        let result = parse_username_from_html(&html);
        assert_eq!(result, None);
    }

    fn tasks_print_html() -> String {
        r#"<!DOCTYPE html>
<html><body>
<div id="main-container">
  <!-- Problem A -->
  <span class="h2">A - Christmas Present</span>
  <div class="part"><section>
    <h3>Sample Input 1</h3><pre>1 2
</pre>
  </section></div>
  <div class="part"><section>
    <h3>Sample Output 1</h3><pre>3
</pre>
  </section></div>
  <!-- Problem B -->
  <span class="h2">B - 333</span>
  <div class="part"><section>
    <h3>Sample Input 1</h3><pre>5
</pre>
  </section></div>
  <div class="part"><section>
    <h3>Sample Output 1</h3><pre>10
</pre>
  </section></div>
</div>
</body></html>"#
            .to_string()
    }

    #[test]
    #[serial]
    fn parse_tasks_print_returns_two_problems() {
        use domain::entity::Sample;
        let html = tasks_print_html();
        let result = parse_tasks_print_from_html(&html, "abc334", &[]);
        assert_eq!(result.len(), 2, "expected 2 problems, got {:?}", result);

        let a = result
            .iter()
            .find(|p| p.code == "a")
            .expect("problem a not found");
        assert_eq!(a.id, "abc334_a");
        assert_eq!(a.title, "Christmas Present");
        assert_eq!(a.samples.len(), 1);
        assert_eq!(
            a.samples[0],
            Sample {
                input: "1 2\n".to_string(),
                output: "3\n".to_string(),
            }
        );

        let b = result
            .iter()
            .find(|p| p.code == "b")
            .expect("problem b not found");
        assert_eq!(b.id, "abc334_b");
        assert_eq!(b.title, "333");
        assert_eq!(b.samples.len(), 1);
        assert_eq!(
            b.samples[0],
            Sample {
                input: "5\n".to_string(),
                output: "10\n".to_string(),
            }
        );
    }

    #[test]
    #[serial]
    fn parse_tasks_print_uses_hints_when_provided() {
        let html = tasks_print_html();
        let hints = vec![
            ("a".to_string(), "arc103_a".to_string()),
            ("b".to_string(), "arc103_b".to_string()),
        ];
        let result = parse_tasks_print_from_html(&html, "abc334", &hints);
        assert_eq!(result.len(), 2, "expected 2 problems, got {:?}", result);

        let a = result
            .iter()
            .find(|p| p.code == "a")
            .expect("problem a not found");
        assert_eq!(a.id, "arc103_a", "expected problem_id from hints");

        let b = result
            .iter()
            .find(|p| p.code == "b")
            .expect("problem b not found");
        assert_eq!(b.id, "arc103_b", "expected problem_id from hints");
    }

    #[test]
    fn build_submit_url_encodes_payload_in_fragment() {
        use usecases::online_judge::OnlineJudge as _;
        let oj = AtCoder::new().expect("AtCoder::new");
        let url = oj.build_submit_url("abc001", "abc001_a", "4026", "fn main() {}");
        // URL structure: https://atcoder.jp/contests/abc001/submit?taskScreenName=abc001_a#ce=<base64>
        assert!(url.starts_with("https://atcoder.jp/contests/abc001/submit?"));
        assert!(url.contains("taskScreenName=abc001_a"));
        let fragment = url.split('#').nth(1).expect("fragment present");
        let encoded = fragment
            .strip_prefix("ce=")
            .expect("fragment starts with ce=");
        let decoded = URL_SAFE.decode(encoded).expect("valid base64");
        let payload: serde_json::Value = serde_json::from_slice(&decoded).expect("valid JSON");
        assert_eq!(payload["lang_id"], "4026");
        assert_eq!(payload["source"], "fn main() {}");
    }

    #[test]
    #[serial]
    fn extract_pre_texts_decodes_html_entities() {
        let html = r#"<h3>Sample Input 1</h3><pre>3 &lt; 5
&amp;foo
</pre>"#;
        let result = extract_pre_texts_after(html, "<h3>Sample Input");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "3 < 5\n&foo\n");
    }

    #[test]
    #[serial]
    fn extract_pre_texts_strips_inline_html_tags() {
        let html = r#"<h3>Sample Input 1</h3><pre><var>N</var>
<var>A</var> <var>B</var>
</pre>"#;
        let result = extract_pre_texts_after(html, "<h3>Sample Input");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "N\nA B\n");
    }

    // ── input_format_raw / constraints_raw extraction ─────────────────────────

    fn single_block_html() -> String {
        r#"<span class="h2">A - Title</span>
<h3>入力</h3>
<pre>N M
A_1 A_2 \ldots A_N
</pre>
<h3>制約</h3>
<p>1 \leq N \leq 10^5</p>
<h3>Sample Input 1</h3><pre>3 2
1 2 3
</pre>
<h3>Sample Output 1</h3><pre>6
</pre>"#
            .to_string()
    }

    fn multiple_blocks_html() -> String {
        r#"<span class="h2">D - Query</span>
<h3>入力</h3>
<pre>Q
query_1
\vdots
query_Q
</pre>
<p>各クエリは以下の形式</p>
<pre>1 x
</pre>
<pre>2 x k
</pre>
<h3>制約</h3>
<p>1 \leq Q \leq 10^5</p>
<h3>Sample Input 1</h3><pre>3
1 5
2 3 1
1 7
</pre>
<h3>Sample Output 1</h3><pre>...</pre>"#
            .to_string()
    }

    fn ul_constraints_html() -> String {
        r#"<span class="h2">B - Problem</span>
<h3>入力</h3>
<pre>N
</pre>
<h3>制約</h3>
<ul>
<li>1 \leq N \leq 100</li>
<li>N は整数</li>
</ul>
<h3>Sample Input 1</h3><pre>5</pre><h3>Sample Output 1</h3><pre>25</pre>"#
            .to_string()
    }

    fn no_input_section_html() -> String {
        r#"<span class="h2">A - Old</span>
<h3>Sample Input 1</h3><pre>1</pre>
<h3>Sample Output 1</h3><pre>2</pre>"#
            .to_string()
    }

    #[test]
    #[serial]
    fn parse_input_format_raw_single_block() {
        let html = single_block_html();
        let result = parse_tasks_print_from_html(&html, "abc001", &[]);
        assert_eq!(result.len(), 1);
        let p = &result[0];
        assert_eq!(
            p.input_format_raw,
            Some("N M\nA_1 A_2 \\ldots A_N\n".to_string())
        );
        assert!(
            p.constraints_raw.is_some(),
            "constraints_raw should be Some"
        );
        let constraints = p.constraints_raw.as_ref().unwrap();
        // Should contain the constraint text stripped of tags
        assert!(
            constraints.contains("1 \\leq N \\leq 10^5"),
            "constraints_raw should contain constraint text, got: {:?}",
            constraints
        );
    }

    #[test]
    #[serial]
    fn parse_input_format_raw_multiple_blocks() {
        let html = multiple_blocks_html();
        let result = parse_tasks_print_from_html(&html, "abc001", &[]);
        assert_eq!(result.len(), 1);
        let p = &result[0];
        assert_eq!(
            p.input_format_raw,
            Some("Q\nquery_1\n\\vdots\nquery_Q\n\n1 x\n\n2 x k\n".to_string())
        );
    }

    #[test]
    #[serial]
    fn parse_constraints_raw_strips_tags() {
        let html = ul_constraints_html();
        let result = parse_tasks_print_from_html(&html, "abc001", &[]);
        assert_eq!(result.len(), 1);
        let p = &result[0];
        let constraints = p
            .constraints_raw
            .as_ref()
            .expect("constraints_raw should be Some");
        // Should not contain any HTML tags
        assert!(
            !constraints.contains('<'),
            "constraints_raw should not contain HTML tags, got: {:?}",
            constraints
        );
        assert!(
            !constraints.contains('>'),
            "constraints_raw should not contain HTML tags, got: {:?}",
            constraints
        );
        // Should contain the text content
        assert!(
            constraints.contains("1 \\leq N \\leq 100") || constraints.contains("N"),
            "constraints_raw should contain constraint text, got: {:?}",
            constraints
        );
    }

    #[test]
    #[serial]
    fn parse_input_format_raw_none_when_no_input_section() {
        let html = no_input_section_html();
        let result = parse_tasks_print_from_html(&html, "abc001", &[]);
        assert_eq!(result.len(), 1);
        let p = &result[0];
        assert_eq!(p.input_format_raw, None);
        assert_eq!(p.constraints_raw, None);
    }
}
