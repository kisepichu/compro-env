use anyhow::Result;
use domain::entity::{Problem, Session, SubmitResult};
use usecases::online_judge::OnlineJudge;

pub struct AtCoder;

impl OnlineJudge for AtCoder {
    fn name(&self) -> &str {
        "atcoder"
    }

    fn whoami(&self, session: &Session) -> Result<String> {
        let client = reqwest::blocking::Client::builder()
            .cookie_store(true)
            .build()?;
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
    // Look for href="/users/{username}" pattern and extract the username.
    let marker = r#"href="/users/"#;
    let start = html.find(marker)? + marker.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    let username = &rest[..end];
    if username.is_empty() {
        None
    } else {
        Some(username.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logged_in_html(username: &str) -> String {
        format!(
            r#"<!DOCTYPE html>
<html>
<head><title>AtCoder</title></head>
<body>
<nav class="navbar">
  <div class="container">
    <ul class="nav navbar-nav navbar-right">
      <li><a href="/users/{username}" class="username"><span class="user-gray">{username}</span></a></li>
      <li><a href="/logout">Sign Out</a></li>
    </ul>
  </div>
</nav>
<div class="container">
  <h1>AtCoder Home</h1>
</div>
</body>
</html>"#,
            username = username
        )
    }

    fn not_logged_in_html() -> String {
        r#"<!DOCTYPE html>
<html>
<head><title>AtCoder</title></head>
<body>
<nav class="navbar">
  <div class="container">
    <ul class="nav navbar-nav navbar-right">
      <li><a href="/login">Sign In</a></li>
    </ul>
  </div>
</nav>
<div class="container">
  <h1>AtCoder Home</h1>
</div>
</body>
</html>"#
            .to_string()
    }

    #[test]
    fn parse_username_returns_username_when_logged_in() {
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
