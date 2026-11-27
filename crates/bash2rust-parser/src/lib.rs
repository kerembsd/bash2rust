//! Brush-based Bash parser and AST-to-IR lowering for `bash2rust`.

pub mod error;
pub mod lower;
mod test_expr;

use std::io::BufReader;
use std::path::Path;

use bash2rust_ir::IrProgram;
use brush_parser::{ast, Parser, ParserOptions, SourceInfo};

pub use error::{LowerError, ParserError};
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
    let options = ParserOptions::default();
    let reader = BufReader::new(source.as_bytes());
    let mut parser = Parser::new(reader, &options);
    let program = parser.parse_program()?;

    Ok(ParsedScript {
        source_info: SourceInfo {
            source: source_name.into(),
        },
        program,
    })
}

/// Reads and parses a Bash script file from disk using `brush-parser`.
pub fn parse_file(path: impl AsRef<Path>) -> Result<ParsedScript, ParserError> {
    let path_ref = path.as_ref();
    let content = std::fs::read_to_string(path_ref)?;
    let options = ParserOptions::default();
    let reader = BufReader::new(content.as_bytes());
    let mut parser = Parser::new(reader, &options);
    let program = parser.parse_program()?;

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
    let parsed = parse_file(path)?;
    lower_program(&parsed.program, parsed.source_info.source)
}
