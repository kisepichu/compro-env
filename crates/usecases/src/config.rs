use anyhow::Result;
use domain::entity::{Language, OJKind};

pub trait Config {
    fn default_language(&self) -> Result<Language>;
    fn default_online_judge(&self) -> OJKind;

    /// Test command template. Placeholders {problem}, {solution}, {file} are substituted.
    fn test_command(&self, lang: &Language) -> String;

    fn run_command(&self, lang: &Language) -> String;

    /// Path of the file to submit (e.g. "src/main.rs").
    fn submit_file(&self, lang: &Language) -> String;

    /// Pre-submission command (empty string if not needed).
    fn submit_preprocess(&self, lang: &Language) -> String;

    /// Language ID passed to the OJ (e.g. "5054" for Rust on AtCoder).
    fn lang_id(&self, lang: &Language, oj: &OJKind) -> Option<String>;
}
