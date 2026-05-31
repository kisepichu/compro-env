use serde::{Deserialize, Serialize};
use std::str::FromStr;

// ─── Value Objects ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OJKind {
    AtCoder,
    LibraryChecker,
}

/// Static descriptor used to detect an `OJKind` from a contest input string.
struct OjDescriptor {
    kind: OJKind,
    /// URL host, e.g. "atcoder.jp".
    url_host: &'static str,
    /// Path segment that precedes the contest id, e.g. "/contests/".
    url_path_prefix: &'static str,
    /// Prefix prepended to the id extracted from a URL, e.g. "librarychecker-".
    /// Empty for OJs whose contest id is used verbatim (AtCoder).
    contest_id_prefix: &'static str,
    /// contest_id prefixes (lowercase), e.g. ["abc", "arc", "agc", "ahc"].
    id_prefixes: &'static [&'static str],
}

/// Descriptor table for all supported online judges.
const OJ_DESCRIPTORS: &[OjDescriptor] = &[
    OjDescriptor {
        kind: OJKind::AtCoder,
        url_host: "atcoder.jp",
        url_path_prefix: "/contests/",
        contest_id_prefix: "",
        id_prefixes: &["abc", "arc", "agc", "ahc"],
    },
    OjDescriptor {
        kind: OJKind::LibraryChecker,
        url_host: "judge.yosupo.jp",
        url_path_prefix: "/problem/",
        // LibraryChecker has no contest concept: a problem is a single-problem
        // "contest". The id is namespaced with "librarychecker-" (e.g.
        // "librarychecker-aplusb") so it cannot collide with AtCoder ids and is
        // recognizable on its own. The bare problem name is recovered at fetch/submit.
        contest_id_prefix: "librarychecker-",
        id_prefixes: &["librarychecker-"],
    },
];

impl OJKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            OJKind::AtCoder => "atcoder",
            OJKind::LibraryChecker => "librarychecker",
        }
    }

    /// Detects the OJ kind and contest id from a contest input string (URL or id).
    ///
    /// Pure: no filesystem, no network, no path-safety validation.
    /// - URL match: `https://{url_host}{url_path_prefix}{id}...` → takes only the
    ///   first path segment after the prefix, lowercased, with `contest_id_prefix`
    ///   prepended. Returns `None` if empty.
    /// - Prefix match: lowercased input starting with any `id_prefix` → the lowercased input.
    /// - Otherwise `None`.
    pub fn detect(input: &str) -> Option<(OJKind, String)> {
        // First, try URL matches.
        for d in OJ_DESCRIPTORS {
            let url_prefix = format!("https://{}{}", d.url_host, d.url_path_prefix);
            if let Some(rest) = input.strip_prefix(&url_prefix) {
                let id = rest.trim_end_matches('/').split('/').next()?;
                if id.is_empty() {
                    return None;
                }
                return Some((
                    d.kind.clone(),
                    format!("{}{}", d.contest_id_prefix, id.to_lowercase()),
                ));
            }
        }
        // Then, try contest-id prefix matches.
        let lower = input.to_lowercase();
        for d in OJ_DESCRIPTORS {
            if d.id_prefixes.iter().any(|p| lower.starts_with(p)) {
                return Some((d.kind.clone(), lower));
            }
        }
        None
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
            "librarychecker" => Ok(OJKind::LibraryChecker),
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
    pub is_jagged: bool,
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
    LoopJagged,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InputOp {
    pub tag: OpTag,
    pub depth: u8,
    pub vars: Vec<VarRef>,
    pub loop_var: Option<String>,
    pub begin: Option<String>,
    pub end: Option<String>,
    pub scalars: Vec<VarRef>,
    pub size_var: Option<VarRef>,
    pub elem_var: Option<VarRef>,
}

/// Upper-triangular matrix input specification.
/// Row i (0-indexed) contains `bound-1-i` elements: A_{i+1, i+2} ... A_{i+1, bound}.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TriangularSpec {
    /// Lowercase variable name (e.g. "a").
    pub name: String,
    /// Original math notation (e.g. "A").
    pub math: String,
    pub var_type: VarType,
    /// Normalized upper-bound expression for the second subscript (e.g. "n", "2*n").
    /// Row count = bound-1; row i length = bound-1-i.
    pub bound: String,
}

/// One query sub-type decoded from a numbered sub-block (e.g. "1 x" or "2 x k").
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueryTypeDecl {
    /// The numeric type identifier ("1", "2", …).
    pub type_id: String,
    /// Whether the sub-block was successfully parsed.
    pub ok: bool,
    /// Local scalar variables for this query type (empty when ok=false).
    pub vars: Vec<VarDecl>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InputSpec {
    pub raw: String,
    pub ok: bool,
    pub vars: Vec<VarDecl>,
    pub ops: Vec<InputOp>,
    /// Non-empty only for query-type input (`\text{query}_Q` form) when numbered
    /// sub-blocks are present.  Each entry corresponds to one query kind.
    pub query_types: Vec<QueryTypeDecl>,
    /// Non-empty only for single-format query input: the first non-numeric sub-block's
    /// scalar variables (e.g. abc334_d's "X" → [x: i64]).
    /// Always empty when `query_types` is non-empty.
    pub query_body: Vec<VarDecl>,
    /// Non-empty only for T-testcases inputs (block 0 = single scalar T, block 1 = body).
    /// Contains the scalar variables of block 1. The loop bound is `vars[0].name`.
    pub testcase_body: Vec<VarDecl>,
    /// Non-empty only when block[1] contains loops/arrays that failed scalar parse.
    /// Parsed as an independent mini-InputSpec from block[1].  Empty when `query_body` or
    /// `testcase_body` is non-empty, or when `query_types` is non-empty.
    pub iteration_vars: Vec<VarDecl>,
    /// Read ops corresponding to `iteration_vars`.  Same structure as `ops`.
    pub iteration_ops: Vec<InputOp>,
    /// Some when the input is an upper-triangular matrix pattern.
    pub triangular: Option<TriangularSpec>,
}

/// Derived format kind of a parsed `InputSpec` — used for `ce init` summary output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputFormatKind {
    /// ok=false
    Fail,
    /// query_types is non-empty; carries the number of distinct query types
    QueryTypes(usize),
    /// query_body is non-empty (single-format loop, scalar body)
    Query,
    /// testcase_body is non-empty (simple T-testcases pattern)
    Testcase,
    /// triangular is Some (upper-triangular matrix)
    Triangle,
    /// iteration_ops is non-empty (complex loop body)
    Iter,
    /// ops contains a LoopBegin (empty-body loop stub)
    Loop,
    /// ok=true with no loop/query/iteration markers
    Plain,
    /// ops contains a LoopJagged (jagged array loop)
    Jagged,
}

impl std::fmt::Display for InputFormatKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InputFormatKind::Fail => write!(f, "FAIL"),
            InputFormatKind::QueryTypes(n) => write!(f, "query({n})"),
            InputFormatKind::Query => write!(f, "query"),
            InputFormatKind::Testcase => write!(f, "testcase"),
            InputFormatKind::Triangle => write!(f, "triangle"),
            InputFormatKind::Iter => write!(f, "iter"),
            InputFormatKind::Loop => write!(f, "loop"),
            InputFormatKind::Plain => write!(f, "plain"),
            InputFormatKind::Jagged => write!(f, "jagged"),
        }
    }
}

impl InputSpec {
    /// Derive the `InputFormatKind` from this spec's fields.
    pub fn kind(&self) -> InputFormatKind {
        if !self.ok {
            return InputFormatKind::Fail;
        }
        if !self.query_types.is_empty() {
            return InputFormatKind::QueryTypes(self.query_types.len());
        }
        if !self.query_body.is_empty() {
            return InputFormatKind::Query;
        }
        if !self.testcase_body.is_empty() {
            return InputFormatKind::Testcase;
        }
        if self.triangular.is_some() {
            return InputFormatKind::Triangle;
        }
        if !self.iteration_ops.is_empty() {
            return InputFormatKind::Iter;
        }
        if self.ops.iter().any(|op| op.tag == OpTag::LoopJagged) {
            return InputFormatKind::Jagged;
        }
        if self.ops.iter().any(|op| op.tag == OpTag::LoopBegin) {
            return InputFormatKind::Loop;
        }
        InputFormatKind::Plain
    }
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

/// OJ session credentials (Value Object).
///
/// Each OJ stores different auth material, so this is an enum that distinguishes
/// the credential kind by type:
/// - `Cookie`: a manually-copied cookie (AtCoder `REVEL_SESSION`).
/// - `Firebase`: Firebase Auth tokens (LibraryChecker `id_token` + `refresh_token`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Session {
    Cookie {
        online_judge: OJKind,
        cookie: String,
    },
    Firebase {
        online_judge: OJKind,
        id_token: String,
        refresh_token: String,
    },
}

impl Session {
    /// The OJ this session authenticates against.
    pub fn online_judge(&self) -> &OJKind {
        match self {
            Session::Cookie { online_judge, .. } => online_judge,
            Session::Firebase { online_judge, .. } => online_judge,
        }
    }

    /// Overrides the OJ. Used by the login service to enforce that the caller-specified
    /// OJ is authoritative for where the session is stored.
    pub fn set_online_judge(&mut self, oj: OJKind) {
        match self {
            Session::Cookie { online_judge, .. } => *online_judge = oj,
            Session::Firebase { online_judge, .. } => *online_judge = oj,
        }
    }
}

#[cfg(test)]
mod input_spec_tests {
    use super::*;

    // Session enum: online_judge() accessor returns the OJ for each variant.
    #[test]
    fn session_online_judge_accessor() {
        let cookie = Session::Cookie {
            online_judge: OJKind::AtCoder,
            cookie: "c".to_string(),
        };
        assert_eq!(cookie.online_judge(), &OJKind::AtCoder);

        let firebase = Session::Firebase {
            online_judge: OJKind::LibraryChecker,
            id_token: "id".to_string(),
            refresh_token: "rf".to_string(),
        };
        assert_eq!(firebase.online_judge(), &OJKind::LibraryChecker);
    }

    // Session enum: set_online_judge() overrides the OJ for either variant.
    #[test]
    fn session_set_online_judge() {
        let mut s = Session::Cookie {
            online_judge: OJKind::LibraryChecker,
            cookie: "c".to_string(),
        };
        s.set_online_judge(OJKind::AtCoder);
        assert_eq!(s.online_judge(), &OJKind::AtCoder);
    }

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
            query_types: vec![],
            query_body: vec![],
            testcase_body: vec![],
            iteration_vars: vec![],
            iteration_ops: vec![],
            triangular: None,
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
                    is_jagged: false,
                },
                VarDecl {
                    name: "a".to_string(),
                    math: "A".to_string(),
                    var_type: VarType::Int,
                    dim: 1,
                    size: vec!["n".to_string()],
                    is_size: false,
                    is_jagged: false,
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
                    scalars: vec![],
                    size_var: None,
                    elem_var: None,
                },
                InputOp {
                    tag: OpTag::LoopBegin,
                    depth: 0,
                    vars: vec![],
                    loop_var: Some("i".to_string()),
                    begin: Some("0".to_string()),
                    end: Some("n".to_string()),
                    scalars: vec![],
                    size_var: None,
                    elem_var: None,
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
                    scalars: vec![],
                    size_var: None,
                    elem_var: None,
                },
                InputOp {
                    tag: OpTag::LoopEnd,
                    depth: 0,
                    vars: vec![],
                    loop_var: None,
                    begin: None,
                    end: None,
                    scalars: vec![],
                    size_var: None,
                    elem_var: None,
                },
            ],
            query_types: vec![],
            query_body: vec![],
            testcase_body: vec![],
            iteration_vars: vec![],
            iteration_ops: vec![],
            triangular: None,
        };
        assert!(spec.ok);
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["vars"][0]["name"], "n");
        assert_eq!(json["ops"][1]["tag"], "loop_begin");
    }

    fn make_spec(ok: bool) -> InputSpec {
        InputSpec {
            raw: "".to_string(),
            ok,
            vars: vec![],
            ops: vec![],
            query_types: vec![],
            query_body: vec![],
            testcase_body: vec![],
            iteration_vars: vec![],
            iteration_ops: vec![],
            triangular: None,
        }
    }

    fn make_loop_op() -> InputOp {
        InputOp {
            tag: OpTag::LoopBegin,
            depth: 0,
            vars: vec![],
            loop_var: Some("i".to_string()),
            begin: Some("0".to_string()),
            end: Some("t".to_string()),
            scalars: vec![],
            size_var: None,
            elem_var: None,
        }
    }

    fn make_var() -> VarDecl {
        VarDecl {
            name: "x".to_string(),
            math: "X".to_string(),
            var_type: VarType::Int,
            dim: 0,
            size: vec![],
            is_size: false,
            is_jagged: false,
        }
    }

    // InputFormatKind: ok=false → Fail
    #[test]
    fn kind_fail_when_ok_false() {
        assert_eq!(make_spec(false).kind(), InputFormatKind::Fail);
    }

    // InputFormatKind: query_types non-empty → QueryTypes(n)
    #[test]
    fn kind_query_types() {
        let mut spec = make_spec(true);
        spec.query_types = vec![
            QueryTypeDecl {
                type_id: "1".to_string(),
                ok: true,
                vars: vec![],
            },
            QueryTypeDecl {
                type_id: "2".to_string(),
                ok: true,
                vars: vec![],
            },
        ];
        assert_eq!(spec.kind(), InputFormatKind::QueryTypes(2));
    }

    // InputFormatKind: query_body non-empty → Query
    #[test]
    fn kind_query() {
        let mut spec = make_spec(true);
        spec.query_body = vec![make_var()];
        assert_eq!(spec.kind(), InputFormatKind::Query);
    }

    // InputFormatKind: testcase_body non-empty → Testcase
    #[test]
    fn kind_testcase() {
        let mut spec = make_spec(true);
        spec.testcase_body = vec![make_var()];
        assert_eq!(spec.kind(), InputFormatKind::Testcase);
    }

    // InputFormatKind: iteration_ops non-empty → Iter
    #[test]
    fn kind_iter() {
        let mut spec = make_spec(true);
        spec.iteration_ops = vec![InputOp {
            tag: OpTag::ReadLine,
            depth: 0,
            vars: vec![],
            loop_var: None,
            begin: None,
            end: None,
            scalars: vec![],
            size_var: None,
            elem_var: None,
        }];
        assert_eq!(spec.kind(), InputFormatKind::Iter);
    }

    // InputFormatKind: ops contains LoopBegin → Loop
    #[test]
    fn kind_loop() {
        let mut spec = make_spec(true);
        spec.ops = vec![make_loop_op()];
        assert_eq!(spec.kind(), InputFormatKind::Loop);
    }

    // InputFormatKind: ok=true, no loop/query/iter → Plain
    #[test]
    fn kind_plain() {
        let mut spec = make_spec(true);
        spec.ops = vec![InputOp {
            tag: OpTag::ReadLine,
            depth: 0,
            vars: vec![],
            loop_var: None,
            begin: None,
            end: None,
            scalars: vec![],
            size_var: None,
            elem_var: None,
        }];
        assert_eq!(spec.kind(), InputFormatKind::Plain);
    }

    // Display: each variant
    #[test]
    fn kind_display() {
        assert_eq!(InputFormatKind::Plain.to_string(), "plain");
        assert_eq!(InputFormatKind::Loop.to_string(), "loop");
        assert_eq!(InputFormatKind::Iter.to_string(), "iter");
        assert_eq!(InputFormatKind::Testcase.to_string(), "testcase");
        assert_eq!(InputFormatKind::Query.to_string(), "query");
        assert_eq!(InputFormatKind::QueryTypes(3).to_string(), "query(3)");
        assert_eq!(InputFormatKind::Fail.to_string(), "FAIL");
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

    fn make_triangular() -> TriangularSpec {
        TriangularSpec {
            name: "a".to_string(),
            math: "A".to_string(),
            var_type: VarType::Int,
            bound: "n".to_string(),
        }
    }

    // TriangularSpec serializes to JSON with expected fields
    #[test]
    fn triangular_spec_serializes() {
        let ts = make_triangular();
        let json = serde_json::to_value(&ts).unwrap();
        assert_eq!(json["name"], "a");
        assert_eq!(json["math"], "A");
        assert_eq!(json["var_type"], "int");
        assert_eq!(json["bound"], "n");
    }

    // InputFormatKind::Triangle when triangular is Some
    #[test]
    fn kind_triangle() {
        let mut spec = make_spec(true);
        spec.triangular = Some(make_triangular());
        assert_eq!(spec.kind(), InputFormatKind::Triangle);
    }

    // Display for Triangle variant
    #[test]
    fn kind_triangle_display() {
        assert_eq!(InputFormatKind::Triangle.to_string(), "triangle");
    }

    // Triangle takes priority over Iter when both are set
    #[test]
    fn kind_triangle_priority_over_iter() {
        let mut spec = make_spec(true);
        spec.triangular = Some(make_triangular());
        spec.iteration_ops = vec![InputOp {
            tag: OpTag::ReadLine,
            depth: 0,
            vars: vec![],
            loop_var: None,
            begin: None,
            end: None,
            scalars: vec![],
            size_var: None,
            elem_var: None,
        }];
        assert_eq!(spec.kind(), InputFormatKind::Triangle);
    }

    // Testcase takes priority over Triangle when both are set
    #[test]
    fn kind_testcase_priority_over_triangle() {
        let mut spec = make_spec(true);
        spec.testcase_body = vec![make_var()];
        spec.triangular = Some(make_triangular());
        assert_eq!(spec.kind(), InputFormatKind::Testcase);
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

#[cfg(test)]
mod detect_tests {
    use super::*;

    #[test]
    fn detects_atcoder_from_contest_url() {
        assert_eq!(
            OJKind::detect("https://atcoder.jp/contests/abc334"),
            Some((OJKind::AtCoder, "abc334".to_string()))
        );
    }

    #[test]
    fn atcoder_url_with_trailing_slash_keeps_first_segment() {
        assert_eq!(
            OJKind::detect("https://atcoder.jp/contests/abc334/"),
            Some((OJKind::AtCoder, "abc334".to_string()))
        );
    }

    #[test]
    fn atcoder_url_with_extra_path_segments_keeps_first_segment() {
        assert_eq!(
            OJKind::detect("https://atcoder.jp/contests/abc334/tasks/abc334_a"),
            Some((OJKind::AtCoder, "abc334".to_string()))
        );
    }

    #[test]
    fn atcoder_url_with_uppercase_id_is_lowercased() {
        assert_eq!(
            OJKind::detect("https://atcoder.jp/contests/ABC334"),
            Some((OJKind::AtCoder, "abc334".to_string()))
        );
    }

    #[test]
    fn detects_atcoder_from_abc_prefix_id() {
        assert_eq!(
            OJKind::detect("abc334"),
            Some((OJKind::AtCoder, "abc334".to_string()))
        );
    }

    #[test]
    fn detects_atcoder_from_agc_prefix_id() {
        assert_eq!(
            OJKind::detect("agc001"),
            Some((OJKind::AtCoder, "agc001".to_string()))
        );
    }

    #[test]
    fn uppercase_prefix_id_is_lowercased() {
        assert_eq!(
            OJKind::detect("ARC100"),
            Some((OJKind::AtCoder, "arc100".to_string()))
        );
    }

    #[test]
    fn unknown_contest_id_returns_none() {
        assert_eq!(OJKind::detect("xyz123"), None);
    }

    #[test]
    fn unknown_url_host_returns_none() {
        assert_eq!(OJKind::detect("https://example.com/contests/abc334"), None);
    }

    #[test]
    fn atcoder_url_with_empty_id_returns_none() {
        assert_eq!(OJKind::detect("https://atcoder.jp/contests/"), None);
    }

    #[test]
    fn detects_librarychecker_from_problem_url() {
        // The id is namespaced with the "librarychecker-" prefix.
        assert_eq!(
            OJKind::detect("https://judge.yosupo.jp/problem/aplusb"),
            Some((OJKind::LibraryChecker, "librarychecker-aplusb".to_string()))
        );
    }

    #[test]
    fn librarychecker_url_with_trailing_slash_keeps_first_segment() {
        assert_eq!(
            OJKind::detect("https://judge.yosupo.jp/problem/aplusb/"),
            Some((OJKind::LibraryChecker, "librarychecker-aplusb".to_string()))
        );
    }

    #[test]
    fn librarychecker_url_with_extra_path_segments_keeps_first_segment() {
        assert_eq!(
            OJKind::detect("https://judge.yosupo.jp/problem/aplusb/submissions"),
            Some((OJKind::LibraryChecker, "librarychecker-aplusb".to_string()))
        );
    }

    #[test]
    fn librarychecker_url_with_empty_id_returns_none() {
        assert_eq!(OJKind::detect("https://judge.yosupo.jp/problem/"), None);
    }

    #[test]
    fn librarychecker_detects_namespaced_id_directly() {
        // A namespaced id (e.g. re-running `ce init librarychecker-aplusb`) is detected
        // via the "librarychecker-" prefix and returned verbatim.
        assert_eq!(
            OJKind::detect("librarychecker-aplusb"),
            Some((OJKind::LibraryChecker, "librarychecker-aplusb".to_string()))
        );
        // A bare problem name has no naming convention and is not detected.
        assert_eq!(OJKind::detect("aplusb"), None);
    }
}

#[cfg(test)]
mod ojkind_str_tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn atcoder_as_str_roundtrips() {
        assert_eq!(OJKind::AtCoder.as_str(), "atcoder");
        assert_eq!(OJKind::from_str("atcoder"), Ok(OJKind::AtCoder));
    }

    #[test]
    fn librarychecker_as_str_roundtrips() {
        assert_eq!(OJKind::LibraryChecker.as_str(), "librarychecker");
        assert_eq!(
            OJKind::from_str("librarychecker"),
            Ok(OJKind::LibraryChecker)
        );
    }

    #[test]
    fn from_str_unknown_errors() {
        assert!(OJKind::from_str("codeforces").is_err());
    }
}
