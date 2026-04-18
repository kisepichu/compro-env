use clap::{Parser, Subcommand};
use domain::entity::{Language, OJKind};
use interfaces::controller::input::{
    InitInput, LoginInput, LogoutInput, NewInput, SubmitInput, TestInput, WhoamiInput,
};

#[derive(Parser)]
#[command(name = "ce", about = "Competitive programming environment")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Log in to an OJ by saving your REVEL_SESSION cookie
    ///
    /// Steps:
    ///   1. Open https://atcoder.jp and log in with your browser.
    ///   2. Open DevTools > Application > Cookies > https://atcoder.jp
    ///   3. Copy the value of REVEL_SESSION.
    ///   4. Run: ce login [atcoder]
    ///      You will be prompted to paste the cookie value.
    ///      Alternatively, pass it directly: ce login [atcoder] --cookie VALUE
    #[command(verbatim_doc_comment)]
    Login {
        /// Target OJ (default: atcoder)
        oj: Option<String>,
        /// REVEL_SESSION cookie value (prompted interactively if omitted)
        #[arg(long)]
        cookie: Option<String>,
    },
    /// Check the username of the currently logged-in user
    Whoami { oj: Option<String> },
    /// Log out from an OJ by removing the saved session
    Logout { oj: Option<String> },
    /// Initialize a contest (fetch problems and create directories)
    Init {
        /// Contest ID or URL
        contest: String,
        /// Language override (e.g. rust, cpp); uses config default if omitted
        #[arg(long)]
        lang: Option<String>,
    },
    /// Manage solution directories
    Solution {
        #[command(subcommand)]
        subcommand: SolutionSubcommand,
    },
    /// Run sample tests
    Test {
        contest: String,
        problem: String,
        solution: Option<String>,
    },
    /// Submit a solution
    #[command(alias = "sub")]
    Submit {
        contest: String,
        problem: String,
        solution: Option<String>,
        #[arg(long)]
        lang: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum SolutionSubcommand {
    /// Add a solution directory
    Add {
        contest: String,
        problem: String,
        /// Solution name (default: main)
        solution: Option<String>,
        #[arg(long)]
        lang: Option<String>,
    },
}

// ─── Input trait implementations ─────────────────────────────────────────────
// clap structs implement the Input traits from the interfaces layer so that
// Controller does not depend on clap.

pub struct LoginCommand {
    pub oj: OJKind,
    pub cookie: String,
}
impl LoginInput for LoginCommand {
    fn oj(&self) -> OJKind {
        self.oj.clone()
    }
    fn cookie(&self) -> String {
        self.cookie.clone()
    }
}

pub struct WhoamiCommand {
    pub oj: OJKind,
}
impl WhoamiInput for WhoamiCommand {
    fn oj(&self) -> OJKind {
        self.oj.clone()
    }
}

pub struct LogoutCommand {
    pub oj: OJKind,
}
impl LogoutInput for LogoutCommand {
    fn oj(&self) -> OJKind {
        self.oj.clone()
    }
}

pub struct InitCommand {
    pub contest_id: String,
    pub oj: OJKind,
    pub language: Language,
}
impl InitInput for InitCommand {
    fn contest_id(&self) -> String {
        self.contest_id.clone()
    }
    fn oj(&self) -> OJKind {
        self.oj.clone()
    }
    fn language(&self) -> Language {
        self.language.clone()
    }
}

pub struct NewCommand {
    pub contest_id: String,
    pub problem_code: String,
    pub solution_name: String,
    pub language: Language,
}
impl NewInput for NewCommand {
    fn contest_id(&self) -> String {
        self.contest_id.clone()
    }
    fn problem_code(&self) -> String {
        self.problem_code.clone()
    }
    fn solution_name(&self) -> String {
        self.solution_name.clone()
    }
    fn language(&self) -> Language {
        self.language.clone()
    }
}

pub struct TestCommand {
    pub contest_id: String,
    pub problem_code: String,
    pub solution_name: String,
}
impl TestInput for TestCommand {
    fn contest_id(&self) -> String {
        self.contest_id.clone()
    }
    fn problem_code(&self) -> String {
        self.problem_code.clone()
    }
    fn solution_name(&self) -> String {
        self.solution_name.clone()
    }
}

pub struct SubmitCommand {
    pub contest_id: String,
    pub problem_code: String,
    pub solution_name: String,
    pub language: Language,
}
impl SubmitInput for SubmitCommand {
    fn contest_id(&self) -> String {
        self.contest_id.clone()
    }
    fn problem_code(&self) -> String {
        self.problem_code.clone()
    }
    fn solution_name(&self) -> String {
        self.solution_name.clone()
    }
    fn language(&self) -> Language {
        self.language.clone()
    }
}
