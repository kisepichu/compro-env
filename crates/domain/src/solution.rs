//! Discovery-level solution model shared by the library platform pipeline.
//!
//! This is intentionally separate from `crate::entity::Solution`, which the
//! existing CLI commands (test/submit) continue to use. The new
//! `PublishedSolution` carries the publication and verify metadata required by
//! the library platform (spec §4.2, §5, §7.2).

use chrono::{DateTime, FixedOffset};

use crate::library::{LanguageId, LibraryId, SolutionId};

/// A public solution discovered under `solutions/` (spec §4.2).
///
/// Publication is opt-in: only solutions whose `ce.toml` declares
/// `publish = true` become `PublishedSolution`s. Private solutions still exist
/// in the repository but are omitted from the discovery manifest passed to
/// adapters (spec §6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedSolution {
    pub id: SolutionId,
    pub language: LanguageId,
    pub root: String,
    pub entry: String,
    pub solved_at: DateTime<FixedOffset>,
    pub test_command: String,
    pub test_timeout_seconds: u32,
    pub verify: Option<VerifySpec>,
}

/// `[verify]` block resolved against project-local OJ language mapping (spec §7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifySpec {
    /// Libraries this solution asserts as verified.
    pub libraries: Vec<LibraryId>,
    /// Resolved OJ submission language ID (from the solution override or the
    /// project-local `[library.languages.<id>.online_judges.<oj>]` mapping).
    pub oj_language_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_utc() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-08-02T14:30:00+09:00").unwrap()
    }

    #[test]
    fn published_solution_stores_verify_spec() {
        let solved_at = fixed_utc();
        let solution = PublishedSolution {
            id: SolutionId::parse("librarychecker-aplusb/aplusb/main").unwrap(),
            language: LanguageId::parse("rust").unwrap(),
            root: "solutions/librarychecker-aplusb/aplusb/main".into(),
            entry: "src/main.rs".into(),
            solved_at,
            test_command: "./test.sh".into(),
            test_timeout_seconds: 600,
            verify: Some(VerifySpec {
                libraries: vec![LibraryId::parse("libraries/rust/algebra/monoid.rs").unwrap()],
                oj_language_id: "rust".into(),
            }),
        };
        assert_eq!(solution.id.solution_name(), "main");
        assert!(solution.verify.is_some());
        assert_eq!(solution.solved_at, solved_at);
    }

    #[test]
    fn published_solution_without_verify_is_not_configured() {
        let solution = PublishedSolution {
            id: SolutionId::parse("abc999/a/main").unwrap(),
            language: LanguageId::parse("rust").unwrap(),
            root: "solutions/abc999/a/main".into(),
            entry: "src/main.rs".into(),
            solved_at: fixed_utc(),
            test_command: "./test.sh".into(),
            test_timeout_seconds: 600,
            verify: None,
        };
        assert!(solution.verify.is_none());
    }
}
