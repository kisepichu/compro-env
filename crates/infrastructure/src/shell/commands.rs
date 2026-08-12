use clap::{Parser, Subcommand};
use domain::entity::{Language, OJKind};
use interfaces::controller::input::{
    CheckInput, InitInput, InternalVerifyPollInput, InternalVerifyPrepareInput,
    InternalVerifyStartInput, LoginInput, LogoutInput, NewInput, SiteDataBuildMode,
    SiteDataGenerateInput, SubmitInput, TestInput, VerifyInput, WhoamiInput,
};
use usecases::online_judge::Credentials;

#[derive(Parser)]
#[command(name = "ce", about = "Competitive programming environment")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Log in to an OJ. The required input depends on the OJ.
    ///
    /// AtCoder uses a manually copied REVEL_SESSION cookie:
    ///   1. Open https://atcoder.jp and log in with your browser.
    ///   2. Open DevTools > Application > Cookies > https://atcoder.jp
    ///   3. Copy the value of REVEL_SESSION.
    ///   4. Run: ce login [atcoder]
    ///      You will be prompted to paste the cookie value.
    ///      Alternatively, pass it directly: ce login [atcoder] --cookie VALUE
    ///
    /// Other OJs may prompt for different credentials (e.g. email + password).
    #[command(verbatim_doc_comment)]
    Login {
        /// Target OJ (default: atcoder)
        oj: Option<String>,
        /// REVEL_SESSION cookie value for cookie-based OJs like AtCoder
        /// (prompted interactively if omitted; ignored by password-based OJs)
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
        /// Prepare the source (incl. preprocess) and print it without submitting
        #[arg(long)]
        dry_run: bool,
    },
    /// Run project-local library checks configured under `[library.languages]`.
    Check {
        /// Limit the run to a single language id (default: all languages).
        #[arg(long)]
        language: Option<String>,
    },
    /// Manage the public library site data JSON.
    SiteData {
        #[command(subcommand)]
        subcommand: SiteDataSubcommand,
    },
    /// Resumable verify of library solutions against their configured OJ.
    Verify {
        /// Optional solution id (e.g. `librarychecker-aplusb/aplusb/main`).
        /// Defaults to walking the entire discovery manifest.
        solution: Option<String>,
    },
    /// Hidden CI-boundary helpers for the verify pipeline (spec §8.1).
    #[command(hide = true)]
    Internal {
        #[command(subcommand)]
        subcommand: InternalSubcommand,
    },
}

#[derive(Subcommand)]
pub enum InternalSubcommand {
    /// Freeze a submission plan JSON so a later `verify-start` can dispatch it.
    /// Writes only the canonical plan bytes; the `Starting` record is persisted
    /// by `verify-start` before it contacts the OJ.
    #[command(hide = true)]
    VerifyPrepare {
        #[arg(long)]
        solution: String,
        #[arg(long = "plan-out")]
        plan_out: String,
    },
    /// Dispatch a previously-prepared plan via the OJ starter.
    #[command(hide = true)]
    VerifyStart {
        #[arg(long = "plan-in")]
        plan_in: String,
    },
    /// Drive the stored record for a solution forward one poll tick.
    #[command(hide = true)]
    VerifyPoll {
        #[arg(long)]
        solution: String,
    },
    /// Persist a candidate `VerificationRecord` through the GitHub state
    /// writer. Reads a plan-hash file and a candidate-record JSON. Never
    /// contacts an online judge; the plan-hash gate lives on this
    /// (secret-bearing) side of the automation split (spec §15.1, §15.4).
    #[command(hide = true)]
    VerifyPersist {
        /// Path to the immutable plan hash file produced by the secretless job.
        #[arg(long)]
        plan_hash_in: String,
        /// Path to the serialized `VerificationRecord` JSON to persist.
        #[arg(long)]
        candidate_in: String,
        /// `owner/repo` slug of the target repository.
        #[arg(long)]
        repository: String,
        /// Main branch commit SHA the plan was built against (40 lowercase hex).
        #[arg(long)]
        base_sha: String,
        /// Name of the env var carrying the App installation token. Its value
        /// is never echoed or logged.
        #[arg(long, default_value = "GITHUB_TOKEN")]
        token_env: String,
    },
    /// Validate that a bot PR's changed files are exclusively verification
    /// result JSONs and that every touched result JSON deserializes as a
    /// `VerificationRecord`. Secretless: no OJ or GitHub App contact
    /// (spec §15.3, §15.4).
    #[command(hide = true)]
    VerifyValidateResultPr {
        /// Base commit SHA (PR base).
        #[arg(long)]
        before: String,
        /// Head commit SHA (PR head).
        #[arg(long)]
        after: String,
        /// Repository root (defaults to CWD).
        #[arg(long, default_value = ".")]
        root: String,
    },
    /// Classify a push's file changes without contacting the OJ. Prints
    /// exactly one of `empty`, `result-only`, or `source-or-config`
    /// (spec §15.3).
    #[command(hide = true)]
    ClassifyChanges {
        /// Previous commit SHA.
        #[arg(long)]
        before: String,
        /// New commit SHA.
        #[arg(long)]
        after: String,
        /// Repository root (defaults to CWD).
        #[arg(long, default_value = ".")]
        root: String,
    },
}

#[derive(Subcommand)]
pub enum SiteDataSubcommand {
    /// Generate `site-data.json` atomically. Runs entirely offline.
    Generate {
        /// Output directory. Defaults to `target/ce-site-data` under the repo.
        #[arg(long)]
        output: Option<String>,
        /// Build mode: `production` (default, strict) or `preview`.
        #[arg(long, default_value = "production")]
        mode: String,
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
    pub credentials: Credentials,
}
impl LoginInput for LoginCommand {
    fn oj(&self) -> OJKind {
        self.oj.clone()
    }
    fn credentials(&self) -> Credentials {
        self.credentials.clone()
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
}

pub struct CheckCommand {
    pub language: Option<String>,
}
impl CheckInput for CheckCommand {
    fn language(&self) -> Option<String> {
        self.language.clone()
    }
}

pub struct SiteDataGenerateCommand {
    pub output: Option<String>,
    pub mode: SiteDataBuildMode,
}

impl SiteDataGenerateInput for SiteDataGenerateCommand {
    fn output(&self) -> Option<String> {
        self.output.clone()
    }
    fn mode(&self) -> SiteDataBuildMode {
        self.mode
    }
}

pub struct VerifyCommand {
    pub solution: Option<String>,
}
impl VerifyInput for VerifyCommand {
    fn solution(&self) -> Option<String> {
        self.solution.clone()
    }
}

pub struct InternalVerifyPrepareCommand {
    pub solution: String,
    pub plan_out: String,
}
impl InternalVerifyPrepareInput for InternalVerifyPrepareCommand {
    fn solution(&self) -> String {
        self.solution.clone()
    }
    fn plan_out(&self) -> String {
        self.plan_out.clone()
    }
}

pub struct InternalVerifyStartCommand {
    pub plan_in: String,
}
impl InternalVerifyStartInput for InternalVerifyStartCommand {
    fn plan_in(&self) -> String {
        self.plan_in.clone()
    }
}

pub struct InternalVerifyPollCommand {
    pub solution: String,
}
impl InternalVerifyPollInput for InternalVerifyPollCommand {
    fn solution(&self) -> String {
        self.solution.clone()
    }
}
