use anyhow::Result;
use chrono::{DateTime, Utc};
use domain::entity::{Problem, Session, SubmitResult};
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
            .and_then(|r| r.text())
            .map_err(|e| anyhow::anyhow!("failed to fetch contest page for {contest_id}: {e}"))?;
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
            anyhow::bail!("not logged in. Run `ce login` first.");
        }
        let problems = parse_tasks_print_from_html(&html, contest_id, problem_id_hints);
        Ok(problems)
    }

    fn submit(
        &self,
        _contest_id: &str,
        _problem_id: &str,
        _lang_id: &str,
        _source: &str,
        _session: &Session,
    ) -> Result<SubmitResult> {
        todo!()
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
    fn parse_start_time_returns_none_when_absent() {
        let html = contest_page_html_without_start_time();
        let result = parse_start_time_from_html(&html);
        assert_eq!(result, None);
    }

    #[test]
    fn parse_username_returns_username_when_logged_in() {
        // The fixture includes ranking user links that must not shadow the
        // logged-in user extracted from userScreenName.
        let html = logged_in_html("kisepichu");
        let result = parse_username_from_html(&html);
        assert_eq!(result, Some("kisepichu".to_string()));
    }

    #[test]
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
    fn extract_pre_texts_decodes_html_entities() {
        let html = r#"<h3>Sample Input 1</h3><pre>3 &lt; 5
&amp;foo
</pre>"#;
        let result = extract_pre_texts_after(html, "<h3>Sample Input");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "3 < 5\n&foo\n");
    }

    #[test]
    fn extract_pre_texts_strips_inline_html_tags() {
        let html = r#"<h3>Sample Input 1</h3><pre><var>N</var>
<var>A</var> <var>B</var>
</pre>"#;
        let result = extract_pre_texts_after(html, "<h3>Sample Input");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "N\nA B\n");
    }
}
