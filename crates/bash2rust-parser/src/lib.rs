//! Brush-based Bash parser and AST-to-IR lowering for `bash2rust`.

pub mod error;
pub mod lower;
mod test_expr;

use std::io::BufReader;
use std::path::Path;

use bash2rust_ir::IrProgram;
use brush_parser::{ast, Parser, ParserOptions, SourceInfo};

pub use error::{format_unsupported_diagnostic, BashDiagnostic, LowerError, ParserError};
pub use lower::lower_program;

/// A parsed Bash script retaining its [`SourceInfo`] and [`ast::Program`].
#[derive(Debug, Clone)]
pub struct ParsedScript {
    pub source_info: SourceInfo,
    pub program: ast::Program,
}

/// Parses an in-memory Bash source string using `brush-parser`.
pub fn parse_str(
    source: &str,
    source_name: impl Into<String>,
) -> Result<ParsedScript, ParserError> {
    let source_name_str = source_name.into();
    let normalized = preprocess_bash_source(source);
    let options = ParserOptions::default();
    let reader = BufReader::new(normalized.as_bytes());
    let mut parser = Parser::new(reader, &options);
    let program = parser.parse_program().map_err(|err| {
        ParserError::RichParse(Box::new(BashDiagnostic::from_parse_error(
            err,
            &source_name_str,
            &normalized,
        )))
    })?;

    Ok(ParsedScript {
        source_info: SourceInfo {
            source: source_name_str,
        },
        program,
    })
}

/// Reads and parses a Bash script file from disk using `brush-parser`.
pub fn parse_file(path: impl AsRef<Path>) -> Result<ParsedScript, ParserError> {
    let path_ref = path.as_ref();
    let content = std::fs::read_to_string(path_ref)?;
    let source_name_str = path_ref.display().to_string();
    let normalized = preprocess_bash_source(&content);
    let options = ParserOptions::default();
    let reader = BufReader::new(normalized.as_bytes());
    let mut parser = Parser::new(reader, &options);
    let program = parser.parse_program().map_err(|err| {
        ParserError::RichParse(Box::new(BashDiagnostic::from_parse_error(
            err,
            &source_name_str,
            &normalized,
        )))
    })?;

    Ok(ParsedScript {
        source_info: SourceInfo::from(path_ref.to_path_buf()),
        program,
    })
}

/// Normalizes Bash syntax constructs that `brush-parser` does not accept directly:
/// 1. `for ((...)) { ... }` and `for var [in ...]; { ... }` -> `for ...; do ...; done`
/// 2. Unquoted regexes with `(` / `)` / `|` after `=~` inside `[[ ... ]]`
fn preprocess_bash_source(source: &str) -> String {
    let mut out = normalize_short_for_loops(source);
    out = normalize_extended_test_regex(&out);
    out
}

fn skip_quoted_or_comment(bytes: &[u8], i: usize) -> Option<usize> {
    match bytes[i] {
        b'\\' if i + 1 < bytes.len() => Some(i + 2),
        b'\'' => {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'\'' {
                j += 1;
            }
            Some((j + 1).min(bytes.len()))
        }
        b'"' => {
            let mut j = i + 1;
            while j < bytes.len() {
                if bytes[j] == b'\\' && j + 1 < bytes.len() {
                    j += 2;
                } else if bytes[j] == b'"' {
                    j += 1;
                    break;
                } else {
                    j += 1;
                }
            }
            Some(j.min(bytes.len()))
        }
        b'#' if i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r' | b';') => {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'\n' {
                j += 1;
            }
            Some(j)
        }
        _ => None,
    }
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn normalize_short_for_loops(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len() + 32);
    let mut i = 0usize;
    let mut close_brace_as_done: Vec<usize> = Vec::new();
    let mut brace_depth = 0usize;

    while i < bytes.len() {
        if let Some(next_i) = skip_quoted_or_comment(bytes, i) {
            out.push_str(&source[i..next_i]);
            i = next_i;
            continue;
        }

        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Skip ${...} without counting its braces as compound-command braces.
            let mut j = i + 2;
            let mut depth = 1usize;
            while j < bytes.len() && depth > 0 {
                if bytes[j] == b'\\' && j + 1 < bytes.len() {
                    j += 2;
                } else if bytes[j] == b'{' {
                    depth += 1;
                    j += 1;
                } else if bytes[j] == b'}' {
                    depth -= 1;
                    j += 1;
                } else {
                    j += 1;
                }
            }
            out.push_str(&source[i..j]);
            i = j;
            continue;
        }

        if bytes[i] == b'{' {
            brace_depth += 1;
            out.push('{');
            i += 1;
            continue;
        }

        if bytes[i] == b'}' {
            if close_brace_as_done.last().copied() == Some(brace_depth) {
                close_brace_as_done.pop();
                brace_depth -= 1;
                if !out.ends_with('\n') && !out.ends_with(';') && !out.ends_with(' ') {
                    out.push(';');
                }
                out.push_str(" done");
                i += 1;
                continue;
            }
            brace_depth = brace_depth.saturating_sub(1);
            out.push('}');
            i += 1;
            continue;
        }

        // Check for `for` keyword at word boundary.
        let prev_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
        if prev_ok
            && bytes[i..].starts_with(b"for")
            && (i + 3 >= bytes.len() || !is_ident_byte(bytes[i + 3]))
        {
            let mut j = i + 3;
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t') {
                j += 1;
            }
            if j + 1 < bytes.len() && bytes[j] == b'(' && bytes[j + 1] == b'(' {
                // `for (( ... ))`
                let mut k = j + 2;
                let mut paren_depth = 0usize;
                while k + 1 < bytes.len() {
                    if bytes[k] == b'(' {
                        paren_depth += 1;
                        k += 1;
                    } else if bytes[k] == b')' {
                        if paren_depth > 0 {
                            paren_depth -= 1;
                            k += 1;
                        } else if bytes[k + 1] == b')' {
                            k += 2;
                            break;
                        } else {
                            k += 1;
                        }
                    } else {
                        k += 1;
                    }
                }
                let mut m = k;
                let mut saw_semi = false;
                while m < bytes.len() && matches!(bytes[m], b' ' | b'\t' | b'\n' | b'\r' | b';') {
                    if bytes[m] == b';' || bytes[m] == b'\n' {
                        saw_semi = true;
                    }
                    m += 1;
                }
                if m < bytes.len() && bytes[m] == b'{' {
                    let header = source[i..k].replace(";;", "; 1; ");
                    out.push_str(&header);
                    if !saw_semi {
                        out.push(';');
                    }
                    out.push_str(" do ");
                    brace_depth += 1;
                    close_brace_as_done.push(brace_depth);
                    i = m + 1;
                    continue;
                }
            } else {
                // `for var [in ...]; { ... }`
                let mut k = j;
                let mut found_semi = false;
                while k < bytes.len() {
                    if let Some(next_k) = skip_quoted_or_comment(bytes, k) {
                        k = next_k;
                        continue;
                    }
                    if bytes[k] == b';' || bytes[k] == b'\n' {
                        found_semi = true;
                        k += 1;
                        break;
                    }
                    k += 1;
                }
                if found_semi {
                    let mut m = k;
                    while m < bytes.len() && matches!(bytes[m], b' ' | b'\t' | b'\n' | b'\r') {
                        m += 1;
                    }
                    if m < bytes.len() && bytes[m] == b'{' {
                        out.push_str(&source[i..k]);
                        out.push_str(" do ");
                        brace_depth += 1;
                        close_brace_as_done.push(brace_depth);
                        i = m + 1;
                        continue;
                    }
                }
            }
        }

        let ch = source[i..].chars().next().expect("valid utf8");
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

fn normalize_extended_test_regex(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0usize;
    let mut in_ext_test = false;

    while i < bytes.len() {
        if let Some(next_i) = skip_quoted_or_comment(bytes, i) {
            out.push_str(&source[i..next_i]);
            i = next_i;
            continue;
        }

        if !in_ext_test && bytes[i..].starts_with(b"[[") {
            let prev_ok = i == 0
                || matches!(
                    bytes[i - 1],
                    b' ' | b'\t' | b'\n' | b';' | b'&' | b'|' | b'('
                );
            let next_ok = i + 2 < bytes.len() && matches!(bytes[i + 2], b' ' | b'\t');
            if prev_ok && next_ok {
                in_ext_test = true;
                out.push_str("[[");
                i += 2;
                continue;
            }
        }

        if in_ext_test && bytes[i..].starts_with(b"]]") {
            in_ext_test = false;
            out.push_str("]]");
            i += 2;
            continue;
        }

        if in_ext_test && bytes[i..].starts_with(b"=~") {
            out.push_str("=~");
            i += 2;
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                out.push(bytes[i] as char);
                i += 1;
            }
            if i < bytes.len() && !matches!(bytes[i], b'\'' | b'"') {
                // Read the regex token until unescaped whitespace outside parens/brackets
                let start = i;
                let mut paren_depth = 0usize;
                let mut bracket_depth = 0usize;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'[' {
                        bracket_depth += 1;
                    } else if bytes[i] == b']' && bracket_depth > 0 {
                        bracket_depth -= 1;
                    } else if bracket_depth == 0 {
                        if bytes[i] == b'(' {
                            paren_depth += 1;
                        } else if bytes[i] == b')' && paren_depth > 0 {
                            paren_depth -= 1;
                        } else if paren_depth == 0
                            && (matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r')
                                || bytes[i..].starts_with(b"]]"))
                        {
                            break;
                        }
                    }
                    i += 1;
                }
                let tok = &source[start..i];
                if tok.contains('(') || tok.contains(')') || tok.contains('|') {
                    out.push_str("\"__B2R_REGEX__");
                    for ch in tok.chars() {
                        if ch == '"' {
                            out.push_str("\\\"");
                        } else {
                            out.push(ch);
                        }
                    }
                    out.push('"');
                } else {
                    out.push_str(tok);
                }
                continue;
            }
        }

        let ch = source[i..].chars().next().expect("valid utf8");
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

/// Parses an in-memory Bash source string and lowers it directly into [`IrProgram`].
pub fn parse_str_to_ir(
    source: &str,
    source_name: impl Into<String>,
) -> Result<IrProgram, LowerError> {
    let parsed = parse_str(source, source_name)?;
    lower_program(&parsed.program, parsed.source_info.source)
}

/// Reads, parses, and lowers a Bash script file into [`IrProgram`].
pub fn parse_file_to_ir(path: impl AsRef<Path>) -> Result<IrProgram, LowerError> {
    let path_ref = path.as_ref();
    let parsed = parse_file(path_ref)?;
    let mut ir = lower_program(&parsed.program, parsed.source_info.source)?;
    if let Some(base_dir) = path_ref.parent() {
        let mut stack = Vec::new();
        if let Ok(canon) = std::fs::canonicalize(path_ref) {
            stack.push(canon);
        }
        resolve_static_sources(&mut ir.statements, base_dir, &mut stack);
    }
    Ok(ir)
}

fn static_literal_path(word: &bash2rust_ir::Word) -> Option<String> {
    if word.parts.is_empty() {
        let raw = word.raw.trim_matches(|c| c == '\'' || c == '"');
        if !raw.is_empty() && !raw.contains('$') {
            return Some(raw.to_owned());
        }
        return None;
    }
    if word.parts.len() == 1 {
        if let bash2rust_ir::WordPart::Literal(s) = &word.parts[0] {
            if !s.is_empty() {
                return Some(s.clone());
            }
        }
    }
    None
}

fn resolve_static_sources(
    stmts: &mut [bash2rust_ir::Stmt],
    base_dir: &Path,
    include_stack: &mut Vec<std::path::PathBuf>,
) {
    use bash2rust_ir::{Block, Stmt};
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::Source { path, body, .. } if body.is_none() => {
                if let Some(rel) = static_literal_path(path) {
                    let candidate = base_dir.join(&rel);
                    if candidate.is_file() {
                        if let Ok(canon) = std::fs::canonicalize(&candidate) {
                            if !include_stack.contains(&canon) {
                                if let Ok(parsed) = parse_file(&canon) {
                                    if let Ok(mut inc_ir) =
                                        lower_program(&parsed.program, parsed.source_info.source)
                                    {
                                        include_stack.push(canon.clone());
                                        let inc_dir = canon.parent().unwrap_or(base_dir);
                                        resolve_static_sources(
                                            &mut inc_ir.statements,
                                            inc_dir,
                                            include_stack,
                                        );
                                        include_stack.pop();
                                        *body = Some(Block::new(inc_ir.statements));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Stmt::If(if_stmt) => {
                resolve_static_sources(
                    &mut if_stmt.then_branch.statements,
                    base_dir,
                    include_stack,
                );
                for elif in &mut if_stmt.elif_branches {
                    resolve_static_sources(&mut elif.body.statements, base_dir, include_stack);
                }
                if let Some(eb) = &mut if_stmt.else_branch {
                    resolve_static_sources(&mut eb.statements, base_dir, include_stack);
                }
            }
            Stmt::Case(case_stmt) => {
                for item in &mut case_stmt.items {
                    resolve_static_sources(&mut item.body.statements, base_dir, include_stack);
                }
            }
            Stmt::For(for_stmt) => {
                resolve_static_sources(&mut for_stmt.body.statements, base_dir, include_stack);
            }
            Stmt::ArithFor(af) => {
                resolve_static_sources(&mut af.body.statements, base_dir, include_stack);
            }
            Stmt::Loop(l) => {
                resolve_static_sources(&mut l.body.statements, base_dir, include_stack);
            }
            Stmt::Function(f) => {
                resolve_static_sources(&mut f.body.statements, base_dir, include_stack);
            }
            Stmt::Block(b) => {
                resolve_static_sources(&mut b.statements, base_dir, include_stack);
            }
            Stmt::AndOr(ao) => {
                resolve_static_sources(
                    std::slice::from_mut(ao.first.as_mut()),
                    base_dir,
                    include_stack,
                );
                for item in &mut ao.rest {
                    resolve_static_sources(
                        std::slice::from_mut(&mut item.rhs),
                        base_dir,
                        include_stack,
                    );
                }
            }
            _ => {}
        }
    }
}
