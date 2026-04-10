use anyhow::Result;
use domain::entity::{Problem, Session, SubmitResult};
use usecases::online_judge::OnlineJudge;

pub struct AtCoder;

impl OnlineJudge for AtCoder {
    fn name(&self) -> &str {
        "atcoder"
    }

    fn whoami(&self, session: &Session) -> Result<String> {
        let client = reqwest::blocking::Client::builder().build()?;
        // Set the REVEL_SESSION cookie before making the request.
        let cookie = format!("REVEL_SESSION={}", session.cookie);
        client
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

    fn get_problems_detail(
        &self,
        _contest_id: &str,
        _session: Option<&Session>,
    ) -> Result<Vec<Problem>> {
        todo!()
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

    fn wait_for_start(&self, _contest_id: &str) -> Result<()> {
        todo!()
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
}
