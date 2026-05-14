use domain::entity::{
    InputOp, InputSpec, OpTag, QueryTypeDecl, TriangularSpec, VarDecl, VarRef, VarType,
};

// ── Lexer ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    Num(String),
    Subscript,
    LBrace,
    RBrace,
    Comma,
    Plus,
    Minus,
    Star,
    Cdots,
    Vdots,
    Space,
}

fn tokenize_line(line: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();

    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' => {
                chars.next();
                // Collapse multiple spaces into one Space token
                while chars.peek() == Some(&' ') || chars.peek() == Some(&'\t') {
                    chars.next();
                }
                tokens.push(Token::Space);
            }
            '_' => {
                chars.next();
                tokens.push(Token::Subscript);
            }
            '{' => {
                chars.next();
                tokens.push(Token::LBrace);
            }
            '}' => {
                chars.next();
                tokens.push(Token::RBrace);
            }
            ',' => {
                chars.next();
                tokens.push(Token::Comma);
            }
            '+' => {
                chars.next();
                tokens.push(Token::Plus);
            }
            '-' => {
                chars.next();
                tokens.push(Token::Minus);
            }
            '*' => {
                chars.next();
                tokens.push(Token::Star);
            }
            '.' => {
                // Count dots
                let mut dot_count = 0;
                while chars.peek() == Some(&'.') {
                    chars.next();
                    dot_count += 1;
                }
                if dot_count >= 3 {
                    tokens.push(Token::Cdots);
                }
                // ignore 1-2 dots
            }
            '\\' => {
                chars.next();
                // Read command name
                let mut cmd = String::new();
                while let Some(&c2) = chars.peek() {
                    if c2.is_ascii_alphabetic() {
                        cmd.push(c2);
                        chars.next();
                    } else {
                        break;
                    }
                }
                match cmd.as_str() {
                    "ldots" | "dots" | "cdots" => tokens.push(Token::Cdots),
                    "vdots" => tokens.push(Token::Vdots),
                    "hspace" => {
                        // consume {arg}
                        if chars.peek() == Some(&'{') {
                            chars.next();
                            while let Some(&c2) = chars.peek() {
                                chars.next();
                                if c2 == '}' {
                                    break;
                                }
                            }
                        }
                        // skip (whitespace command)
                    }
                    "text" | "mathrm" => {
                        // produce a special ident to signal phase2
                        // consume {arg} and emit as ident prefixed with \
                        let mut inner = String::new();
                        if chars.peek() == Some(&'{') {
                            chars.next();
                            while let Some(&c2) = chars.peek() {
                                chars.next();
                                if c2 == '}' {
                                    break;
                                }
                                inner.push(c2);
                            }
                        }
                        tokens.push(Token::Ident(format!("\\{cmd}{{{inner}}}")));
                    }
                    _ => {
                        // ignore unknown commands
                    }
                }
            }
            c if c.is_ascii_alphabetic() => {
                let mut ident = String::new();
                while let Some(&c2) = chars.peek() {
                    if c2.is_ascii_alphanumeric() {
                        ident.push(c2);
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Ident(ident));
            }
            c if c.is_ascii_digit() => {
                let mut num = String::new();
                while let Some(&c2) = chars.peek() {
                    if c2.is_ascii_digit() {
                        num.push(c2);
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Num(num));
            }
            // Unicode vdots character ⋮
            '⋮' => {
                chars.next();
                tokens.push(Token::Vdots);
            }
            ':' => {
                chars.next();
                // ':' can represent vdots in some AtCoder formats
                tokens.push(Token::Vdots);
            }
            _ => {
                chars.next();
                // skip unknown characters
            }
        }
    }

    tokens
}

// ── Parser ─────────────────────────────────────────────────────────────────────

/// A parsed variable reference with raw (possibly uppercase) name
#[derive(Debug, Clone)]
struct RawVar {
    /// Original math name (e.g. "A", "t")
    math: String,
    /// Subscript if any (numeric string or alphabetic)
    subscript: Option<String>,
}

#[derive(Debug, Clone)]
enum RawLine {
    /// Plain scalar list: A B C
    Scalars(Vec<RawVar>),
    /// 1D array horizontal: A_1 A_2 ... A_N  → name="A", size="N"
    Array1D { name: String, size: String },
    /// Vdots line
    Vdots,
    /// Loop row: one line inside a vdots loop with multiple vars (e.g. t_1 k_1)
    LoopRow(Vec<RawVar>),
    /// Grid row: S_{11}...S_{1W} — one row of a character grid, read as one String
    GridRow(RawVar),
    /// A query placeholder line: \text{something}_Q or \mathrm{something}_Q.
    /// Signals a Q-iteration loop of queries; `loop_bound` is the subscript variable
    /// (e.g. "Q" from `\text{query}_Q`).
    QueryLine { loop_bound: String },
    /// One row of a fixed 2D grid: A_{row,1} A_{row,2} ... A_{row,cols}
    /// All elements share the same base name and fixed row index; col subscripts are 1-based sequential.
    Array2DRow {
        name: String,
        row_idx: String,
        col_count: usize,
    },
    /// A jagged-array row: optional scalar vars followed by an array whose last element has
    /// subscript `{row_idx, SIZE_VAR_{row_idx}}`.
    /// Example: `L_N A_{N,1} ... A_{N,L_N}` → row_idx="N", size_var_math="L", elem_var_math="A",
    ///          scalars_before=[RawVar { math:"L", subscript:Some("N") }]
    JaggedRow {
        row_idx: String,
        size_var_math: String,
        elem_var_math: String,
        scalars_before: Vec<RawVar>,
    },
}

/// Parse errors cause ok=false
#[derive(Debug)]
enum ParseError {
    NonNumericSubscript,
    Unknown,
}

/// Returns true when `s` is the literal word "query" (case-insensitive).
/// Used by `parse_line` for QueryLine detection; `has_query_marker` is consistent
/// by delegating to `parse_line` rather than calling this directly.
fn is_query_ident(s: &str) -> bool {
    s.eq_ignore_ascii_case("query")
}

fn parse_line(tokens: &[Token]) -> Result<RawLine, ParseError> {
    // Strip leading/trailing Space tokens
    let tokens = strip_spaces(tokens);

    // Check if this is a pure Vdots line (\vdots or \dots/\cdots used alone as vertical separator)
    if tokens.len() == 1 && matches!(tokens[0], Token::Vdots | Token::Cdots) {
        return Ok(RawLine::Vdots);
    }
    if tokens.is_empty() {
        return Ok(RawLine::Scalars(vec![]));
    }

    // Strip leading `{Ident}` grouping (e.g. `{\rm Query}_Q` where \rm is ignored by the
    // tokenizer, leaving `[LBrace, Space, Ident("Query"), RBrace, ...]`).
    // If tokens start with `{` and the content up to the first `}` contains exactly one
    // Ident (and nothing else besides spaces), unwrap the braces so downstream detection
    // (plain_query_pos) sees `[Ident(...), ...]` directly.
    let tokens = if tokens.first() == Some(&Token::LBrace) {
        if let Some(rbrace_rel) = tokens[1..].iter().position(|t| t == &Token::RBrace) {
            let inner = &tokens[1..1 + rbrace_rel]; // tokens between { and }
            let mut non_space = inner.iter().filter(|t| **t != Token::Space);
            let sole = non_space.next().cloned(); // first non-space token (cloned eagerly)
            let has_more = non_space.next().is_some(); // ensure it's the only one
            if !has_more {
                if let Some(Token::Ident(_)) = &sole {
                    let mut unwrapped = vec![sole.unwrap()];
                    unwrapped.extend_from_slice(&tokens[1 + rbrace_rel + 1..]);
                    unwrapped
                } else {
                    tokens
                }
            } else {
                tokens
            }
        } else {
            tokens
        }
    } else {
        tokens
    };

    // Check for LaTeX query placeholder tokens → QueryLine.
    // \text{...} / \mathrm{...} must be followed by _<subscript>; without one → malformed error.
    let latex_query_pos = tokens.iter().position(
        |t| matches!(t, Token::Ident(s) if s.starts_with("\\text{") || s.starts_with("\\mathrm{")),
    );
    if let Some(pos) = latex_query_pos {
        if pos != 0 {
            return Err(ParseError::Unknown);
        }
        if tokens.get(pos + 1) == Some(&Token::Subscript) {
            let (loop_bound, advance) =
                read_subscript_value(&tokens[pos + 2..]).ok_or(ParseError::Unknown)?;
            if pos + 2 + advance != tokens.len() {
                return Err(ParseError::Unknown);
            }
            return Ok(RawLine::QueryLine { loop_bound });
        }
        return Err(ParseError::Unknown); // LaTeX query form without subscript is malformed
    }

    // Check for plain-text "query" as a query marker, but only when followed by _<subscript>.
    // Without a subscript, fall through to normal scalar parsing so that a variable literally
    // named `query` is not misidentified as a loop marker.
    let plain_query_pos = tokens.iter().enumerate().find_map(|(i, t)| {
        if let Token::Ident(s) = t {
            if is_query_ident(s) && tokens.get(i + 1) == Some(&Token::Subscript) {
                Some(i)
            } else {
                None
            }
        } else {
            None
        }
    });
    if let Some(pos) = plain_query_pos {
        if pos != 0 {
            return Err(ParseError::Unknown);
        }
        // tokens[pos+1] is Token::Subscript (guaranteed by find_map above)
        let (loop_bound, advance) =
            read_subscript_value(&tokens[pos + 2..]).ok_or(ParseError::Unknown)?;
        if pos + 2 + advance != tokens.len() {
            return Err(ParseError::Unknown);
        }
        return Ok(RawLine::QueryLine { loop_bound });
    }

    // Try to detect a jagged-array row: optional scalars + elem_{row_idx,1} ... elem_{row_idx,SIZE_{row_idx}}
    // Must be checked BEFORE try_parse_grid_row because the grid detector can match some jagged
    // patterns (e.g. X_{1,1} ... X_{1,L_1} looks like a GridRow with subscript "1").
    if let Some(result) = try_parse_jagged_row(&tokens) {
        return result;
    }

    // Try to detect a character grid row: X_{row,col_start}...X_{row,col_end}
    // (before array1d, because some grid patterns look like array1d with alphabetic subscripts)
    if let Some(result) = try_parse_grid_row(&tokens) {
        return result;
    }

    // Try to detect a fixed 2D grid row: A_{row,1} A_{row,2} ... A_{row,cols}
    // (before array1d and array1d_no_cdots, since comma subscripts look like multi-subscript vars)
    if let Some(result) = try_parse_array2d_row(&tokens) {
        return result;
    }

    // Try to detect 1D horizontal array: pattern like Ident_Num ... Ident_Num [Cdots] Ident_Num
    // or Ident_Num Space Ident_Num Space Cdots
    if let Some(result) = try_parse_array1d(&tokens) {
        return result;
    }

    // Try to detect 1D fixed array without cdots: A_1 A_2 A_3
    if let Some(result) = try_parse_array1d_no_cdots(&tokens) {
        return result;
    }

    // Parse as scalar/subscripted vars separated by spaces
    parse_var_list(&tokens)
}

fn strip_spaces(tokens: &[Token]) -> Vec<Token> {
    let start = tokens
        .iter()
        .position(|t| t != &Token::Space)
        .unwrap_or(tokens.len());
    let end = tokens
        .iter()
        .rposition(|t| t != &Token::Space)
        .map(|i| i + 1)
        .unwrap_or(0);
    if start >= end {
        vec![]
    } else {
        tokens[start..end].to_vec()
    }
}

/// Try to detect "A_1 A_2 \ldots A_N" pattern
/// Returns Some(Ok/Err) if detected, None if not applicable
fn try_parse_array1d(tokens: &[Token]) -> Option<Result<RawLine, ParseError>> {
    // Must contain a Cdots
    if !tokens.contains(&Token::Cdots) {
        return None;
    }

    // All non-cdots, non-space elements should be subscripted vars of the same base name
    // Pattern: Ident Subscript (Num|Ident) [Space Ident Subscript (Num|Ident)]* [Space] Cdots [Space] Ident Subscript (Num|Ident)
    let mut base_name: Option<String> = None;
    let mut last_subscript: Option<String> = None;
    let mut has_alpha_subscript = false;
    let mut has_numeric_subscript = false;
    // Track whether the previous element was a subscripted var (requires separator before next var).
    let mut need_separator = false;

    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Space | Token::Cdots => {
                need_separator = false;
                i += 1;
            }
            Token::Ident(name) => {
                if need_separator {
                    // Adjacent subscripted vars with no Space/Cdots between them — unsupported.
                    return Some(Err(ParseError::Unknown));
                }
                // expect Subscript next
                if i + 1 < tokens.len() && tokens[i + 1] == Token::Subscript {
                    // subscript: could be Num, Ident, or LBrace...RBrace
                    let opt = read_subscript_value(&tokens[i + 2..]);
                    let (sub, advance) = opt?;
                    i += 2 + advance;

                    // Check base name consistency
                    match &base_name {
                        None => base_name = Some(name.clone()),
                        Some(bn) if bn != name => return None,
                        _ => {}
                    }

                    // Check subscript type
                    if sub.chars().all(|c: char| c.is_ascii_alphabetic()) {
                        has_alpha_subscript = true;
                    } else {
                        has_numeric_subscript = true;
                    }
                    last_subscript = Some(sub);
                    need_separator = true;
                } else {
                    // Ident without subscript in an array context — not a 1D array
                    return None;
                }
            }
            _ => return None,
        }
    }

    let base = base_name?;
    let size = last_subscript?;

    if has_alpha_subscript && !has_numeric_subscript {
        // All subscripts are alphabetic (e.g. A_x A_y ... A_z) — non-numeric subscript
        // pattern, not a supported 1D array
        Some(Err(ParseError::NonNumericSubscript))
    } else {
        Some(Ok(RawLine::Array1D { name: base, size }))
    }
}

/// Try to detect a fixed-size 1D array without cdots: `A_1 A_2 A_3`
/// Requires ≥ 2 elements, same base name, numeric subscripts sequential from 1.
/// Returns Some(Ok(Array1D { size: last_idx })) when matched, None otherwise.
fn try_parse_array1d_no_cdots(tokens: &[Token]) -> Option<Result<RawLine, ParseError>> {
    // Must not contain Cdots (those are handled by try_parse_array1d)
    if tokens.contains(&Token::Cdots) {
        return None;
    }

    let mut base_name: Option<String> = None;
    let mut subscripts: Vec<u64> = Vec::new();
    let mut need_separator = false;
    let mut i = 0;

    while i < tokens.len() {
        match &tokens[i] {
            Token::Space => {
                need_separator = false;
                i += 1;
            }
            Token::Ident(name) => {
                // Adjacent elements without a Space between them → unsupported
                // (consistent with "空白なし隣接要素 → ok:false" rule).
                if need_separator {
                    return Some(Err(ParseError::Unknown));
                }
                // Must have a subscript next
                if i + 1 >= tokens.len() || tokens[i + 1] != Token::Subscript {
                    return None;
                }
                let (sub, advance) = read_subscript_value(&tokens[i + 2..])?;
                i += 2 + advance;

                // Subscript must be a pure numeric literal (no commas)
                let n: u64 = sub.parse().ok()?;

                // Base name must be consistent
                match &base_name {
                    None => base_name = Some(name.clone()),
                    Some(bn) if bn != name => return None,
                    _ => {}
                }
                subscripts.push(n);
                need_separator = true;
            }
            _ => return None,
        }
    }

    let base = base_name?;
    if subscripts.len() < 2 {
        return None;
    }

    // Subscripts must be sequential starting from 1
    let expected: Vec<u64> = (1..=subscripts.len() as u64).collect();
    if subscripts != expected {
        return None;
    }

    let size = subscripts.last()?.to_string();
    Some(Ok(RawLine::Array1D { name: base, size }))
}

/// Try to detect one row of a fixed 2D grid: `A_{row,1} A_{row,2} ... A_{row,cols}`
/// Requires ≥ 2 elements, same base name, fixed row subscript (numeric), col subscripts sequential from 1.
/// Returns Some(Ok(Array2DRow { ... })) when matched, None otherwise.
fn try_parse_array2d_row(tokens: &[Token]) -> Option<Result<RawLine, ParseError>> {
    // Must not contain Cdots
    if tokens.contains(&Token::Cdots) {
        return None;
    }

    let mut base_name: Option<String> = None;
    let mut row_idx: Option<String> = None;
    let mut col_count: usize = 0;
    let mut need_separator = false;
    let mut i = 0;

    while i < tokens.len() {
        match &tokens[i] {
            Token::Space => {
                need_separator = false;
                i += 1;
            }
            Token::Ident(name) => {
                // Adjacent elements without a Space token between them → unsupported
                // (consistent with "空白なし隣接要素 → ok:false" rule).
                if need_separator {
                    return Some(Err(ParseError::Unknown));
                }
                // Must have a subscript next
                if i + 1 >= tokens.len() || tokens[i + 1] != Token::Subscript {
                    return None;
                }
                let (sub, advance) = read_subscript_value(&tokens[i + 2..])?;
                i += 2 + advance;

                // Subscript must be "row,col" form (both numeric)
                let (row_s, col_s) = sub.split_once(',')?;
                let _row_n: u64 = row_s.parse().ok()?;
                let col_n: u64 = col_s.parse().ok()?;

                // Base name consistent
                match &base_name {
                    None => base_name = Some(name.clone()),
                    Some(bn) if bn != name => return None,
                    _ => {}
                }

                // Row idx consistent
                match &row_idx {
                    None => row_idx = Some(row_s.to_string()),
                    Some(r) if r != row_s => return None,
                    _ => {}
                }

                // Col must be sequential from 1
                col_count += 1;
                if col_n != col_count as u64 {
                    return None;
                }
                need_separator = true;
            }
            _ => return None,
        }
    }

    let name = base_name?;
    let row = row_idx?;
    if col_count < 2 {
        return None;
    }

    Some(Ok(RawLine::Array2DRow {
        name,
        row_idx: row,
        col_count,
    }))
}

/// Try to detect a character grid row: `X_{row col_start} ... X_{row col_end}`
/// Returns Some(Ok(GridRow)) when the whole line is exactly this pattern.
/// Returns None when not applicable (falls through to parse_var_list).
fn try_parse_grid_row(tokens: &[Token]) -> Option<Result<RawLine, ParseError>> {
    // Must contain Cdots to be a range expression
    if !tokens.contains(&Token::Cdots) {
        return None;
    }

    let mut i = 0;

    // First: Ident(name)
    let first_name = match tokens.get(i) {
        Some(Token::Ident(n)) => {
            i += 1;
            n.clone()
        }
        _ => return None,
    };

    // Subscript _
    if tokens.get(i) != Some(&Token::Subscript) {
        return None;
    }
    i += 1;

    // 2D subscript: LBrace with multiple content parts (or comma)
    let (row_start, advance1, is_2d_1) = read_2d_subscript_row_part(&tokens[i..])?;
    i += advance1;

    // Consume any additional prefix elements before Cdots.
    // e.g. S_{1,1} S_{1,2} \ldots S_{1,W} — the S_{1,2} must be consumed before Cdots.
    // Both space-separated and no-space adjacent prefix elements are recognized.
    loop {
        while tokens.get(i) == Some(&Token::Space) {
            i += 1;
        }
        // If next is Cdots, break out to existing logic (no space required before Cdots).
        if tokens.get(i) == Some(&Token::Cdots) {
            break;
        }
        // If next is Ident(same name), try to consume as an extra prefix element.
        if let Some(Token::Ident(n)) = tokens.get(i)
            && n == &first_name
        {
            let j = i + 1;
            if tokens.get(j) == Some(&Token::Subscript)
                && let Some((extra_row, adv, extra_is_2d)) =
                    read_2d_subscript_row_part(&tokens[j + 1..])
                && extra_is_2d
                && extra_row == row_start
            {
                i = j + 1 + adv;
                continue;
            }
        }
        // Not Cdots and not a matching extra prefix element — not a GridRow.
        return None;
    }
    i += 1; // consume Cdots
    while tokens.get(i) == Some(&Token::Space) {
        i += 1;
    }

    // Second: Ident(same name)
    let second_name = match tokens.get(i) {
        Some(Token::Ident(n)) => {
            i += 1;
            n.clone()
        }
        _ => return None,
    };
    if second_name != first_name {
        return None;
    }

    // Subscript _
    if tokens.get(i) != Some(&Token::Subscript) {
        return None;
    }
    i += 1;

    // 2D subscript for the end
    let (row_end, advance2, is_2d_2) = read_2d_subscript_row_part(&tokens[i..])?;
    i += advance2;

    // Must be at end (ignoring trailing spaces)
    while tokens.get(i) == Some(&Token::Space) {
        i += 1;
    }
    if i != tokens.len() {
        return None;
    }

    // At least one side must be a 2D subscript
    if !is_2d_1 && !is_2d_2 {
        return None;
    }

    // Use the row part from the end (right-hand) subscript if 2D, else from start.
    // This is the value that becomes the loop bound (e.g. "H" from S_{H1}...S_{HW}).
    let row_part = if is_2d_2 { row_end } else { row_start };

    Some(Ok(RawLine::GridRow(RawVar {
        math: first_name,
        subscript: Some(row_part),
    })))
}

/// Detect a jagged-array line: optional loop-subscripted scalars followed by a 1D array whose
/// last element's subscript has the form `{row_idx, SIZE_VAR_{row_idx}}`.
///
/// Pattern for the jagged part (tokens at position `p`):
///   `Ident(elem) Subscript LBrace Ident(row_idx) Comma Ident(size_var) Subscript Ident(row_idx) RBrace`
/// preceded by `Cdots` and optionally a space.
///
/// Scalars before the cdots segment must be loop-subscripted vars with the same `row_idx`.
fn try_parse_jagged_row(tokens: &[Token]) -> Option<Result<RawLine, ParseError>> {
    // Must contain Cdots
    if !tokens.contains(&Token::Cdots) {
        return None;
    }

    // Find the suffix pattern: Cdots [Space] Ident(elem) _ { row_idx , size_var _ row_idx }
    // We scan from the end backwards to find the closing RBrace of {row_idx, size_var_row_idx}.
    // The structure at the end must be:
    //   ... Cdots [Space] Ident Subscript LBrace Ident Comma Ident Subscript Ident RBrace
    //                                             row_idx      size_var      row_idx
    let n = tokens.len();
    // Minimum suffix length (no space): Cdots(1) Ident(1) _(1) {(1) Ident(1) ,(1) Ident(1) _(1) Ident(1) }(1) = 10
    if n < 10 {
        return None;
    }

    // Work backwards: find the last RBrace and check the suffix
    // Tokens at positions n-10..n (no-space variant) or n-11..n (with space after Cdots)
    // Let's just scan for the required suffix pattern programmatically.

    // Scan from the right: we expect the last 9 tokens (excluding possible Space) to be:
    //   Ident(elem) Subscript LBrace Ident(row) Comma Ident(size) Subscript Ident(row) RBrace
    let suffix_start = {
        let mut found = None;
        // Try without space: tokens[p..p+9] is the suffix, preceded by Cdots at tokens[p-1]
        // Try with space: tokens[p..p+9] is the suffix, preceded by Space at tokens[p-1] and Cdots at tokens[p-2]
        for p in 0..n {
            if n - p < 9 {
                break;
            }
            // Check if tokens[p..p+9] matches:
            //   Ident _ { (Ident|Num) , Ident _ (Ident|Num) }
            // The row_idx positions (3 and 7) can be either Ident or Num (e.g. "1" or "N").
            let slice = &tokens[p..p + 9];
            let is_ident_or_num = |t: &Token| matches!(t, Token::Ident(_) | Token::Num(_));
            let matches_suffix = matches!(&slice[0], Token::Ident(_))
                && slice[1] == Token::Subscript
                && slice[2] == Token::LBrace
                && is_ident_or_num(&slice[3])
                && slice[4] == Token::Comma
                && matches!(&slice[5], Token::Ident(_))
                && slice[6] == Token::Subscript
                && is_ident_or_num(&slice[7])
                && p + 9 == n; // must be at the end
            if !matches_suffix {
                continue;
            }
            // Extract names
            let elem_name = match &slice[0] {
                Token::Ident(s) => s.clone(),
                _ => unreachable!(),
            };
            let row_idx_1 = match &slice[3] {
                Token::Ident(s) | Token::Num(s) => s.clone(),
                _ => unreachable!(),
            };
            let size_var = match &slice[5] {
                Token::Ident(s) => s.clone(),
                _ => unreachable!(),
            };
            let row_idx_2 = match &slice[7] {
                Token::Ident(s) | Token::Num(s) => s.clone(),
                _ => unreachable!(),
            };
            // row_idx must match on both sides
            if row_idx_1 != row_idx_2 {
                continue;
            }
            // The tokens before p must end with: ... Cdots [Space]
            // i.e. tokens[p-1] == Space && tokens[p-2] == Cdots, or tokens[p-1] == Cdots
            let cdots_pos =
                if p >= 2 && tokens[p - 1] == Token::Space && tokens[p - 2] == Token::Cdots {
                    p - 2
                } else if p >= 1 && tokens[p - 1] == Token::Cdots {
                    p - 1
                } else {
                    continue;
                };
            found = Some((p, elem_name, row_idx_1, size_var, cdots_pos));
            break;
        }
        found
    }?;

    let (_suffix_pos, elem_var_math, row_idx, size_var_math, cdots_pos) = suffix_start;

    // Everything before cdots_pos: should be optional prefix of loop-subscripted scalars
    // plus the start of the jagged array (elem__{row_idx,1} ... style before cdots).
    // We need to parse the prefix tokens[0..cdots_pos] for:
    //   - scalars: zero or more `Ident _ Ident(row_idx)` separated by spaces
    //   - the first element of the jagged array: `Ident(elem) _ { row_idx , Num }` or similar
    //     (we don't strictly require it — presence of cdots + suffix is enough)
    // For simplicity: parse prefix as space-separated subscripted scalars where all subscripts == row_idx.
    // Allow the elem var to appear as first element too (it won't be added to scalars_before).

    let prefix = &tokens[..cdots_pos];
    let prefix = if prefix.last() == Some(&Token::Space) {
        &prefix[..prefix.len() - 1]
    } else {
        prefix
    };

    // Parse prefix: space-separated vars of the form `Ident _ Subscript_value`
    // The subscript_value may be:
    //   - Simple: Ident(row_idx) or Num(row_idx) — a scalar with that row index
    //   - 2D start: `{ (Ident|Num) , (Num|Ident) }` — the first element of the jagged array
    //     (e.g. A_{N,1} or A_{1,1}). We identify this as the elem_var's first occurrence.
    // We extract only the scalars_before (non-elem vars with subscript == row_idx).
    let mut scalars_before: Vec<RawVar> = Vec::new();
    let mut pi = 0;
    let prefix = strip_spaces(prefix);
    let prefix = prefix.as_slice();
    while pi < prefix.len() {
        // Skip spaces
        while pi < prefix.len() && prefix[pi] == Token::Space {
            pi += 1;
        }
        if pi >= prefix.len() {
            break;
        }
        // Expect Ident
        let var_name = match &prefix[pi] {
            Token::Ident(s) => s.clone(),
            _ => return Some(Err(ParseError::Unknown)),
        };
        pi += 1;
        // Expect Subscript
        if pi >= prefix.len() || prefix[pi] != Token::Subscript {
            return Some(Err(ParseError::Unknown));
        }
        pi += 1;

        // Try to read a 2D `{ row_idx , col }` subscript first (for elem_var first occurrence).
        // read_subscript_value returns None for `{Ident,Num}` patterns, so we handle it specially.
        if prefix.get(pi) == Some(&Token::LBrace) {
            // Check pattern: { (Ident|Num) , (Ident|Num) }  (any combination with a comma)
            let brace_match = if let (
                Some(Token::LBrace),
                Some(Token::Ident(_) | Token::Num(_)),
                Some(Token::Comma),
                Some(Token::Ident(_) | Token::Num(_)),
                Some(Token::RBrace),
            ) = (
                prefix.get(pi),
                prefix.get(pi + 1),
                prefix.get(pi + 2),
                prefix.get(pi + 3),
                prefix.get(pi + 4),
            ) {
                let row_part = match &prefix[pi + 1] {
                    Token::Ident(s) | Token::Num(s) => s.clone(),
                    _ => unreachable!(),
                };
                Some((row_part, 5usize))
            } else {
                None
            };

            if let Some((brace_row, advance)) = brace_match {
                pi += advance;
                // If this is the elem_var and the row part matches row_idx, skip it
                if var_name == elem_var_math && brace_row == row_idx {
                    continue;
                }
                // Otherwise treat as unexpected — not a valid prefix var
                return None;
            }
            // Fall through to read_subscript_value for other brace patterns
            let (sub_val, advance) = read_subscript_value(&prefix[pi..])?;
            pi += advance;
            if sub_val == row_idx {
                scalars_before.push(RawVar {
                    math: var_name,
                    subscript: Some(row_idx.clone()),
                });
            } else {
                return None;
            }
        } else {
            // Simple subscript: Ident or Num
            let (sub_val, advance) = match read_subscript_value(&prefix[pi..]) {
                Some(r) => r,
                None => return Some(Err(ParseError::Unknown)),
            };
            pi += advance;
            if sub_val == row_idx {
                scalars_before.push(RawVar {
                    math: var_name,
                    subscript: Some(row_idx.clone()),
                });
            } else {
                // Unexpected subscript — not a valid jagged row prefix
                return None;
            }
        }
    }

    Some(Ok(RawLine::JaggedRow {
        row_idx,
        size_var_math,
        elem_var_math,
        scalars_before,
    }))
}

/// Read a subscript brace `{content}` and return `(row_part, tokens_consumed, is_2d)`.
///
/// A "2D" subscript encodes both a row and a column, e.g. `{H1}`, `{HW}`, `{1W}`, `{1,1}`.
/// The "row part" is the identifier/number that represents the row (used as loop bound).
///
/// Detection rules:
/// - Multiple separate tokens in braces (`{1 W}` → parts=["1","W"]): 2D, row = first part.
/// - Comma-separated (`{H,W}`): 2D, row = first part.
/// - Single token that contains a letter AND has length ≥ 2 (`{H1}`, `{HW}`, `{iW}`): 2D, row = first char.
/// - Single purely-numeric token (`{10}`, `{12}`): 1D subscript (multi-digit index), `is_2d = false`.
/// - Single token with length 1 (`{N}`, `{i}`): 1D subscript, `is_2d = false`.
fn read_2d_subscript_row_part(tokens: &[Token]) -> Option<(String, usize, bool)> {
    if tokens.first() != Some(&Token::LBrace) {
        return None;
    }
    let mut i = 1;
    let mut parts: Vec<String> = Vec::new();
    let mut has_comma = false;
    let mut found_rbrace = false;
    while i < tokens.len() {
        match &tokens[i] {
            Token::RBrace => {
                i += 1;
                found_rbrace = true;
                break;
            }
            Token::Num(n) => {
                parts.push(n.clone());
                i += 1;
            }
            Token::Ident(s) => {
                parts.push(s.clone());
                i += 1;
            }
            Token::Comma => {
                has_comma = true;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    if !found_rbrace {
        return None;
    }

    let (row_part, is_2d) = match parts.as_slice() {
        // Multiple separate parts: {1 W}, first part is row
        [first, _, ..] => (first.clone(), true),
        // Single token
        [single] => {
            // A single multi-char token is "2D" only when it contains at least one letter
            // (e.g. {H1}, {1W}, {HW}, {iW}).  Purely-numeric tokens like {10} or {12}
            // are just multi-digit 1D indices and must NOT be treated as 2D.
            let has_letter = single.chars().any(|c| c.is_ascii_alphabetic());
            if has_comma || (single.len() >= 2 && has_letter) {
                // Comma-separated ({H,W}) or token with a letter ({H1}, {HW}, {iW})
                // Row = first character of the token
                let row = single.chars().next()?.to_string();
                (row, true)
            } else {
                // Single-char subscript {N}/{i} or purely-numeric {10}/{12}: 1D, not a grid row
                (single.clone(), false)
            }
        }
        _ => return None,
    };

    Some((row_part, i, is_2d))
}

/// Read a subscript value after `_` token.
/// Returns (value_string, tokens_consumed).
fn read_subscript_value(tokens: &[Token]) -> Option<(String, usize)> {
    match tokens.first() {
        Some(Token::Num(n)) => Some((n.clone(), 1)),
        Some(Token::Ident(s)) => Some((s.clone(), 1)),
        Some(Token::LBrace) => {
            // Read until matching RBrace.
            //
            // Two modes depending on whether a Comma is found:
            //
            // Comma mode — collect comma-separated parts (for 2D subscripts):
            //   {Num} or {Ident}    → single subscript value
            //   {Num,Num}           → 2D numeric: "row,col" for Array2DRow
            //   {Num,Ident} etc.    → None (unsupported)
            //
            // Arithmetic mode (no Comma) — build an expression string:
            //   Adjacent Num then Ident (no operator between) → insert "*"
            //   Plus/Minus/Star → append the operator character as-is
            //   Ident tokens are kept in their original case; lowercasing is deferred to
            //   normalize_expr() in the resolve phase so that collision-avoidance
            //   (normalize_name keeps uppercase when N and n coexist) is respected.
            //   Examples: {2N} → "2*N", {N-1} → "N-1", {2N-1} → "2*N-1"
            //   Invalid expressions (trailing/leading/consecutive operators) → None
            let mut depth = 1;
            let mut has_comma = false;
            // Set to true when an operator (Plus/Minus/Star) is encountered anywhere inside
            // the braces. Used to reject mixed comma+operator subscripts like {1-1,2}.
            let mut has_operator = false;
            // Arithmetic expression builder
            let mut expr = String::new();
            // Track the last "kind" of token appended to expr:
            //   0 = nothing / last was operator, 1 = Num, 2 = Ident
            // Used to detect implicit multiplication and to validate expression structure
            // (no leading operator, no consecutive operators, no trailing operator).
            let mut last_kind: u8 = 0;
            // Set when an invalid operator position is detected (leading, consecutive, etc.)
            let mut invalid_expr = false;
            // For comma mode
            let mut parts: Vec<String> = Vec::new();
            let mut current: Option<String> = None;
            let mut has_ident = false;
            let mut has_empty_part = false;
            let mut closed = false;
            let mut i = 1;
            while i < tokens.len() && depth > 0 {
                match &tokens[i] {
                    Token::LBrace => {
                        depth += 1;
                        i += 1;
                    }
                    Token::RBrace => {
                        depth -= 1;
                        if depth == 0 {
                            closed = true;
                            // Trailing comma: parts exist but current is empty
                            if has_comma && !parts.is_empty() && current.is_none() {
                                has_empty_part = true;
                            }
                            if let Some(s) = current.take() {
                                parts.push(s);
                            }
                        }
                        i += 1;
                    }
                    Token::Comma => {
                        has_comma = true;
                        // Leading or consecutive comma → empty part before this separator
                        if current.is_none() {
                            has_empty_part = true;
                        }
                        if let Some(s) = current.take() {
                            parts.push(s);
                        }
                        i += 1;
                    }
                    Token::Num(n) => {
                        if !has_comma {
                            // Arithmetic mode: insert "*" when Ident precedes Num
                            // (last_kind == 2). Space tokens inside braces are silently
                            // skipped without resetting last_kind, so "N 2" and "N2"
                            // both trigger this branch.
                            // Note: in practice the lexer merges alphanumeric runs into
                            // a single Ident (e.g. "N2" → Ident("N2")), so the bare
                            // Ident→Num adjacency path primarily fires when whitespace
                            // separates them inside braces; the branch also serves as a
                            // safety net for future lexer changes.
                            if last_kind == 2 {
                                expr.push('*');
                            }
                            expr.push_str(n);
                            last_kind = 1;
                        }
                        current = Some(current.map_or_else(|| n.clone(), |c| c + n));
                        i += 1;
                    }
                    Token::Ident(s) => {
                        has_ident = true;
                        if !has_comma {
                            // Arithmetic mode: insert "*" when Num precedes Ident.
                            // Example: {2N} → Num("2") then Ident("N") → "2*n".
                            if last_kind == 1 {
                                expr.push('*');
                            }
                            expr.push_str(s);
                            last_kind = 2;
                        }
                        current = Some(current.map_or_else(|| s.clone(), |c| c + s));
                        i += 1;
                    }
                    Token::Plus => {
                        has_operator = true;
                        if !has_comma {
                            // Leading or consecutive operator → invalid expression.
                            if last_kind == 0 {
                                invalid_expr = true;
                            }
                            expr.push('+');
                            last_kind = 0;
                        }
                        i += 1;
                    }
                    Token::Minus => {
                        has_operator = true;
                        if !has_comma {
                            if last_kind == 0 {
                                invalid_expr = true;
                            }
                            expr.push('-');
                            last_kind = 0;
                        }
                        i += 1;
                    }
                    Token::Star => {
                        has_operator = true;
                        if !has_comma {
                            if last_kind == 0 {
                                invalid_expr = true;
                            }
                            expr.push('*');
                            last_kind = 0;
                        }
                        i += 1;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            if !closed || has_empty_part {
                return None;
            }
            // If a comma was found, use the original comma-separated logic.
            // Reject mixed comma+operator subscripts (e.g. {1-1,2}) to prevent
            // silent corruption of the subscript value.
            if has_comma {
                if has_operator {
                    return None;
                }
                match parts.len() {
                    1 => Some((parts.remove(0), i)),
                    2 if !has_ident => {
                        // {Num,Num} — 2D numeric subscript, returned as "row,col"
                        Some((format!("{},{}", parts[0], parts[1]), i))
                    }
                    _ => None, // Ident parts or 3+ parts → unsupported
                }
            } else {
                // Arithmetic mode: return the expression string built above.
                // Reject empty, trailing-operator ({N-}), or invalid ({2**N}) expressions.
                if expr.is_empty() || last_kind == 0 || invalid_expr {
                    None
                } else {
                    Some((expr, i))
                }
            }
        }
        _ => None,
    }
}

/// Parse a list of vars (possibly subscripted) separated by spaces
fn parse_var_list(tokens: &[Token]) -> Result<RawLine, ParseError> {
    let mut vars: Vec<RawVar> = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        match &tokens[i] {
            Token::Space => {
                i += 1;
            }
            Token::Ident(name) => {
                let math = name.clone();
                // Check for subscript
                if i + 1 < tokens.len() && tokens[i + 1] == Token::Subscript {
                    let (sub, advance) =
                        read_subscript_value(&tokens[i + 2..]).ok_or(ParseError::Unknown)?;
                    // Comma-containing subscripts (e.g. "1,2" from A_{1,2}) must not reach
                    // name normalization — they would generate invalid Rust identifiers.
                    // Such subscripts are only valid inside Array2DRow; reject them here.
                    if sub.contains(',') {
                        return Err(ParseError::Unknown);
                    }
                    i += 2 + advance;

                    vars.push(RawVar {
                        math,
                        subscript: Some(sub),
                    });
                } else {
                    i += 1;
                    vars.push(RawVar {
                        math,
                        subscript: None,
                    });
                }
            }
            Token::Cdots => {
                // Cdots without array pattern — ignore
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    // Determine if this looks like a loop row (has subscripted vars) or plain scalars
    let has_subscripts = vars.iter().any(|v| v.subscript.is_some());
    if has_subscripts {
        Ok(RawLine::LoopRow(vars))
    } else {
        Ok(RawLine::Scalars(vars))
    }
}

// ── Semantic ───────────────────────────────────────────────────────────────────

/// Intermediate op before name resolution
#[derive(Debug, Clone)]
enum IntermOp {
    ReadScalars(Vec<String>), // math names
    ReadArray1D {
        name: String,
        size: String,
    }, // size is math name of size var
    LoopBegin {
        loop_var: String,
        begin: String,
        end: String,
    },
    LoopEnd,
    ReadLoopRow(Vec<String>), // math names of loop-row vars (dim=1)
    ReadGridRow(String),      // math name of grid-row var (dim=1, var_type=Str)
    /// Subscripted scalars from a standalone LoopRow (e.g. A_x A_y, r_1 c_1).
    /// Each entry is (name_seed, base_math):
    ///   name_seed — used by normalize_name to derive the Rust variable name (e.g. "Ax", "r1")
    ///   base_math — original base name stored in VarDecl.math for constraint inference (e.g. "A", "r")
    ReadSubscriptedScalars(Vec<(String, String)>),
    /// Fixed 2D grid: consecutive Array2DRow lines with same name, sequential row indices 1..rows.
    ReadGrid {
        name: String,
        rows: usize,
        cols: usize,
    },
    /// Jagged-array loop: each row i has `size_var[i]` elements in `elem_var[i]`,
    /// plus zero or more scalar arrays (one element per row) in `scalars_math`.
    LoopJagged {
        end: String,               // loop bound (math form, e.g. "N")
        scalars_math: Vec<String>, // scalar array math names (not including size_var)
        size_var_math: String,     // SIZE_VAR math name
        elem_var_math: String,     // element var math name
    },
}

fn build_intermediate(raw_lines: &[RawLine]) -> Result<Vec<IntermOp>, ParseError> {
    let mut ops: Vec<IntermOp> = Vec::new();
    let loop_vars = ["i", "j", "k2", "l", "m2"];
    let mut loop_var_counter = 0usize;

    // Pre-scan: mark which line indices are inside a vdots loop block.
    // A vdots block is: [LoopRow*|GridRow*] Vdots [LoopRow*|GridRow*] where there's at least
    // one LoopRow/GridRow before. We record ranges as (block_start, vdots_idx, after_end).
    // Lines in a vdots block are consumed at Vdots time and skipped otherwise.
    // A LoopRow is eligible for a vdots block only when all its vars share the same subscript
    // (e.g. "t_1 k_1" all have "1"; "S_N" has "N").  Lines where vars have *different*
    // subscripts (e.g. "A_x A_y") are coordinate-style scalars that should not be pulled
    // into a loop block.
    let is_loop_or_grid = |rl: &RawLine| match rl {
        RawLine::LoopRow(vars) => {
            // All vars must have a subscript, and all subscripts must be equal.
            // This rejects mixed rows like "A_1 B" (None subscript on B) and rows
            // with different subscripts like "A_x A_y", keeping only true loop-body
            // rows like "t_1 k_1" (all subscript "1") or "S_N" (subscript "N").
            let first = match vars.first() {
                Some(v) => v.subscript.as_deref(),
                None => return false,
            };
            first.is_some() && vars.iter().all(|v| v.subscript.as_deref() == first)
        }
        RawLine::GridRow(_) => true,
        // Query placeholder lines (\text{query}_Q etc.) are always eligible for a vdots block.
        RawLine::QueryLine { .. } => true,
        // JaggedRow is always eligible for a vdots block (it IS a loop-body row).
        RawLine::JaggedRow { .. } => true,
        _ => false,
    };

    /// Row kind: distinguishes LoopRow, GridRow, QueryLine, and JaggedRow for block-extension checks.
    #[derive(PartialEq)]
    enum RowKind {
        Loop,
        Grid,
        Query,
        Jagged,
    }
    let row_kind = |rl: &RawLine| match rl {
        RawLine::GridRow(_) => RowKind::Grid,
        RawLine::QueryLine { .. } => RowKind::Query,
        RawLine::JaggedRow { .. } => RowKind::Jagged,
        _ => RowKind::Loop,
    };

    // Returns true when `kind` is compatible with a jagged block (LoopRow or JaggedRow).
    let is_jagged_compatible = |kind: &RowKind| matches!(kind, RowKind::Loop | RowKind::Jagged);

    let mut vdots_blocks: Vec<(usize, usize, usize)> = Vec::new(); // (block_start, vdots_idx, after_end)
    {
        let mut j = 0;
        while j < raw_lines.len() {
            if matches!(raw_lines[j], RawLine::Vdots) {
                // Find consecutive LoopRows/GridRows/QueryLines before this vdots.
                // Stop extending when the kind (LoopRow vs GridRow vs QueryLine) changes.
                // Exception: for jagged blocks, LoopRow and JaggedRow are both allowed.
                let last_kind = if j > 0 {
                    row_kind(&raw_lines[j - 1])
                } else {
                    RowKind::Loop
                };
                // A jagged block is detected when the row immediately before the vdots is a
                // JaggedRow (last_kind == Jagged).  In a jagged block, both LoopRow and
                // JaggedRow are allowed so that multi-row bodies like:
                //   L_1            ← LoopRow
                //   X_{1,1}...X_{1,L_1}  ← JaggedRow
                // are included in the same block.
                let is_jagged_block = last_kind == RowKind::Jagged;
                let mut block_start = j;
                while block_start > 0 {
                    let prev = &raw_lines[block_start - 1];
                    let prev_kind = row_kind(prev);
                    let compatible = if is_jagged_block {
                        is_loop_or_grid(prev) && is_jagged_compatible(&prev_kind)
                    } else {
                        is_loop_or_grid(prev) && prev_kind == last_kind
                    };
                    if compatible {
                        block_start -= 1;
                    } else {
                        break;
                    }
                }
                if block_start == j {
                    // No LoopRows/GridRows/QueryLines before — not a vdots loop, skip
                    j += 1;
                    continue;
                }
                // Find rows after this vdots (same kind constraint)
                let mut after_end = j + 1;
                while after_end < raw_lines.len() {
                    let next = &raw_lines[after_end];
                    let next_kind = row_kind(next);
                    let compatible = if is_jagged_block {
                        is_loop_or_grid(next) && is_jagged_compatible(&next_kind)
                    } else {
                        is_loop_or_grid(next) && next_kind == last_kind
                    };
                    if compatible {
                        after_end += 1;
                    } else {
                        break;
                    }
                }
                vdots_blocks.push((block_start, j, after_end));
                j = after_end;
            } else {
                j += 1;
            }
        }
    }

    // Build a set of line indices that are "inside" vdots blocks (the loop rows + vdots itself)
    // We'll process by index, skipping handled ones.

    let mut i = 0;
    while i < raw_lines.len() {
        // Check if this index is the start of a vdots block
        if let Some(&(block_start, vdots_idx, after_end)) =
            vdots_blocks.iter().find(|&&(bs, _, _)| bs == i)
        {
            // Determine block kind: query, grid, or loop-row
            let is_grid = matches!(raw_lines[block_start], RawLine::GridRow(_));
            let is_query = matches!(raw_lines[block_start], RawLine::QueryLine { .. });

            // Check if the last "after" row is a JaggedRow — if so, emit LoopJagged.
            // A jagged block has `after_end > vdots_idx + 1` and the final row is JaggedRow.
            let last_after_is_jagged = after_end > vdots_idx + 1
                && matches!(raw_lines[after_end - 1], RawLine::JaggedRow { .. });
            // Also check if the last "before" row (before vdots) is JaggedRow (for the
            // case where there are no "after" rows).
            let last_before_is_jagged = after_end == vdots_idx + 1
                && vdots_idx > block_start
                && matches!(raw_lines[vdots_idx - 1], RawLine::JaggedRow { .. });

            if last_after_is_jagged || last_before_is_jagged {
                // Jagged block: use the after rows (or before rows) to determine structure.
                // The last row must be JaggedRow; preceding rows are LoopRow scalars.
                let jagged_row = if last_after_is_jagged {
                    match &raw_lines[after_end - 1] {
                        RawLine::JaggedRow {
                            row_idx,
                            size_var_math,
                            elem_var_math,
                            scalars_before,
                        } => (
                            row_idx.clone(),
                            size_var_math.clone(),
                            elem_var_math.clone(),
                            scalars_before.clone(),
                        ),
                        _ => return Err(ParseError::Unknown),
                    }
                } else {
                    match &raw_lines[vdots_idx - 1] {
                        RawLine::JaggedRow {
                            row_idx,
                            size_var_math,
                            elem_var_math,
                            scalars_before,
                        } => (
                            row_idx.clone(),
                            size_var_math.clone(),
                            elem_var_math.clone(),
                            scalars_before.clone(),
                        ),
                        _ => return Err(ParseError::Unknown),
                    }
                };
                let (row_idx, size_var_math, elem_var_math, scalars_before_raw) = jagged_row;

                // Collect scalar math names from:
                // 1. scalars_before in the JaggedRow (e.g. L_N → "L")
                // 2. Non-JaggedRow "after" body rows (raw_lines[vdots_idx+1..after_end-1])
                // 3. (For the no-after case, non-JaggedRow "before" rows: block_start..vdots_idx-1)
                let mut all_scalars_math: Vec<String> = Vec::new();

                // From non-jagged body rows (after vdots, excluding the last JaggedRow)
                if last_after_is_jagged {
                    for rl in raw_lines.iter().take(after_end - 1).skip(vdots_idx + 1) {
                        match rl {
                            RawLine::LoopRow(vars) => {
                                for v in vars {
                                    all_scalars_math.push(v.math.clone());
                                }
                            }
                            _ => return Err(ParseError::Unknown),
                        }
                    }
                } else {
                    // last_before_is_jagged: non-jagged body rows are block_start..vdots_idx-1
                    for rl in raw_lines.iter().take(vdots_idx - 1).skip(block_start) {
                        match rl {
                            RawLine::LoopRow(vars) => {
                                for v in vars {
                                    all_scalars_math.push(v.math.clone());
                                }
                            }
                            _ => return Err(ParseError::Unknown),
                        }
                    }
                }

                // From scalars_before in the JaggedRow itself
                for v in &scalars_before_raw {
                    all_scalars_math.push(v.math.clone());
                }

                // Verify SIZE_VAR is among the collected scalar math names
                if !all_scalars_math.contains(&size_var_math) {
                    return Err(ParseError::Unknown);
                }

                // Remove SIZE_VAR from scalars to get the "other scalars"
                let other_scalars: Vec<String> = all_scalars_math
                    .into_iter()
                    .filter(|s| s != &size_var_math)
                    .collect();

                ops.push(IntermOp::LoopJagged {
                    end: row_idx,
                    scalars_math: other_scalars,
                    size_var_math,
                    elem_var_math,
                });

                i = after_end;
                continue;
            }

            // Verify all lines in the block are the same kind (non-jagged blocks)
            for idx in (block_start..vdots_idx).chain(vdots_idx + 1..after_end) {
                let line_is_grid = matches!(raw_lines[idx], RawLine::GridRow(_));
                let line_is_query = matches!(raw_lines[idx], RawLine::QueryLine { .. });
                if line_is_grid != is_grid || line_is_query != is_query {
                    return Err(ParseError::Unknown);
                }
            }

            // Helper: extract subscript / loop_bound from a block row
            let row_subscript = |rl: &RawLine| -> String {
                match rl {
                    RawLine::LoopRow(vars) => vars
                        .last()
                        .and_then(|v| v.subscript.clone())
                        .unwrap_or_default(),
                    RawLine::GridRow(v) => v.subscript.clone().unwrap_or_default(),
                    RawLine::QueryLine { loop_bound } => loop_bound.clone(),
                    _ => String::new(),
                }
            };

            // Get the loop end from the last "after" row, falling back to last "before" row
            let end_size = if after_end > vdots_idx + 1 {
                row_subscript(&raw_lines[after_end - 1])
            } else {
                row_subscript(&raw_lines[vdots_idx - 1])
            };

            let lv = loop_vars.get(loop_var_counter).copied().unwrap_or("i");
            loop_var_counter += 1;

            ops.push(IntermOp::LoopBegin {
                loop_var: lv.to_string(),
                begin: "0".to_string(),
                end: end_size,
            });
            if is_query {
                // Query loop: body format is unknown; emit an empty loop (no ReadLoopRow).
                // The template generates a TODO stub for the user to fill in.
            } else if is_grid {
                let first_before: Vec<String> = match &raw_lines[block_start] {
                    RawLine::GridRow(v) => vec![v.math.clone()],
                    _ => return Err(ParseError::Unknown),
                };
                ops.push(IntermOp::ReadGridRow(
                    first_before.into_iter().next().unwrap_or_default(),
                ));
            } else {
                let first_before: Vec<String> = match &raw_lines[block_start] {
                    RawLine::LoopRow(vars) => vars.iter().map(|v| v.math.clone()).collect(),
                    _ => return Err(ParseError::Unknown),
                };
                ops.push(IntermOp::ReadLoopRow(first_before));
            }
            ops.push(IntermOp::LoopEnd);

            i = after_end;
            continue;
        }

        // Check if this line is inside any vdots block range (shouldn't happen after above)
        if vdots_blocks.iter().any(|&(bs, _, ae)| i > bs && i < ae) {
            i += 1;
            continue;
        }

        match &raw_lines[i] {
            RawLine::Scalars(vars) => {
                if vars.is_empty() {
                    i += 1;
                    continue;
                }
                ops.push(IntermOp::ReadScalars(
                    vars.iter().map(|v| v.math.clone()).collect(),
                ));
                i += 1;
            }
            RawLine::Array1D { name, size } => {
                ops.push(IntermOp::ReadArray1D {
                    name: name.clone(),
                    size: size.clone(),
                });
                i += 1;
            }
            RawLine::Vdots => {
                // Vdots not part of a loop block — skip
                i += 1;
            }
            RawLine::LoopRow(vars) => {
                // LoopRow not matched to a vdots block — treat each var as an independent
                // scalar.  We need two pieces per var:
                //   name_seed  — math + sub concatenated (e.g. "Ax", "r1") for normalize_name
                //   base_math  — original base name (e.g. "A", "r") stored in VarDecl.math so
                //                constraint inference (contains_subscripted) still works
                let infos: Vec<(String, String)> = vars
                    .iter()
                    .map(|v| {
                        let name_seed = match &v.subscript {
                            Some(sub) => format!("{}{}", v.math, sub),
                            None => v.math.clone(),
                        };
                        (name_seed, v.math.clone())
                    })
                    .collect();
                if !infos.is_empty() {
                    ops.push(IntermOp::ReadSubscriptedScalars(infos));
                }
                i += 1;
            }
            RawLine::GridRow(_) => {
                // GridRow not matched to a vdots block — treat as unsupported
                return Err(ParseError::Unknown);
            }
            RawLine::JaggedRow { .. } => {
                // JaggedRow not matched to a vdots block — treat as unsupported
                return Err(ParseError::Unknown);
            }
            RawLine::QueryLine { .. } => {
                // QueryLine not matched to a vdots block — treat as unsupported
                return Err(ParseError::Unknown);
            }
            RawLine::Array2DRow {
                name,
                row_idx,
                col_count,
            } => {
                // Collect consecutive Array2DRow lines with same name, sequential rows from "1".
                if row_idx != "1" {
                    return Err(ParseError::Unknown);
                }
                let base_name = name.clone();
                let cols = *col_count;
                let mut rows = 1usize;
                let mut j = i + 1;
                loop {
                    match raw_lines.get(j) {
                        Some(RawLine::Array2DRow {
                            name: n2,
                            row_idx: r2,
                            col_count: c2,
                        }) if n2 == &base_name => {
                            let expected_row = (rows + 1).to_string();
                            if r2 != &expected_row || *c2 != cols {
                                return Err(ParseError::Unknown);
                            }
                            rows += 1;
                            j += 1;
                        }
                        _ => break,
                    }
                }
                if rows < 2 {
                    return Err(ParseError::Unknown);
                }
                ops.push(IntermOp::ReadGrid {
                    name: base_name,
                    rows,
                    cols,
                });
                i = j;
                continue;
            }
        }
    }

    Ok(ops)
}

// ── Name normalization ─────────────────────────────────────────────────────────

fn normalize_name(math: &str, all_math_names: &[String]) -> String {
    let lower = math.to_lowercase();
    // Check for collision: does any other math name lowercase to the same value?
    let collision = all_math_names
        .iter()
        .filter(|n| n.as_str() != math)
        .any(|n| n.to_lowercase() == lower);
    if collision && math.chars().any(|c| c.is_ascii_uppercase()) {
        // Keep uppercase
        math.to_string()
    } else {
        lower
    }
}

/// Apply `normalize_name` to each identifier within an arithmetic expression string.
/// For a plain name like `"N"` this is equivalent to `normalize_name("N", ...)`.
/// For `"N-1"` the `"N"` part is normalized individually and the rest (`"-1"`) is kept as-is,
/// so the result respects collision-avoidance (e.g. `"N-1"` → `"N-1"` when `N` and `n` coexist).
fn normalize_expr(expr: &str, all_math_names: &[String]) -> String {
    let mut result = String::new();
    let mut pos = 0;
    let mut ident_start: Option<usize> = None;
    for (i, c) in expr.char_indices() {
        let in_ident = if ident_start.is_some() {
            c.is_ascii_alphanumeric()
        } else {
            c.is_ascii_alphabetic()
        };
        if in_ident {
            if ident_start.is_none() {
                ident_start = Some(i);
            }
        } else if let Some(s) = ident_start.take() {
            result.push_str(&expr[pos..s]);
            result.push_str(&normalize_name(&expr[s..i], all_math_names));
            pos = i;
        }
    }
    if let Some(s) = ident_start {
        result.push_str(&expr[pos..s]);
        result.push_str(&normalize_name(&expr[s..], all_math_names));
    } else {
        result.push_str(&expr[pos..]);
    }
    result
}

// ── Type inference ─────────────────────────────────────────────────────────────

fn infer_types(vars: &mut [VarDecl], constraints: &str) {
    if constraints.is_empty() {
        return;
    }

    let all_int = constraints.contains("All input values are integers")
        || constraints.contains("入力は全て整数");

    if all_int {
        for v in vars.iter_mut() {
            // Only promote Unknown → Int; do not overwrite an explicitly assigned type (e.g. Str).
            if v.var_type == VarType::Unknown {
                v.var_type = VarType::Int;
            }
        }
        return;
    }

    for v in vars.iter_mut() {
        let name = &v.name;
        let math = &v.math;

        // Check for string indicators near the variable name
        let is_str = constraints_mention_str(constraints, name, math);
        let is_int = constraints_mention_int(constraints, name, math);

        if is_str {
            v.var_type = VarType::Str;
        } else if is_int {
            v.var_type = VarType::Int;
        }
    }
}

/// Returns true if `token` appears in `haystack` as a standalone token
/// (not adjacent to an alphanumeric character or underscore).
fn contains_token(haystack: &str, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    let tlen = token.len();
    let mut idx = 0;
    while idx + tlen <= bytes.len() {
        if let Some(rel) = haystack[idx..].find(token) {
            let abs = idx + rel;
            let before_ok = abs == 0
                || !{
                    let b = bytes[abs - 1];
                    b.is_ascii_alphanumeric() || b == b'_'
                };
            let after_pos = abs + tlen;
            let after_ok = after_pos >= bytes.len()
                || !{
                    let b = bytes[after_pos];
                    b.is_ascii_alphanumeric() || b == b'_'
                };
            if before_ok && after_ok {
                return true;
            }
            idx = abs + 1;
        } else {
            break;
        }
    }
    false
}

/// Returns true if `line` mentions `math` as a standalone token ("S")
/// or in subscripted form with a left word boundary ("S_i", "S_1", etc.).
fn line_mentions_var(line: &str, math: &str) -> bool {
    contains_token(line, math) || contains_subscripted(line, math)
}

/// Returns true if `haystack` contains `math` followed immediately by `_`,
/// with a left word boundary (not preceded by alphanumeric or `_`).
fn contains_subscripted(haystack: &str, math: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mlen = math.len();
    let mut idx = 0;
    while idx + mlen < bytes.len() {
        match haystack[idx..].find(math) {
            Some(rel) => {
                let abs = idx + rel;
                let before_ok = abs == 0 || {
                    let b = bytes[abs - 1];
                    !(b.is_ascii_alphanumeric() || b == b'_')
                };
                let after_is_underscore = abs + mlen < bytes.len() && bytes[abs + mlen] == b'_';
                if before_ok && after_is_underscore {
                    return true;
                }
                idx = abs + 1;
            }
            None => break,
        }
    }
    false
}

fn constraints_mention_str(constraints: &str, _name: &str, math: &str) -> bool {
    let str_keywords = [
        "文字列",
        "string",
        "英小文字",
        "英大文字",
        "lowercase",
        "uppercase",
        "部分列",
        "部分文字列",
    ];

    for keyword in &str_keywords {
        if constraints.contains(keyword) {
            for line in constraints.lines() {
                if line.contains(keyword) && line_mentions_var(line, math) {
                    return true;
                }
            }
        }
    }
    false
}

fn constraints_mention_int(constraints: &str, _name: &str, math: &str) -> bool {
    let int_keywords = ["整数", "integers"];

    for keyword in &int_keywords {
        if constraints.contains(keyword) {
            for line in constraints.lines() {
                if line.contains(keyword) && line_mentions_var(line, math) {
                    return true;
                }
            }
        }
    }

    for line in constraints.lines() {
        if (line.contains("\\leq") || line.contains("≤") || line.contains('<'))
            && line_mentions_var(line, math)
        {
            return true;
        }
    }

    false
}

// ── Main parse function ────────────────────────────────────────────────────────

pub fn parse(raw: &str, constraints: &str) -> InputSpec {
    // Empty input
    if raw.trim().is_empty() {
        return InputSpec {
            raw: raw.to_string(),
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
    }

    // Preprocess: \hspace{...}\vdots → \vdots
    let preprocessed = preprocess(raw);

    // Split into blocks by \n\n
    let blocks: Vec<&str> = preprocessed.split("\n\n").collect();

    // Phase 2 early detection
    let block0 = blocks[0];

    // Triangular matrix early detection: single-block only
    if blocks.len() == 1
        && let Some(tri) = detect_triangular(block0, constraints)
    {
        // Build the size VarDecl from line[0] of block0
        let size_math = block0
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        let all_math = vec![size_math.to_string()];
        let size_name = normalize_name(size_math, &all_math);
        let size_var = VarDecl {
            name: size_name.clone(),
            math: size_math.to_string(),
            var_type: VarType::Int,
            dim: 0,
            size: vec![],
            is_size: true,
            is_jagged: false,
        };
        let size_op = InputOp {
            tag: OpTag::ReadLine,
            depth: 0,
            vars: vec![VarRef {
                name: size_name,
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
        };
        return InputSpec {
            raw: raw.to_string(),
            ok: true,
            vars: vec![size_var],
            ops: vec![size_op],
            query_types: vec![],
            query_body: vec![],
            testcase_body: vec![],
            iteration_vars: vec![],
            iteration_ops: vec![],
            triangular: Some(tri),
        };
    }

    // Whether block0 contains a query-placeholder marker.
    // Evaluated only when blocks.len() > 1 (short-circuit &&) to avoid the tokenize +
    // parse_line pass on single-block inputs where the value is unused.
    // Detection reuses parse_line so the rule stays consistent with actual QueryLine parsing:
    // \text{...}_Q, \mathrm{...}_Q, or query_Q (plain-text, subscript required).
    // A standalone `query` without a subscript does NOT count as a marker.
    let has_query_marker = blocks.len() > 1
        && block0.lines().any(|line| {
            let tokens = tokenize_line(line);
            matches!(parse_line(&tokens), Ok(RawLine::QueryLine { .. }))
        });

    // Whether the input uses the T-testcases format:
    // block 0 = single scalar (e.g. T), block 1 = body of each test case.
    // T-testcases: block 0 = single scalar, block 1 = test case body (not digit-start).
    // Digit-start block 1 belongs to unsupported query sub-formats, not T-testcases.
    let is_testcase_format = blocks.len() > 1
        && !has_query_marker
        && !blocks[1].trim().starts_with(|c: char| c.is_ascii_digit())
        && {
            let block0_tokens: Vec<Token> = block0
                .lines()
                .filter(|l| !l.trim().is_empty())
                .flat_map(tokenize_line)
                .filter(|t| t != &Token::Space)
                .collect();
            block0_tokens.len() == 1 && matches!(block0_tokens[0], Token::Ident(_))
        };

    // Check for multiple blocks (only reject non-query multi-block forms)
    if blocks.len() > 1 && !has_query_marker && !is_testcase_format {
        let block1 = blocks[1].trim();
        // blocks[1] starts with digit → query sub-format (unrecognized multi-block form)
        if block1.starts_with(|c: char| c.is_ascii_digit()) {
            return not_ok(raw);
        }
    }

    // Tokenize block[0] line by line
    let lines_raw: Vec<&str> = block0.lines().collect();
    let mut raw_lines: Vec<RawLine> = Vec::new();
    let mut phase2_error = false;

    for line in &lines_raw {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let tokens = tokenize_line(trimmed);
        match parse_line(&tokens) {
            Ok(rl) => raw_lines.push(rl),
            Err(_) => {
                phase2_error = true;
                break;
            }
        }
    }

    if phase2_error {
        return not_ok(raw);
    }

    // Build intermediate ops
    let interm_ops = match build_intermediate(&raw_lines) {
        Ok(ops) => ops,
        Err(_) => return not_ok(raw),
    };

    // Collect all math names for normalization
    let all_math_names: Vec<String> = collect_math_names(&interm_ops);

    // Build VarDecl table and InputOp list
    let mut var_decls: Vec<VarDecl> = Vec::new();
    let mut ops: Vec<InputOp> = Vec::new();
    let mut depth: u8 = 0;
    let mut current_loop_end: Option<String> = None;

    for op in &interm_ops {
        match op {
            IntermOp::ReadScalars(names) => {
                let var_refs: Vec<VarRef> = names
                    .iter()
                    .map(|math| {
                        let name = normalize_name(math, &all_math_names);
                        ensure_var_decl(&mut var_decls, &name, math, 0, vec![]);
                        VarRef {
                            name,
                            dim: 0,
                            size: None,
                            index: None,
                        }
                    })
                    .collect();
                ops.push(InputOp {
                    tag: OpTag::ReadLine,
                    depth,
                    vars: var_refs,
                    loop_var: None,
                    begin: None,
                    end: None,
                    scalars: vec![],
                    size_var: None,
                    elem_var: None,
                });
            }
            IntermOp::ReadArray1D { name, size } => {
                let var_name = normalize_name(name, &all_math_names);
                let size_name = normalize_expr(size, &all_math_names);
                ensure_var_decl(&mut var_decls, &var_name, name, 1, vec![size_name.clone()]);
                ops.push(InputOp {
                    tag: OpTag::ReadLine,
                    depth,
                    vars: vec![VarRef {
                        name: var_name,
                        dim: 1,
                        size: Some(size_name),
                        index: None,
                    }],
                    loop_var: None,
                    begin: None,
                    end: None,
                    scalars: vec![],
                    size_var: None,
                    elem_var: None,
                });
            }
            IntermOp::ReadGrid { name, rows, cols } => {
                let var_name = normalize_name(name, &all_math_names);
                // size: [cols, rows] — both literal strings
                ensure_var_decl(
                    &mut var_decls,
                    &var_name,
                    name,
                    2,
                    vec![cols.to_string(), rows.to_string()],
                );
                ops.push(InputOp {
                    tag: OpTag::ReadLine,
                    depth,
                    vars: vec![VarRef {
                        name: var_name,
                        dim: 2,
                        size: None,
                        index: None,
                    }],
                    loop_var: None,
                    begin: None,
                    end: None,
                    scalars: vec![],
                    size_var: None,
                    elem_var: None,
                });
            }
            IntermOp::LoopBegin {
                loop_var,
                begin,
                end,
            } => {
                let end_name = normalize_expr(end, &all_math_names);
                current_loop_end = Some(end_name.clone());
                ops.push(InputOp {
                    tag: OpTag::LoopBegin,
                    depth,
                    vars: vec![],
                    loop_var: Some(loop_var.clone()),
                    begin: Some(begin.clone()),
                    end: Some(end_name),
                    scalars: vec![],
                    size_var: None,
                    elem_var: None,
                });
                depth += 1;
            }
            IntermOp::ReadLoopRow(names) => {
                let lv = ops
                    .iter()
                    .rev()
                    .find_map(|o| o.loop_var.clone())
                    .unwrap_or_else(|| "i".to_string());
                let loop_end = current_loop_end.clone().unwrap_or_default();
                let var_refs: Vec<VarRef> = names
                    .iter()
                    .map(|math| {
                        let name = normalize_name(math, &all_math_names);
                        ensure_var_decl(&mut var_decls, &name, math, 1, vec![loop_end.clone()]);
                        VarRef {
                            name,
                            dim: 1,
                            size: None,
                            index: Some(lv.clone()),
                        }
                    })
                    .collect();
                ops.push(InputOp {
                    tag: OpTag::ReadLine,
                    depth,
                    vars: var_refs,
                    loop_var: None,
                    begin: None,
                    end: None,
                    scalars: vec![],
                    size_var: None,
                    elem_var: None,
                });
            }
            IntermOp::ReadGridRow(math) => {
                let lv = ops
                    .iter()
                    .rev()
                    .find_map(|o| o.loop_var.clone())
                    .unwrap_or_else(|| "i".to_string());
                let loop_end = current_loop_end.clone().unwrap_or_default();
                let name = normalize_name(math, &all_math_names);
                ensure_var_decl(&mut var_decls, &name, math, 1, vec![loop_end.clone()]);
                // Grid rows are always strings — set type directly, overriding inference.
                if let Some(decl) = var_decls.iter_mut().find(|d| d.name == name) {
                    decl.var_type = VarType::Str;
                }
                ops.push(InputOp {
                    tag: OpTag::ReadLine,
                    depth,
                    vars: vec![VarRef {
                        name,
                        dim: 1,
                        size: None,
                        index: Some(lv),
                    }],
                    loop_var: None,
                    begin: None,
                    end: None,
                    scalars: vec![],
                    size_var: None,
                    elem_var: None,
                });
            }
            IntermOp::ReadSubscriptedScalars(infos) => {
                // Each entry: (name_seed, base_math).
                // name_seed → normalize_name → Rust variable name (e.g. "Ax" → "ax")
                // base_math → stored in VarDecl.math so constraint inference still works
                //             (e.g. "A" allows contains_subscripted to find "A_x" in constraints)
                let var_refs: Vec<VarRef> = infos
                    .iter()
                    .map(|(name_seed, base_math)| {
                        let name = normalize_name(name_seed, &all_math_names);
                        ensure_var_decl(&mut var_decls, &name, base_math, 0, vec![]);
                        VarRef {
                            name,
                            dim: 0,
                            size: None,
                            index: None,
                        }
                    })
                    .collect();
                ops.push(InputOp {
                    tag: OpTag::ReadLine,
                    depth,
                    vars: var_refs,
                    loop_var: None,
                    begin: None,
                    end: None,
                    scalars: vec![],
                    size_var: None,
                    elem_var: None,
                });
            }
            IntermOp::LoopEnd => {
                depth = depth.saturating_sub(1);
                current_loop_end = None;
                ops.push(InputOp {
                    tag: OpTag::LoopEnd,
                    depth,
                    vars: vec![],
                    loop_var: None,
                    begin: None,
                    end: None,
                    scalars: vec![],
                    size_var: None,
                    elem_var: None,
                });
            }
            IntermOp::LoopJagged {
                end,
                scalars_math,
                size_var_math,
                elem_var_math,
            } => {
                let end_name = normalize_name(end, &all_math_names);

                // Declare scalar array vars (dim=1, size=[end_name])
                let scalar_var_refs: Vec<VarRef> = scalars_math
                    .iter()
                    .map(|math| {
                        let name = normalize_name(math, &all_math_names);
                        ensure_var_decl(&mut var_decls, &name, math, 1, vec![end_name.clone()]);
                        VarRef {
                            name,
                            dim: 1,
                            size: None,
                            index: None,
                        }
                    })
                    .collect();

                // Declare size_var (dim=1, size=[end_name])
                let size_name = normalize_name(size_var_math, &all_math_names);
                ensure_var_decl(
                    &mut var_decls,
                    &size_name,
                    size_var_math,
                    1,
                    vec![end_name.clone()],
                );
                let size_var_ref = VarRef {
                    name: size_name,
                    dim: 1,
                    size: None,
                    index: None,
                };

                // Declare elem_var (dim=1, size=[end_name], is_jagged=true — set in post-pass)
                let elem_name = normalize_name(elem_var_math, &all_math_names);
                ensure_var_decl(
                    &mut var_decls,
                    &elem_name,
                    elem_var_math,
                    1,
                    vec![end_name.clone()],
                );
                let elem_var_ref = VarRef {
                    name: elem_name,
                    dim: 1,
                    size: None,
                    index: None,
                };

                ops.push(InputOp {
                    tag: OpTag::LoopJagged,
                    depth,
                    vars: vec![],
                    loop_var: None,
                    begin: None,
                    end: Some(end_name),
                    scalars: scalar_var_refs,
                    size_var: Some(size_var_ref),
                    elem_var: Some(elem_var_ref),
                });
            }
        }
    }

    // Type inference
    infer_types(&mut var_decls, constraints);

    // Compute is_size: a var is a size var if its name (or an identifier extracted from an
    // arithmetic expression) appears in any other VarDecl's size or in a LoopBegin end.
    // For plain names like "n" this is a direct match; for expressions like "2*n" or "n-1"
    // we extract the alphabetic identifiers so that "n" is still marked as is_size.
    // Also: LoopJagged's `end` (the loop bound) and `size_var` name are marked as is_size.
    let size_names: std::collections::HashSet<String> = var_decls
        .iter()
        .flat_map(|v| v.size.iter())
        .flat_map(|s| {
            expr_idents(s)
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .chain(
            ops.iter()
                .filter(|o| o.tag == OpTag::LoopBegin)
                .filter_map(|o| o.end.as_deref())
                .flat_map(|s| {
                    expr_idents(s)
                        .into_iter()
                        .map(String::from)
                        .collect::<Vec<_>>()
                }),
        )
        .chain(
            // LoopJagged: mark the loop bound as is_size
            ops.iter()
                .filter(|o| o.tag == OpTag::LoopJagged)
                .filter_map(|o| o.end.as_deref())
                .flat_map(|s| {
                    expr_idents(s)
                        .into_iter()
                        .map(String::from)
                        .collect::<Vec<_>>()
                }),
        )
        .chain(
            // LoopJagged: mark the size_var as is_size
            ops.iter()
                .filter(|o| o.tag == OpTag::LoopJagged)
                .filter_map(|o| o.size_var.as_ref())
                .map(|v| v.name.clone()),
        )
        .collect();
    for v in &mut var_decls {
        v.is_size = size_names.contains(&v.name);
    }

    // Compute is_jagged: a var is jagged if it appears as elem_var in any LoopJagged op.
    let jagged_names: std::collections::HashSet<String> = ops
        .iter()
        .filter(|o| o.tag == OpTag::LoopJagged)
        .filter_map(|o| o.elem_var.as_ref())
        .map(|v| v.name.clone())
        .collect();
    for v in &mut var_decls {
        if jagged_names.contains(&v.name) {
            v.is_jagged = true;
        }
    }

    // Empty result (no vars and no ops) means the raw text produced nothing
    // meaningful — treat as not_ok so templates use the safe fallback.
    if var_decls.is_empty() && ops.is_empty() {
        return not_ok(raw);
    }

    // Try to flatten single-variable loops to array reads.
    // Multi-var loops (or loops that can't be flattened) are kept in ops; the template handles them.
    ops = flatten_single_var_loops(ops, &mut var_decls);

    // Any remaining LoopBegin or LoopJagged ops will be emitted directly by the template,
    // so validate their bounds before reporting ok=true.
    // Accepts: literal digit strings, declared scalar var names, or arithmetic expressions
    // (e.g. "n-1", "2*n", "n+1") where all identifiers are declared scalars.
    let is_valid_bound = |end: &str| -> bool {
        end.chars().all(|c| c.is_ascii_digit())
            || var_decls.iter().any(|v| v.name == end && v.dim == 0)
            || expr_idents(end)
                .iter()
                .all(|id| var_decls.iter().any(|v| v.name == *id && v.dim == 0))
    };
    let valid_loop_bounds = ops
        .iter()
        .filter(|o| o.tag == OpTag::LoopBegin || o.tag == OpTag::LoopJagged)
        .all(|o| {
            let end = match o.end.as_deref().map(str::trim) {
                Some(end) if !end.is_empty() => end,
                _ => return false,
            };
            is_valid_bound(end)
        });
    if !valid_loop_bounds {
        return not_ok(raw);
    }

    // Parse query sub-blocks (blocks[1..]) when a query marker was present in block0.
    // Numbered sub-blocks → query_types; non-numeric sub-block → query_body (scalar)
    // or iteration_vars/ops (complex body with loops/arrays).
    let (query_types, query_body, iteration_vars, iteration_ops) = if has_query_marker {
        parse_query_subblocks(&blocks[1..], constraints)
    } else {
        (vec![], vec![], vec![], vec![])
    };

    // Parse T-testcases block 1 as scalar vars for testcase_body.
    // On success, mark the single block-0 var (the loop count, e.g. T) as is_size.
    let testcase_body = if is_testcase_format {
        parse_scalar_block(blocks[1], constraints)
    } else {
        vec![]
    };
    // Always mark the block-0 loop-count var as is_size when the format is T-testcases,
    // even when testcase_body is empty (non-scalar block 1 falls back to todo!() stub).
    if is_testcase_format && let Some(v) = var_decls.first_mut() {
        v.is_size = true;
    }

    InputSpec {
        raw: raw.to_string(),
        ok: true,
        vars: var_decls,
        ops,
        query_types,
        query_body,
        testcase_body,
        iteration_vars,
        iteration_ops,
        triangular: None,
    }
}

/// Detect an upper-triangular matrix input pattern and return a `TriangularSpec` if matched.
///
/// The pattern is:
/// ```text
/// N
/// A_{1, 2} A_{1, 3} \ldots A_{1, bound}
/// A_{2, 3} \ldots A_{2, bound}
/// \vdots
/// A_{bound-1, bound}
/// ```
/// where `bound` is an expression like "N" or "2N".
///
/// Returns `Some(TriangularSpec)` when all structural checks pass, `None` otherwise.
fn detect_triangular(block0: &str, constraints: &str) -> Option<TriangularSpec> {
    // Collect non-empty lines
    let lines: Vec<&str> = block0
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    // Need at least 4 lines: size, first data row, line2 (vdots or another row), last element
    if lines.len() < 4 {
        return None;
    }

    // Line 0: single Ident token (size variable, e.g. "N")
    let size_tokens = strip_spaces(&tokenize_line(lines[0]));
    let size_math = match size_tokens.as_slice() {
        [Token::Ident(s)] => s.clone(),
        _ => return None,
    };

    // Line 1: a triangular row — check it has the right structure
    // and extract: var_name (math), bound expression (string from last element's second subscript)
    let (var_math, bound_raw) = detect_triangular_row(lines[1])?;

    // Verify all idents in bound_raw refer to the size variable only.
    // An unknown ident would generate an undefined variable reference in the template.
    if !expr_idents(&bound_raw)
        .iter()
        .all(|id| id.eq_ignore_ascii_case(&size_math))
    {
        return None;
    }

    // Line 2: either another triangular row OR a vdots-like line
    // Check that line 2 is either vdots-like or a triangular row with the same var name
    let line2_ok = {
        let toks = strip_spaces(&tokenize_line(lines[2]));
        // Is it a vdots-like line?
        let is_vdots = toks.len() == 1 && matches!(toks[0], Token::Vdots | Token::Cdots);
        if is_vdots {
            true
        } else {
            // Try as another triangular row with same var name and consistent bound
            matches!(detect_triangular_row(lines[2]), Some((n, b)) if n == var_math && b == bound_raw)
        }
    };
    if !line2_ok {
        return None;
    }

    // Last non-empty line: single element "VAR_{first_idx, bound}" (no Cdots)
    // Fix 2: parse with read_comma_brace_parts to verify:
    //   (a) second subscript matches bound_raw (same canonical form)
    //   (b) no extra tokens remain after RBrace
    let last_line = lines[lines.len() - 1];
    {
        let toks = strip_spaces(&tokenize_line(last_line));
        // Must not contain cdots
        if toks.contains(&Token::Cdots) {
            return None;
        }
        // Must start with same var name
        match toks.first() {
            Some(Token::Ident(n)) if n == &var_math => {}
            _ => return None,
        }
        // Must have a subscript followed by LBrace
        if toks.get(1) != Some(&Token::Subscript) || toks.get(2) != Some(&Token::LBrace) {
            return None;
        }
        // Parse the brace subscript: expect exactly (part1, part2) with no trailing tokens
        let (_, last_second, advance) = read_comma_brace_parts(&toks[3..])?;
        // advance includes RBrace; no tokens should remain
        if 3 + advance != toks.len() {
            return None;
        }
        // Second subscript of last element must match bound_raw (same canonical string)
        if last_second != bound_raw {
            return None;
        }
    }

    // All intermediate lines (between line 1 and last) must be vdots-like or triangular rows
    for &line in &lines[2..lines.len() - 1] {
        let toks = strip_spaces(&tokenize_line(line));
        let is_vdots = toks.len() == 1 && matches!(toks[0], Token::Vdots | Token::Cdots);
        if !is_vdots {
            match detect_triangular_row(line) {
                Some((n, b)) if n == var_math && b == bound_raw => {}
                _ => return None,
            }
        }
    }

    // All checks passed — build the TriangularSpec
    // Normalize names: size variable and bound expression
    // For simplicity, all math names here are just size_math and the bound idents
    let all_math = vec![size_math.clone()];

    // Normalize the bound: parse bound_raw as an expression
    // bound_raw is already in arithmetic form (e.g. "N", "2*N")
    // We need to lowercase the ident part(s) using normalize_name
    let bound = normalize_expr(&bound_raw, &all_math);

    let var_name = normalize_name(&var_math, &all_math);

    // Infer var_type from constraints
    let var_type = {
        let mut decls = vec![VarDecl {
            name: var_name.clone(),
            math: var_math.clone(),
            var_type: VarType::Unknown,
            dim: 2,
            size: vec![],
            is_size: false,
            is_jagged: false,
        }];
        infer_types(&mut decls, constraints);
        // Fall back to Int when constraints give no type information (Unknown).
        // The spec says triangular.var_type defaults to "int".
        if decls[0].var_type == VarType::Unknown {
            VarType::Int
        } else {
            decls[0].var_type.clone()
        }
    };

    Some(TriangularSpec {
        name: var_name,
        math: var_math,
        var_type,
        bound,
    })
}

/// Detect a single triangular row line.
/// A triangular row looks like: `A_{1, 2} A_{1, 3} \ldots A_{1, N}`
/// Returns `Some((var_math, bound_raw))` where `bound_raw` is the arithmetic
/// expression for the second subscript of the last element (e.g. "N", "2*N").
fn detect_triangular_row(line: &str) -> Option<(String, String)> {
    let tokens = strip_spaces(&tokenize_line(line));

    // Must contain exactly one Cdots
    let cdots_count = tokens.iter().filter(|t| **t == Token::Cdots).count();
    if cdots_count != 1 {
        return None;
    }

    // Scan tokens sequentially, skipping Space tokens.
    // Each non-Space, non-Cdots token sequence is expected to form one element:
    //   Ident Subscript LBrace <part1> Comma <part2> RBrace
    // A single Cdots in the stream is allowed; elements may appear before and after it.
    let mut var_math: Option<String> = None;
    let mut first_idx: Option<String> = None; // first subscript (row index, must be ASCII digits)
    let mut last_second_sub: Option<String> = None; // second subscript of last element (the bound)
    let mut found_cdots = false;
    let mut found_element_after_cdots = false;

    let mut i = 0;
    while i < tokens.len() {
        // Skip spaces
        while i < tokens.len() && tokens[i] == Token::Space {
            i += 1;
        }
        if i >= tokens.len() {
            break;
        }

        // Cdots
        if tokens[i] == Token::Cdots {
            found_cdots = true;
            i += 1;
            continue;
        }

        // Element: Ident Subscript { first_part , second_part }
        let name = match &tokens[i] {
            Token::Ident(n) => n.clone(),
            _ => return None,
        };
        i += 1;

        if i >= tokens.len() || tokens[i] != Token::Subscript {
            return None;
        }
        i += 1;

        // Read the subscript: must be brace form with comma
        if i >= tokens.len() || tokens[i] != Token::LBrace {
            return None;
        }
        i += 1; // skip LBrace

        // Read until matching RBrace, collecting tokens for the two parts
        let (part1_raw, part2_raw, advance) = read_comma_brace_parts(&tokens[i..])?;
        i += advance;

        // Fix 3: first subscript (row index) must be purely numeric (e.g. "1", "2", not "i" or "2N")
        if !part1_raw.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }

        // Validate var name consistency
        match &var_math {
            None => var_math = Some(name.clone()),
            Some(n) if n != &name => return None,
            _ => {}
        }

        // Validate first index (row) consistency: all elements on this row must have same first subscript
        match &first_idx {
            None => first_idx = Some(part1_raw.clone()),
            Some(f) if f != &part1_raw => return None,
            _ => {}
        }

        // Track last second subscript (the bound)
        last_second_sub = Some(part2_raw);

        // Fix 1: track whether at least one element appears after cdots
        if found_cdots {
            found_element_after_cdots = true;
        }
    }

    // Fix 1: must have found cdots AND at least one element after it
    if !found_cdots || !found_element_after_cdots {
        return None;
    }

    let _var = var_math?;
    let bound_expr = last_second_sub?;

    // Convert the bound_expr tokens string to arithmetic form
    // bound_expr is already a string like "N" or "2*N" from read_comma_brace_parts
    Some((_var, bound_expr))
}

/// Read the two parts of a comma-separated brace subscript `{part1, part2}`.
/// Assumes the opening `{` has already been consumed.
/// Returns `(part1_str, part2_str, tokens_consumed_including_RBrace)`.
/// `part1_str` / `part2_str` are arithmetic expressions (e.g. "N", "2*N", "N-1").
fn read_comma_brace_parts(tokens: &[Token]) -> Option<(String, String, usize)> {
    let mut i = 0;
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut last_kind: u8 = 0; // 0=start/op, 1=num, 2=ident

    while i < tokens.len() {
        match &tokens[i] {
            Token::RBrace => {
                i += 1;
                if !current.is_empty() {
                    parts.push(current.clone());
                }
                break;
            }
            Token::Comma => {
                if current.is_empty() {
                    return None; // leading comma
                }
                parts.push(current.clone());
                current = String::new();
                last_kind = 0;
                i += 1;
            }
            Token::Space => {
                // ignore spaces inside braces
                i += 1;
            }
            Token::Num(n) => {
                if last_kind == 2 {
                    current.push('*');
                }
                current.push_str(n);
                last_kind = 1;
                i += 1;
            }
            Token::Ident(s) => {
                if last_kind == 1 {
                    current.push('*');
                }
                current.push_str(s);
                last_kind = 2;
                i += 1;
            }
            Token::Plus => {
                if last_kind == 0 {
                    return None;
                }
                current.push('+');
                last_kind = 0;
                i += 1;
            }
            Token::Minus => {
                if last_kind == 0 {
                    return None;
                }
                current.push('-');
                last_kind = 0;
                i += 1;
            }
            Token::Star => {
                if last_kind == 0 {
                    return None;
                }
                current.push('*');
                last_kind = 0;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    if parts.len() != 2 {
        return None;
    }
    Some((parts[0].clone(), parts[1].clone(), i))
}

/// Normalize raw input-format text before tokenization.
///
/// Transformations applied:
/// - Replaces Unicode horizontal ellipsis (U+2026) with `\ldots`
/// - Replaces `\hspace{...}\vdots` sequences with `\vdots`
fn preprocess(raw: &str) -> String {
    // Replace Unicode horizontal ellipsis (U+2026) with \ldots
    let raw = &raw.replace('\u{2026}', "\\ldots");

    // Replace \hspace{...}\vdots with \vdots
    let mut result = String::new();
    let mut rest = raw.as_str();

    while let Some(pos) = rest.find("\\hspace{") {
        result.push_str(&rest[..pos]);
        let after = &rest[pos + 8..]; // skip "\\hspace{"
        // find closing }
        if let Some(close) = after.find('}') {
            let after_close = &after[close + 1..];
            // Check if followed by \vdots
            let trimmed = after_close.trim_start_matches([' ', '\t']);
            if trimmed.starts_with("\\vdots") {
                result.push_str("\\vdots");
                rest = trimmed.strip_prefix("\\vdots").unwrap_or("");
            } else {
                // keep hspace
                result.push_str("\\hspace{");
                result.push_str(&after[..close + 1]);
                rest = after_close;
            }
        } else {
            // malformed, keep as-is
            result.push_str(&rest[pos..]);
            rest = "";
            break;
        }
    }
    result.push_str(rest);
    result
}

/// Attempt to flatten single-variable loops into array reads.
/// A single-var loop looks like: LoopBegin, ReadLine(1 var with index), LoopEnd.
/// Each loop is flattened independently; multi-var loops or those with invalid bounds
/// are left in place so the template can emit loop code for them.
fn flatten_single_var_loops(ops: Vec<InputOp>, var_decls: &mut [VarDecl]) -> Vec<InputOp> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < ops.len() {
        // Attempt to flatten: LoopBegin, ReadLine(1 var with index), LoopEnd
        let can_flatten = ops[i].tag == OpTag::LoopBegin
            && i + 2 < ops.len()
            && ops[i + 1].tag == OpTag::ReadLine
            && ops[i + 1].vars.len() == 1
            && ops[i + 1].vars[0].index.is_some()
            && ops[i + 2].tag == OpTag::LoopEnd;

        if can_flatten {
            let loop_end = ops[i].end.as_deref().unwrap_or("").trim().to_string();
            let v = &ops[i + 1].vars[0];

            // Validate loop_end: must be a numeric literal or a declared scalar.
            let loop_end_valid = !loop_end.is_empty()
                && (loop_end.chars().all(|c| c.is_ascii_digit())
                    || var_decls.iter().any(|d| d.name == loop_end && d.dim == 0));

            // Validate that the VarDecl is compatible (dim=1, consistent size).
            let decl_compatible = var_decls.iter().any(|d| {
                d.name == v.name
                    && d.dim == 1
                    && (d.size.is_empty() || d.size == vec![loop_end.clone()])
            });

            if loop_end_valid && decl_compatible {
                // Mutate the VarDecl size if not yet set.
                if let Some(decl) = var_decls
                    .iter_mut()
                    .find(|d| d.name == v.name && d.size.is_empty())
                {
                    decl.size = vec![loop_end.clone()];
                }
                result.push(InputOp {
                    tag: OpTag::ReadLine,
                    depth: ops[i].depth,
                    vars: vec![VarRef {
                        name: v.name.clone(),
                        dim: 1,
                        size: Some(loop_end),
                        index: None,
                    }],
                    loop_var: None,
                    begin: None,
                    end: None,
                    scalars: vec![],
                    size_var: None,
                    elem_var: None,
                });
                i += 3;
                continue;
            }
        }

        // Not flattenable — push as-is; ReadLine/LoopEnd will be pushed in subsequent iterations.
        result.push(ops[i].clone());
        i += 1;
    }
    result
}

/// Parse a single block as a flat list of scalar variables.
/// Returns the VarDecl list on success, or an empty vec if parsing fails or produces
/// non-scalar results (e.g. arrays, loops).
fn parse_scalar_block(block: &str, constraints: &str) -> Vec<VarDecl> {
    let lines: Vec<&str> = block
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return vec![];
    }

    // Each line must parse as plain scalars (Scalars or LoopRow — subscripted vars
    // like A_x A_y treated as dim=0 scalars). Any other pattern (Array1d with Cdots,
    // GridRow, Vdots, QueryLine, …) means this is not a simple scalar block.
    let mut all_raw_vars: Vec<RawVar> = vec![];
    for line in &lines {
        let toks = tokenize_line(line);
        match parse_line(&toks) {
            Ok(RawLine::Scalars(v)) | Ok(RawLine::LoopRow(v)) => all_raw_vars.extend(v),
            _ => return vec![],
        }
    }
    let raw_vars = all_raw_vars;
    if raw_vars.is_empty() {
        return vec![];
    }

    let seeds: Vec<String> = raw_vars
        .iter()
        .map(|rv| match &rv.subscript {
            Some(s) => format!("{}{}", rv.math, s),
            None => rv.math.clone(),
        })
        .collect();
    let mut var_decls: Vec<VarDecl> = raw_vars
        .iter()
        .zip(seeds.iter())
        .map(|(rv, seed)| {
            let name = normalize_name(seed, &seeds);
            VarDecl {
                name,
                math: rv.math.clone(),
                var_type: VarType::Unknown,
                dim: 0,
                size: vec![],
                is_size: false,
                is_jagged: false,
            }
        })
        .collect();
    infer_types(&mut var_decls, constraints);
    var_decls
}

/// Parse `blocks[1..]` from a query-type input.
///
/// Returns `(query_types, query_body, iteration_vars, iteration_ops)`:
/// - `query_types`: numbered sub-blocks (first token = Num).
/// - `query_body`: scalar vars from the first non-numeric sub-block (scalars only).
/// - `iteration_vars` / `iteration_ops`: from a full re-parse of the first non-numeric
///   sub-block when scalar parse fails (complex body with loops/arrays).
///
/// Priority: `query_types` non-empty → `query_body`/`iteration_*` empty.
///           `iteration_ops` non-empty → `query_body` empty.
fn parse_query_subblocks(
    subblocks: &[&str],
    constraints: &str,
) -> (Vec<QueryTypeDecl>, Vec<VarDecl>, Vec<VarDecl>, Vec<InputOp>) {
    let mut result = Vec::new();
    let mut query_body: Vec<VarDecl> = vec![];
    let mut iteration_vars: Vec<VarDecl> = vec![];
    let mut iteration_ops: Vec<InputOp> = vec![];

    for block in subblocks {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        // Collect all non-empty lines across the sub-block.
        let lines: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if lines.is_empty() {
            continue;
        }

        // Tokenize the first line to extract type_id and first-line vars.
        let first_tokens = tokenize_line(lines[0]);
        let first_stripped = strip_spaces(&first_tokens);

        // First token must be Num → type_id; otherwise → candidate for query_body
        // (step 1: scalar) or iteration_vars/ops (step 2: full re-parse).
        let type_id = match first_stripped.first() {
            Some(Token::Num(n)) => n.clone(),
            _ => {
                // Non-numeric sub-block: try scalar parse first, then full re-parse.
                if iteration_vars.is_empty() && query_body.is_empty() && result.is_empty() {
                    // Step 1: attempt flat scalar var list parse.
                    let mut all_var_tokens: Vec<Token> = first_stripped.to_vec();
                    for line in &lines[1..] {
                        let toks = tokenize_line(line);
                        let stripped = strip_spaces(&toks);
                        all_var_tokens.push(Token::Space);
                        all_var_tokens.extend(stripped);
                    }
                    // Only accept plain scalars (no subscripts) for query_body.
                    // Subscripted vars (LoopRow) indicate array/loop structure → step 2.
                    let raw_vars_opt = match parse_var_list(&all_var_tokens) {
                        Ok(RawLine::Scalars(v)) if !v.is_empty() => Some(v),
                        _ => None,
                    };
                    if let Some(raw_vars) = raw_vars_opt {
                        // Use math+subscript as name seed so that subscripted vars
                        // like x_1 / x_2 yield distinct identifiers (x1, x2).
                        let seeds: Vec<String> = raw_vars
                            .iter()
                            .map(|rv| match &rv.subscript {
                                Some(s) => format!("{}{}", rv.math, s),
                                None => rv.math.clone(),
                            })
                            .collect();
                        let mut var_decls: Vec<VarDecl> = raw_vars
                            .iter()
                            .zip(seeds.iter())
                            .map(|(rv, seed)| {
                                let name = normalize_name(seed, &seeds);
                                VarDecl {
                                    name,
                                    math: rv.math.clone(),
                                    var_type: VarType::Unknown,
                                    dim: 0,
                                    size: vec![],
                                    is_size: false,
                                    is_jagged: false,
                                }
                            })
                            .collect();
                        infer_types(&mut var_decls, constraints);
                        query_body = var_decls;
                    } else {
                        // Step 2: full re-parse of this sub-block as a standalone input spec.
                        // Reconstruct the raw text of the sub-block (all lines joined).
                        let mini_raw = block.trim();
                        let mini = parse(mini_raw, constraints);
                        if mini.ok {
                            iteration_vars = mini.vars;
                            iteration_ops = mini.ops;
                        }
                        // (ok=false → iteration_vars/ops stay empty; template generates TODO stub)
                    }
                }
                continue;
            }
        };

        // Collect all var-name tokens from first-line remainder + subsequent lines.
        // We only support plain scalars (Ident, possibly with subscript).
        let remainder = &first_stripped[1..]; // skip the Num token
        let mut all_var_tokens: Vec<Token> = remainder.to_vec();
        for line in &lines[1..] {
            let toks = tokenize_line(line);
            let stripped = strip_spaces(&toks);
            all_var_tokens.push(Token::Space);
            all_var_tokens.extend(stripped);
        }

        // Parse the collected tokens as a var list.
        let var_list_result = parse_var_list(&all_var_tokens);
        let (ok, raw_vars) = match var_list_result {
            Ok(RawLine::Scalars(vars)) => (true, vars),
            Ok(RawLine::LoopRow(vars)) => {
                // Subscripted vars (e.g. "x_1") — treat each as a plain scalar using
                // the concatenated math+subscript as the name seed.
                (true, vars)
            }
            _ => (false, vec![]),
        };

        if !ok {
            result.push(QueryTypeDecl {
                type_id,
                ok: false,
                vars: vec![],
            });
            continue;
        }

        // Build local VarDecl list (independent scope from main vars).
        // Use math+subscript as name seed so that subscripted vars like x_1 / x_2
        // yield distinct identifiers (x1, x2) rather than colliding as x.
        let seeds: Vec<String> = raw_vars
            .iter()
            .map(|rv| match &rv.subscript {
                Some(s) => format!("{}{}", rv.math, s),
                None => rv.math.clone(),
            })
            .collect();
        let mut var_decls: Vec<VarDecl> = raw_vars
            .iter()
            .zip(seeds.iter())
            .map(|(rv, seed)| {
                let name = normalize_name(seed, &seeds);
                VarDecl {
                    name,
                    math: rv.math.clone(),
                    var_type: VarType::Unknown,
                    dim: 0,
                    size: vec![],
                    is_size: false,
                    is_jagged: false,
                }
            })
            .collect();

        // Apply same type inference as main vars.
        infer_types(&mut var_decls, constraints);

        result.push(QueryTypeDecl {
            type_id,
            ok: true,
            vars: var_decls,
        });
    }

    // Priority: query_types non-empty → discard query_body and iteration_vars/ops.
    if !result.is_empty() {
        query_body = vec![];
        iteration_vars = vec![];
        iteration_ops = vec![];
    }

    // iteration_ops non-empty → discard query_body (complex body takes priority over scalar body).
    if !iteration_ops.is_empty() {
        query_body = vec![];
    }

    (result, query_body, iteration_vars, iteration_ops)
}

fn not_ok(raw: &str) -> InputSpec {
    InputSpec {
        raw: raw.to_string(),
        ok: false,
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

/// Extract all alphabetic identifiers from an arithmetic expression string.
/// For a plain name like "n" returns ["n"]; for "2*n" returns ["n"]; for "n-1" returns ["n"].
/// Used for is_size computation and valid_loop_bounds validation.
fn expr_idents(expr: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in expr.char_indices() {
        // An identifier starts with an ASCII letter and continues with alphanumerics,
        // matching the lexer's Ident token definition (which also consumes trailing digits).
        // This ensures "n2" is extracted as ["n2"] rather than ["n"], preventing false
        // positives in valid_loop_bounds and is_size when expressions like "n2" appear.
        let in_ident = if start.is_some() {
            c.is_ascii_alphanumeric()
        } else {
            c.is_ascii_alphabetic()
        };
        if in_ident {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            result.push(&expr[s..i]);
        }
    }
    if let Some(s) = start {
        result.push(&expr[s..]);
    }
    result
}

fn collect_math_names(ops: &[IntermOp]) -> Vec<String> {
    let mut names = Vec::new();
    for op in ops {
        match op {
            IntermOp::ReadScalars(ns) => names.extend(ns.iter().cloned()),
            IntermOp::ReadArray1D { name, size } => {
                names.push(name.clone());
                names.push(size.clone());
            }
            IntermOp::ReadLoopRow(ns) => names.extend(ns.iter().cloned()),
            IntermOp::ReadGridRow(n) => names.push(n.clone()),
            // Use name_seeds (not base_math) so normalize_name collision detection works correctly.
            IntermOp::ReadSubscriptedScalars(infos) => {
                names.extend(infos.iter().map(|(seed, _)| seed.clone()))
            }
            IntermOp::LoopBegin { end, .. } => names.push(end.clone()),
            IntermOp::LoopEnd => {}
            IntermOp::ReadGrid { name, .. } => names.push(name.clone()),
            IntermOp::LoopJagged {
                end,
                scalars_math,
                size_var_math,
                elem_var_math,
            } => {
                names.push(end.clone());
                names.extend(scalars_math.iter().cloned());
                names.push(size_var_math.clone());
                names.push(elem_var_math.clone());
            }
        }
    }
    names
}

fn ensure_var_decl(decls: &mut Vec<VarDecl>, name: &str, math: &str, dim: u8, size: Vec<String>) {
    if !decls.iter().any(|d| d.name == name) {
        decls.push(VarDecl {
            name: name.to_string(),
            math: math.to_string(),
            var_type: VarType::Unknown,
            dim,
            size,
            is_size: false,
            is_jagged: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::entity::{InputFormatKind, OpTag, VarType};

    // ── helpers ────────────────────────────────────────────────────────────────

    fn scalar_ok(raw: &str) -> InputSpec {
        parse(raw, "")
    }

    fn with_constraints(raw: &str, constraints: &str) -> InputSpec {
        parse(raw, constraints)
    }

    // ── basic: scalar line ─────────────────────────────────────────────────────

    /// "N M\n" → two scalars on one ReadLine
    #[test]
    fn scalars_on_one_line() {
        let spec = scalar_ok("N M\n");
        assert!(spec.ok, "expected ok=true for 'N M'");

        // vars
        assert_eq!(spec.vars.len(), 2);
        let n = &spec.vars[0];
        assert_eq!(n.name, "n");
        assert_eq!(n.math, "N");
        assert_eq!(n.dim, 0);
        let m = &spec.vars[1];
        assert_eq!(m.name, "m");
        assert_eq!(m.math, "M");
        assert_eq!(m.dim, 0);

        // ops: single ReadLine at depth 0 with both vars
        assert_eq!(spec.ops.len(), 1);
        let op = &spec.ops[0];
        assert_eq!(op.tag, OpTag::ReadLine);
        assert_eq!(op.depth, 0);
        assert_eq!(op.vars.len(), 2);
        assert_eq!(op.vars[0].name, "n");
        assert_eq!(op.vars[0].dim, 0);
        assert_eq!(op.vars[1].name, "m");
        assert_eq!(op.vars[1].dim, 0);
    }

    // ── 1-D array (horizontal ldots) ──────────────────────────────────────────

    /// "N\nA_1 A_2 \\ldots A_N\n" → n scalar + a[n] array
    #[test]
    fn one_d_array_horizontal_ldots() {
        let spec = scalar_ok("N\nA_1 A_2 \\ldots A_N\n");
        assert!(spec.ok, "expected ok=true for 1D array");

        assert_eq!(spec.vars.len(), 2);
        let n = &spec.vars[0];
        assert_eq!(n.name, "n");
        assert_eq!(n.dim, 0);
        let a = &spec.vars[1];
        assert_eq!(a.name, "a");
        assert_eq!(a.dim, 1);
        assert_eq!(a.size, vec!["n".to_string()]);

        assert_eq!(spec.ops.len(), 2);
        // first ReadLine: n
        assert_eq!(spec.ops[0].tag, OpTag::ReadLine);
        assert_eq!(spec.ops[0].vars[0].name, "n");
        // second ReadLine: a (dim 1, size "n")
        assert_eq!(spec.ops[1].tag, OpTag::ReadLine);
        let av = &spec.ops[1].vars[0];
        assert_eq!(av.name, "a");
        assert_eq!(av.dim, 1);
        assert_eq!(av.size, Some("n".to_string()));
    }

    // ── multiple vars per iteration (vdots) ───────────────────────────────────

    /// "Q\nt_1 k_1\nt_2 k_2\n\\vdots\nt_Q k_Q\n"
    #[test]
    fn multi_var_loop_vdots() {
        // Phase 2: multi-var loops produce ok=true; LoopBegin/LoopEnd ops are kept for template codegen
        let spec = scalar_ok("Q\nt_1 k_1\nt_2 k_2\n\\vdots\nt_Q k_Q\n");
        assert!(
            spec.ok,
            "expected ok=true for multi-var vdots loop (Phase 2: template handles loop codegen)"
        );
        assert!(spec.ops.iter().any(|o| o.tag == OpTag::LoopBegin));
    }

    // ── \\hspace{} vdots normalisation ────────────────────────────────────────

    /// \\hspace{0.4cm}\\vdots should be treated identically to a bare \\vdots
    #[test]
    fn hspace_vdots_normalised() {
        // Phase 2: both plain and hspace+vdots produce ok=true with LoopBegin ops.
        // The key assertion is that they produce the same result (hspace is
        // normalised to plain vdots before parsing).
        let spec_plain = scalar_ok("Q\nt_1 k_1\nt_2 k_2\n\\vdots\nt_Q k_Q\n");
        let spec_hspace = scalar_ok("Q\nt_1 k_1\n\\hspace{0.4cm}\\vdots\nt_Q k_Q\n");

        assert!(
            spec_hspace.ok,
            "expected ok=true for hspace+vdots (Phase 2: template handles loop codegen)"
        );
        assert_eq!(
            spec_hspace.ok, spec_plain.ok,
            "hspace vdots should behave identically to plain vdots"
        );
    }

    // ── Query type: \\text{query}_Q ───────────────────────────────────────────

    /// \text{query}_Q vdots block → ok=true, preamble var q, LoopBegin(0..q)+LoopEnd
    #[test]
    fn text_query_vdots_ok() {
        let spec = scalar_ok("Q\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n");
        assert!(spec.ok, "expected ok=true for \\text{{query}} vdots block");
        assert_eq!(spec.vars.len(), 1);
        assert_eq!(spec.vars[0].name, "q");
        assert!(spec.vars[0].is_size, "q should be is_size=true");
        // ops: ReadLine(q), LoopBegin(0..q), LoopEnd
        assert_eq!(spec.ops.len(), 3);
        assert_eq!(spec.ops[0].tag, OpTag::ReadLine);
        assert_eq!(spec.ops[1].tag, OpTag::LoopBegin);
        assert_eq!(spec.ops[1].end.as_deref(), Some("q"));
        assert_eq!(spec.ops[2].tag, OpTag::LoopEnd);
    }

    // ── Query type: \\mathrm{Query}_Q ─────────────────────────────────────────

    /// \mathrm{Query}_Q vdots block with preamble → ok=true
    #[test]
    fn mathrm_query_vdots_ok() {
        let spec = scalar_ok("N Q\n\\mathrm{Query}_1\n\\vdots\n\\mathrm{Query}_Q\n");
        assert!(
            spec.ok,
            "expected ok=true for \\mathrm{{Query}} vdots block"
        );
        // preamble vars n and q
        assert!(spec.vars.iter().any(|v| v.name == "n"), "expected var n");
        let qv = spec
            .vars
            .iter()
            .find(|v| v.name == "q")
            .expect("expected var q");
        assert!(qv.is_size, "q should be is_size=true");
        // ops: ReadLine(n,q), LoopBegin(0..q), LoopEnd
        assert_eq!(spec.ops.len(), 3);
        assert_eq!(spec.ops[1].tag, OpTag::LoopBegin);
        assert_eq!(spec.ops[1].end.as_deref(), Some("q"));
    }

    // ── Query type: multi-block with \\text{query} marker ─────────────────────

    /// Multi-block with \text{query}_Q in block[0] → ok=true (extra blocks ignored)
    #[test]
    fn text_query_multi_block_ok() {
        let spec =
            scalar_ok("N Q\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n\n1 x\n\n2 x k\n\n3 l r\n");
        assert!(
            spec.ok,
            "expected ok=true for multi-block \\text{{query}} form"
        );
        assert!(spec.vars.iter().any(|v| v.name == "n"));
        assert!(spec.vars.iter().any(|v| v.name == "q"));
        assert_eq!(spec.ops.len(), 3);
        assert_eq!(spec.ops[1].tag, OpTag::LoopBegin);
        assert_eq!(spec.ops[1].end.as_deref(), Some("q"));
    }

    // ── Query type without subscript → still ok=false ─────────────────────────

    /// \text{query} with no subscript → ok=false (malformed)
    #[test]
    fn text_query_no_subscript_not_ok() {
        let spec = scalar_ok("Q\n\\text{query}\n");
        assert!(
            !spec.ok,
            "expected ok=false for \\text{{query}} without subscript"
        );
    }

    // ── Phase 2: multiple blocks (non-query digit sub-format) → still ok=false

    /// Two \\n\\n-separated blocks where block[1] starts with a digit and no query marker
    /// Uses q_i (short ident, not "query") → no marker detected → ok=false
    #[test]
    fn multi_block_digit_no_query_marker_not_ok() {
        let spec = scalar_ok("Q\nq_1\n\\vdots\nq_Q\n\n1 x\n\n2 x k");
        assert!(
            !spec.ok,
            "expected ok=false for multi-block with short q_i marker (not 'query')"
        );
    }

    // ── Phase 2: T-testcases ──────────────────────────────────────────────────

    // T-testcases is now supported: ok=true with testcase_body populated.
    // The detailed assertions live in the testcase_body_* tests below.
    #[test]
    fn phase2_t_testcases_now_supported() {
        let spec = scalar_ok("T\n\na s");
        assert!(spec.ok, "T-testcases should now parse as ok=true");
        assert!(
            !spec.testcase_body.is_empty(),
            "testcase_body should be populated"
        );
    }

    // ── Phase 1: subscripted scalars (A_x A_y) — now supported ──────────────

    #[test]
    fn subscripted_scalars_ok() {
        // A_x A_y with alphabetic subscripts → scalars ax, ay (TASK-016)
        let spec = scalar_ok("A_x A_y\n");
        assert!(spec.ok, "expected ok=true for subscripted scalars A_x A_y");
        assert!(spec.vars.iter().any(|v| v.name == "ax"), "expected var ax");
        assert!(spec.vars.iter().any(|v| v.name == "ay"), "expected var ay");
    }

    // ── empty string ─────────────────────────────────────────────────────────

    #[test]
    fn empty_raw_returns_not_ok() {
        let spec = scalar_ok("");
        assert!(!spec.ok, "expected ok=false for empty raw");
        assert_eq!(spec.raw, "");
    }

    // ── type inference from constraints ───────────────────────────────────────

    /// Constraints mention "S は英小文字からなる文字列" → S inferred as Str
    #[test]
    fn type_inference_string_from_constraints() {
        let spec = with_constraints(
            "N\nS\n",
            "S は英小文字からなる文字列\n1 \\leq N \\leq 10^5\n",
        );
        // N → Int, S → Str
        let n_var = spec
            .vars
            .iter()
            .find(|v| v.name == "n")
            .expect("var n not found");
        let s_var = spec
            .vars
            .iter()
            .find(|v| v.name == "s")
            .expect("var s not found");
        assert_eq!(n_var.var_type, VarType::Int);
        assert_eq!(s_var.var_type, VarType::Str);
    }

    // ── is_size field ─────────────────────────────────────────────────────────

    /// "N M\n" — neither N nor M is used as a size, so both have is_size=false
    #[test]
    fn scalars_only_is_size_false() {
        let spec = scalar_ok("N M\n");
        assert!(spec.ok);
        assert_eq!(spec.vars.len(), 2);
        let n = &spec.vars[0];
        assert_eq!(n.name, "n");
        assert!(!n.is_size, "n should have is_size=false");
        let m = &spec.vars[1];
        assert_eq!(m.name, "m");
        assert!(!m.is_size, "m should have is_size=false");
    }

    /// "N\nA_1 A_2 \\ldots A_N\n" — N is the array size so is_size=true; a is not
    #[test]
    fn array_size_var_is_size_true() {
        let spec = scalar_ok("N\nA_1 A_2 \\ldots A_N\n");
        assert!(spec.ok);
        assert_eq!(spec.vars.len(), 2);
        let n = spec
            .vars
            .iter()
            .find(|v| v.name == "n")
            .expect("n not found");
        assert!(n.is_size, "n should have is_size=true (it is A's size)");
        let a = spec
            .vars
            .iter()
            .find(|v| v.name == "a")
            .expect("a not found");
        assert!(!a.is_size, "a should have is_size=false");
    }

    /// "Q\nt_1 k_1\nt_2 k_2\n\\vdots\nt_Q k_Q\n" — Q is the LoopBegin end, so is_size=true
    #[test]
    fn multi_var_loop_returns_ok_true_with_loop_ops() {
        // Phase 2: multi-var loops produce ok=true; LoopBegin ops are kept for template codegen.
        // Q is is_size=true because it is the loop bound.
        let spec = scalar_ok("Q\nt_1 k_1\nt_2 k_2\n\\vdots\nt_Q k_Q\n");
        assert!(spec.ok, "multi-var loop should produce ok=true in Phase 2");
        assert!(spec.ops.iter().any(|o| o.tag == OpTag::LoopBegin));
        let q = spec
            .vars
            .iter()
            .find(|v| v.name == "q")
            .expect("var q not found");
        assert!(q.is_size, "q should be is_size=true (loop bound)");
    }

    /// "N\nA_1 \\ldots A_N\nB_1 \\ldots B_N\n" — N is size of both A and B, so is_size=true
    #[test]
    fn multiple_arrays_size_var_is_size_true() {
        let spec = scalar_ok("N\nA_1 \\ldots A_N\nB_1 \\ldots B_N\n");
        assert!(spec.ok);
        let n = spec
            .vars
            .iter()
            .find(|v| v.name == "n")
            .expect("n not found");
        assert!(
            n.is_size,
            "n should have is_size=true (size of both A and B)"
        );
        let a = spec
            .vars
            .iter()
            .find(|v| v.name == "a")
            .expect("a not found");
        assert!(!a.is_size, "a should have is_size=false");
        let b = spec
            .vars
            .iter()
            .find(|v| v.name == "b")
            .expect("b not found");
        assert!(!b.is_size, "b should have is_size=false");
    }

    // ── case-collision: N and n on same line ──────────────────────────────────

    // ── TASK-013: single-variable vdots loop flattening ───────────────────────

    /// "N L\nS_1\n\\vdots\nS_N\n" — single-var vdots loop should flatten to Vec<String>
    /// Expected: ok=true, vars=[n(is_size=true), l, s(dim=1,size="n")],
    /// ops=[ReadLine([n,l]), ReadLine([s:dim=1,size="n"])]
    #[test]
    fn single_var_loop_flattened_to_array() {
        let spec = scalar_ok("N L\nS_1\n\\vdots\nS_N\n");
        assert!(
            spec.ok,
            "expected ok=true: single-var vdots loop should flatten to array"
        );

        assert_eq!(spec.vars.len(), 3, "expected vars: n, l, s");

        let n = spec
            .vars
            .iter()
            .find(|v| v.name == "n")
            .expect("var n not found");
        assert_eq!(n.dim, 0);
        assert!(n.is_size, "n should be is_size=true (loop bound)");

        let l = spec
            .vars
            .iter()
            .find(|v| v.name == "l")
            .expect("var l not found");
        assert_eq!(l.dim, 0);
        assert!(!l.is_size, "l should be is_size=false");

        let s = spec
            .vars
            .iter()
            .find(|v| v.name == "s")
            .expect("var s not found");
        assert_eq!(s.dim, 1, "s should be dim=1 (array)");
        assert_eq!(s.size, vec!["n".to_string()], "s size should be n");

        assert_eq!(spec.ops.len(), 2, "expected 2 ops");
        assert_eq!(spec.ops[0].tag, OpTag::ReadLine);
        assert_eq!(spec.ops[0].vars.len(), 2);
        assert_eq!(spec.ops[0].vars[0].name, "n");
        assert_eq!(spec.ops[0].vars[1].name, "l");

        assert_eq!(spec.ops[1].tag, OpTag::ReadLine);
        assert_eq!(spec.ops[1].vars.len(), 1);
        let sv = &spec.ops[1].vars[0];
        assert_eq!(sv.name, "s");
        assert_eq!(sv.dim, 1);
        assert_eq!(sv.size, Some("n".to_string()));
    }

    /// Multi-var vdots loop (2 vars per row) cannot be flattened but is kept for template codegen
    #[test]
    fn multi_var_loop_kept_for_template_codegen() {
        // Phase 2: multi-var loops produce ok=true with LoopBegin ops retained so the
        // template can emit Vec::new() + for-loop + push() code.
        let spec = scalar_ok("S\nQ\nt_1 k_1\n\\hspace{0.4cm}\\vdots\nt_Q k_Q\n");
        assert!(
            spec.ok,
            "expected ok=true: multi-var loops are handled by template loop codegen"
        );
        assert!(
            spec.ops.iter().any(|o| o.tag == OpTag::LoopBegin),
            "expected LoopBegin op"
        );
    }

    /// "H W\nS_1\n:\nS_H\n" — standalone `:` normalizes to vdots, then single-var loop flattens
    /// Expected: ok=true, vars=[h(is_size=true), w, s(dim=1,size="h")],
    /// ops=[ReadLine([h,w]), ReadLine([s:dim=1,size="h"])]
    #[test]
    fn colon_vdots_single_var_flattened() {
        let spec = scalar_ok("H W\nS_1\n:\nS_H\n");
        assert!(
            spec.ok,
            "expected ok=true: colon normalizes to vdots and single-var loop flattens"
        );

        assert_eq!(spec.vars.len(), 3, "expected vars: h, w, s");

        let h = spec
            .vars
            .iter()
            .find(|v| v.name == "h")
            .expect("var h not found");
        assert_eq!(h.dim, 0);
        assert!(h.is_size, "h should be is_size=true (loop bound)");

        let w = spec
            .vars
            .iter()
            .find(|v| v.name == "w")
            .expect("var w not found");
        assert_eq!(w.dim, 0);
        assert!(!w.is_size, "w should be is_size=false");

        let s = spec
            .vars
            .iter()
            .find(|v| v.name == "s")
            .expect("var s not found");
        assert_eq!(s.dim, 1, "s should be dim=1 (array)");
        assert_eq!(s.size, vec!["h".to_string()], "s size should be h");

        assert_eq!(spec.ops.len(), 2, "expected 2 ops");
        assert_eq!(spec.ops[0].tag, OpTag::ReadLine);
        assert_eq!(spec.ops[0].vars.len(), 2);
        assert_eq!(spec.ops[0].vars[0].name, "h");
        assert_eq!(spec.ops[0].vars[1].name, "w");

        assert_eq!(spec.ops[1].tag, OpTag::ReadLine);
        assert_eq!(spec.ops[1].vars.len(), 1);
        let sv = &spec.ops[1].vars[0];
        assert_eq!(sv.name, "s");
        assert_eq!(sv.dim, 1);
        assert_eq!(sv.size, Some("h".to_string()));
    }

    // ── case-collision: N and n on same line ──────────────────────────────────

    /// When both uppercase and lowercase of the same letter appear,
    /// the names must not collapse to the same identifier.
    #[test]
    fn case_collision_uppercase_preserved() {
        let spec = scalar_ok("N n\n");
        assert_eq!(spec.vars.len(), 2);
        // Both names must be distinct; the uppercase one keeps its math token
        let names: Vec<&str> = spec.vars.iter().map(|v| v.name.as_str()).collect();
        assert!(
            names.contains(&"N") && names.contains(&"n"),
            "expected both N and n preserved, got {:?}",
            names
        );

        // math tokens must match original case
        let n_upper = spec
            .vars
            .iter()
            .find(|v| v.math == "N")
            .expect("N not found");
        let n_lower = spec
            .vars
            .iter()
            .find(|v| v.math == "n")
            .expect("n not found");
        // when collision occurs, uppercase name is preserved as-is ("N")
        assert_eq!(n_upper.name, "N");
        assert_eq!(n_lower.name, "n");
    }

    /// Phase 2: multi-var vdots loop should return ok=true and keep LoopBegin/LoopEnd ops
    #[test]
    fn multi_var_loop_produces_ok_true_with_loop_ops() {
        let spec = scalar_ok("Q\nt_1 k_1\n\\vdots\nt_Q k_Q\n");
        assert!(
            spec.ok,
            "expected ok=true: multi-var vdots loop should be handled in Phase 2"
        );
        assert_eq!(spec.vars.len(), 3, "expected vars: q, t, k");

        let q = spec
            .vars
            .iter()
            .find(|v| v.name == "q")
            .expect("var q not found");
        assert_eq!(q.dim, 0);
        assert!(q.is_size, "q should be is_size=true (loop bound)");

        let t = spec
            .vars
            .iter()
            .find(|v| v.name == "t")
            .expect("var t not found");
        assert_eq!(t.dim, 1, "t should be dim=1 (array)");

        let k = spec
            .vars
            .iter()
            .find(|v| v.name == "k")
            .expect("var k not found");
        assert_eq!(k.dim, 1, "k should be dim=1 (array)");

        assert!(
            spec.ops.iter().any(|o| o.tag == OpTag::LoopBegin),
            "expected a LoopBegin op in ops"
        );
        assert!(
            spec.ops.iter().any(|o| o.tag == OpTag::LoopEnd),
            "expected a LoopEnd op in ops"
        );
    }

    // ── \dots vertical separator + subscripted type inference ────────────────

    /// "N L\nS_1\nS_2\n\dots\nS_N\n" — \dots on its own line treated as vdots,
    /// single-var loop flattens to a ReadLine(dim=1) op (abc246-f style)
    #[test]
    fn dots_on_own_line_treated_as_vdots() {
        let spec = parse("N L\nS_1\nS_2\n\\dots\nS_N\n", "");
        assert!(
            spec.ok,
            "expected ok=true: \\dots should be treated as vdots"
        );
        let s_var = spec.vars.iter().find(|v| v.name == "s").expect("var s");
        assert_eq!(s_var.dim, 1, "s should be 1D array");
        assert!(
            !spec.ops.iter().any(|o| o.tag == OpTag::LoopBegin),
            "loop should be flattened, not kept as LoopBegin ops"
        );
        let s_op = spec
            .ops
            .iter()
            .find(|o| o.vars.iter().any(|v| v.name == "s"))
            .expect("ReadLine op for s");
        assert_eq!(
            s_op.vars[0].size.as_deref(),
            Some("n"),
            "flattened op should have size=n"
        );
    }

    /// Constraint "S_i は abcdefghijklmnopqrstuvwxyz の部分列" → Str type
    #[test]
    fn type_inference_subsequence_constraint_is_str() {
        let spec = with_constraints(
            "N\nS_1\n\\vdots\nS_N\n",
            "S_i は abcdefghijklmnopqrstuvwxyz の(連続とは限らない)空でない部分列\n1 \\leq N \\leq 18\n",
        );
        let s_var = spec.vars.iter().find(|v| v.name == "s").expect("var s");
        assert_eq!(
            s_var.var_type,
            VarType::Str,
            "s should be Str from 部分列 constraint"
        );
    }

    // ── TASK-015: 文字グリッド (2D添字) ──────────────────────────────────────────

    /// abc151-d style: "H W\nS_{11}...S_{1W}\n:\nS_{H1}...S_{HW}\n"
    /// → ok=true, s: Vec<String>, flattened to [String; h]
    #[test]
    fn grid_row_2d_subscript_numeric_flattened() {
        let spec = parse("H W\nS_{11}...S_{1W}\n:\nS_{H1}...S_{HW}\n", "");
        assert!(spec.ok, "expected ok=true for 2D grid row");
        let s_var = spec.vars.iter().find(|v| v.name == "s").expect("var s");
        assert_eq!(s_var.dim, 1, "s should be Vec (dim=1)");
        assert_eq!(s_var.var_type, VarType::Str, "s should be Str");
        assert!(
            !spec.ops.iter().any(|o| o.tag == OpTag::LoopBegin),
            "loop should be flattened"
        );
        let s_op = spec
            .ops
            .iter()
            .find(|o| o.vars.iter().any(|v| v.name == "s"))
            .expect("ReadLine op for s");
        assert_eq!(
            s_op.vars[0].size.as_deref(),
            Some("h"),
            "flattened op should have size=h"
        );
    }

    /// Loop variable form: "H W\nS_{i1}...S_{iW}\n\\vdots\nS_{H1}...S_{HW}\n"
    #[test]
    fn grid_row_loop_var_form_flattened() {
        let spec = parse("H W\nS_{i1}...S_{iW}\n\\vdots\nS_{H1}...S_{HW}\n", "");
        assert!(spec.ok, "expected ok=true for loop-var grid row");
        let s_var = spec.vars.iter().find(|v| v.name == "s").expect("var s");
        assert_eq!(s_var.dim, 1);
        assert_eq!(s_var.var_type, VarType::Str);
        assert!(!spec.ops.iter().any(|o| o.tag == OpTag::LoopBegin));
    }

    /// Comma-separated 2D subscript: "H W\nS_{1,1}...S_{1,W}\n:\nS_{H,1}...S_{H,W}\n"
    #[test]
    fn grid_row_comma_subscript_flattened() {
        let spec = parse("H W\nS_{1,1}...S_{1,W}\n:\nS_{H,1}...S_{H,W}\n", "");
        assert!(spec.ok, "expected ok=true for comma-separated grid row");
        let s_var = spec.vars.iter().find(|v| v.name == "s").expect("var s");
        assert_eq!(s_var.dim, 1);
        assert_eq!(s_var.var_type, VarType::Str);
        assert!(!spec.ops.iter().any(|o| o.tag == OpTag::LoopBegin));
    }

    /// Regression: A_{1}...A_{10} must parse as Array1D, not GridRow.
    /// Multi-digit purely-numeric subscripts like {10} are 1D indices, not 2D.
    #[test]
    fn array1d_multi_digit_subscript_not_grid_row() {
        let spec = parse("N\nA_{1}...A_{10}\n", "");
        assert!(
            spec.ok,
            "expected ok=true for 1D array with multi-digit subscript"
        );
        let a_var = spec.vars.iter().find(|v| v.name == "a").expect("var a");
        assert_eq!(a_var.dim, 1, "a should be Vec (dim=1)");
        // Should not be misclassified as Str
        assert_ne!(
            a_var.var_type,
            VarType::Str,
            "purely-numeric multi-digit subscript should not trigger GridRow detection"
        );
    }

    /// abc453-D style: each row has two leading elements before cdots.
    /// "H W\nS_{1,1} S_{1,2} \\ldots S_{1,W}\nS_{2,1} S_{2,2} \\ldots S_{2,W}\n\\vdots\nS_{H,1} S_{H,2} \\ldots S_{H,W}\n"
    /// → ok=true, s: dim=1, VarType::Str, flattened with size="h" (no LoopBegin)
    #[test]
    fn abc453d_grid_row_multi_prefix() {
        let raw = "H W\nS_{1,1} S_{1,2} \\ldots S_{1,W}\nS_{2,1} S_{2,2} \\ldots S_{2,W}\n\\vdots\nS_{H,1} S_{H,2} \\ldots S_{H,W}\n";
        let spec = parse(raw, "");
        assert!(
            spec.ok,
            "expected ok=true for abc453-D style multi-prefix grid row"
        );
        let s_var = spec.vars.iter().find(|v| v.name == "s").expect("var s");
        assert_eq!(s_var.dim, 1, "s should be Vec (dim=1)");
        assert_eq!(s_var.var_type, VarType::Str, "s should be Str");
        assert!(
            !spec.ops.iter().any(|o| o.tag == OpTag::LoopBegin),
            "ops should be flattened (no LoopBegin)"
        );
        let s_op = spec
            .ops
            .iter()
            .find(|o| o.vars.iter().any(|v| v.name == "s"))
            .expect("ReadLine op for s");
        assert_eq!(
            s_op.vars[0].size.as_deref(),
            Some("h"),
            "flattened op should have size=h"
        );
    }

    /// abc450-C style: no-space adjacent prefix elements before cdots should be a GridRow.
    /// "H W\nS_{1,1}S_{1,2}\dots S_{1,W}\n\vdots\nS_{H,1}S_{H,2}\dots S_{H,W}\n"
    /// → ok=true, s: dim=1, VarType::Str, flattened with size="h" (no LoopBegin)
    #[test]
    fn abc450c_grid_row_no_space() {
        let raw = "H W\nS_{1,1}S_{1,2}\\dots S_{1,W}\n\\vdots\nS_{H,1}S_{H,2}\\dots S_{H,W}\n";
        let spec = parse(raw, "");
        assert!(
            spec.ok,
            "expected ok=true for abc450-C style no-space multi-prefix grid row"
        );
        let h_var = spec.vars.iter().find(|v| v.name == "h").expect("var h");
        let w_var = spec.vars.iter().find(|v| v.name == "w").expect("var w");
        assert_eq!(h_var.dim, 0, "h should be scalar (dim=0)");
        assert_eq!(w_var.dim, 0, "w should be scalar (dim=0)");
        let s_var = spec.vars.iter().find(|v| v.name == "s").expect("var s");
        assert_eq!(s_var.dim, 1, "s should be Vec (dim=1)");
        assert_eq!(s_var.var_type, VarType::Str, "s should be Str");
        assert!(
            !spec.ops.iter().any(|o| o.tag == OpTag::LoopBegin),
            "ops should be flattened (no LoopBegin)"
        );
        let s_op = spec
            .ops
            .iter()
            .find(|o| o.vars.iter().any(|v| v.name == "s"))
            .expect("ReadLine op for s");
        assert_eq!(
            s_op.vars[0].size.as_deref(),
            Some("h"),
            "flattened op should have size=h"
        );
    }

    // ── TASK-016: 非数値添字スカラー ───────────────────────────────────────────────

    /// Single line with alphabetic subscripts: "A_x A_y" → scalars ax, ay
    #[test]
    fn alpha_subscript_scalars_single_line() {
        let spec = parse("A_x A_y\n", "");
        assert!(spec.ok, "expected ok=true for A_x A_y");
        let ax = spec.vars.iter().find(|v| v.name == "ax").expect("var ax");
        let ay = spec.vars.iter().find(|v| v.name == "ay").expect("var ay");
        assert_eq!(ax.dim, 0, "ax should be scalar (dim=0)");
        assert_eq!(ay.dim, 0, "ay should be scalar (dim=0)");
    }

    /// Two lines with numeric subscripts, no vdots: "r_1 c_1\nr_2 c_2" → scalars r1,c1,r2,c2
    #[test]
    fn numeric_subscript_scalars_no_vdots() {
        let spec = parse("r_1 c_1\nr_2 c_2\n", "");
        assert!(
            spec.ok,
            "expected ok=true for r_1 c_1 / r_2 c_2 without vdots"
        );
        for name in &["r1", "c1", "r2", "c2"] {
            let v = spec
                .vars
                .iter()
                .find(|v| v.name == *name)
                .unwrap_or_else(|| panic!("var {name} not found"));
            assert_eq!(v.dim, 0, "{name} should be scalar (dim=0)");
        }
    }

    /// abc246-E style: "N\nA_x A_y\nB_x B_y\nS_1\n\vdots\nS_N\n"
    /// → ok=true, scalars ax,ay,bx,by; array s
    #[test]
    fn abc246e_style_alpha_subscript_with_array() {
        let spec = parse("N\nA_x A_y\nB_x B_y\nS_1\n\\vdots\nS_N\n", "");
        assert!(spec.ok, "expected ok=true for abc246-E style input");
        for name in &["ax", "ay", "bx", "by"] {
            let v = spec
                .vars
                .iter()
                .find(|v| v.name == *name)
                .unwrap_or_else(|| panic!("var {name} not found"));
            assert_eq!(v.dim, 0, "{name} should be scalar (dim=0)");
        }
        let s = spec.vars.iter().find(|v| v.name == "s").expect("var s");
        assert_eq!(s.dim, 1, "s should be Vec (dim=1)");
    }

    /// abc176-D style: "H W\nr_1 c_1\nr_2 c_2\nS_{H1}...S_{HW}\n:\nS_{H1}...S_{HW}\n"
    /// → ok=true, scalars h,w,r1,c1,r2,c2; array s (Vec<String>)
    #[test]
    fn abc176d_style_numeric_subscript_scalars_with_grid() {
        let spec = parse(
            "H W\nr_1 c_1\nr_2 c_2\nS_{i1}...S_{iW}\n\\vdots\nS_{H1}...S_{HW}\n",
            "",
        );
        assert!(spec.ok, "expected ok=true for abc176-D style input");
        for name in &["r1", "c1", "r2", "c2"] {
            let v = spec
                .vars
                .iter()
                .find(|v| v.name == *name)
                .unwrap_or_else(|| panic!("var {name} not found"));
            assert_eq!(v.dim, 0, "{name} should be scalar (dim=0)");
        }
        let s = spec.vars.iter().find(|v| v.name == "s").expect("var s");
        assert_eq!(s.dim, 1, "s should be Vec<String> (dim=1)");
        assert_eq!(s.var_type, VarType::Str, "s should be Str");
    }

    // ── TASK-019: query sub-block parsing ─────────────────────────────────────

    /// abc241-D style: numbered sub-blocks → query_types populated
    #[test]
    fn text_query_multi_block_numbered() {
        let spec = with_constraints(
            "N Q\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n\n1 x\n\n2 x k\n\n3 l r\n",
            "1 \\leq N, Q \\leq 2 \\times 10^5\n1 \\leq x \\leq 10^9\n1 \\leq k \\leq 10^9\n1 \\leq l \\leq r \\leq 10^9",
        );
        assert!(spec.ok, "expected ok=true");
        assert_eq!(spec.query_types.len(), 3, "expected 3 query types");

        let qt1 = &spec.query_types[0];
        assert_eq!(qt1.type_id, "1");
        assert!(qt1.ok);
        assert_eq!(qt1.vars.len(), 1);
        assert_eq!(qt1.vars[0].name, "x");

        let qt2 = &spec.query_types[1];
        assert_eq!(qt2.type_id, "2");
        assert!(qt2.ok);
        assert_eq!(qt2.vars.len(), 2);
        assert_eq!(qt2.vars[0].name, "x");
        assert_eq!(qt2.vars[1].name, "k");

        let qt3 = &spec.query_types[2];
        assert_eq!(qt3.type_id, "3");
        assert!(qt3.ok);
        assert_eq!(qt3.vars.len(), 2);
        assert_eq!(qt3.vars[0].name, "l");
        assert_eq!(qt3.vars[1].name, "r");
    }

    /// abc334-D style: sub-block starts with ident (not digit) → query_body populated
    #[test]
    fn query_body_single_var() {
        let spec =
            scalar_ok("N Q\nR_1 \\ldots R_N\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n\nX\n");
        assert!(spec.ok, "expected ok=true");
        assert_eq!(
            spec.query_types.len(),
            0,
            "non-numeric sub-block should not create query_types"
        );
        assert_eq!(
            spec.query_body.len(),
            1,
            "non-numeric sub-block should populate query_body"
        );
        assert_eq!(spec.query_body[0].name, "x");
        assert_eq!(spec.query_body[0].dim, 0);
    }

    /// No sub-blocks → query_types and query_body both empty
    #[test]
    fn query_no_subblocks_empty_query_types() {
        let spec = scalar_ok("Q\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n");
        assert!(spec.ok, "expected ok=true");
        assert_eq!(
            spec.query_types.len(),
            0,
            "no sub-blocks means query_types is empty"
        );
        assert_eq!(
            spec.query_body.len(),
            0,
            "no sub-blocks means query_body is empty"
        );
    }

    /// Type inference applies to query sub-block vars
    #[test]
    fn query_subblock_type_inference() {
        let spec = with_constraints(
            "N Q\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n\n1 x\n",
            "1 \\leq x \\leq 10^9",
        );
        assert!(spec.ok);
        assert_eq!(spec.query_types.len(), 1);
        let qt = &spec.query_types[0];
        assert!(qt.ok);
        assert_eq!(qt.vars[0].name, "x");
        assert_eq!(qt.vars[0].var_type, VarType::Int);
    }

    /// Multi-var non-numeric sub-block → query_body has all vars
    #[test]
    fn query_body_multi_var() {
        let spec = scalar_ok("Q\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n\nL R\n");
        assert!(spec.ok);
        assert_eq!(spec.query_types.len(), 0);
        assert_eq!(spec.query_body.len(), 2);
        assert_eq!(spec.query_body[0].name, "l");
        assert_eq!(spec.query_body[1].name, "r");
    }

    /// When query_types is non-empty, query_body is always empty
    #[test]
    fn query_body_ignored_when_query_types_present() {
        let spec = scalar_ok("Q\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n\n1 x\n\nX\n");
        assert!(spec.ok);
        assert_eq!(spec.query_types.len(), 1);
        assert_eq!(
            spec.query_body.len(),
            0,
            "query_body must be empty when query_types is non-empty"
        );
    }

    // ── plain-text query marker (query_i pattern) ────────────────────────────

    /// abc212-D style: plain-text "query_i" with ":" vdots separator
    #[test]
    fn plain_query_marker_abc212d() {
        // Q\nquery_1\nquery_2\n:\nquery_Q\n\n1 X\n\n2 X\n\n3\n
        let spec = scalar_ok("Q\nquery_1\nquery_2\n:\nquery_Q\n\n1 X\n\n2 X\n\n3\n");
        assert!(spec.ok, "expected ok=true for plain query_i marker");
        assert_eq!(spec.query_types.len(), 3, "expected 3 query types");
        assert_eq!(spec.query_types[0].type_id, "1");
        assert!(spec.query_types[0].ok);
        assert_eq!(spec.query_types[0].vars.len(), 1);
        assert_eq!(spec.query_types[0].vars[0].name, "x");
        assert_eq!(spec.query_types[1].type_id, "2");
        assert!(spec.query_types[1].ok);
        assert_eq!(spec.query_types[1].vars.len(), 1);
        assert_eq!(spec.query_types[2].type_id, "3");
        assert!(spec.query_types[2].ok);
        assert_eq!(spec.query_types[2].vars.len(), 0);
    }

    /// Short ident q_i (not "query") must NOT trigger query marker detection
    #[test]
    fn plain_query_marker_not_triggered_for_short_ident() {
        let spec = scalar_ok("Q\nq_1\n:\nq_Q\n\n1 x\n");
        assert!(
            !spec.ok,
            "expected ok=false: q_i is too short to be a query marker"
        );
    }

    // A variable literally named `query` (no subscript) must not be misidentified as a
    // query loop marker — it should parse as a normal scalar variable.
    #[test]
    fn standalone_query_variable_parses_as_scalar() {
        // Input format: single line with one variable named "query"
        let spec = parse("query", "1 \u{2264} query \u{2264} 10^9");
        assert!(
            spec.ok,
            "expected ok=true: 'query' without subscript is a scalar var"
        );
        assert_eq!(spec.vars.len(), 1);
        assert_eq!(spec.vars[0].name, "query");
        assert!(spec.query_types.is_empty());
        assert!(spec.query_body.is_empty());
    }

    // ── T-testcases ────────────────────────────────────────────────────────────────

    /// abc238-D style: block 0 = single scalar T, block 1 = `a s`
    /// Expected: ok=true, vars=[t(is_size:true)], testcase_body=[a, s]
    #[test]
    fn testcase_body_abc238d() {
        let spec = parse("T\n\na s", "1 ≤ T ≤ 100\n1 ≤ a ≤ 10^9\ns is a string");
        assert!(spec.ok, "expected ok=true for T-testcases format");
        assert_eq!(spec.vars.len(), 1);
        assert_eq!(spec.vars[0].name, "t");
        assert!(spec.vars[0].is_size, "T var must be is_size:true");
        assert_eq!(spec.testcase_body.len(), 2);
        assert_eq!(spec.testcase_body[0].name, "a");
        assert_eq!(spec.testcase_body[1].name, "s");
        assert_eq!(
            spec.testcase_body[1].var_type,
            VarType::Str,
            "s should be inferred as VarType::Str"
        );
        assert!(spec.query_types.is_empty());
        assert!(spec.query_body.is_empty());
    }

    /// Single-block input: testcase_body must remain empty
    #[test]
    fn testcase_body_empty_for_single_block() {
        let spec = scalar_ok("N\nA_1 \\ldots A_N");
        assert!(spec.ok);
        assert!(spec.testcase_body.is_empty());
    }

    /// T-testcases with a 1D array in block 1: ok=true but testcase_body is empty
    /// (falls back to a TODO-only stub in the template).
    #[test]
    fn testcase_body_empty_when_block1_not_scalar() {
        // Block 1 has a 1D array pattern (not plain scalars)
        let spec = parse("T\n\nA_1 \\ldots A_N", "");
        // ok=true: block 0 is a single ident and block 1 is not digit-start
        assert!(
            spec.ok,
            "T-testcases with non-scalar block1 should still be ok=true"
        );
        // testcase_body empty: 1D cdots array is not a flat scalar list
        assert!(spec.testcase_body.is_empty());
        // block-0 var is still is_size=true even when testcase_body is empty
        assert!(spec.vars[0].is_size, "loop-count var must be is_size:true");
    }

    // ── Fixed-size 1D (cdots) ──────────────────────────────────────────────────

    /// A_1 \ldots A_3 — cdots with numeric end subscript → Array1D(size="3")
    #[test]
    fn array1d_fixed_cdots() {
        let spec = scalar_ok("A_1 \\ldots A_3");
        assert_eq!(spec.vars.len(), 1);
        assert_eq!(spec.vars[0].name, "a");
        assert_eq!(spec.vars[0].dim, 1);
        assert_eq!(spec.vars[0].size, vec!["3"]);
        assert!(!spec.vars[0].is_size, "literal size has no is_size var");
        // ops: single ReadLine with dim=1, size="3"
        assert_eq!(spec.ops.len(), 1);
        assert_eq!(spec.ops[0].vars[0].name, "a");
        assert_eq!(spec.ops[0].vars[0].dim, 1);
        assert_eq!(spec.ops[0].vars[0].size.as_deref(), Some("3"));
    }

    // ── Fixed-size 1D (no cdots) ───────────────────────────────────────────────

    /// A_1 A_2 A_3 — same name, sequential numeric subscripts, no cdots → Array1D(size="3")
    #[test]
    fn array1d_fixed_no_cdots() {
        let spec = scalar_ok("A_1 A_2 A_3");
        assert!(spec.ok, "expected ok=true");
        assert_eq!(spec.vars.len(), 1);
        assert_eq!(spec.vars[0].name, "a");
        assert_eq!(spec.vars[0].dim, 1);
        assert_eq!(spec.vars[0].size, vec!["3"]);
        assert_eq!(spec.ops[0].vars[0].size.as_deref(), Some("3"));
    }

    /// A_1 A_2 — 2 elements is the minimum
    #[test]
    fn array1d_fixed_no_cdots_two_elements() {
        let spec = scalar_ok("A_1 A_2");
        assert!(spec.ok);
        assert_eq!(spec.vars[0].dim, 1);
        assert_eq!(spec.vars[0].size, vec!["2"]);
    }

    /// A_1 alone — not enough for an array; treated as scalar
    #[test]
    fn array1d_fixed_no_cdots_single_element_is_scalar() {
        let spec = scalar_ok("A_1");
        assert_eq!(spec.vars[0].dim, 0, "single element should be scalar");
    }

    /// A_1 B_1 — different base names → separate scalars, not array
    #[test]
    fn array1d_fixed_no_cdots_different_names_are_scalars() {
        let spec = scalar_ok("A_1 B_1");
        // Two distinct scalars
        assert_eq!(spec.vars.len(), 2);
        assert!(spec.vars.iter().all(|v| v.dim == 0));
    }

    /// A_1 A_3 — non-sequential subscripts → not an array
    #[test]
    fn array1d_fixed_no_cdots_nonsequential_are_scalars() {
        let spec = scalar_ok("A_1 A_3");
        // Falls through to scalars (non-sequential). Seeds are "A1" and "A3" →
        // distinct normalized names "a1" and "a3".
        assert!(spec.vars.iter().all(|v| v.dim == 0));
    }

    // ── Fixed 2D grid (comma subscripts) ──────────────────────────────────────

    /// A_{1,1} ... A_{1,6} × 3 rows → dim=2, size=["6","3"]
    #[test]
    fn array2d_fixed_grid_abc456b() {
        let input = "A_{1,1} A_{1,2} A_{1,3} A_{1,4} A_{1,5} A_{1,6}\nA_{2,1} A_{2,2} A_{2,3} A_{2,4} A_{2,5} A_{2,6}\nA_{3,1} A_{3,2} A_{3,3} A_{3,4} A_{3,5} A_{3,6}";
        let spec = scalar_ok(input);
        assert!(spec.ok, "expected ok=true for 2D fixed grid");
        assert_eq!(spec.vars.len(), 1);
        assert_eq!(spec.vars[0].name, "a");
        assert_eq!(spec.vars[0].dim, 2);
        assert_eq!(spec.vars[0].size, vec!["6", "3"]);
        assert_eq!(spec.ops.len(), 1);
        assert_eq!(spec.ops[0].vars[0].dim, 2);
    }

    /// 2×2 minimal grid
    #[test]
    fn array2d_fixed_grid_minimal_2x2() {
        let input = "A_{1,1} A_{1,2}\nA_{2,1} A_{2,2}";
        let spec = scalar_ok(input);
        assert!(spec.ok);
        assert_eq!(spec.vars[0].dim, 2);
        assert_eq!(spec.vars[0].size, vec!["2", "2"]);
    }

    /// Single Array2DRow (1 row) — not enough rows to form a 2D grid
    #[test]
    fn array2d_single_row_not_grouped() {
        // One row of comma-subscript elements cannot form a 2D grid (rows < 2 → Err → ok=false)
        let spec = parse("A_{1,1} A_{1,2} A_{1,3}", "");
        assert!(!spec.ok);
    }

    /// Row subscripts not starting from 1 → no 2D grid grouping → ok=false
    #[test]
    fn array2d_row_not_starting_from_1() {
        let input = "A_{2,1} A_{2,2}\nA_{3,1} A_{3,2}";
        let spec = parse(input, "");
        assert!(!spec.ok);
    }

    /// Mismatched col counts → ok=false
    #[test]
    fn array2d_mismatched_col_counts() {
        let input = "A_{1,1} A_{1,2} A_{1,3}\nA_{2,1} A_{2,2}";
        let spec = parse(input, "");
        assert!(!spec.ok);
    }

    /// Adjacent array1d_no_cdots elements (no Space) → ok=false
    #[test]
    fn array1d_no_cdots_adjacent_no_space() {
        // A_1A_2 has no Space token between them — must be ok=false
        let spec = parse("A_1A_2", "");
        assert!(!spec.ok);
    }

    /// Unclosed brace subscript → ok=false
    #[test]
    fn subscript_unclosed_brace() {
        // {1,2 without closing } must not produce a valid parse
        let spec = parse("A_{1,2", "");
        assert!(!spec.ok);
    }

    /// Trailing comma in brace subscript → ok=false
    #[test]
    fn subscript_trailing_comma() {
        let spec = parse("A_{1,}", "");
        assert!(!spec.ok);
    }

    /// Leading comma in brace subscript → ok=false
    #[test]
    fn subscript_leading_comma() {
        let spec = parse("A_{,1}", "");
        assert!(!spec.ok);
    }

    // ── iteration_vars / iteration_ops ────────────────────────────────────────

    /// abc456_f style: query-marker block[0] + complex block[1] (1D array) →
    /// iteration_vars non-empty, query_body empty.
    #[test]
    fn iteration_vars_simple_array_body() {
        // block[0]: T + case loop marker
        // block[1]: N K \n A_1 A_2 \ldots A_N
        let raw = "T\n\\mathrm{case}_1\n\\vdots\n\\mathrm{case}_T\n\nN K\nA_1 A_2 \\ldots A_N";
        let spec = parse(raw, "");
        assert!(spec.ok, "expected ok=true for abc456_f style");
        assert!(
            spec.query_body.is_empty(),
            "query_body must be empty when iteration_ops is populated"
        );
        assert!(
            !spec.iteration_vars.is_empty(),
            "iteration_vars should be non-empty for complex body"
        );
        assert!(
            !spec.iteration_ops.is_empty(),
            "iteration_ops should be non-empty for complex body"
        );
        // iteration_vars: n (is_size), k, a (dim=1, size=["n"])
        let n = spec.iteration_vars.iter().find(|v| v.name == "n");
        let k = spec.iteration_vars.iter().find(|v| v.name == "k");
        let a = spec.iteration_vars.iter().find(|v| v.name == "a");
        assert!(n.is_some(), "expected var n in iteration_vars");
        assert!(n.unwrap().is_size, "n should be is_size=true");
        assert!(k.is_some(), "expected var k in iteration_vars");
        assert!(a.is_some(), "expected var a in iteration_vars");
        let a = a.unwrap();
        assert_eq!(a.dim, 1, "a should be dim=1");
        assert_eq!(a.size, vec!["n"], "a should have size=[\"n\"]");

        // iteration_ops: ReadLine([n, k]) then ReadLine([a, size=Some("n")])
        assert_eq!(
            spec.iteration_ops.len(),
            2,
            "expected exactly 2 iteration_ops"
        );
        assert_eq!(spec.iteration_ops[0].tag, OpTag::ReadLine);
        assert_eq!(spec.iteration_ops[0].depth, 0);
        let op0_names: Vec<&str> = spec.iteration_ops[0]
            .vars
            .iter()
            .map(|v| v.name.as_str())
            .collect();
        assert_eq!(op0_names, vec!["n", "k"], "first op should read n and k");

        assert_eq!(spec.iteration_ops[1].tag, OpTag::ReadLine);
        assert_eq!(spec.iteration_ops[1].vars.len(), 1);
        assert_eq!(spec.iteration_ops[1].vars[0].name, "a");
        assert_eq!(spec.iteration_ops[1].vars[0].dim, 1);
        assert_eq!(
            spec.iteration_ops[1].vars[0].size.as_deref(),
            Some("n"),
            "a's op should have size=n"
        );
    }

    /// abc456_e style: complex body with multiple loops and arrays →
    /// iteration_vars non-empty, query_body empty.
    #[test]
    fn iteration_vars_complex_multi_loop_body() {
        let raw = concat!(
            "T\n\\mathrm{case}_1\n\\mathrm{case}_2\n\\vdots\n\\mathrm{case}_T\n\n",
            "N M\nU_1 V_1\nU_2 V_2\n\\vdots\nU_M V_M\nW\nS_1\nS_2\n\\vdots\nS_N"
        );
        let spec = parse(raw, "");
        assert!(spec.ok, "expected ok=true for abc456_e style");
        assert!(
            spec.query_body.is_empty(),
            "query_body must be empty when iteration_ops is populated"
        );
        assert!(
            !spec.iteration_vars.is_empty(),
            "iteration_vars should be non-empty"
        );
        assert!(
            !spec.iteration_ops.is_empty(),
            "iteration_ops should be non-empty"
        );
    }

    /// abc334_d style: scalar sub-block → query_body populated, iteration_vars empty (regression).
    #[test]
    fn iteration_vars_empty_for_scalar_body() {
        let raw = "N Q\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n\nX";
        let spec = parse(raw, "");
        assert!(spec.ok);
        assert!(
            !spec.query_body.is_empty(),
            "scalar sub-block should populate query_body"
        );
        assert!(
            spec.iteration_vars.is_empty(),
            "iteration_vars must be empty for scalar body"
        );
        assert!(
            spec.iteration_ops.is_empty(),
            "iteration_ops must be empty for scalar body"
        );
    }

    /// abc453_g (Copy Query): {\rm Query}_Q + 3 numbered sub-blocks → query_types(3)
    #[test]
    fn abc453g_rm_query_numbered_subtypes() {
        let raw = "N M Q\r\n{\\rm Query}_1\r\n{\\rm Query}_2\r\n\\vdots\r\n{\\rm Query}_Q\r\n\n1 X_i Y_i\r\n\n2 X_i Y_i Z_i\r\n\n3 X_i L_i R_i\r\n";
        let spec = parse(raw, "");
        assert!(
            spec.ok,
            "expected ok=true, got ok=false. vars={:?} ops={:?}",
            spec.vars, spec.ops
        );
        assert_eq!(spec.vars.len(), 3); // n, m, q
        assert_eq!(spec.query_types.len(), 3);
        // type 1: 1 X_i Y_i → 2 vars
        assert_eq!(spec.query_types[0].type_id, "1");
        assert!(spec.query_types[0].ok);
        assert_eq!(spec.query_types[0].vars.len(), 2);
        // type 2: 2 X_i Y_i Z_i → 3 vars
        assert_eq!(spec.query_types[1].type_id, "2");
        assert!(spec.query_types[1].ok);
        assert_eq!(spec.query_types[1].vars.len(), 3);
        // type 3: 3 X_i L_i R_i → 3 vars
        assert_eq!(spec.query_types[2].type_id, "3");
        assert!(spec.query_types[2].ok);
        assert_eq!(spec.query_types[2].vars.len(), 3);
    }

    // ── arithmetic expression subscripts ─────────────────────────────────────

    /// abc448_d: `N / A_1 \dots A_N / U_1 V_1 / \vdots / U_{N-1} V_{N-1}`
    /// {N-1} subscript → loop end = "n-1", ok=true
    #[test]
    fn arithmetic_subscript_n_minus_1_loop() {
        let raw = "N\nA_1 A_2 \\dots A_N\nU_1 V_1\nU_2 V_2\n\\vdots\nU_{N-1} V_{N-1}\n";
        let spec = scalar_ok(raw);
        assert!(spec.ok, "expected ok=true for {{N-1}} loop bound");
        // n should be is_size (used as array size for A, and in expression for U/V)
        let n = spec.vars.iter().find(|v| v.name == "n").expect("n var");
        assert!(n.is_size, "n should be is_size");
        // u and v should be dim=1
        let u = spec.vars.iter().find(|v| v.name == "u").expect("u var");
        let v = spec.vars.iter().find(|v| v.name == "v").expect("v var");
        assert_eq!(u.dim, 1);
        assert_eq!(v.dim, 1);
        // loop_begin end should be "n-1"
        let lb = spec
            .ops
            .iter()
            .find(|o| o.tag == OpTag::LoopBegin)
            .expect("LoopBegin");
        assert_eq!(lb.end.as_deref(), Some("n-1"), "loop end should be n-1");
    }

    /// tupc2024_k: `N / A_1 A_2 \ldots A_{2N}`
    /// {2N} subscript → array size = "2*n", ok=true, n.is_size=true
    #[test]
    fn arithmetic_subscript_2n_array_size() {
        let raw = "N\nA_1 A_2 \\ldots A_{2N}\n";
        let spec = scalar_ok(raw);
        assert!(spec.ok, "expected ok=true for {{2N}} array size");
        let a = spec.vars.iter().find(|v| v.name == "a").expect("a var");
        assert_eq!(a.dim, 1);
        assert_eq!(a.size, vec!["2*n".to_string()], "array size should be 2*n");
        // n should be is_size (referenced in expression "2*n")
        let n = spec.vars.iter().find(|v| v.name == "n").expect("n var");
        assert!(
            n.is_size,
            "n should be is_size when referenced in 2*n expression"
        );
        // ReadLine for a: size = "2*n"
        let rl = spec
            .ops
            .iter()
            .find(|o| o.vars.iter().any(|v| v.name == "a"))
            .expect("ReadLine for a");
        assert_eq!(rl.vars[0].size.as_deref(), Some("2*n"));
    }

    /// {N+1} subscript → loop end = "n+1", ok=true
    #[test]
    fn arithmetic_subscript_n_plus_1() {
        let raw = "N\nA_1 A_2 \\dots A_N\nX_1\nX_2\n\\vdots\nX_{N+1}\n";
        let spec = scalar_ok(raw);
        assert!(spec.ok, "expected ok=true for {{N+1}} loop bound");
        let lb = spec
            .ops
            .iter()
            .find(|o| o.tag == OpTag::LoopBegin)
            .expect("LoopBegin");
        assert_eq!(lb.end.as_deref(), Some("n+1"), "loop end should be n+1");
    }

    /// {2N-1} complex: Num * Ident - Num
    #[test]
    fn arithmetic_subscript_2n_minus_1() {
        let raw = "N\nA_1 A_2 \\ldots A_{2N-1}\n";
        let spec = scalar_ok(raw);
        assert!(spec.ok, "expected ok=true for {{2N-1}} array size");
        let a = spec.vars.iter().find(|v| v.name == "a").expect("a var");
        assert_eq!(
            a.size,
            vec!["2*n-1".to_string()],
            "array size should be 2*n-1"
        );
    }

    /// {N-} trailing operator → ok: false (invalid arithmetic expression)
    #[test]
    fn arithmetic_subscript_trailing_operator_is_not_ok() {
        let raw = "N\nA_1 A_2 \\ldots A_{N-}\n";
        let spec = scalar_ok(raw);
        assert!(
            !spec.ok,
            "trailing operator in subscript should give ok=false"
        );
    }

    /// {2**N} consecutive operators → ok: false
    #[test]
    fn arithmetic_subscript_consecutive_operators_is_not_ok() {
        let raw = "N\nA_1 A_2 \\ldots A_{2**N}\n";
        let spec = scalar_ok(raw);
        assert!(
            !spec.ok,
            "consecutive operators in subscript should give ok=false"
        );
    }

    /// {1-1,2} comma + operator mix → treated as None subscript, not a valid 2D subscript
    #[test]
    fn arithmetic_subscript_comma_operator_mix_is_not_ok() {
        // {1-1,2} must not silently corrupt to {11,2} and produce a spurious Array2DRow.
        // The whole line should fail to parse as Array2DRow and fall through to ok:false.
        let raw = "A_{1-1,2} A_{1-1,3}\n";
        let spec = scalar_ok(raw);
        assert!(
            !spec.ok,
            "comma+operator subscript mix should give ok=false"
        );
    }

    /// When both `N` and `n` coexist (case collision), arithmetic subscripts like `{N-1}`
    /// must produce the uppercase-preserved ident `"N"` in the expression (e.g. `"N-1"`),
    /// so `valid_loop_bounds` and `is_size` still work correctly.
    #[test]
    fn arithmetic_subscript_collision_uppercase_preserved() {
        // "N n\nA_1 A_2 \ldots A_N\nU_1 V_1\nU_2 V_2\n\\vdots\nU_{N-1} V_{N-1}\n"
        // N and n both appear → collision → normalize_name("N") = "N" (uppercase preserved)
        // The {N-1} subscript must produce loop end "N-1" (not "n-1") so valid_loop_bounds passes.
        let raw = "N n\nA_1 A_2 \\ldots A_N\nU_1 V_1\nU_2 V_2\n\\vdots\nU_{N-1} V_{N-1}\n";
        let spec = scalar_ok(raw);
        assert!(
            spec.ok,
            "expected ok=true when N/n collision and {{N-1}} loop bound; vars={:?} ops={:?}",
            spec.vars, spec.ops
        );
        // N is uppercase-preserved due to collision
        let n_upper = spec.vars.iter().find(|v| v.math == "N").expect("N var");
        assert_eq!(n_upper.name, "N");
        assert!(
            n_upper.is_size,
            "N should be is_size=true (loop bound and array size)"
        );
        // loop end should be "N-1" (uppercase preserved)
        let lb = spec
            .ops
            .iter()
            .find(|o| o.tag == OpTag::LoopBegin)
            .expect("LoopBegin");
        assert_eq!(lb.end.as_deref(), Some("N-1"), "loop end should be N-1");
    }

    /// Numbered query_types → iteration_vars/ops always empty.
    #[test]
    fn iteration_vars_empty_when_query_types_present() {
        let raw = "N Q\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n\n1 x\n2 x k";
        let spec = parse(raw, "");
        assert!(spec.ok);
        assert!(
            !spec.query_types.is_empty(),
            "expected query_types non-empty"
        );
        assert!(
            spec.iteration_vars.is_empty(),
            "iteration_vars must be empty when query_types present"
        );
        assert!(
            spec.iteration_ops.is_empty(),
            "iteration_ops must be empty when query_types present"
        );
    }

    // ── triangular matrix ──────────────────────────────────────────────────────

    /// ABC451-E style: N rows, upper-triangular, bound = N → bound = "n"
    #[test]
    fn triangular_abc451e_bound_n() {
        let raw =
            "N\nA_{1, 2} A_{1, 3} \\ldots A_{1, N}\nA_{2, 3} \\ldots A_{2, N}\n\\vdots\nA_{N-1,N}";
        let constraints = "All input values are integers";
        let spec = with_constraints(raw, constraints);
        assert!(spec.ok, "expected ok=true; spec={spec:?}");
        assert!(spec.triangular.is_some(), "expected triangular to be Some");
        let tri = spec.triangular.as_ref().unwrap();
        assert_eq!(tri.name, "a", "triangular name should be 'a'");
        assert_eq!(tri.bound, "n", "triangular bound should be 'n'");
        assert_eq!(
            tri.var_type,
            VarType::Int,
            "triangular var_type should be Int"
        );
        assert_eq!(spec.vars.len(), 1, "expected exactly 1 var (size)");
        assert_eq!(spec.vars[0].name, "n", "size var name should be 'n'");
        assert!(spec.vars[0].is_size, "size var should have is_size=true");
        assert_eq!(spec.ops.len(), 1, "expected exactly 1 op (ReadLine for n)");
        assert_eq!(
            spec.kind(),
            InputFormatKind::Triangle,
            "expected kind Triangle"
        );
    }

    /// ABC236-D style: 2N-1 rows, upper-triangular with bound = 2N → bound = "2*n"
    #[test]
    fn triangular_abc236d_bound_2n() {
        let raw = "N\nA_{1, 2} A_{1, 3} A_{1, 4} \\cdots A_{1, 2N}\nA_{2, 3} A_{2, 4} \\cdots A_{2, 2N}\nA_{3, 4} \\cdots A_{3, 2N}\n\\vdots\nA_{2N-1, 2N}";
        let constraints = "All input values are integers";
        let spec = with_constraints(raw, constraints);
        assert!(spec.ok, "expected ok=true; spec={spec:?}");
        assert!(spec.triangular.is_some(), "expected triangular to be Some");
        let tri = spec.triangular.as_ref().unwrap();
        assert_eq!(tri.bound, "2*n", "triangular bound should be '2*n'");
        assert_eq!(tri.name, "a", "triangular name should be 'a'");
    }

    /// A line consisting only of \ldots (cdots tokens) should be treated as Vdots
    /// and accepted as the vertical separator in a triangular matrix pattern.
    #[test]
    fn triangular_cdots_only_line_as_vdots() {
        let raw = "N\nA_{1, 2} \\ldots A_{1, N}\nA_{2, 3} \\ldots A_{2, N}\n\\ldots\nA_{N-1,N}";
        let spec = scalar_ok(raw);
        assert!(spec.ok, "expected ok=true; spec={spec:?}");
        assert!(
            spec.triangular.is_some(),
            "expected triangular to be Some when \\ldots used as vertical separator"
        );
    }

    /// Trailing cdots with no element after — must NOT be detected as triangular.
    #[test]
    fn triangular_trailing_cdots_no_element_after_is_not_ok() {
        // "A_{1,2} \ldots" — cdots at end, no bound element following
        let raw = "N\nA_{1, 2} \\ldots\nA_{2, 3} \\ldots A_{2, N}\n\\vdots\nA_{N-1,N}";
        let spec = with_constraints(raw, "All input values are integers");
        assert!(
            spec.triangular.is_none(),
            "expected triangular=None when cdots has no element after it, got: {spec:?}"
        );
    }

    /// First subscript that is an ident (not a number) — must NOT be detected as triangular.
    #[test]
    fn triangular_ident_first_subscript_is_not_triangular() {
        // "A_{i, j}" style — first subscript is a variable, not a literal number
        let raw =
            "N\nA_{i, 2} A_{i, 3} \\ldots A_{i, N}\nA_{j, 3} \\ldots A_{j, N}\n\\vdots\nA_{N-1,N}";
        let spec = with_constraints(raw, "All input values are integers");
        assert!(
            spec.triangular.is_none(),
            "expected triangular=None when first subscript is ident, got: {spec:?}"
        );
    }

    /// Last line with extra tokens after the subscript — must NOT be detected as triangular.
    #[test]
    fn triangular_last_line_extra_tokens_is_not_triangular() {
        // "A_{N-1,N} extra" — trailing token after the subscript
        let raw = "N\nA_{1, 2} A_{1, 3} \\ldots A_{1, N}\nA_{2, 3} \\ldots A_{2, N}\n\\vdots\nA_{N-1,N} X";
        let spec = with_constraints(raw, "All input values are integers");
        assert!(
            spec.triangular.is_none(),
            "expected triangular=None when last line has extra tokens, got: {spec:?}"
        );
    }

    /// Line 2 has a different bound than line 1 — must NOT be detected as triangular.
    #[test]
    fn triangular_inconsistent_bound_line2_is_not_triangular() {
        // Line 1 bound = N, line 2 bound = M — different
        let raw =
            "N\nA_{1, 2} A_{1, 3} \\ldots A_{1, N}\nA_{2, 3} \\ldots A_{2, M}\n\\vdots\nA_{N-1,N}";
        let spec = with_constraints(raw, "All input values are integers");
        assert!(
            spec.triangular.is_none(),
            "expected triangular=None when line 2 has a different bound, got: {spec:?}"
        );
    }

    /// An intermediate row has a different bound than line 1 — must NOT be detected as triangular.
    #[test]
    fn triangular_inconsistent_bound_intermediate_is_not_triangular() {
        // Lines: size, row1 (bound N), row2 (bound M), vdots, last
        let raw = "N\nA_{1, 2} A_{1, 3} \\ldots A_{1, N}\nA_{2, 3} \\ldots A_{2, N}\nA_{3, 4} \\ldots A_{3, M}\n\\vdots\nA_{N-1,N}";
        let spec = with_constraints(raw, "All input values are integers");
        assert!(
            spec.triangular.is_none(),
            "expected triangular=None when an intermediate row has a different bound, got: {spec:?}"
        );
    }

    /// bound_raw references a variable that is not the size variable — must NOT be detected.
    #[test]
    fn triangular_bound_unknown_ident_is_not_triangular() {
        // bound is "M" but size variable is "N" — M is undefined in the template context
        let raw = "N\nA_{1, 2} A_{1, 3} \\ldots A_{1, M}\n\\vdots\nA_{N-1,M}";
        let spec = with_constraints(raw, "All input values are integers");
        assert!(
            spec.triangular.is_none(),
            "expected triangular=None when bound references an unknown ident, got: {spec:?}"
        );
    }

    /// When constraints are empty, var_type should default to Int (not Unknown).
    #[test]
    fn triangular_unknown_var_type_falls_back_to_int() {
        let raw =
            "N\nA_{1, 2} A_{1, 3} \\ldots A_{1, N}\nA_{2, 3} \\ldots A_{2, N}\n\\vdots\nA_{N-1,N}";
        // Pass empty constraints so infer_types produces Unknown
        let spec = with_constraints(raw, "");
        assert!(
            spec.triangular.is_some(),
            "expected triangular to be detected; spec={spec:?}"
        );
        let tri = spec.triangular.unwrap();
        assert_eq!(
            tri.var_type,
            VarType::Int,
            "expected var_type=Int when constraints are empty, got: {:?}",
            tri.var_type
        );
    }

    /// Unicode horizontal ellipsis (U+2026) should be substituted for \ldots before tokenization.
    #[test]
    fn triangular_unicode_ellipsis() {
        // Replace \ldots with the actual Unicode … character (U+2026)
        let raw = "N\nA_{1, 2} A_{1, 3} \u{2026} A_{1, N}\nA_{2, 3} \u{2026} A_{2, N}\n\\vdots\nA_{N-1,N}";
        let constraints = "All input values are integers";
        let spec = with_constraints(raw, constraints);
        assert!(
            spec.ok,
            "expected ok=true with Unicode ellipsis; spec={spec:?}"
        );
        assert!(
            spec.triangular.is_some(),
            "expected triangular to be Some when Unicode … used instead of \\ldots"
        );
    }

    // ── TASK-032: jagged array input detection ────────────────────────────────

    /// abc457_b: single scalar (L_N) is the size_var, followed by a jagged array A
    /// "N\nL_1 A_{1,1} \ldots A_{1,L_1}\n\vdots\nL_N A_{N,1} \ldots A_{N,L_N}\n"
    #[test]
    fn jagged_abc457b_single_scalar_size_var() {
        let raw = "N\nL_1 A_{1,1} \\ldots A_{1,L_1}\n\\vdots\nL_N A_{N,1} \\ldots A_{N,L_N}\n";
        let constraints = "1 \\leq N \\leq 10^5";
        let spec = with_constraints(raw, constraints);
        assert!(
            spec.ok,
            "expected ok=true for abc457_b jagged pattern; spec={spec:?}"
        );
        assert_eq!(
            spec.ops.len(),
            2,
            "expected exactly 2 ops (read_line for N + loop_jagged); ops={:?}",
            spec.ops
        );
        assert_eq!(
            spec.ops[1].tag,
            OpTag::LoopJagged,
            "expected ops[1].tag == LoopJagged; got {:?}",
            spec.ops[1].tag
        );
        assert_eq!(
            spec.ops[1].end.as_deref(),
            Some("n"),
            "expected loop end == 'n'; got {:?}",
            spec.ops[1].end
        );
        assert!(
            spec.ops[1].scalars.is_empty(),
            "expected scalars to be empty for abc457_b (L is the size_var itself); scalars={:?}",
            spec.ops[1].scalars
        );
        assert_eq!(
            spec.ops[1].size_var.as_ref().map(|v| v.name.as_str()),
            Some("l"),
            "expected size_var.name == 'l'; got {:?}",
            spec.ops[1].size_var
        );
        assert_eq!(
            spec.ops[1].elem_var.as_ref().map(|v| v.name.as_str()),
            Some("a"),
            "expected elem_var.name == 'a'; got {:?}",
            spec.ops[1].elem_var
        );
        assert_eq!(
            spec.kind(),
            InputFormatKind::Jagged,
            "expected kind == Jagged; got {:?}",
            spec.kind()
        );
    }

    /// abc446_b: size_var L is on its own line before the jagged array X, multi-row body
    /// "N M\nL_1\nX_{1,1} \cdots X_{1,L_1}\n\vdots\nL_N\nX_{N,1} \cdots X_{N,L_N}\n"
    #[test]
    fn jagged_abc446b_two_body_rows() {
        let raw = "N M\nL_1\nX_{1,1} \\cdots X_{1,L_1}\n\\vdots\nL_N\nX_{N,1} \\cdots X_{N,L_N}\n";
        let spec = with_constraints(raw, "");
        assert!(
            spec.ok,
            "expected ok=true for abc446_b jagged pattern; spec={spec:?}"
        );
        assert!(
            spec.ops.iter().any(|o| o.tag == OpTag::LoopJagged),
            "expected at least one LoopJagged op; ops={:?}",
            spec.ops
        );
        assert_eq!(
            spec.ops[1].tag,
            OpTag::LoopJagged,
            "expected ops[1].tag == LoopJagged; got {:?}",
            spec.ops[1].tag
        );
        assert_eq!(
            spec.ops[1].end.as_deref(),
            Some("n"),
            "expected loop end == 'n'; got {:?}",
            spec.ops[1].end
        );
        assert!(
            spec.ops[1].scalars.is_empty(),
            "expected scalars to be empty for abc446_b; scalars={:?}",
            spec.ops[1].scalars
        );
        assert_eq!(
            spec.ops[1].size_var.as_ref().map(|v| v.name.as_str()),
            Some("l"),
            "expected size_var.name == 'l'; got {:?}",
            spec.ops[1].size_var
        );
        assert_eq!(
            spec.ops[1].elem_var.as_ref().map(|v| v.name.as_str()),
            Some("x"),
            "expected elem_var.name == 'x'; got {:?}",
            spec.ops[1].elem_var
        );
    }

    /// abc226_c: two scalars T_N K_N before jagged array A_{N,1}...A_{N,K_N}
    /// "N\nT_1 K_1 A_{1,1} \ldots A_{1,K_1}\n\vdots\nT_N K_N A_{N,1} \ldots A_{N,K_N}\n"
    #[test]
    fn jagged_abc226c_two_scalars() {
        let raw =
            "N\nT_1 K_1 A_{1,1} \\ldots A_{1,K_1}\n\\vdots\nT_N K_N A_{N,1} \\ldots A_{N,K_N}\n";
        let spec = with_constraints(raw, "");
        assert!(
            spec.ok,
            "expected ok=true for abc226_c jagged pattern; spec={spec:?}"
        );
        assert!(
            spec.ops.iter().any(|o| o.tag == OpTag::LoopJagged),
            "expected at least one LoopJagged op; ops={:?}",
            spec.ops
        );
        assert_eq!(
            spec.ops[1].tag,
            OpTag::LoopJagged,
            "expected ops[1].tag == LoopJagged; got {:?}",
            spec.ops[1].tag
        );
        assert_eq!(
            spec.ops[1].end.as_deref(),
            Some("n"),
            "expected loop end == 'n'; got {:?}",
            spec.ops[1].end
        );
        assert_eq!(
            spec.ops[1].scalars.len(),
            1,
            "expected 1 scalar (T) in scalars; scalars={:?}",
            spec.ops[1].scalars
        );
        assert_eq!(
            spec.ops[1].scalars[0].name, "t",
            "expected scalars[0].name == 't'; got {:?}",
            spec.ops[1].scalars[0].name
        );
        assert_eq!(
            spec.ops[1].size_var.as_ref().map(|v| v.name.as_str()),
            Some("k"),
            "expected size_var.name == 'k'; got {:?}",
            spec.ops[1].size_var
        );
        assert_eq!(
            spec.ops[1].elem_var.as_ref().map(|v| v.name.as_str()),
            Some("a"),
            "expected elem_var.name == 'a'; got {:?}",
            spec.ops[1].elem_var
        );
    }
}
