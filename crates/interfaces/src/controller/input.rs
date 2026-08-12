use domain::entity::{Language, OJKind};
use usecases::online_judge::Credentials;

pub trait LoginInput {
    fn oj(&self) -> OJKind;
    fn credentials(&self) -> Credentials;
}

pub trait WhoamiInput {
    fn oj(&self) -> OJKind;
}

pub trait LogoutInput {
    fn oj(&self) -> OJKind;
}

pub trait InitInput {
    fn contest_id(&self) -> String;
    fn oj(&self) -> OJKind;
    fn language(&self) -> Language;
}

pub trait NewInput {
    fn contest_id(&self) -> String;
    fn problem_code(&self) -> String;
    fn solution_name(&self) -> String;
    fn language(&self) -> Language;
}

pub trait TestInput {
    fn contest_id(&self) -> String;
    fn problem_code(&self) -> String;
    fn solution_name(&self) -> String;
}

pub trait SubmitInput {
    fn contest_id(&self) -> String;
    fn problem_code(&self) -> String;
    fn solution_name(&self) -> String;
}

pub trait CheckInput {
    /// `--language <id>` argument if the user narrowed the run; `None` means
    /// "check every configured language".
    fn language(&self) -> Option<String>;
}

pub trait VerifyInput {
    /// Optional positional `[solution-id]` argument. `None` means "walk the
    /// entire discovery manifest".
    fn solution(&self) -> Option<String>;
}

pub trait InternalVerifyPrepareInput {
    fn solution(&self) -> String;
    fn plan_out(&self) -> String;
    /// Optional `--starting-out FILE`: when set, also emit the `Starting`
    /// `VerificationRecord` JSON so the App-only persist job can push it
    /// without contacting the OJ (spec §15.4, dry-run path).
    fn starting_out(&self) -> Option<String>;
}

pub trait InternalVerifyStartInput {
    fn plan_in(&self) -> String;
}

pub trait InternalVerifyPollInput {
    fn solution(&self) -> String;
}

pub trait SiteDataGenerateInput {
    /// `--output <dir>` argument. `None` means the default under
    /// `target/ce-site-data`.
    fn output(&self) -> Option<String>;
    /// `--mode production|preview`; `production` is strict.
    fn mode(&self) -> SiteDataBuildMode;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteDataBuildMode {
    Production,
    Preview,
}
