//! Error types and rich `miette` diagnostics for `bash2rust-parser`.

use brush_parser::ParseError;
use miette::{Diagnostic, GraphicalReportHandler, GraphicalTheme, NamedSource, SourceSpan};
use thiserror::Error;

/// Rich compiler diagnostic with source code snippet, 1-based line/column, and byte span.
#[derive(Debug, Clone, Error, Diagnostic)]
#[error("{message}")]
pub struct BashDiagnostic {
    pub message: String,
    pub line: usize,
    pub column: usize,
    #[source_code]
    pub src: NamedSource<String>,
    #[label("{label}")]
    pub span: SourceSpan,
    pub label: String,
    #[help]
    pub help: Option<String>,
}

impl BashDiagnostic {
    /// Builds a [`BashDiagnostic`] from a [`brush_parser::ParseError`] and its source text.
    pub fn from_parse_error(err: ParseError, source_name: impl Into<String>, source: &str) -> Self {
        let name = source_name.into();
        let raw_msg = err.to_string();

        let (line, column, help) = match &err {
            ParseError::ParsingNear(pos) => (
                pos.line.max(1),
                pos.column.max(1),
                Some("check for missing `fi`, `done`, `esac`, `}`, or mismatched quotes near this token".to_owned()),
            ),
            ParseError::ParsingAtEndOfInput => {
                let (l, c) = end_of_input_line_col(source);
                (
                    l,
                    c,
                    Some("unexpected end of file; check for an unclosed `if`/`for`/`while`/`case` block or unclosed quote".to_owned()),
                )
            }
            ParseError::Tokenizing { position, .. } => {
                if let Some(pos) = position {
                    (
                        pos.line.max(1),
                        pos.column.max(1),
                        Some("check for an unclosed string literal, escape sequence, or invalid token".to_owned()),
                    )
                } else {
                    let (l, c) = fallback_line_col(&raw_msg).unwrap_or_else(|| end_of_input_line_col(source));
                    (
                        l,
                        c,
                        Some("check for an unclosed string literal or invalid token".to_owned()),
                    )
                }
            }
        };

        let offset = line_col_to_offset(source, line, column);
        let len = span_len_at_offset(source, offset);

        Self {
            message: format!("Bash syntax error at {name}:{line}:{column}: {raw_msg}"),
            line,
            column,
            src: NamedSource::new(name, source.to_owned()),
            span: SourceSpan::new(offset.into(), len),
            label: raw_msg,
            help,
        }
    }

    /// Builds a [`BashDiagnostic`] for an unsupported Bash construct using its [`bash2rust_ir::SourceSpan`].
    pub fn for_unsupported(
        source_name: impl Into<String>,
        source: &str,
        raw: &str,
        reason: &str,
        ir_span: Option<bash2rust_ir::SourceSpan>,
    ) -> Self {
        let name = source_name.into();
        let (line, column) = ir_span
            .map(|s| (s.start.line.max(1), s.start.column.max(1)))
            .unwrap_or((1, 1));
        let offset = line_col_to_offset(source, line, column);
        let len = raw
            .len()
            .max(1)
            .min(source.len().saturating_sub(offset).max(1));

        Self {
            message: format!("unsupported Bash construct at {name}:{line}:{column}: {reason}"),
            line,
            column,
            src: NamedSource::new(name, source.to_owned()),
            span: SourceSpan::new(offset.into(), len),
            label: format!("unsupported construct `{}`", raw.trim()),
            help: Some(
                "rewrite this construct in supported Bash or pass `--ai ollama|anthropic|openai` to translate it automatically".to_owned(),
            ),
        }
    }

    /// Renders this diagnostic into a human-readable report string (without ANSI color codes).
    pub fn render_plain(&self) -> String {
        let mut out = String::new();
        let handler = GraphicalReportHandler::new_themed(GraphicalTheme::unicode_nocolor());
        let _ = handler.render_report(&mut out, self);
        out
    }
}

/// Converts 1-based `(line, column)` to a byte offset in `source`.
pub fn line_col_to_offset(source: &str, line: usize, column: usize) -> usize {
    if source.is_empty() {
        return 0;
    }
    let target_line = line.max(1);
    let target_col = column.max(1);

    let mut cur_line = 1usize;
    let mut line_start = 0usize;

    for (idx, ch) in source.char_indices() {
        if cur_line == target_line {
            break;
        }
        if ch == '\n' {
            cur_line += 1;
            line_start = idx + 1;
        }
    }

    if cur_line < target_line {
        return source.len();
    }

    let line_slice = &source[line_start..];
    let line_end = line_slice.find('\n').unwrap_or(line_slice.len());
    let line_content = &line_slice[..line_end];

    let col_bytes = line_content
        .char_indices()
        .nth(target_col.saturating_sub(1))
        .map(|(i, _)| i)
        .unwrap_or(line_content.len());

    (line_start + col_bytes).min(source.len())
}

fn end_of_input_line_col(source: &str) -> (usize, usize) {
    if source.is_empty() {
        return (1, 1);
    }
    let mut line = 1usize;
    let mut col = 1usize;
    for ch in source.chars() {
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn span_len_at_offset(source: &str, offset: usize) -> usize {
    if offset >= source.len() {
        return 0;
    }
    let rest = &source[offset..];
    let token_len = rest
        .chars()
        .take_while(|c| !c.is_whitespace())
        .map(char::len_utf8)
        .sum::<usize>();
    if token_len > 0 {
        token_len
    } else {
        rest.chars().next().map(char::len_utf8).unwrap_or(0)
    }
}

fn fallback_line_col(msg: &str) -> Option<(usize, usize)> {
    let idx = msg.find("line ")?;
    let rest = &msg[idx + 5..];
    let mut parts = rest.split_whitespace();
    let line = parts.next()?.parse::<usize>().ok()?;
    let col = if parts.next() == Some("col") {
        parts
            .next()
            .and_then(|s| s.trim_end_matches([')', ',', ':']).parse::<usize>().ok())
            .unwrap_or(1)
    } else {
        1
    };
    Some((line.max(1), col.max(1)))
}

/// Formats an unsupported [`bash2rust_ir::Stmt::Unsupported`] diagnostic into a plain report string.
pub fn format_unsupported_diagnostic(
    source_name: impl Into<String>,
    source: &str,
    raw: &str,
    reason: &str,
    span: Option<bash2rust_ir::SourceSpan>,
) -> String {
    BashDiagnostic::for_unsupported(source_name, source, raw, reason, span).render_plain()
}

/// Errors produced while reading or parsing Bash source code.
#[derive(Debug, Error, Diagnostic)]
pub enum ParserError {
    #[error("I/O error reading Bash source: {0}")]
    Io(#[from] std::io::Error),

    #[error("Bash parse error: {0}")]
    Parse(#[from] brush_parser::ParseError),

    #[error(transparent)]
    #[diagnostic(transparent)]
    RichParse(Box<BashDiagnostic>),
}

/// Errors produced while lowering a `brush_parser` AST into `bash2rust-ir`.
#[derive(Debug, Error, Diagnostic)]
pub enum LowerError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Parser(#[from] ParserError),

    #[error("unsupported construct `{kind}`: {message}")]
    Unsupported { kind: String, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_str;

    #[test]
    fn line_col_to_offset_computes_exact_byte_positions() {
        let src = "echo one\nif true; then\n  echo two\n";
        assert_eq!(line_col_to_offset(src, 1, 1), 0);
        assert_eq!(line_col_to_offset(src, 2, 1), 9);
        assert_eq!(line_col_to_offset(src, 2, 4), 12);
        assert_eq!(line_col_to_offset(src, 3, 3), 25);
    }

    #[test]
    fn reports_rich_diagnostic_for_unclosed_if() {
        let src = "echo start\nif [ -f /etc/hosts ]; then\n  echo ok\n";
        let err = parse_str(src, "unclosed_if.sh").expect_err("expected parse error");
        let ParserError::RichParse(diag) = err else {
            panic!("expected RichParse, got {err:?}");
        };
        assert!(diag.line >= 2);
        assert!(diag.column >= 1);
        let rendered = diag.render_plain();
        assert!(
            rendered.contains("unclosed_if.sh"),
            "missing filename in:\n{rendered}"
        );
        assert!(rendered.contains("help:"), "missing help in:\n{rendered}");
    }

    #[test]
    fn reports_rich_diagnostic_for_unclosed_quote() {
        let src = "x=1\ny=\"unclosed string\necho done\n";
        let err = parse_str(src, "unclosed_quote.sh").expect_err("expected parse error");
        let ParserError::RichParse(diag) = err else {
            panic!("expected RichParse, got {err:?}");
        };
        assert!(diag.line >= 2);
        let rendered = diag.render_plain();
        assert!(
            rendered.contains("unclosed_quote.sh"),
            "missing filename in:\n{rendered}"
        );
        assert!(rendered.contains("help:"), "missing help in:\n{rendered}");
    }

    #[test]
    fn reports_rich_diagnostic_for_malformed_case() {
        let src = "case \"$x\" in\n  foo) echo 1 ;;\n";
        let err = parse_str(src, "bad_case.sh").expect_err("expected parse error");
        let ParserError::RichParse(diag) = err else {
            panic!("expected RichParse, got {err:?}");
        };
        assert!(diag.line >= 2);
        let rendered = diag.render_plain();
        assert!(
            rendered.contains("bad_case.sh"),
            "missing filename in:\n{rendered}"
        );
    }

    #[test]
    fn formats_unsupported_diagnostic_with_span() {
        let src = "echo a\neval \"echo hi\"\necho b\n";
        let span = bash2rust_ir::SourceSpan {
            start: bash2rust_ir::SourcePos {
                index: 7,
                line: 2,
                column: 1,
            },
            end: bash2rust_ir::SourcePos {
                index: 21,
                line: 2,
                column: 15,
            },
        };
        let rendered = format_unsupported_diagnostic(
            "test.sh",
            src,
            "eval \"echo hi\"",
            "dynamic eval",
            Some(span),
        );
        assert!(rendered.contains("test.sh:2:1"), "got:\n{rendered}");
        assert!(rendered.contains("eval \"echo hi\""), "got:\n{rendered}");
        assert!(rendered.contains("help:"), "got:\n{rendered}");
    }
}
