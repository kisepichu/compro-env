use domain::entity::{InputOp, InputSpec, OpTag, QueryTypeDecl, VarDecl, VarRef, VarType};

// ── Lexer ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    Num(String),
    Subscript,
    LBrace,
    RBrace,
    Comma,
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
}

/// Parse errors cause ok=false
#[derive(Debug)]
enum ParseError {
    NonNumericSubscript,
    Unknown,
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

    // Check for \text{} or \mathrm{} tokens → query placeholder line.
    // A valid query line is exactly: \text{...}_<subscript> or \mathrm{...}_<subscript>.
    // Anything else (extra tokens, missing subscript) falls through to Err.
    let query_pos = tokens.iter().position(
        |t| matches!(t, Token::Ident(s) if s.starts_with("\\text{") || s.starts_with("\\mathrm{")),
    );
    if let Some(pos) = query_pos {
        // Must be the only meaningful token (at position 0 after strip_spaces)
        if pos != 0 {
            return Err(ParseError::Unknown);
        }
        // Expect: _<subscript> and nothing after
        if tokens.get(pos + 1) == Some(&Token::Subscript) {
            let (loop_bound, advance) =
                read_subscript_value(&tokens[pos + 2..]).ok_or(ParseError::Unknown)?;
            // Nothing should follow
            if pos + 2 + advance != tokens.len() {
                return Err(ParseError::Unknown);
            }
            return Ok(RawLine::QueryLine { loop_bound });
        }
        return Err(ParseError::Unknown);
    }

    // Try to detect a character grid row: X_{row,col_start}...X_{row,col_end}
    // (before array1d, because some grid patterns look like array1d with alphabetic subscripts)
    if let Some(result) = try_parse_grid_row(&tokens) {
        return result;
    }

    // Try to detect 1D horizontal array: pattern like Ident_Num ... Ident_Num [Cdots] Ident_Num
    // or Ident_Num Space Ident_Num Space Cdots
    if let Some(result) = try_parse_array1d(&tokens) {
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

    // Skip spaces, then Cdots
    while tokens.get(i) == Some(&Token::Space) {
        i += 1;
    }
    if tokens.get(i) != Some(&Token::Cdots) {
        return None;
    }
    i += 1;
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
            // Read until matching RBrace, collecting content
            let mut depth = 1;
            let mut content_parts: Vec<String> = Vec::new();
            let mut i = 1;
            while i < tokens.len() && depth > 0 {
                match &tokens[i] {
                    Token::LBrace => {
                        depth += 1;
                        i += 1;
                    }
                    Token::RBrace => {
                        depth -= 1;
                        i += 1;
                    }
                    Token::Num(n) => {
                        content_parts.push(n.clone());
                        i += 1;
                    }
                    Token::Ident(s) => {
                        content_parts.push(s.clone());
                        i += 1;
                    }
                    Token::Comma => {
                        // A_{1,1} style — not supported (phase 2)
                        return None;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            if content_parts.len() == 1 {
                Some((content_parts[0].clone(), i))
            } else {
                // Multiple parts (comma-separated) — not handled
                None
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
        _ => false,
    };

    /// Row kind: distinguishes LoopRow, GridRow, and QueryLine for block-extension checks.
    #[derive(PartialEq)]
    enum RowKind {
        Loop,
        Grid,
        Query,
    }
    let row_kind = |rl: &RawLine| match rl {
        RawLine::GridRow(_) => RowKind::Grid,
        RawLine::QueryLine { .. } => RowKind::Query,
        _ => RowKind::Loop,
    };

    let mut vdots_blocks: Vec<(usize, usize, usize)> = Vec::new(); // (block_start, vdots_idx, after_end)
    {
        let mut j = 0;
        while j < raw_lines.len() {
            if matches!(raw_lines[j], RawLine::Vdots) {
                // Find consecutive LoopRows/GridRows/QueryLines before this vdots.
                // Stop extending when the kind (LoopRow vs GridRow vs QueryLine) changes.
                let last_kind = if j > 0 {
                    row_kind(&raw_lines[j - 1])
                } else {
                    RowKind::Loop
                };
                let mut block_start = j;
                while block_start > 0 {
                    let prev = &raw_lines[block_start - 1];
                    if is_loop_or_grid(prev) && row_kind(prev) == last_kind {
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
                    if is_loop_or_grid(next) && row_kind(next) == last_kind {
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

            // Verify all lines in the block are the same kind
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
            RawLine::QueryLine { .. } => {
                // QueryLine not matched to a vdots block — treat as unsupported
                return Err(ParseError::Unknown);
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
        };
    }

    // Preprocess: \hspace{...}\vdots → \vdots
    let preprocessed = preprocess(raw);

    // Split into blocks by \n\n
    let blocks: Vec<&str> = preprocessed.split("\n\n").collect();

    // Phase 2 early detection
    let block0 = blocks[0];

    // Whether block0 contains a query-placeholder marker (\text{...} or \mathrm{...}).
    // If so, we attempt to parse block0 normally (QueryLine handling in parse_line/build_intermediate)
    // rather than rejecting early.
    let has_query_marker = block0.contains("\\text{") || block0.contains("\\mathrm{");

    // Check for multiple blocks (only reject non-query multi-block forms)
    if blocks.len() > 1 && !has_query_marker {
        let block1 = blocks[1].trim();
        // blocks[1] starts with digit → query sub-format (unrecognized multi-block form)
        if block1.starts_with(|c: char| c.is_ascii_digit()) {
            return not_ok(raw);
        }
        // blocks[0] is single token → T-testcases type
        let block0_tokens: Vec<Token> = block0
            .lines()
            .filter(|l| !l.trim().is_empty())
            .flat_map(tokenize_line)
            .filter(|t| t != &Token::Space)
            .collect();
        if block0_tokens.len() == 1
            && let Token::Ident(_) = &block0_tokens[0]
        {
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
                });
            }
            IntermOp::ReadArray1D { name, size } => {
                let var_name = normalize_name(name, &all_math_names);
                let size_name = normalize_name(size, &all_math_names);
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
                });
            }
            IntermOp::LoopBegin {
                loop_var,
                begin,
                end,
            } => {
                let end_name = normalize_name(end, &all_math_names);
                current_loop_end = Some(end_name.clone());
                ops.push(InputOp {
                    tag: OpTag::LoopBegin,
                    depth,
                    vars: vec![],
                    loop_var: Some(loop_var.clone()),
                    begin: Some(begin.clone()),
                    end: Some(end_name),
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
                });
            }
        }
    }

    // Type inference
    infer_types(&mut var_decls, constraints);

    // Compute is_size: a var is a size var if its name appears in any other VarDecl's size,
    // or if it is the `end` of any LoopBegin op.
    let size_names: std::collections::HashSet<String> = var_decls
        .iter()
        .flat_map(|v| v.size.iter().cloned())
        .chain(
            ops.iter()
                .filter(|o| o.tag == OpTag::LoopBegin)
                .filter_map(|o| o.end.clone()),
        )
        .collect();
    for v in &mut var_decls {
        v.is_size = size_names.contains(&v.name);
    }

    // Empty result (no vars and no ops) means the raw text produced nothing
    // meaningful — treat as not_ok so templates use the safe fallback.
    if var_decls.is_empty() && ops.is_empty() {
        return not_ok(raw);
    }

    // Try to flatten single-variable loops to array reads.
    // Multi-var loops (or loops that can't be flattened) are kept in ops; the template handles them.
    ops = flatten_single_var_loops(ops, &mut var_decls);

    // Any remaining LoopBegin ops will be emitted directly by the template, so validate
    // their bounds before reporting ok=true.
    let valid_loop_bounds = ops.iter().filter(|o| o.tag == OpTag::LoopBegin).all(|o| {
        let end = match o.end.as_deref().map(str::trim) {
            Some(end) if !end.is_empty() => end,
            _ => return false,
        };
        end.chars().all(|c| c.is_ascii_digit())
            || var_decls.iter().any(|v| v.name == end && v.dim == 0)
    });
    if !valid_loop_bounds {
        return not_ok(raw);
    }

    // Parse query sub-blocks (blocks[1..]) when a query marker was present in block0.
    // Each sub-block whose first token is a Num becomes a QueryTypeDecl.
    // Sub-blocks that start with a non-Num token (e.g. abc334-D's "X") are skipped entirely.
    let query_types = if has_query_marker {
        parse_query_subblocks(&blocks[1..], constraints)
    } else {
        vec![]
    };

    InputSpec {
        raw: raw.to_string(),
        ok: true,
        vars: var_decls,
        ops,
        query_types,
    }
}

fn preprocess(raw: &str) -> String {
    // Replace \hspace{...}\vdots with \vdots
    let mut result = String::new();
    let mut rest = raw;

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

/// Parse `blocks[1..]` from a query-type input into `QueryTypeDecl` entries.
///
/// Each sub-block is a `\n\n`-separated chunk of text.  Only sub-blocks whose
/// first non-empty line starts with a `Num` token are processed; others are skipped.
fn parse_query_subblocks(subblocks: &[&str], constraints: &str) -> Vec<QueryTypeDecl> {
    let mut result = Vec::new();

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

        // First token must be Num → type_id.
        let type_id = match first_stripped.first() {
            Some(Token::Num(n)) => n.clone(),
            _ => continue, // not a numbered sub-block → skip
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
        let math_names: Vec<String> = raw_vars.iter().map(|v| v.math.clone()).collect();
        let mut var_decls: Vec<VarDecl> = raw_vars
            .iter()
            .map(|rv| {
                let name = normalize_name(&rv.math, &math_names);
                VarDecl {
                    name,
                    math: rv.math.clone(),
                    var_type: VarType::Unknown,
                    dim: 0,
                    size: vec![],
                    is_size: false,
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

    result
}

fn not_ok(raw: &str) -> InputSpec {
    InputSpec {
        raw: raw.to_string(),
        ok: false,
        vars: vec![],
        ops: vec![],
        query_types: vec![],
    }
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
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::entity::{OpTag, VarType};

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
    #[test]
    fn multi_block_digit_no_query_marker_not_ok() {
        let spec = scalar_ok("Q\nquery_1\n\\vdots\nquery_Q\n\n1 x\n\n2 x k");
        assert!(
            !spec.ok,
            "expected ok=false for multi-block query sub-format without \\text{{}} marker"
        );
    }

    // ── Phase 2: T-testcases ──────────────────────────────────────────────────

    #[test]
    fn phase2_t_testcases() {
        let spec = scalar_ok("T\n\na s");
        assert!(!spec.ok, "expected ok=false for T-testcases pattern");
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

    /// abc334-D style: sub-block starts with ident (not digit) → skipped, query_types empty
    #[test]
    fn query_subblock_non_numeric_skipped() {
        let spec =
            scalar_ok("N Q\nR_1 \\ldots R_N\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n\nX\n");
        assert!(spec.ok, "expected ok=true");
        assert_eq!(
            spec.query_types.len(),
            0,
            "non-numeric sub-block should be skipped"
        );
    }

    /// No sub-blocks → query_types empty
    #[test]
    fn query_no_subblocks_empty_query_types() {
        let spec = scalar_ok("Q\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n");
        assert!(spec.ok, "expected ok=true");
        assert_eq!(
            spec.query_types.len(),
            0,
            "no sub-blocks means query_types is empty"
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
}
