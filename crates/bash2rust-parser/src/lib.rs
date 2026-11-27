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
    let options = ParserOptions::default();
    let reader = BufReader::new(source.as_bytes());
    let mut parser = Parser::new(reader, &options);
    let program = parser.parse_program().map_err(|err| {
        ParserError::RichParse(Box::new(BashDiagnostic::from_parse_error(
            err,
            &source_name_str,
            source,
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
    let options = ParserOptions::default();
    let reader = BufReader::new(content.as_bytes());
    let mut parser = Parser::new(reader, &options);
    let program = parser.parse_program().map_err(|err| {
        ParserError::RichParse(Box::new(BashDiagnostic::from_parse_error(
            err,
            &source_name_str,
            &content,
        )))
    })?;

    Ok(ParsedScript {
        source_info: SourceInfo::from(path_ref.to_path_buf()),
        program,
    })
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
