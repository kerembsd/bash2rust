//! Error types for `bash2rust-parser`.

use thiserror::Error;

/// Errors produced while reading or parsing Bash source code.
#[derive(Debug, Error)]
pub enum ParserError {
    #[error("I/O error reading Bash source: {0}")]
    Io(#[from] std::io::Error),

    #[error("Bash parse error: {0}")]
    Parse(#[from] brush_parser::ParseError),
}

/// Errors produced while lowering a `brush_parser` AST into `bash2rust-ir`.
#[derive(Debug, Error)]
pub enum LowerError {
    #[error(transparent)]
    Parser(#[from] ParserError),

    #[error("unsupported construct `{kind}`: {message}")]
    Unsupported { kind: String, message: String },
}
