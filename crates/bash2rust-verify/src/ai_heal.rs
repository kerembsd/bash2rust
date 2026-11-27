//! AI self-healing compilation loop (1 initial attempt + 1 automatic compiler-feedback retry).

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use bash2rust_ai::AiProvider;
use bash2rust_core::codegen::parse_rust_snippet;
use bash2rust_ir::{Block, IrProgram, Stmt};

use crate::{compile_check_rust_source, VerifyError};

/// Outcome of compiling an [`IrProgram`] with the AI self-healing loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfHealReport {
    pub ir: IrProgram,
    pub rust_code: String,
    pub translated_nodes: usize,
    pub attempts: usize,
    pub healed: bool,
    pub initial_error: Option<String>,
}

/// Compiles `initial_ir` with AI fallback and up to 1 automatic self-healing retry if
/// `syn` snippet validation or `rustc` type-checking fails on the initial attempt.
pub async fn compile_ir_with_ai_self_healing(
    initial_ir: &IrProgram,
    provider: &dyn AiProvider,
) -> Result<SelfHealReport, VerifyError> {
    let mut ir_1 = initial_ir.clone();
    let mut prev_codes: Vec<String> = Vec::new();
    let mut translated_nodes = 0usize;

    let attempt_1_err = match translate_and_check_initial(
        &mut ir_1,
        provider,
        &mut prev_codes,
        &mut translated_nodes,
    )
    .await?
    {
        Ok(rust_code) => {
            return Ok(SelfHealReport {
                ir: ir_1,
                rust_code,
                translated_nodes,
                attempts: 1,
                healed: false,
                initial_error: None,
            });
        }
        Err(err) => err,
    };

    // Attempt 2: feed the real `syn` or `rustc` error back into `provider.repair_rust`.
    let mut ir_2 = initial_ir.clone();
    let mut idx = 0usize;
    repair_stmts(
        &mut ir_2.statements,
        provider,
        &prev_codes,
        &attempt_1_err,
        &mut idx,
    )
    .await?;

    let rust_code = bash2rust_core::generate_rust_from_ir(&ir_2)?;
    compile_check_rust_source(&rust_code)?;

    Ok(SelfHealReport {
        ir: ir_2,
        rust_code,
        translated_nodes,
        attempts: 2,
        healed: true,
        initial_error: Some(attempt_1_err),
    })
}

/// Convenience wrapper parsing `source` to IR and running [`compile_ir_with_ai_self_healing`].
pub async fn compile_str_with_ai_self_healing(
    source: &str,
    source_name: impl Into<String>,
    provider: &dyn AiProvider,
) -> Result<SelfHealReport, VerifyError> {
    let output = bash2rust_core::compile_str_to_ir(source, source_name)?;
    compile_ir_with_ai_self_healing(&output.ir, provider).await
}

/// Convenience wrapper reading `path`, parsing to IR, and running [`compile_ir_with_ai_self_healing`].
pub async fn compile_file_with_ai_self_healing(
    path: impl AsRef<Path>,
    provider: &dyn AiProvider,
) -> Result<SelfHealReport, VerifyError> {
    let output = bash2rust_core::compile_file_to_ir(path)?;
    compile_ir_with_ai_self_healing(&output.ir, provider).await
}

async fn translate_and_check_initial(
    ir: &mut IrProgram,
    provider: &dyn AiProvider,
    prev_codes: &mut Vec<String>,
    translated_nodes: &mut usize,
) -> Result<Result<String, String>, VerifyError> {
    let mut first_syn_err: Option<String> = None;
    translate_stmts_initial(
        &mut ir.statements,
        provider,
        prev_codes,
        translated_nodes,
        &mut first_syn_err,
    )
    .await?;

    if let Some(err) = first_syn_err {
        return Ok(Err(err));
    }

    let rust_code = match bash2rust_core::generate_rust_from_ir(ir) {
        Ok(c) => c,
        Err(e) => return Ok(Err(e.to_string())),
    };
    if let Err(e) = compile_check_rust_source(&rust_code) {
        return Ok(Err(e.to_string()));
    }
    Ok(Ok(rust_code))
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

fn translate_block_initial<'a>(
    block: &'a mut Block,
    provider: &'a dyn AiProvider,
    prev_codes: &'a mut Vec<String>,
    translated_nodes: &'a mut usize,
    first_syn_err: &'a mut Option<String>,
) -> BoxFuture<'a, Result<(), VerifyError>> {
    Box::pin(async move {
        translate_stmts_initial(
            &mut block.statements,
            provider,
            prev_codes,
            translated_nodes,
            first_syn_err,
        )
        .await
    })
}

fn translate_stmts_initial<'a>(
    stmts: &'a mut [Stmt],
    provider: &'a dyn AiProvider,
    prev_codes: &'a mut Vec<String>,
    translated_nodes: &'a mut usize,
    first_syn_err: &'a mut Option<String>,
) -> BoxFuture<'a, Result<(), VerifyError>> {
    Box::pin(async move {
        for stmt in stmts.iter_mut() {
            translate_stmt_initial(stmt, provider, prev_codes, translated_nodes, first_syn_err)
                .await?;
        }
        Ok(())
    })
}

fn translate_stmt_initial<'a>(
    stmt: &'a mut Stmt,
    provider: &'a dyn AiProvider,
    prev_codes: &'a mut Vec<String>,
    translated_nodes: &'a mut usize,
    first_syn_err: &'a mut Option<String>,
) -> BoxFuture<'a, Result<(), VerifyError>> {
    Box::pin(async move {
        match stmt {
            Stmt::Unsupported { raw, span, .. } => {
                let raw_bash = raw.clone();
                let span_copy = *span;
                let code = provider.translate_unsupported(&raw_bash).await?;
                match bash2rust_ai::validate_ai_rust(&code) {
                    Ok(()) => {}
                    Err(bash2rust_ai::SecurityError::Rejected(v)) => {
                        return Err(VerifyError::Ai(anyhow::anyhow!(
                            "security filter rejected AI snippet: {v}"
                        )));
                    }
                    Err(bash2rust_ai::SecurityError::ParseError(err)) => {
                        prev_codes.push(code);
                        *translated_nodes += 1;
                        if first_syn_err.is_none() {
                            *first_syn_err = Some(err);
                        }
                        return Ok(());
                    }
                }
                prev_codes.push(code.clone());
                *translated_nodes += 1;

                match parse_rust_snippet(&code) {
                    Ok(_) => {
                        *stmt = Stmt::RawRust {
                            code,
                            original_bash: Some(raw_bash),
                            span: span_copy,
                        };
                    }
                    Err(err) => {
                        if first_syn_err.is_none() {
                            *first_syn_err = Some(err.to_string());
                        }
                    }
                }
            }
            Stmt::If(if_stmt) => {
                translate_block_initial(
                    &mut if_stmt.condition,
                    provider,
                    prev_codes,
                    translated_nodes,
                    first_syn_err,
                )
                .await?;
                translate_block_initial(
                    &mut if_stmt.then_branch,
                    provider,
                    prev_codes,
                    translated_nodes,
                    first_syn_err,
                )
                .await?;
                for branch in &mut if_stmt.elif_branches {
                    translate_block_initial(
                        &mut branch.condition,
                        provider,
                        prev_codes,
                        translated_nodes,
                        first_syn_err,
                    )
                    .await?;
                    translate_block_initial(
                        &mut branch.body,
                        provider,
                        prev_codes,
                        translated_nodes,
                        first_syn_err,
                    )
                    .await?;
                }
                if let Some(else_block) = &mut if_stmt.else_branch {
                    translate_block_initial(
                        else_block,
                        provider,
                        prev_codes,
                        translated_nodes,
                        first_syn_err,
                    )
                    .await?;
                }
            }
            Stmt::Case(case_stmt) => {
                for item in &mut case_stmt.items {
                    translate_block_initial(
                        &mut item.body,
                        provider,
                        prev_codes,
                        translated_nodes,
                        first_syn_err,
                    )
                    .await?;
                }
            }
            Stmt::For(for_stmt) => {
                translate_block_initial(
                    &mut for_stmt.body,
                    provider,
                    prev_codes,
                    translated_nodes,
                    first_syn_err,
                )
                .await?;
            }
            Stmt::ArithFor(arith_for) => {
                translate_block_initial(
                    &mut arith_for.body,
                    provider,
                    prev_codes,
                    translated_nodes,
                    first_syn_err,
                )
                .await?;
            }
            Stmt::Loop(loop_stmt) => {
                translate_block_initial(
                    &mut loop_stmt.condition,
                    provider,
                    prev_codes,
                    translated_nodes,
                    first_syn_err,
                )
                .await?;
                translate_block_initial(
                    &mut loop_stmt.body,
                    provider,
                    prev_codes,
                    translated_nodes,
                    first_syn_err,
                )
                .await?;
            }
            Stmt::Function(func_def) => {
                translate_block_initial(
                    &mut func_def.body,
                    provider,
                    prev_codes,
                    translated_nodes,
                    first_syn_err,
                )
                .await?;
            }
            Stmt::AndOr(and_or) => {
                translate_stmt_initial(
                    &mut and_or.first,
                    provider,
                    prev_codes,
                    translated_nodes,
                    first_syn_err,
                )
                .await?;
                for item in &mut and_or.rest {
                    translate_stmt_initial(
                        &mut item.rhs,
                        provider,
                        prev_codes,
                        translated_nodes,
                        first_syn_err,
                    )
                    .await?;
                }
            }
            Stmt::Pipeline(pipeline) => {
                translate_stmts_initial(
                    &mut pipeline.commands,
                    provider,
                    prev_codes,
                    translated_nodes,
                    first_syn_err,
                )
                .await?;
            }
            Stmt::Block(block) => {
                translate_block_initial(
                    block,
                    provider,
                    prev_codes,
                    translated_nodes,
                    first_syn_err,
                )
                .await?;
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

fn repair_block<'a>(
    block: &'a mut Block,
    provider: &'a dyn AiProvider,
    prev_codes: &'a [String],
    compiler_error: &'a str,
    idx: &'a mut usize,
) -> BoxFuture<'a, Result<(), VerifyError>> {
    Box::pin(async move {
        repair_stmts(
            &mut block.statements,
            provider,
            prev_codes,
            compiler_error,
            idx,
        )
        .await
    })
}

fn repair_stmts<'a>(
    stmts: &'a mut [Stmt],
    provider: &'a dyn AiProvider,
    prev_codes: &'a [String],
    compiler_error: &'a str,
    idx: &'a mut usize,
) -> BoxFuture<'a, Result<(), VerifyError>> {
    Box::pin(async move {
        for stmt in stmts.iter_mut() {
            repair_stmt(stmt, provider, prev_codes, compiler_error, idx).await?;
        }
        Ok(())
    })
}

fn repair_stmt<'a>(
    stmt: &'a mut Stmt,
    provider: &'a dyn AiProvider,
    prev_codes: &'a [String],
    compiler_error: &'a str,
    idx: &'a mut usize,
) -> BoxFuture<'a, Result<(), VerifyError>> {
    Box::pin(async move {
        match stmt {
            Stmt::Unsupported { raw, span, .. } => {
                let raw_bash = raw.clone();
                let span_copy = *span;
                let prev = prev_codes.get(*idx).map(String::as_str).unwrap_or("");
                *idx += 1;

                let repaired = provider
                    .repair_rust(&raw_bash, prev, compiler_error)
                    .await?;
                bash2rust_ai::validate_ai_rust(&repaired).map_err(|e| {
                    VerifyError::Ai(anyhow::anyhow!(
                        "security filter rejected AI repair snippet: {e}"
                    ))
                })?;
                let _ =
                    parse_rust_snippet(&repaired).map_err(bash2rust_core::CompileError::from)?;
                *stmt = Stmt::RawRust {
                    code: repaired,
                    original_bash: Some(raw_bash),
                    span: span_copy,
                };
            }
            Stmt::If(if_stmt) => {
                repair_block(
                    &mut if_stmt.condition,
                    provider,
                    prev_codes,
                    compiler_error,
                    idx,
                )
                .await?;
                repair_block(
                    &mut if_stmt.then_branch,
                    provider,
                    prev_codes,
                    compiler_error,
                    idx,
                )
                .await?;
                for branch in &mut if_stmt.elif_branches {
                    repair_block(
                        &mut branch.condition,
                        provider,
                        prev_codes,
                        compiler_error,
                        idx,
                    )
                    .await?;
                    repair_block(&mut branch.body, provider, prev_codes, compiler_error, idx)
                        .await?;
                }
                if let Some(else_block) = &mut if_stmt.else_branch {
                    repair_block(else_block, provider, prev_codes, compiler_error, idx).await?;
                }
            }
            Stmt::Case(case_stmt) => {
                for item in &mut case_stmt.items {
                    repair_block(&mut item.body, provider, prev_codes, compiler_error, idx).await?;
                }
            }
            Stmt::For(for_stmt) => {
                repair_block(
                    &mut for_stmt.body,
                    provider,
                    prev_codes,
                    compiler_error,
                    idx,
                )
                .await?;
            }
            Stmt::ArithFor(arith_for) => {
                repair_block(
                    &mut arith_for.body,
                    provider,
                    prev_codes,
                    compiler_error,
                    idx,
                )
                .await?;
            }
            Stmt::Loop(loop_stmt) => {
                repair_block(
                    &mut loop_stmt.condition,
                    provider,
                    prev_codes,
                    compiler_error,
                    idx,
                )
                .await?;
                repair_block(
                    &mut loop_stmt.body,
                    provider,
                    prev_codes,
                    compiler_error,
                    idx,
                )
                .await?;
            }
            Stmt::Function(func_def) => {
                repair_block(
                    &mut func_def.body,
                    provider,
                    prev_codes,
                    compiler_error,
                    idx,
                )
                .await?;
            }
            Stmt::AndOr(and_or) => {
                repair_stmt(&mut and_or.first, provider, prev_codes, compiler_error, idx).await?;
                for item in &mut and_or.rest {
                    repair_stmt(&mut item.rhs, provider, prev_codes, compiler_error, idx).await?;
                }
            }
            Stmt::Pipeline(pipeline) => {
                repair_stmts(
                    &mut pipeline.commands,
                    provider,
                    prev_codes,
                    compiler_error,
                    idx,
                )
                .await?;
            }
            Stmt::Block(block) => {
                repair_block(block, provider, prev_codes, compiler_error, idx).await?;
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
