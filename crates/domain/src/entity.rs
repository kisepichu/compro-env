use serde::{Deserialize, Serialize};
use std::str::FromStr;

// ─── Value Objects ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OJKind {
    AtCoder,
}

impl OJKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            OJKind::AtCoder => "atcoder",
        }
    }

    /// Infer OJ kind from contest ID prefix.
    pub fn from_contest_id_prefix(id: &str) -> Option<Self> {
        let lower = id.to_lowercase();
        if lower.starts_with("abc")
            || lower.starts_with("arc")
            || lower.starts_with("ahc")
            || lower.starts_with("agc")
        {
            Some(OJKind::AtCoder)
        } else {
            None
        }
    }
}

impl std::fmt::Display for OJKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for OJKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "atcoder" => Ok(OJKind::AtCoder),
            _ => Err(format!("unknown online judge: {s}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    Rust,
    Cpp,
}

impl Language {
    /// Directory name used under solutions/.
    pub fn dir_name(&self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::Cpp => "cpp",
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.dir_name())
    }
}

impl FromStr for Language {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "rust" => Ok(Language::Rust),
            "cpp" => Ok(Language::Cpp),
            _ => Err(format!("unknown language: {s}")),
        }
    }
}

// ─── Entities ─────────────────────────────────────────────────────────────────

/// Contest (Aggregate Root)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contest {
    pub id: String,
    pub online_judge: OJKind,
    pub problems: Vec<Problem>,
}

/// Problem (Entity under Contest)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// OJ-specific problem ID (e.g. "abc334_a" for AtCoder)
    pub id: String,
    /// Code used for directory names and command arguments (e.g. "a", "ex", "practice_2")
    pub code: String,
    pub title: String,
    pub samples: Vec<Sample>,
}

/// Sample input/output (Value Object)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sample {
    pub input: String,
    pub output: String,
}

/// Solution (independent Aggregate)
///
/// Cargo package name: "{problem_code}-{solution_name}" for uniqueness within the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Solution {
    pub contest_id: String,
    pub problem_code: String,
    pub name: String,
    pub language: Language,
}

impl Solution {
    /// Cargo package name. Uses "-" separator to avoid duplicates within the same workspace.
    pub fn cargo_package_name(&self) -> String {
        format!("{}-{}", self.problem_code, self.name)
    }
}

/// OJ session credentials (Value Object)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub online_judge: OJKind,
    pub cookie: String,
}

/// Result of a submission
#[derive(Debug, Clone)]
pub struct SubmitResult {
    pub submission_url: String,
}
