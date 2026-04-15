use domain::entity::{Language, OJKind};

pub trait LoginInput {
    fn oj(&self) -> OJKind;
    fn cookie(&self) -> String;
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
    fn language(&self) -> Language;
}
