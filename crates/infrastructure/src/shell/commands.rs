use clap::{Parser, Subcommand};
use domain::entity::{Language, OJKind};
use interfaces::controller::input::{
    InitInput, LoginInput, NewInput, SubmitInput, TestInput, WhoamiInput,
};

#[derive(Parser)]
#[command(name = "ce", about = "Competitive programming environment")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Log in to an OJ (manually enter the REVEL_SESSION cookie)
    Login {
        /// Target OJ (defaults to the value in config)
        oj: Option<String>,
    },
    /// Check the username of the currently logged-in user
    Whoami { oj: Option<String> },
    /// Initialize a contest (fetch problems and create directories)
    Init {
        /// Contest ID or URL
        contest: String,
    },
    /// Add a solution directory
    New {
        contest: String,
        problem: String,
        /// Solution name (default: main)
        solution: Option<String>,
        #[arg(long)]
        lang: Option<String>,
    },
    /// Run sample tests
    Test {
        contest: String,
        problem: String,
        solution: Option<String>,
        #[arg(long)]
        lang: Option<String>,
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
    pub language: Language,
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
    fn language(&self) -> Language {
        self.language.clone()
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
