//! Core compilation pipeline and orchestration for `bash2rust`.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

pub use bash2rust_ai as ai;
pub use bash2rust_codegen as codegen;
pub use bash2rust_parser::{BashDiagnostic, LowerError, ParserError};
pub use bash2rust_runtime as runtime;

use bash2rust_ai::AiProvider;
use bash2rust_codegen::{generate_rust, parse_rust_snippet, CodegenError};
use bash2rust_ir::{Block, IrProgram, Stmt};
use bash2rust_parser::{parse_file_to_ir, parse_str_to_ir};
use thiserror::Error;

/// Errors returned by the `bash2rust` compilation pipeline.
#[derive(Debug, Error)]
pub enum CompileError {
    #[error("frontend parse/lowering failed: {0}")]
    Frontend(#[from] LowerError),

    #[error("Rust codegen failed: {0}")]
    Codegen(#[from] CodegenError),

    #[error("AI translation failed: {0}")]
    Ai(#[from] anyhow::Error),
}

/// Output produced by the compilation frontend/pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileOutput {
    pub ir: IrProgram,
}

/// Summary of an AI fallback resolution pass over an [`IrProgram`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AiResolutionReport {
    pub resolved_count: usize,
}

/// Parses and lowers an in-memory Bash script into IR.
pub fn compile_str_to_ir(
    source: &str,
    source_name: impl Into<String>,
) -> Result<CompileOutput, CompileError> {
    let ir = parse_str_to_ir(source, source_name)?;
    Ok(CompileOutput { ir })
}

/// Reads, parses, and lowers a Bash script file into IR.
pub fn compile_file_to_ir(path: impl AsRef<Path>) -> Result<CompileOutput, CompileError> {
    let ir = parse_file_to_ir(path)?;
    Ok(CompileOutput { ir })
}

/// Generates formatted Rust source code from an [`IrProgram`].
pub fn generate_rust_from_ir(ir: &IrProgram) -> Result<String, CompileError> {
    Ok(generate_rust(ir)?)
}

/// Compiles an in-memory Bash script into formatted Rust source code.
pub fn compile_str_to_rust(
    source: &str,
    source_name: impl Into<String>,
) -> Result<String, CompileError> {
    let ir = parse_str_to_ir(source, source_name)?;
    Ok(generate_rust(&ir)?)
}

/// Reads and compiles a Bash script file into formatted Rust source code.
pub fn compile_file_to_rust(path: impl AsRef<Path>) -> Result<String, CompileError> {
    let ir = parse_file_to_ir(path)?;
    Ok(generate_rust(&ir)?)
}

/// Recursively resolves all [`Stmt::Unsupported`] nodes in `program` via `provider`,
/// validating each generated snippet with `syn` and replacing it with [`Stmt::RawRust`].
pub async fn resolve_unsupported_with_ai(
    program: &mut IrProgram,
    provider: &dyn AiProvider,
) -> Result<AiResolutionReport, CompileError> {
    let mut report = AiResolutionReport::default();
    resolve_stmts(&mut program.statements, provider, &mut report).await?;
    Ok(report)
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

fn resolve_block<'a>(
    block: &'a mut Block,
    provider: &'a dyn AiProvider,
    report: &'a mut AiResolutionReport,
) -> BoxFuture<'a, Result<(), CompileError>> {
    Box::pin(async move { resolve_stmts(&mut block.statements, provider, report).await })
}

fn resolve_stmts<'a>(
    stmts: &'a mut [Stmt],
    provider: &'a dyn AiProvider,
    report: &'a mut AiResolutionReport,
) -> BoxFuture<'a, Result<(), CompileError>> {
    Box::pin(async move {
        for stmt in stmts.iter_mut() {
            resolve_stmt(stmt, provider, report).await?;
        }
        Ok(())
    })
}

fn resolve_stmt<'a>(
    stmt: &'a mut Stmt,
    provider: &'a dyn AiProvider,
    report: &'a mut AiResolutionReport,
) -> BoxFuture<'a, Result<(), CompileError>> {
    Box::pin(async move {
        match stmt {
            Stmt::Unsupported { raw, span, .. } => {
                let raw_bash = raw.clone();
                let span_copy = *span;
                let code = provider.translate_unsupported(&raw_bash).await?;
                bash2rust_ai::validate_ai_rust(&code)
                    .map_err(|e| anyhow::anyhow!("security filter rejected AI snippet: {e}"))?;
                let _ = parse_rust_snippet(&code)?;
                *stmt = Stmt::RawRust {
                    code,
                    original_bash: Some(raw_bash),
                    span: span_copy,
                };
                report.resolved_count += 1;
            }
            Stmt::If(if_stmt) => {
                resolve_block(&mut if_stmt.condition, provider, report).await?;
                resolve_block(&mut if_stmt.then_branch, provider, report).await?;
                for branch in &mut if_stmt.elif_branches {
                    resolve_block(&mut branch.condition, provider, report).await?;
                    resolve_block(&mut branch.body, provider, report).await?;
                }
                if let Some(else_block) = &mut if_stmt.else_branch {
                    resolve_block(else_block, provider, report).await?;
                }
            }
            Stmt::Case(case_stmt) => {
                for item in &mut case_stmt.items {
                    resolve_block(&mut item.body, provider, report).await?;
                }
            }
            Stmt::For(for_stmt) => {
                resolve_block(&mut for_stmt.body, provider, report).await?;
            }
            Stmt::ArithFor(arith_for) => {
                resolve_block(&mut arith_for.body, provider, report).await?;
            }
            Stmt::Loop(loop_stmt) => {
                resolve_block(&mut loop_stmt.condition, provider, report).await?;
                resolve_block(&mut loop_stmt.body, provider, report).await?;
            }
            Stmt::Function(func_def) => {
                resolve_block(&mut func_def.body, provider, report).await?;
            }
            Stmt::AndOr(and_or) => {
                resolve_stmt(&mut and_or.first, provider, report).await?;
                for item in &mut and_or.rest {
                    resolve_stmt(&mut item.rhs, provider, report).await?;
                }
            }
            Stmt::Pipeline(pipeline) => {
                resolve_stmts(&mut pipeline.commands, provider, report).await?;
            }
            Stmt::Block(block) => {
                resolve_block(block, provider, report).await?;
            }
            Stmt::Source { body, .. } => {
                if let Some(block) = body {
                    resolve_block(block, provider, report).await?;
                }
            }
            Stmt::Assign(_)
            | Stmt::Arith(_)
            | Stmt::Return { .. }
            | Stmt::Command(_)
            | Stmt::Test(_)
            | Stmt::RawRust { .. } => {}
        }
        Ok(())
    })
}

/// Compiles an in-memory Bash script to Rust, resolving unsupported constructs with `provider`.
pub async fn compile_str_to_rust_with_ai(
    source: &str,
    source_name: impl Into<String>,
    provider: &dyn AiProvider,
) -> Result<String, CompileError> {
    let mut ir = parse_str_to_ir(source, source_name)?;
    let _ = resolve_unsupported_with_ai(&mut ir, provider).await?;
    Ok(generate_rust(&ir)?)
}

/// Reads and compiles a Bash script file to Rust, resolving unsupported constructs with `provider`.
pub async fn compile_file_to_rust_with_ai(
    path: impl AsRef<Path>,
    provider: &dyn AiProvider,
) -> Result<String, CompileError> {
    let mut ir = parse_file_to_ir(path)?;
    let _ = resolve_unsupported_with_ai(&mut ir, provider).await?;
    Ok(generate_rust(&ir)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct MockAiProvider;

    #[async_trait]
    impl AiProvider for MockAiProvider {
        async fn translate_unsupported(&self, raw_bash: &str) -> anyhow::Result<String> {
            Ok(format!("let _ai_translated = {raw_bash:?};"))
        }
    }

    #[tokio::test]
    async fn resolves_nested_unsupported_nodes_with_ai() {
        let script = r#"
coproc { echo top; }
if [ -d /tmp ]; then
    coproc { echo inner; }
fi
for x in a b; do
    case "$x" in
        a) coproc { echo nested; } ;;
    esac
done
"#;
        let mut ir = parse_str_to_ir(script, "hybrid.sh").expect("parse");
        let report = resolve_unsupported_with_ai(&mut ir, &MockAiProvider)
            .await
            .expect("resolve with AI");
        assert_eq!(report.resolved_count, 3);

        let rust_code = generate_rust_from_ir(&ir).expect("generate rust");
        assert!(rust_code.contains("_ai_translated"));
        syn::parse_file(&rust_code).expect("valid syn::File");
    }
}
