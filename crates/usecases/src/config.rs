use anyhow::Result;
use domain::entity::{Language, OJKind};
use std::path::Path;

pub trait Config {
    fn default_language(&self) -> Result<Language>;
    fn default_online_judge(&self) -> OJKind;

    /// Path of the file to submit (e.g. "src/main.rs").
    fn submit_file(&self, lang: &Language) -> String;

    /// Pre-submission hook command from `[submit].preprocess` (None if not configured).
    /// Language/OJ branching is the script's responsibility (passed via env), so this
    /// takes no `Language`: there is a single global hook, not a per-language one.
    fn submit_preprocess(&self) -> Option<String>;

    /// Language ID passed to the OJ (e.g. "5054" for Rust on AtCoder).
    fn lang_id(&self, lang: &Language, oj: &OJKind) -> Option<String>;

    /// Repository root — the directory holding the project-local `config.toml`.
    /// Preprocess hooks receive it via `$CE_PROJECT_ROOT`, and infrastructure
    /// uses it to absolutise project-local relative paths in `submit_preprocess`.
    fn project_root(&self) -> &Path;
}
