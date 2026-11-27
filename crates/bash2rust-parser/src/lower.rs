//! AST-to-IR lowering from `brush_parser::ast` to `bash2rust_ir`.

use bash2rust_ir::{
    AndOrItem, AndOrStmt, ArithExpr, ArithForStmt, ArrayItem, AssignTarget, AssignValue,
    Assignment, Block, CaseItem, CaseModifyMode, CaseStmt, CommandCall, ElifBranch, ForStmt,
    FunctionDef, IfStmt, IrProgram, LogicalOp, LoopKind, LoopStmt, ParamExpansionOp, PipelineStmt,
    Redirect, RedirectKind, SourcePos, SourceSpan, Stmt, TestStmt, VarQualifier, Word, WordPart,
};
use brush_parser::{ast, SourcePosition, SourceSpan as BrushSourceSpan};

use crate::error::LowerError;
use crate::test_expr::{lower_bracket_test, lower_extended_test};

/// Lowers a parsed [`ast::Program`] into a [`IrProgram`].
pub fn lower_program(
    program: &ast::Program,
    source_name: impl Into<String>,
) -> Result<IrProgram, LowerError> {
    let mut statements = Vec::new();
    for complete_cmd in &program.complete_commands {
        statements.extend(lower_compound_list(complete_cmd));
    }
    Ok(IrProgram {
        source: source_name.into(),
        statements,
    })
}

fn lower_pos(pos: &SourcePosition) -> SourcePos {
    SourcePos {
        index: pos.index,
        line: pos.line,
        column: pos.column,
    }
}

fn lower_span(span: &BrushSourceSpan) -> SourceSpan {
    SourceSpan {
        start: lower_pos(&span.start),
        end: lower_pos(&span.end),
    }
}

fn lower_word(word: &ast::Word) -> Word {
    let (parts, has_quotes, part_quoted) = parse_word_parts(&word.value);
    Word {
        raw: word.value.clone(),
        span: word.loc.as_ref().map(lower_span),
        parts,
        has_quotes,
        part_quoted,
    }
}

fn lower_word_with_braces(word: &ast::Word) -> Vec<Word> {
    let span = word.loc.as_ref().map(lower_span);
    expand_braces_raw(&word.value)
        .into_iter()
        .map(|raw| {
            let (parts, has_quotes, part_quoted) = parse_word_parts(&raw);
            Word {
                raw,
                span,
                parts,
                has_quotes,
                part_quoted,
            }
        })
        .collect()
}

fn expand_braces_raw(raw: &str) -> Vec<String> {
    if !raw.contains('{') {
        return vec![raw.to_owned()];
    }
    let bytes = raw.as_bytes();
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if b == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if b == b'\'' && !in_double {
            in_single = true;
            i += 1;
            continue;
        }
        if b == b'"' {
            in_double = !in_double;
            i += 1;
            continue;
        }
        if !in_double && b == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = find_matching_brace(bytes, i + 2) {
                i = end + 1;
                continue;
            }
        }
        if !in_double && b == b'{' {
            if let Some((items, end_idx)) = try_parse_brace_group(raw, i) {
                let prefix = &raw[..i];
                let suffix = &raw[end_idx + 1..];
                let mut out = Vec::new();
                for item in items {
                    let combined = format!("{prefix}{item}{suffix}");
                    out.extend(expand_braces_raw(&combined));
                }
                return out;
            }
        }
        i += 1;
    }
    vec![raw.to_owned()]
}

fn try_parse_brace_group(raw: &str, open_idx: usize) -> Option<(Vec<String>, usize)> {
    let bytes = raw.as_bytes();
    let mut i = open_idx + 1;
    let mut depth = 1usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut comma_positions = Vec::new();

    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if b == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if b == b'\'' && !in_double {
            in_single = true;
            i += 1;
            continue;
        }
        if b == b'"' {
            in_double = !in_double;
            i += 1;
            continue;
        }
        if !in_double && b == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = find_matching_brace(bytes, i + 2) {
                i = end + 1;
                continue;
            }
        }
        if !in_double {
            if b == b'{' {
                depth += 1;
            } else if b == b'}' {
                depth -= 1;
                if depth == 0 {
                    let inner = &raw[open_idx + 1..i];
                    if !comma_positions.is_empty() {
                        let mut items = Vec::new();
                        let mut prev = open_idx + 1;
                        for pos in comma_positions {
                            items.push(raw[prev..pos].to_owned());
                            prev = pos + 1;
                        }
                        items.push(raw[prev..i].to_owned());
                        return Some((items, i));
                    }
                    if let Some(range_items) = try_expand_brace_range(inner) {
                        return Some((range_items, i));
                    }
                    return None;
                }
            } else if b == b',' && depth == 1 {
                comma_positions.push(i);
            }
        }
        i += 1;
    }
    None
}

fn try_expand_brace_range(inner: &str) -> Option<Vec<String>> {
    let parts: Vec<&str> = inner.split("..").collect();
    if parts.len() != 2 && parts.len() != 3 {
        return None;
    }
    let start_s = parts[0];
    let end_s = parts[1];
    let step_opt = if parts.len() == 3 {
        let s = parts[2].parse::<i64>().ok()?;
        if s == 0 {
            return None;
        }
        Some(s.unsigned_abs() as i64)
    } else {
        None
    };

    if let (Ok(start), Ok(end)) = (start_s.parse::<i64>(), end_s.parse::<i64>()) {
        let step = step_opt.unwrap_or(1);
        let pad_width = [start_s, end_s]
            .iter()
            .filter_map(|s| {
                let digits = s.strip_prefix('-').unwrap_or(s);
                if digits.len() > 1 && digits.starts_with('0') {
                    Some(s.len())
                } else {
                    None
                }
            })
            .max()
            .unwrap_or(0);
        let mut out = Vec::new();
        let mut cur = start;
        if start <= end {
            while cur <= end && out.len() < 10_000 {
                if pad_width > 0 {
                    out.push(format!("{cur:0pad_width$}"));
                } else {
                    out.push(cur.to_string());
                }
                cur = cur.saturating_add(step);
            }
        } else {
            while cur >= end && out.len() < 10_000 {
                if pad_width > 0 {
                    out.push(format!("{cur:0pad_width$}"));
                } else {
                    out.push(cur.to_string());
                }
                cur = cur.saturating_sub(step);
            }
        }
        return Some(out);
    }

    if start_s.len() == 1 && end_s.len() == 1 {
        let sc = start_s.chars().next()?;
        let ec = end_s.chars().next()?;
        if sc.is_ascii_alphabetic() && ec.is_ascii_alphabetic() {
            let step = step_opt.unwrap_or(1) as u8;
            let sb = sc as u8;
            let eb = ec as u8;
            let mut out = Vec::new();
            if sb <= eb {
                let mut cur = sb;
                while cur <= eb {
                    out.push((cur as char).to_string());
                    cur = match cur.checked_add(step) {
                        Some(n) => n,
                        None => break,
                    };
                }
            } else {
                let mut cur = sb;
                while cur >= eb {
                    out.push((cur as char).to_string());
                    cur = match cur.checked_sub(step) {
                        Some(n) => n,
                        None => break,
                    };
                }
            }
            return Some(out);
        }
    }

    None
}

/// Scans a Bash word using segment-by-segment quote-state tokenization
/// (`Unquoted`, `SingleQuoted`, `DoubleQuoted`) for `$var`, `${var}`, `$?`, `$#`, and `$(...)`.
///
/// Single-quoted words `'...'` do not interpolate and return `Vec::new()`.
fn parse_word_parts(raw: &str) -> (Vec<WordPart>, bool, Vec<bool>) {
    parse_word_parts_impl(raw, false, false)
}

fn parse_word_parts_in_context(
    raw: &str,
    in_double_quote: bool,
) -> (Vec<WordPart>, bool, Vec<bool>) {
    parse_word_parts_impl(raw, in_double_quote, false)
}

fn parse_param_pattern_parts(raw: &str, in_double_quote: bool) -> (Vec<WordPart>, bool, Vec<bool>) {
    parse_word_parts_impl(raw, in_double_quote, true)
}

fn push_glob_escaped(dst: &mut String, s: &str) {
    for ch in s.chars() {
        if matches!(ch, '*' | '?' | '[' | ']') {
            dst.push('\\');
        }
        dst.push(ch);
    }
}

fn parse_word_parts_impl(
    raw: &str,
    in_double_quote: bool,
    escape_quoted_glob: bool,
) -> (Vec<WordPart>, bool, Vec<bool>) {
    if !in_double_quote
        && raw.len() >= 2
        && raw.starts_with('\'')
        && raw.ends_with('\'')
        && !raw[1..raw.len() - 1].contains('\'')
    {
        return (Vec::new(), true, Vec::new());
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum QuoteState {
        Unquoted,
        SingleQuoted,
        DoubleQuoted,
    }

    let bytes = raw.as_bytes();
    let mut parts: Vec<WordPart> = Vec::new();
    let mut part_quoted: Vec<bool> = Vec::new();
    let mut literal_buf = String::new();
    let mut literal_quoted = in_double_quote;
    let mut state = if in_double_quote {
        QuoteState::DoubleQuoted
    } else {
        QuoteState::Unquoted
    };
    let mut i = 0;
    let mut found_special = false;
    let mut has_quotes = in_double_quote;
    let mut quote_segments = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        match state {
            QuoteState::SingleQuoted => {
                if b == b'\'' {
                    state = QuoteState::Unquoted;
                    i += 1;
                } else {
                    let ch = raw[i..].chars().next().expect("valid char");
                    if escape_quoted_glob && matches!(ch, '*' | '?' | '[' | ']') {
                        literal_buf.push('\\');
                        found_special = true;
                    }
                    literal_buf.push(ch);
                    literal_quoted = true;
                    i += ch.len_utf8();
                }
            }
            QuoteState::DoubleQuoted => {
                if b == b'"' {
                    state = QuoteState::Unquoted;
                    i += 1;
                } else if b == b'\\' && i + 1 < bytes.len() {
                    let next = bytes[i + 1];
                    if matches!(next, b'"' | b'\\' | b'$' | b'`' | b'\n') {
                        literal_buf.push(next as char);
                        literal_quoted = true;
                        found_special = true;
                        i += 2;
                    } else {
                        literal_buf.push('\\');
                        let ch = raw[i + 1..].chars().next().expect("valid char");
                        literal_buf.push(ch);
                        literal_quoted = true;
                        i += 1 + ch.len_utf8();
                    }
                } else if in_double_quote
                    && b == b'$'
                    && i + 1 < bytes.len()
                    && bytes[i + 1] == b'\''
                {
                    found_special = true;
                    if escape_quoted_glob {
                        let mut decoded = String::new();
                        i = decode_ansi_c_quoted(raw, bytes, i + 2, &mut decoded);
                        push_glob_escaped(&mut literal_buf, &decoded);
                    } else {
                        i = decode_ansi_c_quoted(raw, bytes, i + 2, &mut literal_buf);
                    }
                } else if b == b'$' && i + 1 < bytes.len() {
                    if !literal_buf.is_empty() {
                        flush_literal(&mut literal_buf, &mut parts);
                        part_quoted.push(literal_quoted);
                        literal_quoted = false;
                    }
                    if let Some(next_idx) =
                        try_parse_dollar(raw, bytes, i, &mut literal_buf, &mut parts, true)
                    {
                        part_quoted.push(true);
                        found_special = true;
                        i = next_idx;
                    } else {
                        literal_buf.push('$');
                        literal_quoted = true;
                        i += 1;
                    }
                } else {
                    let ch = raw[i..].chars().next().expect("valid char");
                    literal_buf.push(ch);
                    literal_quoted = true;
                    i += ch.len_utf8();
                }
            }
            QuoteState::Unquoted => {
                if b == b'\'' {
                    if !literal_buf.is_empty() && !literal_quoted {
                        flush_literal(&mut literal_buf, &mut parts);
                        part_quoted.push(false);
                    }
                    state = QuoteState::SingleQuoted;
                    has_quotes = true;
                    quote_segments += 1;
                    if quote_segments > 1 || i > 0 {
                        found_special = true;
                    }
                    i += 1;
                } else if b == b'"' {
                    if !literal_buf.is_empty() && !literal_quoted {
                        flush_literal(&mut literal_buf, &mut parts);
                        part_quoted.push(false);
                    }
                    state = QuoteState::DoubleQuoted;
                    has_quotes = true;
                    quote_segments += 1;
                    if quote_segments > 1 || i > 0 {
                        found_special = true;
                    }
                    i += 1;
                } else if b == b'\\' && i + 1 < bytes.len() {
                    if !literal_buf.is_empty() && !literal_quoted {
                        flush_literal(&mut literal_buf, &mut parts);
                        part_quoted.push(false);
                    }
                    let ch = raw[i + 1..].chars().next().expect("valid char");
                    literal_buf.push(ch);
                    literal_quoted = true;
                    has_quotes = true;
                    found_special = true;
                    i += 1 + ch.len_utf8();
                } else if b == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    if !literal_buf.is_empty() && !literal_quoted {
                        flush_literal(&mut literal_buf, &mut parts);
                        part_quoted.push(false);
                    }
                    has_quotes = true;
                    found_special = true;
                    quote_segments += 1;
                    literal_quoted = true;
                    if escape_quoted_glob {
                        let mut decoded = String::new();
                        i = decode_ansi_c_quoted(raw, bytes, i + 2, &mut decoded);
                        push_glob_escaped(&mut literal_buf, &decoded);
                    } else {
                        i = decode_ansi_c_quoted(raw, bytes, i + 2, &mut literal_buf);
                    }
                } else if in_double_quote
                    && b == b'$'
                    && i + 1 < bytes.len()
                    && bytes[i + 1] == b'\''
                {
                    found_special = true;
                    if escape_quoted_glob {
                        let mut decoded = String::new();
                        i = decode_ansi_c_quoted(raw, bytes, i + 2, &mut decoded);
                        push_glob_escaped(&mut literal_buf, &decoded);
                    } else {
                        i = decode_ansi_c_quoted(raw, bytes, i + 2, &mut literal_buf);
                    }
                } else if b == b'$' && i + 1 < bytes.len() {
                    if !literal_buf.is_empty() {
                        flush_literal(&mut literal_buf, &mut parts);
                        part_quoted.push(literal_quoted);
                        literal_quoted = false;
                    }
                    if let Some(next_idx) =
                        try_parse_dollar(raw, bytes, i, &mut literal_buf, &mut parts, false)
                    {
                        part_quoted.push(false);
                        found_special = true;
                        i = next_idx;
                    } else {
                        if !literal_buf.is_empty() && literal_quoted {
                            flush_literal(&mut literal_buf, &mut parts);
                            part_quoted.push(true);
                            literal_quoted = false;
                        }
                        literal_buf.push('$');
                        i += 1;
                    }
                } else {
                    if !literal_buf.is_empty() && literal_quoted {
                        flush_literal(&mut literal_buf, &mut parts);
                        part_quoted.push(true);
                        literal_quoted = false;
                    }
                    let ch = raw[i..].chars().next().expect("valid char");
                    literal_buf.push(ch);
                    if quote_segments > 0 {
                        found_special = true;
                    }
                    i += ch.len_utf8();
                }
            }
        }
    }

    if !found_special {
        return (Vec::new(), has_quotes, Vec::new());
    }

    if !literal_buf.is_empty() {
        flush_literal(&mut literal_buf, &mut parts);
        part_quoted.push(literal_quoted);
    }
    if parts.is_empty() {
        parts.push(WordPart::Literal(String::new()));
        part_quoted.push(has_quotes);
    }
    (parts, has_quotes, part_quoted)
}

fn split_replace_parts(s: &str) -> (String, String) {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == b'/' {
            return (s[..i].to_owned(), s[i + 1..].to_owned());
        }
        i += 1;
    }
    (s.to_owned(), String::new())
}

fn unescape_param_pattern(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
            out.push_str("$'");
            i += 2;
            while i < bytes.len() {
                let b = bytes[i];
                if b == b'\\' && i + 1 < bytes.len() {
                    let ch = s[i + 1..].chars().next().expect("valid char");
                    out.push('\\');
                    out.push(ch);
                    i += 1 + ch.len_utf8();
                    continue;
                }
                let ch = s[i..].chars().next().expect("valid char");
                out.push(ch);
                i += ch.len_utf8();
                if b == b'\'' {
                    break;
                }
            }
            continue;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let next = s[i + 1..].chars().next().expect("valid char");
            if matches!(
                next,
                '#' | '%' | '/' | '*' | '?' | '[' | ']' | '\\' | '\'' | '"' | '{' | '}' | ';'
            ) {
                out.push(next);
            } else {
                out.push('\\');
                out.push(next);
            }
            i += 1 + next.len_utf8();
        } else {
            let ch = s[i..].chars().next().expect("valid char");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn decode_ansi_c_quoted(raw: &str, bytes: &[u8], mut i: usize, out: &mut String) -> usize {
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\'' {
            return i + 1;
        }
        if b == b'\\' && i + 1 < bytes.len() {
            i += 1;
            match bytes[i] {
                b'a' => {
                    out.push('\x07');
                    i += 1;
                }
                b'b' => {
                    out.push('\x08');
                    i += 1;
                }
                b'e' | b'E' => {
                    out.push('\x1b');
                    i += 1;
                }
                b'f' => {
                    out.push('\x0c');
                    i += 1;
                }
                b'n' => {
                    out.push('\n');
                    i += 1;
                }
                b'r' => {
                    out.push('\r');
                    i += 1;
                }
                b't' => {
                    out.push('\t');
                    i += 1;
                }
                b'v' => {
                    out.push('\x0b');
                    i += 1;
                }
                b'\\' => {
                    out.push('\\');
                    i += 1;
                }
                b'\'' => {
                    out.push('\'');
                    i += 1;
                }
                b'"' => {
                    out.push('"');
                    i += 1;
                }
                b'?' => {
                    out.push('?');
                    i += 1;
                }
                b'x' => {
                    i += 1;
                    let mut val = 0u8;
                    let mut count = 0;
                    while count < 2 && i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                        let d = (bytes[i] as char).to_digit(16).unwrap_or(0) as u8;
                        val = val * 16 + d;
                        i += 1;
                        count += 1;
                    }
                    if count > 0 {
                        out.push(val as char);
                    } else {
                        out.push_str("\\x");
                    }
                }
                b'u' | b'U' => {
                    let max_digits = if bytes[i] == b'u' { 4 } else { 8 };
                    let esc = bytes[i] as char;
                    i += 1;
                    let mut val = 0u32;
                    let mut count = 0;
                    while count < max_digits && i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                        let d = (bytes[i] as char).to_digit(16).unwrap_or(0);
                        val = val * 16 + d;
                        i += 1;
                        count += 1;
                    }
                    if count > 0 {
                        if let Some(ch) = char::from_u32(val) {
                            out.push(ch);
                        }
                    } else {
                        out.push('\\');
                        out.push(esc);
                    }
                }
                c if (b'0'..=b'7').contains(&c) => {
                    let mut val = 0u32;
                    let mut count = 0;
                    while count < 3 && i < bytes.len() && (b'0'..=b'7').contains(&bytes[i]) {
                        val = val * 8 + u32::from(bytes[i] - b'0');
                        i += 1;
                        count += 1;
                    }
                    out.push((val as u8) as char);
                }
                _ => {
                    let ch = raw[i..].chars().next().expect("valid char");
                    out.push('\\');
                    out.push(ch);
                    i += ch.len_utf8();
                }
            }
        } else {
            let ch = raw[i..].chars().next().expect("valid char");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    i
}

fn try_parse_dollar(
    raw: &str,
    bytes: &[u8],
    i: usize,
    literal_buf: &mut String,
    parts: &mut Vec<WordPart>,
    in_double_quote: bool,
) -> Option<usize> {
    let next = bytes[i + 1];

    // Arithmetic expansion `$((...))` and command substitution `$(...)`.
    if next == b'(' {
        if i + 2 < bytes.len() && bytes[i + 2] == b'(' {
            if let Some(end_idx) = find_matching_double_paren(bytes, i + 3) {
                let expr_src = &raw[i + 3..end_idx];
                flush_literal(literal_buf, parts);
                parts.push(WordPart::ArithExpansion(expr_src.to_owned()));
                return Some(end_idx + 2);
            }
            return None;
        }
        if let Some(end_idx) = find_matching_paren(bytes, i + 2) {
            let cmd_src = &raw[i + 2..end_idx];
            if let Ok(ir) = crate::parse_str_to_ir(cmd_src, "<command_subst>") {
                if ir.statements.len() == 1 {
                    if let Some(stmt) = ir.statements.into_iter().next() {
                        flush_literal(literal_buf, parts);
                        parts.push(WordPart::CommandSubst(Box::new(stmt)));
                        return Some(end_idx + 1);
                    }
                } else if ir.statements.len() > 1 {
                    flush_literal(literal_buf, parts);
                    parts.push(WordPart::CommandSubst(Box::new(Stmt::Block(Block::new(
                        ir.statements,
                    )))));
                    return Some(end_idx + 1);
                }
            }
        }
        return None;
    }

    // Braced parameter expansion `${var}`, `${#var}`, `${var:-default}`, `${var-default}`.
    if next == b'{' {
        if let Some(close_idx) = find_matching_brace(bytes, i + 2) {
            let inner = &raw[i + 2..close_idx];
            if let Some(var_name) = inner.strip_prefix('#') {
                if is_simple_var_ref(var_name)
                    || var_name.ends_with("[@]")
                    || var_name.ends_with("[*]")
                {
                    flush_literal(literal_buf, parts);
                    parts.push(WordPart::ParamExpansion {
                        name: var_name.to_owned(),
                        op: ParamExpansionOp::Length,
                    });
                    return Some(close_idx + 1);
                }
            }
            if let Some(target) = inner.strip_prefix('!') {
                if let Some(arr_name) = target
                    .strip_suffix("[@]")
                    .or_else(|| target.strip_suffix("[*]"))
                {
                    if is_simple_var_ref(arr_name) {
                        flush_literal(literal_buf, parts);
                        parts.push(WordPart::ParamExpansion {
                            name: arr_name.to_owned(),
                            op: ParamExpansionOp::ArrayKeys,
                        });
                        return Some(close_idx + 1);
                    }
                }
                if is_simple_var_ref(target) {
                    flush_literal(literal_buf, parts);
                    parts.push(WordPart::ParamExpansion {
                        name: target.to_owned(),
                        op: ParamExpansionOp::Indirect,
                    });
                    return Some(close_idx + 1);
                }
            }
            if is_simple_var_ref(inner) {
                flush_literal(literal_buf, parts);
                parts.push(WordPart::VarRef(inner.to_owned()));
                return Some(close_idx + 1);
            }
            if let Some(arr_name) = inner
                .strip_suffix("[@]")
                .or_else(|| inner.strip_suffix("[*]"))
            {
                if is_simple_var_ref(arr_name) {
                    flush_literal(literal_buf, parts);
                    parts.push(WordPart::ParamExpansion {
                        name: arr_name.to_owned(),
                        op: ParamExpansionOp::ArrayAll,
                    });
                    return Some(close_idx + 1);
                }
            }
            if let Some((arr_name, _after_plus)) = inner
                .split_once("[@]+")
                .or_else(|| inner.split_once("[*]+"))
            {
                if is_simple_var_ref(arr_name) {
                    flush_literal(literal_buf, parts);
                    parts.push(WordPart::ParamExpansion {
                        name: arr_name.to_owned(),
                        op: ParamExpansionOp::ArrayAllIfSet,
                    });
                    return Some(close_idx + 1);
                }
            }
            if let Some((arr_name, after_slash)) = inner
                .split_once("[@]//")
                .or_else(|| inner.split_once("[*]//"))
                .map(|(n, r)| (n, (true, r)))
                .or_else(|| {
                    inner
                        .split_once("[@]/")
                        .or_else(|| inner.split_once("[*]/"))
                        .map(|(n, r)| (n, (false, r)))
                })
            {
                if is_simple_var_ref(arr_name) {
                    let (replace_all, rest_s) = after_slash;
                    let (pat_raw, repl_raw) = split_replace_parts(rest_s);
                    let pat_clean = unescape_param_pattern(&pat_raw);
                    let (pp, phq, ppq) = parse_param_pattern_parts(&pat_clean, in_double_quote);
                    let (rp, rhq, rpq) = parse_word_parts_in_context(&repl_raw, in_double_quote);
                    flush_literal(literal_buf, parts);
                    parts.push(WordPart::ParamExpansion {
                        name: arr_name.to_owned(),
                        op: ParamExpansionOp::ArrayReplace {
                            pattern: Box::new(Word {
                                raw: pat_clean,
                                span: None,
                                parts: pp,
                                has_quotes: phq,
                                part_quoted: ppq,
                            }),
                            replacement: Box::new(Word {
                                raw: repl_raw,
                                span: None,
                                parts: rp,
                                has_quotes: rhq,
                                part_quoted: rpq,
                            }),
                            replace_all,
                        },
                    });
                    return Some(close_idx + 1);
                }
            }
            if let Some((arr_name, slice_spec)) = inner
                .split_once("[@]:")
                .or_else(|| inner.split_once("[*]:"))
            {
                if is_simple_var_ref(arr_name) {
                    let (off_s, len_opt_s) = match slice_spec.split_once(':') {
                        Some((o, l)) => (o.trim(), Some(l.trim())),
                        None => (slice_spec.trim(), None),
                    };
                    let (op, ohq, opq) = parse_word_parts_in_context(off_s, in_double_quote);
                    let offset = Box::new(Word {
                        raw: off_s.to_owned(),
                        span: None,
                        parts: op,
                        has_quotes: ohq,
                        part_quoted: opq,
                    });
                    let length = len_opt_s.map(|ls| {
                        let (lp, lhq, lpq) = parse_word_parts_in_context(ls, in_double_quote);
                        Box::new(Word {
                            raw: ls.to_owned(),
                            span: None,
                            parts: lp,
                            has_quotes: lhq,
                            part_quoted: lpq,
                        })
                    });
                    flush_literal(literal_buf, parts);
                    parts.push(WordPart::ParamExpansion {
                        name: arr_name.to_owned(),
                        op: ParamExpansionOp::ArraySlice { offset, length },
                    });
                    return Some(close_idx + 1);
                }
            }
            if let Some(br_open) = inner.find('[') {
                if let Some(br_close) = find_matching_bracket(inner.as_bytes(), br_open + 1) {
                    if br_open < br_close {
                        let name = &inner[..br_open];
                        let idx_src = inner[br_open + 1..br_close].trim();
                        if is_simple_var_ref(name)
                            && !idx_src.is_empty()
                            && idx_src != "@"
                            && idx_src != "*"
                        {
                            let (idx_parts, idx_has_quotes, idx_part_quoted) =
                                parse_word_parts_in_context(idx_src, in_double_quote);
                            let index = Box::new(Word {
                                raw: idx_src.to_owned(),
                                span: None,
                                parts: idx_parts,
                                has_quotes: idx_has_quotes,
                                part_quoted: idx_part_quoted,
                            });
                            let rest = &inner[br_close + 1..];
                            if rest.is_empty() {
                                flush_literal(literal_buf, parts);
                                parts.push(WordPart::ParamExpansion {
                                    name: name.to_owned(),
                                    op: ParamExpansionOp::ArrayElement { index },
                                });
                                return Some(close_idx + 1);
                            }
                            if let Some((pat_str, is_suffix, longest)) = rest
                                .strip_prefix("##")
                                .map(|p| (p, false, true))
                                .or_else(|| rest.strip_prefix('#').map(|p| (p, false, false)))
                                .or_else(|| rest.strip_prefix("%%").map(|p| (p, true, true)))
                                .or_else(|| rest.strip_prefix('%').map(|p| (p, true, false)))
                            {
                                let (pp, phq, ppq) =
                                    parse_word_parts_in_context(pat_str, in_double_quote);
                                let pattern = Box::new(Word {
                                    raw: pat_str.to_owned(),
                                    span: None,
                                    parts: pp,
                                    has_quotes: phq,
                                    part_quoted: ppq,
                                });
                                flush_literal(literal_buf, parts);
                                parts.push(WordPart::ParamExpansion {
                                    name: name.to_owned(),
                                    op: ParamExpansionOp::ArrayElemTrim {
                                        index,
                                        pattern,
                                        is_suffix,
                                        longest,
                                    },
                                });
                                return Some(close_idx + 1);
                            }
                            if let Some((replace_all, after_slash)) = rest
                                .strip_prefix("//")
                                .map(|r| (true, r))
                                .or_else(|| rest.strip_prefix('/').map(|r| (false, r)))
                            {
                                let (pat_raw, repl_raw) = split_replace_parts(after_slash);
                                let pat_clean = unescape_param_pattern(&pat_raw);
                                let (pp, phq, ppq) =
                                    parse_param_pattern_parts(&pat_clean, in_double_quote);
                                let (rp, rhq, rpq) =
                                    parse_word_parts_in_context(&repl_raw, in_double_quote);
                                flush_literal(literal_buf, parts);
                                parts.push(WordPart::ParamExpansion {
                                    name: name.to_owned(),
                                    op: ParamExpansionOp::ArrayElemReplace {
                                        index,
                                        pattern: Box::new(Word {
                                            raw: pat_clean,
                                            span: None,
                                            parts: pp,
                                            has_quotes: phq,
                                            part_quoted: ppq,
                                        }),
                                        replacement: Box::new(Word {
                                            raw: repl_raw,
                                            span: None,
                                            parts: rp,
                                            has_quotes: rhq,
                                            part_quoted: rpq,
                                        }),
                                        replace_all,
                                    },
                                });
                                return Some(close_idx + 1);
                            }
                            if let Some(slice_spec) = rest
                                .strip_prefix(':')
                                .filter(|s| !s.starts_with(['+', '-', '=', '?']))
                            {
                                let (off_s, len_opt_s) = match slice_spec.split_once(':') {
                                    Some((o, l)) => (o.trim(), Some(l.trim())),
                                    None => (slice_spec.trim(), None),
                                };
                                let (op, ohq, opq) =
                                    parse_word_parts_in_context(off_s, in_double_quote);
                                let offset = Box::new(Word {
                                    raw: off_s.to_owned(),
                                    span: None,
                                    parts: op,
                                    has_quotes: ohq,
                                    part_quoted: opq,
                                });
                                let length = len_opt_s.map(|ls| {
                                    let (lp, lhq, lpq) =
                                        parse_word_parts_in_context(ls, in_double_quote);
                                    Box::new(Word {
                                        raw: ls.to_owned(),
                                        span: None,
                                        parts: lp,
                                        has_quotes: lhq,
                                        part_quoted: lpq,
                                    })
                                });
                                flush_literal(literal_buf, parts);
                                parts.push(WordPart::ParamExpansion {
                                    name: name.to_owned(),
                                    op: ParamExpansionOp::ArrayElemSubstring {
                                        index,
                                        offset,
                                        length,
                                    },
                                });
                                return Some(close_idx + 1);
                            }
                        }
                    }
                }
            }
            let op_pos_opt = if inner.len() > 1
                && matches!(inner.as_bytes()[0], b'*' | b'@' | b'?' | b'#')
                && inner.as_bytes()[1] != b'['
            {
                Some(1usize)
            } else {
                inner
                    .bytes()
                    .position(|b| !(b.is_ascii_alphanumeric() || b == b'_'))
            };
            if let Some(op_pos) = op_pos_opt {
                let name = &inner[..op_pos];
                let rest = &inner[op_pos..];
                if is_simple_var_ref(name) {
                    if let Some(def_str) =
                        rest.strip_prefix(":-").or_else(|| rest.strip_prefix('-'))
                    {
                        let colon = rest.starts_with(":-");
                        let (def_parts, def_has_quotes, def_part_quoted) =
                            parse_word_parts_in_context(def_str, in_double_quote);
                        let default = Word {
                            raw: def_str.to_owned(),
                            span: None,
                            parts: def_parts,
                            has_quotes: def_has_quotes,
                            part_quoted: def_part_quoted,
                        };
                        flush_literal(literal_buf, parts);
                        parts.push(WordPart::ParamExpansion {
                            name: name.to_owned(),
                            op: ParamExpansionOp::UseDefault { default, colon },
                        });
                        return Some(close_idx + 1);
                    }
                    if let Some(def_str) =
                        rest.strip_prefix(":=").or_else(|| rest.strip_prefix('='))
                    {
                        let colon = rest.starts_with(":=");
                        let (def_parts, def_has_quotes, def_part_quoted) =
                            parse_word_parts_in_context(def_str, in_double_quote);
                        let default = Word {
                            raw: def_str.to_owned(),
                            span: None,
                            parts: def_parts,
                            has_quotes: def_has_quotes,
                            part_quoted: def_part_quoted,
                        };
                        flush_literal(literal_buf, parts);
                        parts.push(WordPart::ParamExpansion {
                            name: name.to_owned(),
                            op: ParamExpansionOp::AssignDefault { default, colon },
                        });
                        return Some(close_idx + 1);
                    }
                    if let Some(alt_str) =
                        rest.strip_prefix(":+").or_else(|| rest.strip_prefix('+'))
                    {
                        let colon = rest.starts_with(":+");
                        let (alt_parts, alt_has_quotes, alt_part_quoted) =
                            parse_word_parts_in_context(alt_str, in_double_quote);
                        let alt = Word {
                            raw: alt_str.to_owned(),
                            span: None,
                            parts: alt_parts,
                            has_quotes: alt_has_quotes,
                            part_quoted: alt_part_quoted,
                        };
                        flush_literal(literal_buf, parts);
                        parts.push(WordPart::ParamExpansion {
                            name: name.to_owned(),
                            op: ParamExpansionOp::UseAlt { alt, colon },
                        });
                        return Some(close_idx + 1);
                    }
                    if let Some((pat_str, is_suffix, longest)) = rest
                        .strip_prefix("##")
                        .map(|p| (p, false, true))
                        .or_else(|| rest.strip_prefix('#').map(|p| (p, false, false)))
                        .or_else(|| rest.strip_prefix("%%").map(|p| (p, true, true)))
                        .or_else(|| rest.strip_prefix('%').map(|p| (p, true, false)))
                    {
                        let (pp, phq, ppq) = parse_word_parts_in_context(pat_str, in_double_quote);
                        let pattern = Box::new(Word {
                            raw: pat_str.to_owned(),
                            span: None,
                            parts: pp,
                            has_quotes: phq,
                            part_quoted: ppq,
                        });
                        flush_literal(literal_buf, parts);
                        parts.push(WordPart::ParamExpansion {
                            name: name.to_owned(),
                            op: ParamExpansionOp::Trim {
                                pattern,
                                is_suffix,
                                longest,
                            },
                        });
                        return Some(close_idx + 1);
                    }
                    if let Some((replace_all, after_slash)) = rest
                        .strip_prefix("//")
                        .map(|r| (true, r))
                        .or_else(|| rest.strip_prefix('/').map(|r| (false, r)))
                    {
                        let (pat_raw, repl_raw) = split_replace_parts(after_slash);
                        let pat_clean = unescape_param_pattern(&pat_raw);
                        let (pp, phq, ppq) = parse_param_pattern_parts(&pat_clean, in_double_quote);
                        let (rp, rhq, rpq) =
                            parse_word_parts_in_context(&repl_raw, in_double_quote);
                        flush_literal(literal_buf, parts);
                        parts.push(WordPart::ParamExpansion {
                            name: name.to_owned(),
                            op: ParamExpansionOp::Replace {
                                pattern: Box::new(Word {
                                    raw: pat_clean,
                                    span: None,
                                    parts: pp,
                                    has_quotes: phq,
                                    part_quoted: ppq,
                                }),
                                replacement: Box::new(Word {
                                    raw: repl_raw,
                                    span: None,
                                    parts: rp,
                                    has_quotes: rhq,
                                    part_quoted: rpq,
                                }),
                                replace_all,
                            },
                        });
                        return Some(close_idx + 1);
                    }
                    if let Some((mode, pat_s)) = rest
                        .strip_prefix(",,")
                        .map(|p| (CaseModifyMode::LowerAll, p))
                        .or_else(|| {
                            rest.strip_prefix(',')
                                .map(|p| (CaseModifyMode::LowerFirst, p))
                        })
                        .or_else(|| {
                            rest.strip_prefix("^^")
                                .map(|p| (CaseModifyMode::UpperAll, p))
                        })
                        .or_else(|| {
                            rest.strip_prefix('^')
                                .map(|p| (CaseModifyMode::UpperFirst, p))
                        })
                        .or_else(|| {
                            rest.strip_prefix("~~")
                                .map(|p| (CaseModifyMode::ToggleAll, p))
                        })
                        .or_else(|| {
                            rest.strip_prefix('~')
                                .map(|p| (CaseModifyMode::ToggleFirst, p))
                        })
                    {
                        let pattern = if pat_s.is_empty() {
                            None
                        } else {
                            let (pp, phq, ppq) =
                                parse_word_parts_in_context(pat_s, in_double_quote);
                            Some(Box::new(Word {
                                raw: pat_s.to_owned(),
                                span: None,
                                parts: pp,
                                has_quotes: phq,
                                part_quoted: ppq,
                            }))
                        };
                        flush_literal(literal_buf, parts);
                        parts.push(WordPart::ParamExpansion {
                            name: name.to_owned(),
                            op: ParamExpansionOp::CaseModify { mode, pattern },
                        });
                        return Some(close_idx + 1);
                    }
                    if let Some(slice_spec) = rest
                        .strip_prefix(':')
                        .filter(|s| !s.starts_with(['+', '-', '=', '?']))
                    {
                        let (off_s, len_opt_s) = match slice_spec.split_once(':') {
                            Some((o, l)) => (o.trim(), Some(l.trim())),
                            None => (slice_spec.trim(), None),
                        };
                        let (op, ohq, opq) = parse_word_parts_in_context(off_s, in_double_quote);
                        let offset = Box::new(Word {
                            raw: off_s.to_owned(),
                            span: None,
                            parts: op,
                            has_quotes: ohq,
                            part_quoted: opq,
                        });
                        let length = len_opt_s.map(|ls| {
                            let (lp, lhq, lpq) = parse_word_parts_in_context(ls, in_double_quote);
                            Box::new(Word {
                                raw: ls.to_owned(),
                                span: None,
                                parts: lp,
                                has_quotes: lhq,
                                part_quoted: lpq,
                            })
                        });
                        flush_literal(literal_buf, parts);
                        parts.push(WordPart::ParamExpansion {
                            name: name.to_owned(),
                            op: ParamExpansionOp::Substring { offset, length },
                        });
                        return Some(close_idx + 1);
                    }
                }
            }
        }
        return None;
    }

    // Unbraced variable `$VAR` or positional/special `$1`, `$@`, `$*`, `$#`, `$?`.
    if next.is_ascii_alphabetic() || next == b'_' {
        let mut end = i + 2;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        let var_name = &raw[i + 1..end];
        flush_literal(literal_buf, parts);
        parts.push(WordPart::VarRef(var_name.to_owned()));
        return Some(end);
    }

    if next.is_ascii_digit() || matches!(next, b'@' | b'*' | b'#' | b'?') {
        let var_name = &raw[i + 1..i + 2];
        flush_literal(literal_buf, parts);
        parts.push(WordPart::VarRef(var_name.to_owned()));
        return Some(i + 2);
    }

    None
}

fn find_matching_double_paren(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut idx = start;
    while idx < bytes.len() {
        match bytes[idx] {
            b'(' => {
                depth += 1;
                idx += 1;
            }
            b')' => {
                if depth == 0 && idx + 1 < bytes.len() && bytes[idx + 1] == b')' {
                    return Some(idx);
                }
                depth = depth.saturating_sub(1);
                idx += 1;
            }
            _ => idx += 1,
        }
    }
    None
}

fn find_matching_bracket(bytes: &[u8], mut i: usize) -> Option<usize> {
    let mut depth = 1usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = find_matching_brace(bytes, i + 2) {
                i = end + 1;
                continue;
            }
        }
        if bytes[i] == b'[' {
            depth += 1;
        } else if bytes[i] == b']' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn find_matching_brace(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut idx = start;
    while idx < bytes.len() {
        match bytes[idx] {
            b'\\' if idx + 1 < bytes.len() => idx += 2,
            b'{' => {
                depth += 1;
                idx += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
                idx += 1;
            }
            _ => idx += 1,
        }
    }
    None
}

fn flush_literal(buf: &mut String, parts: &mut Vec<WordPart>) {
    if !buf.is_empty() {
        parts.push(WordPart::Literal(std::mem::take(buf)));
    }
}

fn is_simple_var_ref(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if matches!(name, "@" | "*" | "#" | "?") {
        return true;
    }
    if name.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    let mut chars = name.chars();
    let first = chars.next().expect("non-empty");
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn find_matching_paren(bytes: &[u8], mut i: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut escaped = false;
    let mut in_single = false;
    let mut in_double = false;

    while i < bytes.len() {
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        let b = bytes[i];
        if b == b'\\' && !in_single {
            escaped = true;
            i += 1;
            continue;
        }
        if b == b'\'' && !in_double {
            in_single = !in_single;
            i += 1;
            continue;
        }
        if b == b'"' && !in_single {
            in_double = !in_double;
            i += 1;
            continue;
        }
        if !in_single && !in_double {
            if b == b'(' {
                depth += 1;
            } else if b == b')' {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

fn lower_arith(expr: &ast::UnexpandedArithmeticExpr) -> ArithExpr {
    ArithExpr {
        raw: expr.value.clone(),
    }
}

fn lower_compound_list(list: &ast::CompoundList) -> Vec<Stmt> {
    list.0
        .iter()
        .map(|item| lower_and_or_list(&item.0))
        .collect()
}

fn lower_and_or_list(list: &ast::AndOrList) -> Stmt {
    let first = lower_pipeline(&list.first);
    if list.additional.is_empty() {
        return first;
    }

    let rest = list
        .additional
        .iter()
        .map(|item| match item {
            ast::AndOr::And(pipeline) => AndOrItem {
                op: LogicalOp::And,
                rhs: lower_pipeline(pipeline),
            },
            ast::AndOr::Or(pipeline) => AndOrItem {
                op: LogicalOp::Or,
                rhs: lower_pipeline(pipeline),
            },
        })
        .collect();

    Stmt::AndOr(AndOrStmt {
        first: Box::new(first),
        rest,
    })
}

fn lower_pipeline(pipeline: &ast::Pipeline) -> Stmt {
    if !pipeline.bang && pipeline.timed.is_none() && pipeline.seq.len() == 1 {
        return lower_command(&pipeline.seq[0]);
    }

    let commands = pipeline.seq.iter().map(lower_command).collect();
    Stmt::Pipeline(PipelineStmt {
        negated: pipeline.bang,
        commands,
    })
}

fn lower_command(cmd: &ast::Command) -> Stmt {
    match cmd {
        ast::Command::Simple(simple) => lower_simple_command(simple),
        ast::Command::Compound(compound, redirects) => {
            let mut stmt = lower_compound_command(compound);
            if let Some(redir_list) = redirects {
                let mut lowered = Vec::new();
                for redir in &redir_list.0 {
                    lower_io_redirects(redir, &mut lowered);
                }
                match &mut stmt {
                    Stmt::Loop(loop_stmt) => loop_stmt.redirects = lowered,
                    Stmt::Block(block) => block.redirects = lowered,
                    _ => {}
                }
            }
            stmt
        }
        ast::Command::Function(func) => {
            let body_stmts = match &func.body.0 {
                ast::CompoundCommand::BraceGroup(bg) => lower_compound_list(&bg.list),
                other => vec![lower_compound_command(other)],
            };
            Stmt::Function(FunctionDef {
                name: func.fname.value.clone(),
                body: Block::new(body_stmts),
                span: func.fname.loc.as_ref().map(lower_span),
            })
        }
        ast::Command::ExtendedTest(ext_cmd, _redirects) => {
            if let Some(expr) = lower_extended_test(&ext_cmd.expr, &lower_word) {
                Stmt::Test(TestStmt {
                    expr,
                    span: Some(lower_span(&ext_cmd.loc)),
                })
            } else {
                Stmt::Unsupported {
                    construct: "extended_test".to_owned(),
                    raw: format!("{ext_cmd}"),
                    span: Some(lower_span(&ext_cmd.loc)),
                }
            }
        }
    }
}

fn lower_compound_command(compound: &ast::CompoundCommand) -> Stmt {
    match compound {
        ast::CompoundCommand::IfClause(if_cmd) => lower_if_clause(if_cmd),
        ast::CompoundCommand::CaseClause(case_cmd) => Stmt::Case(lower_case_clause(case_cmd)),
        ast::CompoundCommand::ForClause(for_cmd) => Stmt::For(ForStmt {
            variable: for_cmd.variable_name.clone(),
            values: for_cmd
                .values
                .as_ref()
                .map(|vals| vals.iter().flat_map(lower_word_with_braces).collect()),
            body: Block::new(lower_compound_list(&for_cmd.body.list)),
            span: Some(lower_span(&for_cmd.loc)),
        }),
        ast::CompoundCommand::ArithmeticForClause(arith_for) => Stmt::ArithFor(ArithForStmt {
            initializer: arith_for.initializer.as_ref().map(lower_arith),
            condition: arith_for.condition.as_ref().map(lower_arith),
            updater: arith_for.updater.as_ref().map(lower_arith),
            body: Block::new(lower_compound_list(&arith_for.body.list)),
            span: Some(lower_span(&arith_for.loc)),
        }),
        ast::CompoundCommand::Arithmetic(arith_cmd) => Stmt::Arith(lower_arith(&arith_cmd.expr)),
        ast::CompoundCommand::BraceGroup(bg) => {
            Stmt::Block(Block::new(lower_compound_list(&bg.list)))
        }
        ast::CompoundCommand::Subshell(sub) => {
            Stmt::Block(Block::subshell(lower_compound_list(&sub.list)))
        }
        ast::CompoundCommand::WhileClause(while_cmd) => Stmt::Loop(LoopStmt {
            loop_kind: LoopKind::While,
            condition: Block::new(lower_compound_list(&while_cmd.0)),
            body: Block::new(lower_compound_list(&while_cmd.1.list)),
            redirects: Vec::new(),
            span: Some(lower_span(&while_cmd.2)),
        }),
        ast::CompoundCommand::UntilClause(until_cmd) => Stmt::Loop(LoopStmt {
            loop_kind: LoopKind::Until,
            condition: Block::new(lower_compound_list(&until_cmd.0)),
            body: Block::new(lower_compound_list(&until_cmd.1.list)),
            redirects: Vec::new(),
            span: Some(lower_span(&until_cmd.2)),
        }),
        other => Stmt::Unsupported {
            construct: "compound_command".to_owned(),
            raw: format!("{other}"),
            span: None,
        },
    }
}

fn lower_if_clause(if_cmd: &ast::IfClauseCommand) -> Stmt {
    let condition = Block::new(lower_compound_list(&if_cmd.condition));
    let then_branch = Block::new(lower_compound_list(&if_cmd.then));

    let mut elif_branches = Vec::new();
    let mut else_branch = None;

    if let Some(elses) = &if_cmd.elses {
        for clause in elses {
            let body = Block::new(lower_compound_list(&clause.body));
            match &clause.condition {
                Some(cond) => elif_branches.push(ElifBranch {
                    condition: Block::new(lower_compound_list(cond)),
                    body,
                }),
                None => {
                    else_branch = Some(body);
                }
            }
        }
    }

    Stmt::If(IfStmt {
        condition,
        then_branch,
        elif_branches,
        else_branch,
        span: Some(lower_span(&if_cmd.loc)),
    })
}

fn lower_case_clause(case_cmd: &ast::CaseClauseCommand) -> CaseStmt {
    let items = case_cmd
        .cases
        .iter()
        .map(|item| CaseItem {
            patterns: item.patterns.iter().map(lower_word).collect(),
            body: Block::new(
                item.cmd
                    .as_ref()
                    .map(lower_compound_list)
                    .unwrap_or_default(),
            ),
        })
        .collect();
    CaseStmt {
        value: lower_word(&case_cmd.value),
        items,
        span: Some(lower_span(&case_cmd.loc)),
    }
}

fn lower_simple_command(simple: &ast::SimpleCommand) -> Stmt {
    let mut env_assignments = Vec::new();
    let mut args = Vec::new();
    let mut raw_arg_words = Vec::new();
    let mut redirects = Vec::new();

    if let Some(prefix) = &simple.prefix {
        for item in &prefix.0 {
            match item {
                ast::CommandPrefixOrSuffixItem::AssignmentWord(assign, _) => {
                    env_assignments.push(lower_assignment(assign, VarQualifier::Global));
                }
                ast::CommandPrefixOrSuffixItem::IoRedirect(redir) => {
                    lower_io_redirects(redir, &mut redirects);
                }
                _ => {}
            }
        }
    }

    // Pure variable assignment(s) (e.g. `x=1` or `IFS=" " arr[k]=1` with no command word).
    if simple.word_or_name.is_none()
        && !env_assignments.is_empty()
        && redirects.is_empty()
        && simple
            .suffix
            .as_ref()
            .is_none_or(|suffix| suffix.0.is_empty())
    {
        if env_assignments.len() == 1 {
            return Stmt::Assign(env_assignments.remove(0));
        }
        return Stmt::Block(Block::new(
            env_assignments.into_iter().map(Stmt::Assign).collect(),
        ));
    }

    // Declaration builtins (`local x=1`, `export x=1`, `readonly x=1`).
    if let Some(cmd_word) = &simple.word_or_name {
        if let Some(qualifier) = match cmd_word.value.as_str() {
            "local" => Some(VarQualifier::Local),
            "export" => Some(VarQualifier::Export),
            "readonly" => Some(VarQualifier::Readonly),
            _ => None,
        } {
            if let Some(suffix) = &simple.suffix {
                if suffix.0.len() == 1 {
                    match &suffix.0[0] {
                        ast::CommandPrefixOrSuffixItem::AssignmentWord(assign, _) => {
                            return Stmt::Assign(lower_assignment(assign, qualifier));
                        }
                        ast::CommandPrefixOrSuffixItem::Word(w)
                            if qualifier == VarQualifier::Local =>
                        {
                            return Stmt::Assign(Assignment {
                                qualifier: VarQualifier::Local,
                                target: AssignTarget::Variable {
                                    name: w.value.clone(),
                                },
                                value: AssignValue::Scalar(Word {
                                    raw: String::new(),
                                    span: w.loc.as_ref().map(lower_span),
                                    parts: Vec::new(),
                                    has_quotes: false,
                                    part_quoted: Vec::new(),
                                }),
                                append: false,
                                span: w.loc.as_ref().map(lower_span),
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    if let Some(suffix) = &simple.suffix {
        for item in &suffix.0 {
            match item {
                ast::CommandPrefixOrSuffixItem::Word(word) => {
                    args.extend(lower_word_with_braces(word));
                    raw_arg_words.push(word.clone());
                }
                ast::CommandPrefixOrSuffixItem::AssignmentWord(_, word) => {
                    args.extend(lower_word_with_braces(word));
                    raw_arg_words.push(word.clone());
                }
                ast::CommandPrefixOrSuffixItem::IoRedirect(redir) => {
                    lower_io_redirects(redir, &mut redirects);
                }
                _ => {}
            }
        }
    }

    // Complex shell builtins (`eval`) fall back to `Stmt::Unsupported` for hybrid AI translation.
    if let Some(cmd_word) = &simple.word_or_name {
        if cmd_word.value == "eval" {
            let mut parts = vec![cmd_word.value.clone()];
            for w in &raw_arg_words {
                parts.push(w.value.clone());
            }
            return Stmt::Unsupported {
                construct: cmd_word.value.clone(),
                raw: parts.join(" "),
                span: cmd_word.loc.as_ref().map(lower_span),
            };
        }
        if cmd_word.value == "return" && redirects.is_empty() && env_assignments.is_empty() {
            return Stmt::Return {
                code: args.into_iter().next(),
                span: cmd_word.loc.as_ref().map(lower_span),
            };
        }
        if matches!(cmd_word.value.as_str(), "source" | ".")
            && !args.is_empty()
            && redirects.is_empty()
            && env_assignments.is_empty()
        {
            let mut arg_iter = args.iter().cloned();
            if let Some(path) = arg_iter.next() {
                return Stmt::Source {
                    path,
                    args: arg_iter.collect(),
                    body: None,
                    span: cmd_word.loc.as_ref().map(lower_span),
                };
            }
        }
    }

    // Lower `[ ... ]` and `test ...` into `Stmt::Test`.
    if let Some(cmd_word) = &simple.word_or_name {
        let test_slice =
            if cmd_word.value == "[" && raw_arg_words.last().is_some_and(|w| w.value == "]") {
                Some(&raw_arg_words[..raw_arg_words.len() - 1])
            } else if cmd_word.value == "test" {
                Some(raw_arg_words.as_slice())
            } else {
                None
            };

        if let Some(slice) = test_slice {
            if let Some(expr) = lower_bracket_test(slice, &lower_word) {
                return Stmt::Test(TestStmt {
                    expr,
                    span: cmd_word.loc.as_ref().map(lower_span),
                });
            }
        }
    }

    let name = simple.word_or_name.as_ref().map(lower_word);
    let span = name.as_ref().and_then(|w| w.span);

    Stmt::Command(CommandCall {
        name,
        args,
        env_assignments,
        redirects,
        span,
    })
}

fn empty_word(raw: impl Into<String>) -> Word {
    Word {
        raw: raw.into(),
        span: None,
        parts: Vec::new(),
        has_quotes: false,
        part_quoted: Vec::new(),
    }
}

fn lower_io_redirects(redir: &ast::IoRedirect, out: &mut Vec<Redirect>) {
    if let ast::IoRedirect::OutputAndError(target, append) = redir {
        out.push(Redirect {
            fd: Some(1),
            kind: if *append {
                RedirectKind::OutputAppend
            } else {
                RedirectKind::OutputTruncate
            },
            target: lower_word(target),
            proc_subst: None,
        });
        out.push(Redirect {
            fd: Some(2),
            kind: RedirectKind::DupOutput { target_fd: 1 },
            target: empty_word("1"),
            proc_subst: None,
        });
        return;
    }
    if let Some(r) = lower_io_redirect(redir) {
        out.push(r);
    }
}

fn lower_io_redirect(redir: &ast::IoRedirect) -> Option<Redirect> {
    match redir {
        ast::IoRedirect::HereString(fd, word) => Some(Redirect {
            fd: fd.and_then(|v| u32::try_from(v).ok()),
            kind: RedirectKind::HereString,
            target: lower_word(word),
            proc_subst: None,
        }),
        ast::IoRedirect::HereDocument(fd, doc) => {
            let mut body_text = doc.doc.value.clone();
            if doc.remove_tabs {
                body_text = body_text
                    .split_inclusive('\n')
                    .map(|line| line.trim_start_matches('\t'))
                    .collect();
            }
            let target = if doc.requires_expansion {
                let mut w = doc.doc.clone();
                w.value = body_text;
                let mut lowered = lower_word(&w);
                lowered.has_quotes = true;
                lowered
            } else {
                Word {
                    raw: body_text.clone(),
                    span: doc.doc.loc.as_ref().map(lower_span),
                    parts: vec![WordPart::Literal(body_text)],
                    has_quotes: true,
                    part_quoted: vec![true],
                }
            };
            Some(Redirect {
                fd: fd.and_then(|v| u32::try_from(v).ok()),
                kind: RedirectKind::HereDoc {
                    strip_tabs: doc.remove_tabs,
                },
                target,
                proc_subst: None,
            })
        }
        ast::IoRedirect::File(fd, kind, target) => {
            let fd_u32 = fd.and_then(|v| u32::try_from(v).ok());
            match kind {
                ast::IoFileRedirectKind::Write | ast::IoFileRedirectKind::Clobber => {
                    if let ast::IoFileRedirectTarget::Filename(w) = target {
                        Some(Redirect {
                            fd: fd_u32,
                            kind: RedirectKind::OutputTruncate,
                            target: lower_word(w),
                            proc_subst: None,
                        })
                    } else {
                        None
                    }
                }
                ast::IoFileRedirectKind::Append => {
                    if let ast::IoFileRedirectTarget::Filename(w) = target {
                        Some(Redirect {
                            fd: fd_u32,
                            kind: RedirectKind::OutputAppend,
                            target: lower_word(w),
                            proc_subst: None,
                        })
                    } else {
                        None
                    }
                }
                ast::IoFileRedirectKind::ReadAndWrite => match target {
                    ast::IoFileRedirectTarget::ProcessSubstitution(
                        ast::ProcessSubstitutionKind::Read,
                        subshell_cmd,
                    ) => {
                        let stmts = lower_compound_list(&subshell_cmd.list);
                        let inner_stmt = if stmts.len() == 1 {
                            stmts.into_iter().next().unwrap()
                        } else {
                            Stmt::Block(Block::new(stmts))
                        };
                        Some(Redirect {
                            fd: fd_u32,
                            kind: RedirectKind::ReadWriteProcSubst,
                            target: empty_word(format!("{subshell_cmd}")),
                            proc_subst: Some(Box::new(inner_stmt)),
                        })
                    }
                    ast::IoFileRedirectTarget::Filename(w) => Some(Redirect {
                        fd: fd_u32,
                        kind: RedirectKind::Input,
                        target: lower_word(w),
                        proc_subst: None,
                    }),
                    _ => None,
                },
                ast::IoFileRedirectKind::Read => match target {
                    ast::IoFileRedirectTarget::Filename(w) => Some(Redirect {
                        fd: fd_u32,
                        kind: RedirectKind::Input,
                        target: lower_word(w),
                        proc_subst: None,
                    }),
                    ast::IoFileRedirectTarget::ProcessSubstitution(
                        ast::ProcessSubstitutionKind::Read,
                        subshell_cmd,
                    ) => {
                        let stmts = lower_compound_list(&subshell_cmd.list);
                        let inner_stmt = if stmts.len() == 1 {
                            stmts.into_iter().next().unwrap()
                        } else {
                            Stmt::Block(Block::new(stmts))
                        };
                        Some(Redirect {
                            fd: fd_u32,
                            kind: RedirectKind::InputProcSubst,
                            target: empty_word(format!("{subshell_cmd}")),
                            proc_subst: Some(Box::new(inner_stmt)),
                        })
                    }
                    _ => None,
                },
                ast::IoFileRedirectKind::DuplicateOutput => match target {
                    ast::IoFileRedirectTarget::Fd(t) => {
                        u32::try_from(*t).ok().map(|target_fd| Redirect {
                            fd: fd_u32,
                            kind: RedirectKind::DupOutput { target_fd },
                            target: empty_word(target_fd.to_string()),
                            proc_subst: None,
                        })
                    }
                    ast::IoFileRedirectTarget::Duplicate(w) => {
                        let trimmed = w.value.trim();
                        if trimmed == "-" {
                            Some(Redirect {
                                fd: fd_u32,
                                kind: RedirectKind::CloseFd,
                                target: lower_word(w),
                                proc_subst: None,
                            })
                        } else {
                            trimmed.parse::<u32>().ok().map(|target_fd| Redirect {
                                fd: fd_u32,
                                kind: RedirectKind::DupOutput { target_fd },
                                target: lower_word(w),
                                proc_subst: None,
                            })
                        }
                    }
                    _ => None,
                },
                _ => None,
            }
        }
        _ => None,
    }
}

fn lower_assignment(assign: &ast::Assignment, qualifier: VarQualifier) -> Assignment {
    let target = match &assign.name {
        ast::AssignmentName::VariableName(name) => AssignTarget::Variable { name: name.clone() },
        ast::AssignmentName::ArrayElementName(name, index) => AssignTarget::ArrayElement {
            name: name.clone(),
            index: index.clone(),
        },
    };

    let value = match &assign.value {
        ast::AssignmentValue::Scalar(word) => AssignValue::Scalar(lower_word(word)),
        ast::AssignmentValue::Array(items) => AssignValue::Array(
            items
                .iter()
                .map(|(key, val)| ArrayItem {
                    key: key.as_ref().map(lower_word),
                    value: lower_word(val),
                })
                .collect(),
        ),
    };

    Assignment {
        target,
        value,
        append: assign.append,
        qualifier,
        span: Some(lower_span(&assign.loc)),
    }
}
