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
pub struct Language(String);

impl Language {
    pub fn new(s: &str) -> Self {
        Language(s.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Directory name used under solutions/.
    pub fn dir_name(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Language {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            Err("language must not be empty".to_string())
        } else {
            Ok(Language(s.to_string()))
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
    pub input_format_raw: Option<String>,
    pub constraints_raw: Option<String>,
}

// ─── InputSpec types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VarType {
    Int,
    Str,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VarDecl {
    pub name: String,
    pub math: String,
    pub var_type: VarType,
    pub dim: u8,
    pub size: Vec<String>,
    pub is_size: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VarRef {
    pub name: String,
    pub dim: u8,
    pub size: Option<String>,
    pub index: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpTag {
    ReadLine,
    LoopBegin,
    LoopEnd,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InputOp {
    pub tag: OpTag,
    pub depth: u8,
    pub vars: Vec<VarRef>,
    pub loop_var: Option<String>,
    pub begin: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InputSpec {
    pub raw: String,
    pub ok: bool,
    pub vars: Vec<VarDecl>,
    pub ops: Vec<InputOp>,
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
    /// Problem title — used to populate the `problem.title` Tera variable when expanding templates.
    pub problem_title: String,
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

#[cfg(test)]
mod input_spec_tests {
    use super::*;

    // 1. Problem can be constructed with input_format_raw: None and constraints_raw: None
    #[test]
    fn problem_with_optional_fields_none() {
        let p = Problem {
            id: "abc001_a".to_string(),
            code: "a".to_string(),
            title: "Test".to_string(),
            samples: vec![],
            input_format_raw: None,
            constraints_raw: None,
        };
        assert_eq!(p.input_format_raw, None);
        assert_eq!(p.constraints_raw, None);
    }

    // 2. InputSpec { ok: false, raw: "", vars: vec![], ops: vec![] } can be constructed
    #[test]
    fn input_spec_empty_can_be_constructed() {
        let spec = InputSpec {
            raw: "".to_string(),
            ok: false,
            vars: vec![],
            ops: vec![],
        };
        assert!(!spec.ok);
        assert!(spec.vars.is_empty());
        assert!(spec.ops.is_empty());
    }

    // 3. InputSpec with vars and ops can be constructed and serialized to JSON
    #[test]
    fn input_spec_with_data_serializes_to_json() {
        let spec = InputSpec {
            raw: "N\nA_1 ... A_N".to_string(),
            ok: true,
            vars: vec![
                VarDecl {
                    name: "n".to_string(),
                    math: "N".to_string(),
                    var_type: VarType::Int,
                    dim: 0,
                    size: vec![],
                    is_size: false,
                },
                VarDecl {
                    name: "a".to_string(),
                    math: "A".to_string(),
                    var_type: VarType::Int,
                    dim: 1,
                    size: vec!["n".to_string()],
                    is_size: false,
                },
            ],
            ops: vec![
                InputOp {
                    tag: OpTag::ReadLine,
                    depth: 0,
                    vars: vec![VarRef {
                        name: "n".to_string(),
                        dim: 0,
                        size: None,
                        index: None,
                    }],
                    loop_var: None,
                    begin: None,
                    end: None,
                },
                InputOp {
                    tag: OpTag::LoopBegin,
                    depth: 0,
                    vars: vec![],
                    loop_var: Some("i".to_string()),
                    begin: Some("0".to_string()),
                    end: Some("n".to_string()),
                },
                InputOp {
                    tag: OpTag::ReadLine,
                    depth: 1,
                    vars: vec![VarRef {
                        name: "a".to_string(),
                        dim: 1,
                        size: None,
                        index: Some("i".to_string()),
                    }],
                    loop_var: None,
                    begin: None,
                    end: None,
                },
                InputOp {
                    tag: OpTag::LoopEnd,
                    depth: 0,
                    vars: vec![],
                    loop_var: None,
                    begin: None,
                    end: None,
                },
            ],
        };
        assert!(spec.ok);
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["vars"][0]["name"], "n");
        assert_eq!(json["ops"][1]["tag"], "loop_begin");
    }

    // 4. VarType::Int serializes to "int"
    #[test]
    fn var_type_int_serializes_to_int() {
        let v = VarType::Int;
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json, "int");
    }

    // 5. OpTag::ReadLine serializes to "read_line"
    #[test]
    fn op_tag_read_line_serializes_to_read_line() {
        let t = OpTag::ReadLine;
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json, "read_line");
    }
}

#[cfg(test)]
mod language_tests {
    use super::Language;
    use std::str::FromStr;

    // Language::new constructs from any string
    #[test]
    fn new_returns_language_with_given_string() {
        let lang = Language::new("rust");
        assert_eq!(lang.as_str(), "rust");
    }

    #[test]
    fn new_accepts_arbitrary_string() {
        let lang = Language::new("haskell");
        assert_eq!(lang.as_str(), "haskell");
    }

    // as_str returns the inner string slice
    #[test]
    fn as_str_returns_inner_string() {
        let lang = Language::new("cpp");
        assert_eq!(lang.as_str(), "cpp");
    }

    // Display shows the inner string
    #[test]
    fn display_shows_inner_string() {
        let lang = Language::new("rust");
        assert_eq!(format!("{}", lang), "rust");
    }

    #[test]
    fn display_shows_arbitrary_language() {
        let lang = Language::new("python3");
        assert_eq!(format!("{}", lang), "python3");
    }

    // FromStr parses any non-empty string
    #[test]
    fn from_str_parses_rust() {
        let lang: Language = "rust".parse().unwrap();
        assert_eq!(lang.as_str(), "rust");
    }

    #[test]
    fn from_str_parses_cpp() {
        let lang: Language = "cpp".parse().unwrap();
        assert_eq!(lang.as_str(), "cpp");
    }

    #[test]
    fn from_str_parses_arbitrary_non_empty_string() {
        let lang: Language = "go".parse().unwrap();
        assert_eq!(lang.as_str(), "go");
    }

    #[test]
    fn from_str_errors_on_empty_string() {
        let result = Language::from_str("");
        assert!(result.is_err());
    }

    #[test]
    fn from_str_trims_whitespace() {
        let lang: Language = "rust ".parse().unwrap();
        assert_eq!(lang.as_str(), "rust");
    }

    #[test]
    fn from_str_errors_on_whitespace_only() {
        let result = Language::from_str("   ");
        assert!(result.is_err());
    }

    // dir_name returns same as as_str (used for template directory lookup)
    #[test]
    fn dir_name_returns_same_as_as_str() {
        let lang = Language::new("rust");
        assert_eq!(lang.dir_name(), lang.as_str());
    }

    #[test]
    fn dir_name_returns_inner_string() {
        let lang = Language::new("cpp");
        assert_eq!(lang.dir_name(), "cpp");
    }

    // PartialEq / Clone derived
    #[test]
    fn equality_holds_for_same_string() {
        let a = Language::new("rust");
        let b = Language::new("rust");
        assert_eq!(a, b);
    }

    #[test]
    fn inequality_holds_for_different_strings() {
        let a = Language::new("rust");
        let b = Language::new("cpp");
        assert_ne!(a, b);
    }

    #[test]
    fn clone_produces_equal_value() {
        let a = Language::new("rust");
        let b = a.clone();
        assert_eq!(a, b);
    }

    // Serde round-trip
    #[test]
    fn serde_round_trip() {
        let original = Language::new("rust");
        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: Language = serde_json::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }
}
