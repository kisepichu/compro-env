use anyhow::Result;
use chrono::{DateTime, Utc};
use domain::entity::{OJKind, Problem, Session};

pub struct ContestMeta {
    pub start_time: Option<DateTime<Utc>>,
    pub problem_id_hints: Vec<(String, String)>, // (problem_code, problem_id)
}

/// The kind of credential an OJ requires for login. The shell uses this to decide
/// what to prompt the user for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialKind {
    /// A manually copied session cookie (e.g. AtCoder's REVEL_SESSION).
    Cookie,
    /// An email address and password (e.g. LibraryChecker via Firebase).
    EmailPassword,
}

/// Credentials supplied to `OnlineJudge::login`.
#[derive(Debug, Clone)]
pub enum Credentials {
    Cookie(String),
    Password {
        identifier: String,
        password: String,
    },
}

pub trait OnlineJudge {
    fn name(&self) -> &str;

    /// The credential kind this OJ expects for login.
    fn credential_kind(&self) -> CredentialKind;

    /// Authenticates with the OJ and returns a `Session`.
    ///
    /// AtCoder simply wraps the manually copied cookie. OJs with programmatic login
    /// (e.g. LibraryChecker) perform a network request to obtain a token.
    fn login(&self, credentials: &Credentials) -> Result<Session>;

    /// Returns the username of the currently logged-in user.
    fn whoami(&self, session: &Session) -> Result<String>;

    /// Returns contest metadata including start time and problem id hints.
    fn get_contest_meta(&self, contest_id: &str) -> Result<ContestMeta>;

    /// Fetches all problems with their samples.
    /// Public contests do not require a session; private contests require Some(&session).
    fn get_problems_detail(
        &self,
        contest_id: &str,
        session: Option<&Session>,
        problem_id_hints: &[(String, String)],
    ) -> Result<Vec<Problem>>;

    /// Builds a URL to open in the browser for submitting a solution.
    /// The URL encodes the source and language ID so that the Tampermonkey userscript
    /// can auto-fill the submission form. See docs/userscript.md.
    fn build_submit_url(
        &self,
        contest_id: &str,
        problem_id: &str,
        lang_id: &str,
        source: &str,
    ) -> String;
}

/// Resolves the `OnlineJudge` implementation for a given `OJKind`.
///
/// `Service` holds a registry rather than a single `OnlineJudge`, so commands can
/// target the OJ recorded in `.ce.toml` (see `submit`) instead of a fixed implementation.
pub trait OnlineJudgeRegistry {
    /// Returns the implementation for `oj`, or an error if the OJ is unsupported.
    fn get(&self, oj: &OJKind) -> Result<&dyn OnlineJudge>;
}

/// A registry holding a single `OnlineJudge`, returned for any `OJKind`.
///
/// Useful for single-OJ contexts and tests. Production builds use a registry that
/// matches on `OJKind`.
pub struct SingleOnlineJudge(Box<dyn OnlineJudge>);

impl SingleOnlineJudge {
    pub fn new(oj: Box<dyn OnlineJudge>) -> Self {
        Self(oj)
    }
}

impl OnlineJudgeRegistry for SingleOnlineJudge {
    fn get(&self, _oj: &OJKind) -> Result<&dyn OnlineJudge> {
        Ok(self.0.as_ref())
    }
}
