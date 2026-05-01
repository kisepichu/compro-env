use domain::entity::{InputOp, InputSpec, OpTag, VarDecl, VarRef, VarType};

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

    // Check if this is a pure Vdots line
    if tokens.len() == 1 && tokens[0] == Token::Vdots {
        return Ok(RawLine::Vdots);
    }
    if tokens.is_empty() {
        return Ok(RawLine::Scalars(vec![]));
    }

    // Check for \text{} or \mathrm{} tokens → signal phase2
    for tok in &tokens {
        if let Token::Ident(s) = tok
            && (s.starts_with("\\text{") || s.starts_with("\\mathrm{"))
        {
            return Err(ParseError::Unknown);
        }
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
}

fn build_intermediate(raw_lines: &[RawLine]) -> Result<Vec<IntermOp>, ParseError> {
    let mut ops: Vec<IntermOp> = Vec::new();
    let loop_vars = ["i", "j", "k2", "l", "m2"];
    let mut loop_var_counter = 0usize;

    // Pre-scan: mark which line indices are inside a vdots loop block.
    // A vdots block is: [LoopRow*] Vdots [LoopRow*] where there's at least one LoopRow before.
    // We record ranges as (loop_row_before_start, vdots_idx, loop_row_after_end).
    // Lines in a vdots block are consumed at Vdots time and skipped otherwise.
    let mut vdots_blocks: Vec<(usize, usize, usize)> = Vec::new(); // (block_start, vdots_idx, after_end)
    {
        let mut j = 0;
        while j < raw_lines.len() {
            if matches!(raw_lines[j], RawLine::Vdots) {
                // Find consecutive LoopRows before this vdots
                let mut block_start = j;
                while block_start > 0 {
                    match &raw_lines[block_start - 1] {
                        RawLine::LoopRow(_) => block_start -= 1,
                        _ => break,
                    }
                }
                if block_start == j {
                    // No LoopRows before — not a vdots loop, skip
                    j += 1;
                    continue;
                }
                // Find LoopRows after this vdots
                let mut after_end = j + 1;
                while after_end < raw_lines.len() {
                    match &raw_lines[after_end] {
                        RawLine::LoopRow(_) => after_end += 1,
                        _ => break,
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
            // Get the representative loop row vars from the first "before" row
            let first_before = match &raw_lines[block_start] {
                RawLine::LoopRow(vars) => vars.iter().map(|v| v.math.clone()).collect::<Vec<_>>(),
                _ => return Err(ParseError::Unknown),
            };

            // Get the loop end from the last "after" row's last var's subscript
            let end_size = if after_end > vdots_idx + 1 {
                match &raw_lines[after_end - 1] {
                    RawLine::LoopRow(vars) => vars
                        .last()
                        .and_then(|v| v.subscript.clone())
                        .unwrap_or_default(),
                    _ => String::new(),
                }
            } else {
                // No after row — use last before row's last var's subscript
                match &raw_lines[vdots_idx - 1] {
                    RawLine::LoopRow(vars) => vars
                        .last()
                        .and_then(|v| v.subscript.clone())
                        .unwrap_or_default(),
                    _ => String::new(),
                }
            };

            let lv = loop_vars.get(loop_var_counter).copied().unwrap_or("i");
            loop_var_counter += 1;

            ops.push(IntermOp::LoopBegin {
                loop_var: lv.to_string(),
                begin: "0".to_string(),
                end: end_size,
            });
            ops.push(IntermOp::ReadLoopRow(first_before));
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
            RawLine::LoopRow(_) => {
                // LoopRow not matched to a vdots block — unsupported fixed enumeration
                // (e.g. "A_1 A_2" without \vdots). Silently degrading to scalars would
                // produce duplicate variable names; treat as unsupported.
                return Err(ParseError::NonNumericSubscript);
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
            v.var_type = VarType::Int;
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

fn constraints_mention_str(constraints: &str, _name: &str, math: &str) -> bool {
    let str_keywords = [
        "文字列",
        "string",
        "英小文字",
        "英大文字",
        "lowercase",
        "uppercase",
    ];

    for keyword in &str_keywords {
        if constraints.contains(keyword) {
            for line in constraints.lines() {
                if line.contains(keyword) && contains_token(line, math) {
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
                if line.contains(keyword) && contains_token(line, math) {
                    return true;
                }
            }
        }
    }

    for line in constraints.lines() {
        if (line.contains("\\leq") || line.contains("≤") || line.contains('<'))
            && contains_token(line, math)
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
        };
    }

    // Preprocess: \hspace{...}\vdots → \vdots
    let preprocessed = preprocess(raw);

    // Split into blocks by \n\n
    let blocks: Vec<&str> = preprocessed.split("\n\n").collect();

    // Phase 2 early detection
    let block0 = blocks[0];

    // Check for \text{ or \mathrm{
    if block0.contains("\\text{") || block0.contains("\\mathrm{") {
        return not_ok(raw);
    }

    // Check for multiple blocks
    if blocks.len() > 1 {
        let block1 = blocks[1].trim();
        // blocks[1] starts with digit → query sub-format
        if block1.starts_with(|c: char| c.is_ascii_digit()) {
            return not_ok(raw);
        }
        // blocks[0] is single token → T-testcases
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
    // Multi-var loops are kept in ops; the template handles them.
    ops = match flatten_single_var_loops(ops, &mut var_decls) {
        Ok(flattened) => flattened,
        Err(original) => original, // keep loop ops; template will generate loop code
    };

    InputSpec {
        raw: raw.to_string(),
        ok: true,
        vars: var_decls,
        ops,
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
/// Multi-var loops (ReadLine with 2+ vars) cannot be flattened → return Err(original_ops).
fn flatten_single_var_loops(
    ops: Vec<InputOp>,
    var_decls: &mut [VarDecl],
) -> Result<Vec<InputOp>, Vec<InputOp>> {
    let original = ops.clone();
    let mut result = Vec::new();
    let mut i = 0;
    while i < ops.len() {
        match ops[i].tag {
            OpTag::LoopBegin => {
                // Expect: ops[i] = LoopBegin, ops[i+1] = ReadLine(1 var with index), ops[i+2] = LoopEnd
                if i + 2 < ops.len()
                    && ops[i + 1].tag == OpTag::ReadLine
                    && ops[i + 1].vars.len() == 1
                    && ops[i + 1].vars[0].index.is_some()
                    && ops[i + 2].tag == OpTag::LoopEnd
                {
                    let loop_end = ops[i].end.clone().ok_or_else(|| original.clone())?;
                    let v = &ops[i + 1].vars[0];

                    // Validate loop_end: must be a numeric literal or a declared scalar.
                    // If it's neither, the generated code would reference an undefined variable.
                    let loop_end_valid = loop_end.chars().all(|c| c.is_ascii_digit())
                        || var_decls.iter().any(|d| d.name == loop_end && d.dim == 0);
                    if !loop_end_valid {
                        return Err(original);
                    }

                    let flattened_var = VarRef {
                        name: v.name.clone(),
                        dim: 1,
                        size: Some(loop_end.clone()),
                        index: None,
                    };

                    // Update the corresponding VarDecl; treat missing or incompatible decl as error
                    // to avoid producing internally inconsistent InputSpec.
                    let decl = var_decls
                        .iter_mut()
                        .find(|d| d.name == v.name)
                        .ok_or_else(|| original.clone())?;
                    if decl.dim != 1 {
                        return Err(original.clone());
                    }
                    if decl.size.is_empty() {
                        decl.size = vec![loop_end.clone()];
                    } else if decl.size != vec![loop_end.clone()] {
                        return Err(original.clone());
                    }
                    result.push(InputOp {
                        tag: OpTag::ReadLine,
                        depth: ops[i].depth,
                        vars: vec![flattened_var],
                        loop_var: None,
                        begin: None,
                        end: None,
                    });
                    i += 3;
                } else {
                    // Can't flatten (multi-var, nested, etc.) — return original ops
                    return Err(original);
                }
            }
            _ => {
                result.push(ops[i].clone());
                i += 1;
            }
        }
    }
    Ok(result)
}

fn not_ok(raw: &str) -> InputSpec {
    InputSpec {
        raw: raw.to_string(),
        ok: false,
        vars: vec![],
        ops: vec![],
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

    // ── Phase 2 early-exit: \\text{query} ─────────────────────────────────────

    /// Opaque query blocks → ok: false
    #[test]
    fn phase2_text_query_returns_not_ok() {
        let spec = scalar_ok("Q\n\\text{query}_1\n\\vdots\n\\text{query}_Q\n");
        assert!(!spec.ok, "expected ok=false for \\text{{query}} block");
        assert!(spec.vars.is_empty());
        assert!(spec.ops.is_empty());
    }

    // ── Phase 2 early-exit: \\mathrm{Query} ───────────────────────────────────

    #[test]
    fn phase2_mathrm_query_returns_not_ok() {
        let spec = scalar_ok("N\nQ\n\\mathrm{Query}_1\n\\vdots\n\\mathrm{Query}_Q\n");
        assert!(!spec.ok, "expected ok=false for \\mathrm{{Query}}");
    }

    // ── Phase 2: multiple blocks (query sub-format) ───────────────────────────

    /// Two \\n\\n-separated blocks where the tail blocks encode sub-format
    #[test]
    fn phase2_multi_block_query_subformat() {
        let spec = scalar_ok("Q\nquery_1\n\\vdots\nquery_Q\n\n1 x\n\n2 x k");
        assert!(
            !spec.ok,
            "expected ok=false for multi-block query sub-format"
        );
    }

    // ── Phase 2: T-testcases ──────────────────────────────────────────────────

    #[test]
    fn phase2_t_testcases() {
        let spec = scalar_ok("T\n\na s");
        assert!(!spec.ok, "expected ok=false for T-testcases pattern");
    }

    // ── Phase 2: non-numeric subscript scalar (A_x A_y) ──────────────────────

    #[test]
    fn phase2_non_numeric_subscript_scalars() {
        let spec = scalar_ok("A_x A_y\n");
        assert!(
            !spec.ok,
            "expected ok=false for non-numeric subscript on single-element"
        );
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
}
