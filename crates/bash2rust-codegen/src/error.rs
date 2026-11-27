//! Error types for `bash2rust-codegen`.

use thiserror::Error;

/// Errors produced while emitting or validating generated Rust code.
#[derive(Debug, Error)]
pub enum CodegenError {
    #[error("generated Rust tokens failed syn::File validation: {0}")]
    InvalidRustSyntax(#[from] syn::Error),

    #[error("AI-generated Rust snippet failed syn parsing ({message}):\n{code}")]
    InvalidAiRust { message: String, code: String },
}
