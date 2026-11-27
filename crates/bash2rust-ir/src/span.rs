//! Source location types for `bash2rust-ir`.

use serde::{Deserialize, Serialize};

/// A position in a Bash source file (0-based character `index`, 1-based `line` and `column`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct SourcePos {
    pub index: usize,
    pub line: usize,
    pub column: usize,
}

/// A half-open source span `[start, end)` in a Bash source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: SourcePos,
    pub end: SourcePos,
}
