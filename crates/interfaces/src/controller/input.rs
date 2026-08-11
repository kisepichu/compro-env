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
