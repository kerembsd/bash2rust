//! Rust code generation from `bash2rust-ir`.

pub mod error;

use std::collections::{HashMap, HashSet};

use bash2rust_ir::{
    AndOrStmt, ArithForStmt, AssignTarget, AssignValue, Assignment, BinaryTestOp, Block, CaseStmt,
    CommandCall, FileTestOp, ForStmt, FunctionDef, IfStmt, IrProgram, LogicalOp, LoopKind,
    LoopStmt, ParamExpansionOp, PipelineStmt, Redirect, RedirectKind, Stmt, TestExpr, TestStmt,
    UnaryTestOp, VarQualifier, Word, WordPart,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

pub use error::CodegenError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VarType {
    OwnedString,
    OwnedArray,
    StrSlice,
}

#[derive(Debug, Clone)]
struct VarBinding {
    ident: syn::Ident,
    ty: VarType,
    readonly: bool,
}

#[derive(Debug, Clone, Default)]
struct Scope {
    vars: HashMap<String, VarBinding>,
    used_idents: HashSet<String>,
    in_function: bool,
    in_source: bool,
    unguarded: bool,
    source_name: String,
    functions: HashMap<String, syn::Ident>,
}

impl Scope {
    fn new(in_function: bool) -> Self {
        Self::with_source(in_function, "<inline>")
    }

    fn with_source(in_function: bool, source_name: &str) -> Self {
        Self {
            vars: HashMap::new(),
            used_idents: HashSet::new(),
            in_function,
            in_source: false,
            unguarded: false,
            source_name: source_name.to_owned(),
            functions: HashMap::new(),
        }
    }

    fn from_ctx(ctx: &CodegenCtx, in_function: bool) -> Self {
        Self {
            vars: HashMap::new(),
            used_idents: HashSet::new(),
            in_function,
            in_source: false,
            unguarded: false,
            source_name: ctx.source_name.clone(),
            functions: ctx.functions.clone(),
        }
    }

    fn declare(&mut self, bash_name: &str, ty: VarType) -> syn::Ident {
        if let Some(existing) = self.vars.get_mut(bash_name) {
            existing.ty = ty;
            return existing.ident.clone();
        }
        let base = to_snake_ident_base(bash_name);
        let mut candidate = base.clone();
        let mut suffix = 2usize;
        while self.used_idents.contains(&candidate) {
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
        self.used_idents.insert(candidate.clone());
        let ident = safe_ident(&candidate);
        self.vars.insert(
            bash_name.to_owned(),
            VarBinding {
                ident: ident.clone(),
                ty,
                readonly: false,
            },
        );
        ident
    }

    fn mark_readonly(&mut self, bash_name: &str) {
        if let Some(existing) = self.vars.get_mut(bash_name) {
            existing.readonly = true;
        }
    }

    fn get(&self, bash_name: &str) -> Option<&VarBinding> {
        self.vars.get(bash_name)
    }
}

struct CodegenCtx {
    functions: HashMap<String, syn::Ident>,
    all_functions: Vec<(syn::Ident, FunctionDef)>,
    source_name: String,
}

impl CodegenCtx {
    fn empty() -> Self {
        Self {
            functions: HashMap::new(),
            all_functions: Vec::new(),
            source_name: String::from("<inline>"),
        }
    }

    fn from_program(program: &IrProgram) -> Self {
        let mut functions = HashMap::new();
        let mut all_functions = Vec::new();
        let mut used = HashSet::new();
        collect_functions_in_stmts(
            &program.statements,
            &mut functions,
            &mut all_functions,
            &mut used,
        );
        Self {
            functions,
            all_functions,
            source_name: program.source.clone(),
        }
    }
}

fn collect_functions_in_stmts(
    stmts: &[Stmt],
    functions: &mut HashMap<String, syn::Ident>,
    all_functions: &mut Vec<(syn::Ident, FunctionDef)>,
    used: &mut HashSet<String>,
) {
    for stmt in stmts {
        collect_functions_in_stmt(stmt, functions, all_functions, used);
    }
}

fn collect_functions_in_stmt(
    stmt: &Stmt,
    functions: &mut HashMap<String, syn::Ident>,
    all_functions: &mut Vec<(syn::Ident, FunctionDef)>,
    used: &mut HashSet<String>,
) {
    match stmt {
        Stmt::Function(func) => {
            let base = to_snake_ident_base(&func.name);
            let mut candidate = base.clone();
            let mut suffix = 2usize;
            while used.contains(&candidate) || candidate == "main" {
                candidate = format!("{base}_{suffix}");
                suffix += 1;
            }
            used.insert(candidate.clone());
            let ident = safe_ident(&candidate);
            functions.insert(func.name.clone(), ident.clone());
            all_functions.push((ident, func.clone()));
            collect_functions_in_stmts(&func.body.statements, functions, all_functions, used);
        }
        Stmt::If(i) => {
            collect_functions_in_stmts(&i.condition.statements, functions, all_functions, used);
            collect_functions_in_stmts(&i.then_branch.statements, functions, all_functions, used);
            for b in &i.elif_branches {
                collect_functions_in_stmts(&b.condition.statements, functions, all_functions, used);
                collect_functions_in_stmts(&b.body.statements, functions, all_functions, used);
            }
            if let Some(e) = &i.else_branch {
                collect_functions_in_stmts(&e.statements, functions, all_functions, used);
            }
        }
        Stmt::Case(c) => {
            for item in &c.items {
                collect_functions_in_stmts(&item.body.statements, functions, all_functions, used);
            }
        }
        Stmt::For(f) => {
            collect_functions_in_stmts(&f.body.statements, functions, all_functions, used);
        }
        Stmt::ArithFor(f) => {
            collect_functions_in_stmts(&f.body.statements, functions, all_functions, used);
        }
        Stmt::Loop(l) => {
            collect_functions_in_stmts(&l.condition.statements, functions, all_functions, used);
            collect_functions_in_stmts(&l.body.statements, functions, all_functions, used);
        }
        Stmt::Block(b) => {
            collect_functions_in_stmts(&b.statements, functions, all_functions, used);
        }
        Stmt::Source { body: Some(b), .. } => {
            collect_functions_in_stmts(&b.statements, functions, all_functions, used);
        }
        Stmt::AndOr(ao) => {
            collect_functions_in_stmt(&ao.first, functions, all_functions, used);
            for r in &ao.rest {
                collect_functions_in_stmt(&r.rhs, functions, all_functions, used);
            }
        }
        Stmt::Pipeline(p) => {
            collect_functions_in_stmts(&p.commands, functions, all_functions, used);
        }
        _ => {}
    }
}

/// Parses an AI-generated Rust snippet into [`syn::Stmt`]s.
///
/// Accepts either a full `fn main() { ... }` file (unwrapping its body statements while
/// preserving sibling items such as `use` imports) or bare statements parsed inside `{ ... }`.
pub fn parse_rust_snippet(code: &str) -> Result<Vec<syn::Stmt>, CodegenError> {
    if let Ok(file) = syn::parse_file(code) {
        if file
            .items
            .iter()
            .any(|item| matches!(item, syn::Item::Fn(f) if f.sig.ident == "main"))
        {
            let mut stmts = Vec::new();
            for item in file.items {
                match item {
                    syn::Item::Fn(item_fn) if item_fn.sig.ident == "main" => {
                        stmts.extend(item_fn.block.stmts);
                    }
                    other => stmts.push(syn::Stmt::Item(other)),
                }
            }
            return Ok(stmts);
        }
    }

    let wrapped = format!("{{ {code} }}");
    match syn::parse_str::<syn::Block>(&wrapped) {
        Ok(block) => Ok(block.stmts),
        Err(err) => Err(CodegenError::InvalidAiRust {
            message: err.to_string(),
            code: code.to_owned(),
        }),
    }
}

fn validate_raw_rust_in_block(block: &Block) -> Result<(), CodegenError> {
    for stmt in &block.statements {
        validate_raw_rust_in_stmt(stmt)?;
    }
    Ok(())
}

fn validate_raw_rust_in_stmt(stmt: &Stmt) -> Result<(), CodegenError> {
    match stmt {
        Stmt::RawRust { code, .. } => {
            let _ = parse_rust_snippet(code)?;
            Ok(())
        }
        Stmt::If(i) => {
            validate_raw_rust_in_block(&i.condition)?;
            validate_raw_rust_in_block(&i.then_branch)?;
            for b in &i.elif_branches {
                validate_raw_rust_in_block(&b.condition)?;
                validate_raw_rust_in_block(&b.body)?;
            }
            if let Some(e) = &i.else_branch {
                validate_raw_rust_in_block(e)?;
            }
            Ok(())
        }
        Stmt::For(f) => validate_raw_rust_in_block(&f.body),
        Stmt::ArithFor(f) => validate_raw_rust_in_block(&f.body),
        Stmt::Loop(l) => {
            validate_raw_rust_in_block(&l.condition)?;
            validate_raw_rust_in_block(&l.body)
        }
        Stmt::Function(f) => validate_raw_rust_in_block(&f.body),
        Stmt::Block(b) => validate_raw_rust_in_block(b),
        Stmt::Source { body: Some(b), .. } => validate_raw_rust_in_block(b),
        Stmt::Source { body: None, .. } => Ok(()),
        Stmt::Case(c) => {
            for item in &c.items {
                validate_raw_rust_in_block(&item.body)?;
            }
            Ok(())
        }
        Stmt::AndOr(ao) => {
            validate_raw_rust_in_stmt(&ao.first)?;
            for item in &ao.rest {
                validate_raw_rust_in_stmt(&item.rhs)?;
            }
            Ok(())
        }
        Stmt::Pipeline(p) => {
            for c in &p.commands {
                validate_raw_rust_in_stmt(c)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Emits a valid Rust `fn main() -> Result<(), Box<dyn std::error::Error>>` [`TokenStream`]
/// along with any hoisted top-level Bash function definitions.
pub fn emit_program_tokens(program: &IrProgram) -> Result<TokenStream, CodegenError> {
    for stmt in &program.statements {
        validate_raw_rust_in_stmt(stmt)?;
    }

    let ctx = CodegenCtx::from_program(program);
    let mut fn_items = Vec::new();
    for (fn_ident, func) in &ctx.all_functions {
        fn_items.push(emit_function_def(&ctx, fn_ident, func));
    }
    let mut main_stmts = Vec::new();
    let mut main_scope = Scope::from_ctx(&ctx, false);

    let mut nested_assigns = Vec::new();
    for stmt in &program.statements {
        if !matches!(stmt, Stmt::Function(_) | Stmt::Assign(_)) {
            collect_stmt_assigns(stmt, &mut nested_assigns);
        }
    }
    let mut hoisted_main = Vec::new();
    for (name, is_array) in nested_assigns {
        if main_scope.get(&name).is_none() {
            let ty = if is_array {
                VarType::OwnedArray
            } else {
                VarType::OwnedString
            };
            let ident = main_scope.declare(&name, ty);
            if is_array {
                hoisted_main.push(quote! { let mut #ident: Vec<String> = Vec::new(); });
            } else {
                hoisted_main.push(quote! { let mut #ident = String::new(); });
            }
        }
    }

    for stmt in &program.statements {
        if !matches!(stmt, Stmt::Function(_)) {
            main_stmts.push(emit_stmt(&ctx, &mut main_scope, stmt));
        }
    }

    let dynamic_arms: Vec<TokenStream> = ctx
        .functions
        .iter()
        .map(|(bash_name, fn_ident)| {
            quote! {
                #bash_name => {
                    #fn_ident(__shell, args)?;
                    Ok(Some(__shell.last_status))
                }
            }
        })
        .collect();

    Ok(quote! {
        #(#fn_items)*

        #[allow(dead_code)]
        fn __dispatch_dynamic_fn(
            __shell: &mut bash2rust_runtime::ShellState,
            prog: &str,
            args: &[&str],
        ) -> Result<Option<i32>, Box<dyn std::error::Error>> {
            match prog {
                #(#dynamic_arms)*
                _ => {
                    if let Some(__res) = __shell.call_sourced_fn(prog, args) {
                        Ok(Some(__res?))
                    } else {
                        Ok(None)
                    }
                }
            }
        }

        fn main() -> Result<(), Box<dyn std::error::Error>> {
            let mut __shell = bash2rust_runtime::ShellState::default();
            __shell.set_fn_dispatcher(__dispatch_dynamic_fn);
            let _ = &mut __shell;
            let __cli_args: Vec<String> = std::env::args().skip(1).collect();
            #[allow(unused_mut)]
            let mut args: Vec<&str> = __cli_args.iter().map(String::as_str).collect();
            let _ = &args;
            __shell.set_positional_args(&args);
            #(#hoisted_main)*
            #(#main_stmts)*
            __shell.run_exit_trap();
            Ok(())
        }
    })
}

/// Emits and validates a [`syn::File`] for the given [`IrProgram`].
pub fn emit_program_file(program: &IrProgram) -> Result<syn::File, CodegenError> {
    let tokens = emit_program_tokens(program)?;
    Ok(syn::parse2::<syn::File>(tokens)?)
}

/// Generates formatted Rust source code for the given [`IrProgram`].
pub fn generate_rust(program: &IrProgram) -> Result<String, CodegenError> {
    let file = emit_program_file(program)?;
    Ok(prettyplease::unparse(&file))
}

/// Emits a [`Block`] of statements as a [`TokenStream`].
pub fn emit_block(block: &Block) -> TokenStream {
    let ctx = CodegenCtx::empty();
    let mut scope = Scope::with_source(false, &ctx.source_name);
    emit_block_with(&ctx, &mut scope, block)
}

fn emit_block_with(ctx: &CodegenCtx, scope: &mut Scope, block: &Block) -> TokenStream {
    let stmts: Vec<TokenStream> = block
        .statements
        .iter()
        .map(|s| emit_stmt(ctx, scope, s))
        .collect();
    quote! { #(#stmts)* }
}

/// Emits a single IR [`Stmt`] as a [`TokenStream`].
pub fn emit_stmt_standalone(stmt: &Stmt) -> TokenStream {
    let ctx = CodegenCtx::empty();
    let mut scope = Scope::with_source(false, &ctx.source_name);
    emit_stmt(&ctx, &mut scope, stmt)
}

fn emit_stmt(ctx: &CodegenCtx, scope: &mut Scope, stmt: &Stmt) -> TokenStream {
    match stmt {
        Stmt::Assign(assign) => emit_assignment_with(scope, assign),
        Stmt::If(if_stmt) => emit_if_stmt_with(ctx, scope, if_stmt),
        Stmt::Case(case_stmt) => emit_case_stmt_with(ctx, scope, case_stmt),
        Stmt::For(for_stmt) => emit_for_stmt_with(ctx, scope, for_stmt),
        Stmt::ArithFor(arith_for) => emit_arith_for_stmt_with(ctx, scope, arith_for),
        Stmt::Arith(arith) => emit_arith_stmt(scope, arith),
        Stmt::Loop(loop_stmt) => emit_loop_stmt_with(ctx, scope, loop_stmt),
        Stmt::Function(_) => quote! {},
        Stmt::Return { code, .. } => emit_return_stmt(scope, code.as_ref()),
        Stmt::Source {
            path, args, body, ..
        } => {
            if let Some(inc_block) = body {
                let mut hoisted_assigns = Vec::new();
                if !scope.in_function {
                    collect_block_assigns(inc_block, &mut hoisted_assigns);
                }
                let mut hoisted = Vec::new();
                for (name, is_array) in hoisted_assigns {
                    if scope.get(&name).is_none() {
                        if is_array {
                            let ident = scope.declare(&name, VarType::OwnedArray);
                            hoisted.push(quote! {
                                let mut #ident: Vec<String> = Vec::new();
                            });
                        } else {
                            let ident = scope.declare(&name, VarType::OwnedString);
                            hoisted.push(quote! {
                                let mut #ident = String::new();
                            });
                        }
                    }
                }
                let saved_in_source = scope.in_source;
                scope.in_source = true;
                let inner = emit_block_with(ctx, scope, inc_block);
                scope.in_source = saved_in_source;
                let errexit_stmt = if scope.unguarded {
                    quote! {}
                } else {
                    quote! { __shell.check_errexit()?; }
                };
                if args.is_empty() {
                    quote! {
                        #(#hoisted)*
                        {
                            let __src_res = (|| -> Result<(), Box<dyn std::error::Error>> {
                                #inner
                                Ok(())
                            })();
                            __src_res?;
                            #errexit_stmt
                        }
                    }
                } else {
                    let (args_setup, args_slice) = emit_command_args_setup(scope, args);
                    quote! {
                        #(#hoisted)*
                        {
                            #args_setup
                            let __saved_pos = __shell.positional_args.clone();
                            let __saved_local_args = args;
                            let mut args: Vec<&str> = (#args_slice).to_vec();
                            let _ = &args;
                            __shell.set_positional_args(&args);
                            let __src_res = (|| -> Result<(), Box<dyn std::error::Error>> {
                                #inner
                                Ok(())
                            })();
                            args = __saved_local_args;
                            let _ = &args;
                            __shell.set_positional_args(&args);
                            __shell.positional_args = __saved_pos;
                            __src_res?;
                            #errexit_stmt
                        }
                    }
                }
            } else {
                let path_expr = emit_word_string_expr(scope, path);
                let (args_setup, args_slice) = emit_command_args_setup(scope, args);
                let post_sync = emit_post_call_sync(scope);
                let errexit_stmt = if scope.unguarded {
                    quote! {}
                } else {
                    quote! { __shell.check_errexit()?; }
                };
                quote! {
                    {
                        let __src_path = #path_expr;
                        #args_setup
                        let _ = __shell.source_file(&__src_path, #args_slice)?;
                        #post_sync
                        #errexit_stmt
                    }
                }
            }
        }
        Stmt::Block(block) => {
            if block.is_subshell {
                let mut sub_scope = scope.clone();
                let inner = emit_block_with(ctx, &mut sub_scope, block);
                let shadows: Vec<TokenStream> = scope
                    .vars
                    .values()
                    .map(|b| {
                        let ident = &b.ident;
                        match b.ty {
                            VarType::OwnedString | VarType::OwnedArray => {
                                quote! { let mut #ident = #ident.clone(); }
                            }
                            VarType::StrSlice => {
                                quote! { let #ident = #ident; }
                            }
                        }
                    })
                    .collect();
                let has_stdin_redir = block.redirects.iter().any(|r| {
                    r.fd == Some(0)
                        || (r.fd.is_none()
                            && matches!(
                                r.kind,
                                RedirectKind::Input
                                    | RedirectKind::HereDoc { .. }
                                    | RedirectKind::HereString
                                    | RedirectKind::InputProcSubst
                            ))
                });
                let (save_stdin, restore_stdin) = if has_stdin_redir {
                    (
                        quote! { let __saved_stdin = __shell.take_stdin_override(); },
                        quote! { __shell.set_stdin_override(__saved_stdin); },
                    )
                } else {
                    (quote! {}, quote! {})
                };
                let (pre_redir, post_redir) = if block.redirects.is_empty() {
                    (quote! {}, quote! {})
                } else {
                    let (redir_bindings, redir_specs_in) =
                        emit_redirect_specs_with_bindings(scope, &block.redirects);
                    let redir_specs_out = redir_specs_in.clone();
                    (
                        quote! {
                            #(#redir_bindings)*
                            __shell.push_stdin_from_redirects(&[#(#redir_specs_in),*])?;
                            __shell.push_stdout_capture();
                        },
                        quote! {
                            let __captured = __shell.pop_stdout_capture_raw();
                            __shell.apply_captured_redirects(&__captured, &[#(#redir_specs_out),*])?;
                        },
                    )
                };
                let errexit_after = if scope.unguarded {
                    quote! {}
                } else {
                    quote! { __shell.check_errexit()?; }
                };
                quote! {
                    {
                        let __saved_cwd = std::env::current_dir().ok();
                        let __saved_vars = __shell.vars_snapshot();
                        #save_stdin
                        let __saved_opts = (
                            __shell.errexit,
                            __shell.nounset,
                            __shell.pipefail,
                            __shell.ifs.clone(),
                        );
                        #pre_redir
                        __shell.enter_subshell();
                        let __sub_res = (|| -> Result<(), Box<dyn std::error::Error>> {
                            #(#shadows)*
                            #inner
                            Ok(())
                        })();
                        __shell.leave_subshell();
                        __shell.restore_vars(__saved_vars);
                        __shell.errexit = __saved_opts.0;
                        __shell.nounset = __saved_opts.1;
                        __shell.pipefail = __saved_opts.2;
                        __shell.ifs = __saved_opts.3;
                        if let Some(__d) = __saved_cwd {
                            let _ = std::env::set_current_dir(__d);
                        }
                        #post_redir
                        #restore_stdin
                        if let Err(__e) = __sub_res {
                            if let Some(bash2rust_runtime::RuntimeError::SubshellExit(__code)) =
                                __e.downcast_ref::<bash2rust_runtime::RuntimeError>()
                            {
                                __shell.set_status(*__code);
                            } else {
                                return Err(__e);
                            }
                        }
                        #errexit_after
                    }
                }
            } else {
                let inner = emit_block_with(ctx, scope, block);
                if block.redirects.is_empty() {
                    quote! { { #inner } }
                } else {
                    let (redir_bindings, redir_specs_in) =
                        emit_redirect_specs_with_bindings(scope, &block.redirects);
                    let redir_specs_out = redir_specs_in.clone();
                    quote! {
                        {
                            #(#redir_bindings)*
                            __shell.push_stdin_from_redirects(&[#(#redir_specs_in),*])?;
                            __shell.push_stdout_capture();
                            #inner
                            let __captured = __shell.pop_stdout_capture_raw();
                            __shell.apply_captured_redirects(&__captured, &[#(#redir_specs_out),*])?;
                        }
                    }
                }
            }
        }
        Stmt::Command(cmd) => emit_command_call(ctx, scope, cmd),
        Stmt::Test(test_stmt) => emit_test_stmt(scope, test_stmt),
        Stmt::AndOr(and_or) => emit_and_or_stmt(ctx, scope, and_or),
        Stmt::Pipeline(pipeline) => emit_pipeline_stmt(ctx, scope, pipeline),
        Stmt::Unsupported { construct, raw, .. } => {
            quote! {
                let _ = (#construct, #raw);
            }
        }
        Stmt::RawRust { code, .. } => match parse_rust_snippet(code) {
            Ok(stmts) => quote! { #(#stmts)* },
            Err(_) => quote! {},
        },
    }
}

fn emit_return_stmt(scope: &Scope, code: Option<&Word>) -> TokenStream {
    if scope.in_function || scope.in_source {
        match code {
            None => quote! { return Ok(()); },
            Some(w) => {
                let s = emit_word_string_expr(scope, w);
                quote! {
                    {
                        let __rc = (#s).trim().parse::<i32>().unwrap_or(0);
                        __shell.set_status(__rc);
                        return Ok(());
                    }
                }
            }
        }
    } else {
        match code {
            None => quote! { std::process::exit(__shell.last_status); },
            Some(w) => {
                let s = emit_word_string_expr(scope, w);
                quote! {
                    {
                        let __rc = (#s).trim().parse::<i32>().unwrap_or(0);
                        std::process::exit(__rc);
                    }
                }
            }
        }
    }
}

fn emit_function_def(ctx: &CodegenCtx, fn_ident: &syn::Ident, func: &FunctionDef) -> TokenStream {
    let mut fn_scope = Scope::from_ctx(ctx, true);
    let mut local_nested = Vec::new();
    for stmt in &func.body.statements {
        if !matches!(stmt, Stmt::Assign(_)) {
            collect_stmt_local_assigns(stmt, &mut local_nested);
        }
    }
    let mut hoisted_fn = Vec::new();
    for (name, is_array) in local_nested {
        if fn_scope.get(&name).is_none() {
            let ty = if is_array {
                VarType::OwnedArray
            } else {
                VarType::OwnedString
            };
            let ident = fn_scope.declare(&name, ty);
            if is_array {
                hoisted_fn.push(quote! { let mut #ident: Vec<String> = Vec::new(); });
            } else {
                hoisted_fn.push(quote! { let mut #ident = String::new(); });
            }
        }
    }
    let body = emit_block_with(ctx, &mut fn_scope, &func.body);

    quote! {
        fn #fn_ident(__shell: &mut bash2rust_runtime::ShellState, __raw_args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
            let _ = &mut *__shell;
            #[allow(unused_mut)]
            let mut args: Vec<&str> = __raw_args.to_vec();
            let _ = &args;
            __shell.set_positional_args(&args);
            #(#hoisted_fn)*
            #body
            Ok(())
        }
    }
}

fn is_positional_var(name: &str) -> bool {
    matches!(name, "@" | "*" | "#")
        || (!name.is_empty() && name.chars().all(|c| c.is_ascii_digit()))
}

/// Emits a Rust variable binding, mutation, or environment export for an [`Assignment`].
pub fn emit_assignment(assign: &Assignment) -> TokenStream {
    let mut scope = Scope::new(false);
    emit_assignment_with(&mut scope, assign)
}

fn word_has_command_subst(word: &Word) -> bool {
    word.parts.iter().any(|p| match p {
        WordPart::CommandSubst(_) => true,
        WordPart::ParamExpansion {
            op: ParamExpansionOp::UseDefault { default, .. },
            ..
        } => word_has_command_subst(default),
        _ => false,
    })
}

fn assign_has_command_subst(val: &AssignValue) -> bool {
    match val {
        AssignValue::Scalar(w) => word_has_command_subst(w),
        AssignValue::Array(items) => items.iter().any(|i| word_has_command_subst(&i.value)),
    }
}

fn emit_assignment_with(scope: &mut Scope, assign: &Assignment) -> TokenStream {
    let body = emit_assignment_with_inner(scope, assign);
    let nameref_sync = if !scope.in_function {
        let post_sync = emit_post_call_sync(scope);
        if post_sync.is_empty() {
            quote! {}
        } else {
            quote! {
                if __shell.has_namerefs() {
                    #post_sync
                }
            }
        }
    } else {
        quote! {}
    };
    if assign_has_command_subst(&assign.value) {
        if scope.unguarded {
            quote! {
                #body
                #nameref_sync
            }
        } else {
            quote! {
                #body
                #nameref_sync
                __shell.check_errexit()?;
            }
        }
    } else {
        quote! {
            #body
            #nameref_sync
            __shell.set_status(0);
        }
    }
}

fn emit_assignment_with_inner(scope: &mut Scope, assign: &Assignment) -> TokenStream {
    let val_expr = match &assign.value {
        AssignValue::Scalar(word) => emit_word_string_expr(scope, word),
        AssignValue::Array(items) => {
            let words: Vec<Word> = items.iter().map(|item| item.value.clone()).collect();
            if words.iter().any(word_needs_expansion) {
                let expand_stmts = emit_expanded_word_vec(scope, &words, quote! { __arr_expanded });
                quote! {
                    {
                        #expand_stmts
                        __arr_expanded
                    }
                }
            } else {
                let elems: Vec<TokenStream> = words
                    .iter()
                    .map(|w| emit_word_string_expr(scope, w))
                    .collect();
                quote! { vec![#(#elems),*] }
            }
        }
    };
    let is_array_val = matches!(assign.value, AssignValue::Array(_));

    match &assign.target {
        AssignTarget::Variable { name } => {
            let use_global_shell = scope.in_function
                && assign.qualifier != VarQualifier::Local
                && scope.get(name).is_none();

            if use_global_shell {
                let ifs_update = if name == "IFS" {
                    quote! { __shell.ifs = __shell.get_var("IFS").map(str::to_owned); }
                } else {
                    quote! {}
                };
                return if assign.append {
                    match &assign.value {
                        AssignValue::Scalar(word) => {
                            let s_expr = emit_word_string_expr(scope, word);
                            quote! {
                                {
                                    let __s = #s_expr;
                                    __shell.append_var(#name, &__s);
                                    #ifs_update
                                }
                            }
                        }
                        AssignValue::Array(_) => quote! {
                            {
                                let __arr_val = #val_expr;
                                __shell.append_array(#name, __arr_val);
                            }
                        },
                    }
                } else {
                    match assign.qualifier {
                        VarQualifier::Export => {
                            if is_array_val {
                                quote! {
                                    {
                                        let __arr_val = #val_expr;
                                        __shell.set_array(#name, __arr_val);
                                    }
                                }
                            } else {
                                quote! {
                                    {
                                        let __val = #val_expr;
                                        __shell.set_var(#name, __val);
                                        if let Some(__v) = __shell.get_var(#name) {
                                            std::env::set_var(#name, __v);
                                        }
                                        #ifs_update
                                    }
                                }
                            }
                        }
                        VarQualifier::Global | VarQualifier::Local | VarQualifier::Readonly => {
                            if is_array_val {
                                quote! {
                                    {
                                        let __arr_val = #val_expr;
                                        __shell.set_array(#name, __arr_val);
                                    }
                                }
                            } else {
                                quote! {
                                    {
                                        let __val = #val_expr;
                                        __shell.set_var(#name, __val);
                                        #ifs_update
                                    }
                                }
                            }
                        }
                    }
                };
            }

            let target_ty = if is_array_val {
                VarType::OwnedArray
            } else {
                VarType::OwnedString
            };
            let already_owned = scope.get(name).is_some_and(|b| b.ty == target_ty);
            let ident = scope.declare(name, target_ty);
            let ifs_update = if name == "IFS" {
                quote! { __shell.ifs = Some(#ident.clone()); }
            } else {
                quote! {}
            };
            let shell_sync = if !scope.in_function {
                if is_array_val {
                    quote! { __shell.set_array(#name, #ident.clone()); }
                } else {
                    quote! { __shell.set_var(#name, #ident.clone()); }
                }
            } else {
                quote! {}
            };

            if assign.append {
                match &assign.value {
                    AssignValue::Scalar(word) => {
                        let s_expr = if word.parts.is_empty() {
                            let unquoted = unquote_bash_word(&word.raw);
                            quote! { #unquoted }
                        } else {
                            let se = emit_word_string_expr(scope, word);
                            quote! { &(#se) }
                        };
                        quote! {
                            #ident.push_str(#s_expr);
                            #shell_sync
                            #ifs_update
                        }
                    }
                    AssignValue::Array(items) => {
                        let elems: Vec<TokenStream> = items
                            .iter()
                            .map(|item| emit_word_string_expr(scope, &item.value))
                            .collect();
                        quote! {
                            #ident.extend([#(#elems),*]);
                            #shell_sync
                        }
                    }
                }
            } else {
                let ty_ann = if matches!(&assign.value, AssignValue::Array(items) if items.is_empty())
                {
                    quote! { : Vec<String> }
                } else {
                    quote! {}
                };
                match assign.qualifier {
                    VarQualifier::Readonly => {
                        scope.mark_readonly(name);
                        quote! {
                            let #ident #ty_ann = #val_expr;
                            #shell_sync
                            #ifs_update
                        }
                    }
                    VarQualifier::Export => {
                        if already_owned {
                            quote! {
                                #ident = #val_expr;
                                std::env::set_var(#name, &#ident);
                                #shell_sync
                                #ifs_update
                            }
                        } else {
                            quote! {
                                let mut #ident #ty_ann = #val_expr;
                                std::env::set_var(#name, &#ident);
                                #shell_sync
                                #ifs_update
                            }
                        }
                    }
                    VarQualifier::Global | VarQualifier::Local => {
                        if already_owned {
                            quote! {
                                #ident = #val_expr;
                                #shell_sync
                                #ifs_update
                            }
                        } else {
                            quote! {
                                let mut #ident #ty_ann = #val_expr;
                                #shell_sync
                                #ifs_update
                            }
                        }
                    }
                }
            }
        }
        AssignTarget::ArrayElement { name, index } => {
            if scope.in_function && scope.get(name).is_none() {
                if let Ok(idx) = index.trim().parse::<usize>() {
                    return quote! {
                        {
                            let __val = #val_expr;
                            __shell.set_array_elem(#name, #idx, __val);
                        }
                    };
                }
                let idx_i64 = emit_arith_eval_i64(scope, index);
                return quote! {
                    {
                        let __idx = (#idx_i64).max(0) as usize;
                        let __val = #val_expr;
                        __shell.set_array_elem(#name, __idx, __val);
                    }
                };
            }
            let ident = scope.declare(name, VarType::OwnedArray);
            let shell_sync = if !scope.in_function {
                quote! { __shell.set_array(#name, #ident.clone()); }
            } else {
                quote! {}
            };
            if let Ok(idx) = index.trim().parse::<usize>() {
                let idx_lit = syn::Index::from(idx);
                quote! {
                    if #idx >= #ident.len() {
                        #ident.resize(#idx + 1, String::new());
                    }
                    #ident[#idx_lit] = #val_expr;
                    #shell_sync
                }
            } else {
                let idx_i64 = emit_arith_eval_i64(scope, index);
                quote! {
                    {
                        let __idx = (#idx_i64).max(0) as usize;
                        if __idx >= #ident.len() {
                            #ident.resize(__idx + 1, String::new());
                        }
                        #ident[__idx] = #val_expr;
                        #shell_sync
                    }
                }
            }
        }
    }
}

/// Emits a Rust `if` / `else if` / `else` statement for an [`IfStmt`].
pub fn emit_if_stmt(if_stmt: &IfStmt) -> TokenStream {
    let ctx = CodegenCtx::empty();
    let mut scope = Scope::with_source(false, &ctx.source_name);
    emit_if_stmt_with(&ctx, &mut scope, if_stmt)
}

fn emit_if_stmt_with(ctx: &CodegenCtx, scope: &mut Scope, if_stmt: &IfStmt) -> TokenStream {
    // Hoist variables assigned inside if/elif/else branches so they remain in scope after `if`.
    let mut hoisted = Vec::new();
    let mut branch_assigns = Vec::new();
    if scope.in_function {
        for s in &if_stmt.then_branch.statements {
            collect_stmt_local_assigns(s, &mut branch_assigns);
        }
        for branch in &if_stmt.elif_branches {
            for s in &branch.body.statements {
                collect_stmt_local_assigns(s, &mut branch_assigns);
            }
        }
        if let Some(else_block) = &if_stmt.else_branch {
            for s in &else_block.statements {
                collect_stmt_local_assigns(s, &mut branch_assigns);
            }
        }
    } else {
        collect_block_assigns(&if_stmt.then_branch, &mut branch_assigns);
        for branch in &if_stmt.elif_branches {
            collect_block_assigns(&branch.body, &mut branch_assigns);
        }
        if let Some(else_block) = &if_stmt.else_branch {
            collect_block_assigns(else_block, &mut branch_assigns);
        }
    }
    for (name, is_array) in branch_assigns {
        if scope.get(&name).is_none() {
            let ident = scope.declare(&name, VarType::OwnedString);
            if is_array {
                hoisted.push(quote! { let mut #ident: Vec<String> = Vec::new(); });
            } else {
                hoisted.push(quote! { let mut #ident = String::new(); });
            }
        }
    }

    let cond = emit_condition_expr(ctx, scope, &if_stmt.condition);
    let mut then_scope = scope.clone();
    let then_body = emit_block_with(ctx, &mut then_scope, &if_stmt.then_branch);

    let elif_tokens: Vec<TokenStream> = if_stmt
        .elif_branches
        .iter()
        .map(|branch| {
            let elif_cond = emit_condition_expr(ctx, scope, &branch.condition);
            let mut elif_scope = scope.clone();
            let elif_body = emit_block_with(ctx, &mut elif_scope, &branch.body);
            quote! {
                else if #elif_cond {
                    #elif_body
                }
            }
        })
        .collect();

    let else_tokens = if let Some(else_block) = &if_stmt.else_branch {
        let mut else_scope = scope.clone();
        let else_body = emit_block_with(ctx, &mut else_scope, else_block);
        quote! {
            else {
                #else_body
            }
        }
    } else {
        quote! {
            else {
                __shell.set_status(0);
            }
        }
    };

    quote! {
        #(#hoisted)*
        if #cond {
            #then_body
        } #(#elif_tokens)* #else_tokens
    }
}

fn emit_case_stmt_with(ctx: &CodegenCtx, scope: &mut Scope, case_stmt: &CaseStmt) -> TokenStream {
    let mut hoisted = Vec::new();
    let mut branch_assigns = Vec::new();
    if scope.in_function {
        for item in &case_stmt.items {
            for s in &item.body.statements {
                collect_stmt_local_assigns(s, &mut branch_assigns);
            }
        }
    } else {
        for item in &case_stmt.items {
            collect_block_assigns(&item.body, &mut branch_assigns);
        }
    }
    for (name, is_array) in branch_assigns {
        if scope.get(&name).is_none() {
            let ident = scope.declare(&name, VarType::OwnedString);
            if is_array {
                hoisted.push(quote! { let mut #ident: Vec<String> = Vec::new(); });
            } else {
                hoisted.push(quote! { let mut #ident = String::new(); });
            }
        }
    }

    if case_stmt.items.is_empty() {
        let val_expr = emit_word_string_expr(scope, &case_stmt.value);
        return quote! {
            #(#hoisted)*
            let _ = #val_expr;
        };
    }

    let val_expr = emit_word_string_expr(scope, &case_stmt.value);
    let mut branches = Vec::new();
    for (idx, item) in case_stmt.items.iter().enumerate() {
        let mut branch_scope = scope.clone();
        let body = emit_block_with(ctx, &mut branch_scope, &item.body);
        let is_wildcard = item.patterns.iter().any(|p| p.raw == "*" && !p.has_quotes);

        if is_wildcard {
            if idx == 0 {
                return quote! {
                    #(#hoisted)*
                    let __case_val = #val_expr;
                    let _ = &__case_val;
                    #body
                };
            }
            branches.push(quote! {
                else {
                    #body
                }
            });
            break;
        }

        let pat_checks: Vec<TokenStream> = item
            .patterns
            .iter()
            .map(|pat| {
                let pat_expr = emit_word_str_expr(scope, pat);
                quote! { bash2rust_runtime::pattern_match(#pat_expr, &__case_val) }
            })
            .collect();
        let cond = if pat_checks.is_empty() {
            quote! { false }
        } else {
            quote! { #(#pat_checks)||* }
        };

        if idx == 0 {
            branches.push(quote! {
                if #cond {
                    #body
                }
            });
        } else {
            branches.push(quote! {
                else if #cond {
                    #body
                }
            });
        }
    }

    quote! {
        #(#hoisted)*
        {
            let __case_val = #val_expr;
            #(#branches)*
        }
    }
}

fn collect_block_assigns(block: &Block, out: &mut Vec<(String, bool)>) {
    for stmt in &block.statements {
        collect_stmt_assigns(stmt, out);
    }
}

fn collect_stmt_assigns(stmt: &Stmt, out: &mut Vec<(String, bool)>) {
    match stmt {
        Stmt::Assign(a) => {
            if let AssignTarget::Variable { name } = &a.target {
                let is_array = matches!(a.value, AssignValue::Array(_));
                out.push((name.clone(), is_array));
            }
        }
        Stmt::If(i) => {
            collect_block_assigns(&i.then_branch, out);
            for b in &i.elif_branches {
                collect_block_assigns(&b.body, out);
            }
            if let Some(e) = &i.else_branch {
                collect_block_assigns(e, out);
            }
        }
        Stmt::Case(c) => {
            for item in &c.items {
                collect_block_assigns(&item.body, out);
            }
        }
        Stmt::For(f) => collect_block_assigns(&f.body, out),
        Stmt::ArithFor(f) => collect_block_assigns(&f.body, out),
        Stmt::Loop(l) => {
            collect_block_assigns(&l.condition, out);
            collect_block_assigns(&l.body, out);
        }
        Stmt::Block(b) => collect_block_assigns(b, out),
        Stmt::AndOr(ao) => {
            collect_stmt_assigns(&ao.first, out);
            for r in &ao.rest {
                collect_stmt_assigns(&r.rhs, out);
            }
        }
        Stmt::Pipeline(p) => {
            for c in &p.commands {
                collect_stmt_assigns(c, out);
            }
        }
        _ => {}
    }
}

fn collect_stmt_local_assigns(stmt: &Stmt, out: &mut Vec<(String, bool)>) {
    match stmt {
        Stmt::Assign(a) if a.qualifier == VarQualifier::Local => {
            if let AssignTarget::Variable { name } = &a.target {
                let is_array = matches!(a.value, AssignValue::Array(_));
                out.push((name.clone(), is_array));
            }
        }
        Stmt::If(i) => {
            for s in &i.then_branch.statements {
                collect_stmt_local_assigns(s, out);
            }
            for b in &i.elif_branches {
                for s in &b.body.statements {
                    collect_stmt_local_assigns(s, out);
                }
            }
            if let Some(e) = &i.else_branch {
                for s in &e.statements {
                    collect_stmt_local_assigns(s, out);
                }
            }
        }
        Stmt::Case(c) => {
            for item in &c.items {
                for s in &item.body.statements {
                    collect_stmt_local_assigns(s, out);
                }
            }
        }
        Stmt::For(f) => {
            for s in &f.body.statements {
                collect_stmt_local_assigns(s, out);
            }
        }
        Stmt::ArithFor(f) => {
            for s in &f.body.statements {
                collect_stmt_local_assigns(s, out);
            }
        }
        Stmt::Loop(l) => {
            for s in &l.body.statements {
                collect_stmt_local_assigns(s, out);
            }
        }
        Stmt::Block(b) => {
            for s in &b.statements {
                collect_stmt_local_assigns(s, out);
            }
        }
        Stmt::AndOr(ao) => {
            collect_stmt_local_assigns(&ao.first, out);
            for r in &ao.rest {
                collect_stmt_local_assigns(&r.rhs, out);
            }
        }
        _ => {}
    }
}

/// Emits a Rust `for` loop for a [`ForStmt`].
pub fn emit_for_stmt(for_stmt: &ForStmt) -> TokenStream {
    let ctx = CodegenCtx::empty();
    let mut scope = Scope::with_source(false, &ctx.source_name);
    emit_for_stmt_with(&ctx, &mut scope, for_stmt)
}

fn emit_for_stmt_with(ctx: &CodegenCtx, scope: &mut Scope, for_stmt: &ForStmt) -> TokenStream {
    let mut body_scope = scope.clone();
    let var_ty = if for_stmt.values.is_some() {
        VarType::StrSlice
    } else {
        VarType::OwnedString
    };
    let var_ident = body_scope.declare(&for_stmt.variable, var_ty);
    let body = emit_block_with(ctx, &mut body_scope, &for_stmt.body);

    match &for_stmt.values {
        Some(words) if words.iter().any(word_needs_expansion) => {
            let expand_stmts = emit_expanded_word_vec(scope, words, quote! { __items });
            quote! {
                {
                    #expand_stmts
                    for #var_ident in __items.iter().map(String::as_str) {
                        #body
                    }
                }
            }
        }
        Some(words) => {
            let items: Vec<TokenStream> =
                words.iter().map(|w| emit_word_str_expr(scope, w)).collect();
            quote! {
                for #var_ident in [#(#items),*] {
                    #body
                }
            }
        }
        None => quote! {
            for #var_ident in std::env::args().skip(1) {
                #body
            }
        },
    }
}

/// Emits a Rust loop block for an [`ArithForStmt`].
pub fn emit_arith_for_stmt(arith: &ArithForStmt) -> TokenStream {
    let ctx = CodegenCtx::empty();
    let mut scope = Scope::with_source(false, &ctx.source_name);
    emit_arith_for_stmt_with(&ctx, &mut scope, arith)
}

fn emit_arith_for_stmt_with(
    ctx: &CodegenCtx,
    scope: &mut Scope,
    arith: &ArithForStmt,
) -> TokenStream {
    let init_stmt = arith
        .initializer
        .as_ref()
        .map(|a| {
            let e = emit_arith_eval_i64(scope, &a.raw);
            quote! { let _ = #e; }
        })
        .unwrap_or_default();

    let cond_expr = arith
        .condition
        .as_ref()
        .map(|a| {
            let e = emit_arith_eval_i64(scope, &a.raw);
            quote! { #e != 0 }
        })
        .unwrap_or_else(|| quote! { true });

    let step_stmt = arith
        .updater
        .as_ref()
        .map(|a| {
            let e = emit_arith_eval_i64(scope, &a.raw);
            quote! { let _ = #e; }
        })
        .unwrap_or_default();

    let mut body_scope = scope.clone();
    let body = emit_block_with(ctx, &mut body_scope, &arith.body);

    quote! {
        {
            #init_stmt
            while #cond_expr {
                #body
                #step_stmt
            }
        }
    }
}

fn emit_arith_stmt(scope: &mut Scope, arith: &bash2rust_ir::ArithExpr) -> TokenStream {
    let eval_expr = emit_arith_eval_i64(scope, &arith.raw);
    quote! {
        {
            let __arith_val = #eval_expr;
            __shell.set_status(if __arith_val != 0 { 0 } else { 1 });
        }
    }
}

/// Emits a Rust `while` loop for a [`LoopStmt`] (`while` or `until`).
pub fn emit_loop_stmt(loop_stmt: &LoopStmt) -> TokenStream {
    let ctx = CodegenCtx::empty();
    let mut scope = Scope::with_source(false, &ctx.source_name);
    emit_loop_stmt_with(&ctx, &mut scope, loop_stmt)
}

fn emit_loop_stmt_with(ctx: &CodegenCtx, scope: &mut Scope, loop_stmt: &LoopStmt) -> TokenStream {
    let cond = emit_condition_expr(ctx, scope, &loop_stmt.condition);
    let mut body_scope = scope.clone();
    let body = emit_block_with(ctx, &mut body_scope, &loop_stmt.body);

    let loop_tokens = match loop_stmt.loop_kind {
        LoopKind::While => quote! {
            while #cond {
                #body
            }
        },
        LoopKind::Until => quote! {
            while !(#cond) {
                #body
            }
        },
    };

    if loop_stmt.redirects.is_empty() {
        loop_tokens
    } else {
        let (redir_bindings, redir_specs) =
            emit_redirect_specs_with_bindings(scope, &loop_stmt.redirects);
        let guarded_cond = quote! {
            {
                __shell.set_stdin_override(__loop_stdin.take());
                let __c = #cond;
                __loop_stdin = __shell.take_stdin_override();
                __c
            }
        };
        let guarded_loop = match loop_stmt.loop_kind {
            LoopKind::While => quote! {
                while #guarded_cond {
                    #body
                }
            },
            LoopKind::Until => quote! {
                while !(#guarded_cond) {
                    #body
                }
            },
        };
        quote! {
            {
                #(#redir_bindings)*
                __shell.push_stdin_from_redirects(&[#(#redir_specs),*])?;
                let mut __loop_stdin = __shell.take_stdin_override();
                #guarded_loop
                __shell.set_stdin_override(None);
            }
        }
    }
}

fn emit_test_stmt(scope: &Scope, test_stmt: &TestStmt) -> TokenStream {
    let expr = emit_test_expr_with(scope, &test_stmt.expr);
    quote! {
        {
            let __t = #expr;
            __shell.set_status(if __t { 0 } else { 1 });
        }
    }
}

/// Emits a Rust `bool` expression for a [`TestExpr`].
pub fn emit_test_expr(expr: &TestExpr) -> TokenStream {
    let scope = Scope::new(false);
    emit_test_expr_with(&scope, expr)
}

fn emit_test_expr_with(scope: &Scope, expr: &TestExpr) -> TokenStream {
    match expr {
        TestExpr::FileTest { op, path } => {
            let path_expr = emit_word_str_expr(scope, path);
            match op {
                FileTestOp::Dir => quote! { std::path::Path::new(#path_expr).is_dir() },
                FileTestOp::File => quote! { std::path::Path::new(#path_expr).is_file() },
                FileTestOp::Exists => quote! { std::path::Path::new(#path_expr).exists() },
                FileTestOp::Readable => quote! { bash2rust_runtime::test_readable(#path_expr) },
                FileTestOp::Writable => quote! { bash2rust_runtime::test_writable(#path_expr) },
                FileTestOp::NonEmptyFile => {
                    quote! { bash2rust_runtime::test_non_empty_file(#path_expr) }
                }
                FileTestOp::Symlink => quote! { bash2rust_runtime::test_symlink(#path_expr) },
                FileTestOp::BlockDevice => {
                    quote! { bash2rust_runtime::test_block_device(#path_expr) }
                }
                FileTestOp::CharDevice => {
                    quote! { bash2rust_runtime::test_char_device(#path_expr) }
                }
                FileTestOp::Executable => quote! { bash2rust_runtime::test_executable(#path_expr) },
                FileTestOp::FdIsTerminal => {
                    quote! { bash2rust_runtime::test_fd_terminal(#path_expr) }
                }
            }
        }
        TestExpr::Unary { op, operand } => {
            let op_expr = emit_word_str_expr(scope, operand);
            match op {
                UnaryTestOp::StrEmpty => quote! { (#op_expr).is_empty() },
                UnaryTestOp::StrNonEmpty => quote! { !(#op_expr).is_empty() },
            }
        }
        TestExpr::Binary { op, lhs, rhs } => {
            let lhs_expr = emit_word_str_expr(scope, lhs);
            let rhs_expr = emit_word_str_expr(scope, rhs);
            match op {
                BinaryTestOp::StrEq => quote! { (#lhs_expr) == (#rhs_expr) },
                BinaryTestOp::StrNe => quote! { (#lhs_expr) != (#rhs_expr) },
                BinaryTestOp::IntEq => {
                    quote! { bash2rust_runtime::test_int_eq(#lhs_expr, #rhs_expr) }
                }
                BinaryTestOp::IntNe => {
                    quote! { bash2rust_runtime::test_int_ne(#lhs_expr, #rhs_expr) }
                }
                BinaryTestOp::IntLt => {
                    quote! { bash2rust_runtime::test_int_lt(#lhs_expr, #rhs_expr) }
                }
                BinaryTestOp::IntLe => {
                    quote! { bash2rust_runtime::test_int_le(#lhs_expr, #rhs_expr) }
                }
                BinaryTestOp::IntGt => {
                    quote! { bash2rust_runtime::test_int_gt(#lhs_expr, #rhs_expr) }
                }
                BinaryTestOp::IntGe => {
                    quote! { bash2rust_runtime::test_int_ge(#lhs_expr, #rhs_expr) }
                }
                BinaryTestOp::RegexMatch => {
                    quote! { bash2rust_runtime::test_regex_match(#lhs_expr, #rhs_expr) }
                }
                BinaryTestOp::PatternMatch => {
                    quote! { bash2rust_runtime::pattern_match(#rhs_expr, #lhs_expr) }
                }
            }
        }
        TestExpr::Not(inner) => {
            let inner_expr = emit_test_expr_with(scope, inner);
            quote! { !(#inner_expr) }
        }
        TestExpr::And(lhs, rhs) => {
            let lhs_expr = emit_test_expr_with(scope, lhs);
            let rhs_expr = emit_test_expr_with(scope, rhs);
            quote! { (#lhs_expr) && (#rhs_expr) }
        }
        TestExpr::Or(lhs, rhs) => {
            let lhs_expr = emit_test_expr_with(scope, lhs);
            let rhs_expr = emit_test_expr_with(scope, rhs);
            quote! { (#lhs_expr) || (#rhs_expr) }
        }
    }
}

fn emit_cmd_env_assignment(scope: &mut Scope, a: &Assignment) -> TokenStream {
    if let AssignTarget::Variable { name } = &a.target {
        if name == "IFS" {
            if let AssignValue::Scalar(w) = &a.value {
                let val_expr = emit_word_string_expr(scope, w);
                return quote! { __shell.ifs = Some(#val_expr); };
            }
        }
    }
    emit_assignment_with(scope, a)
}

fn emit_command_call(ctx: &CodegenCtx, scope: &mut Scope, cmd: &CommandCall) -> TokenStream {
    let env_stmts: Vec<TokenStream> = cmd
        .env_assignments
        .iter()
        .map(|a| emit_cmd_env_assignment(scope, a))
        .collect();
    let Some(name_word) = &cmd.name else {
        return quote! { #(#env_stmts)* };
    };

    // Shell `set` builtin (e.g. `set -euo pipefail`) configures `__shell` strict-mode flags.
    if name_word.raw == "set" && cmd.redirects.is_empty() {
        let set_args: Vec<TokenStream> = cmd
            .args
            .iter()
            .map(|w| emit_word_str_expr(scope, w))
            .collect();
        return quote! {
            __shell.apply_set_flags(&[#(#set_args),*]);
        };
    }

    // Shell `unset` builtin (e.g. `unset IFS` or `unset var`).
    if name_word.raw == "unset" && cmd.redirects.is_empty() {
        let mut unset_stmts = Vec::new();
        for arg in &cmd.args {
            let var_name = unquote_bash_word(&arg.raw);
            if var_name == "IFS" {
                unset_stmts.push(quote! { __shell.ifs = None; });
            } else if let Some(binding) = scope.get(&var_name) {
                if binding.ty == VarType::OwnedString {
                    let ident = binding.ident.clone();
                    unset_stmts.push(quote! { #ident.clear(); });
                }
            }
        }
        return quote! { #(#unset_stmts)* };
    }

    // Shell `exit [n]` builtin when not inside `$(...)` or a function.
    if name_word.raw == "exit" && cmd.redirects.is_empty() && !scope.in_function {
        let code_expr = cmd
            .args
            .first()
            .map(|w| {
                let s = emit_word_string_expr(scope, w);
                quote! { (#s).trim().parse::<i32>().unwrap_or(0) }
            })
            .unwrap_or_else(|| quote! { __shell.last_status });
        return quote! {
            {
                let __exit_code = #code_expr;
                if __shell.in_subshell() {
                    return Err(Box::new(bash2rust_runtime::RuntimeError::SubshellExit(
                        __exit_code,
                    )));
                }
                __shell.run_exit_trap();
                std::process::exit(__exit_code);
            }
        };
    }

    // Shell `break` / `continue` builtins.
    if name_word.raw == "break" && cmd.redirects.is_empty() && cmd.args.is_empty() {
        return quote! { break; };
    }
    if name_word.raw == "continue" && cmd.redirects.is_empty() && cmd.args.is_empty() {
        return quote! { continue; };
    }

    // Shell `shift [n]` builtin mutates local `args` and syncs `__shell`.
    if name_word.raw == "shift" && cmd.redirects.is_empty() {
        let errexit_stmt = if scope.unguarded {
            quote! {}
        } else {
            quote! { __shell.check_errexit()?; }
        };
        let shift_stmt = emit_shift_stmt(scope, &cmd.args);
        return quote! {
            {
                #shift_stmt
                #errexit_stmt
            }
        };
    }

    // `command <name> ...` (without flags like `-v`/`-V`) and `builtin <name> ...` bypass defined Bash functions.
    let (effective_name, effective_args, bypass_fns) =
        if matches!(name_word.raw.as_str(), "command" | "builtin")
            && !cmd.args.is_empty()
            && !cmd.args[0].raw.starts_with('-')
        {
            (&cmd.args[0], &cmd.args[1..], true)
        } else {
            (name_word, cmd.args.as_slice(), false)
        };

    let (args_setup, args_slice) = emit_command_args_setup(scope, effective_args);

    let errexit_stmt = if scope.unguarded {
        quote! {}
    } else {
        quote! { __shell.check_errexit()?; }
    };

    // Direct call to a hoisted Bash function if defined in this script (including redirects).
    if !bypass_fns {
        if let Some(fn_ident) = ctx.functions.get(&effective_name.raw) {
            let call_block =
                emit_direct_fn_call(scope, fn_ident, &args_setup, &args_slice, &cmd.redirects);
            return quote! {
                {
                    #call_block
                    #errexit_stmt
                }
            };
        }
    }

    let (prog_setup, prog_ref, args_setup, args_slice) = if is_quoted_all_args(effective_name) {
        let (extra_setup, _) = emit_command_args_setup(scope, effective_args);
        (
            quote! { let __prog = args.first().copied().unwrap_or("").to_owned(); },
            quote! { &__prog },
            quote! {
                #extra_setup
                let mut __full_args: Vec<String> = args.iter().skip(1).map(|s| (*s).to_owned()).collect();
                __full_args.extend(__args);
                let __full_arg_refs: Vec<&str> = __full_args.iter().map(String::as_str).collect();
            },
            quote! { &__full_arg_refs },
        )
    } else {
        let prog_string = emit_word_string_expr(scope, effective_name);
        (
            quote! { let __prog = #prog_string; },
            quote! { &__prog },
            args_setup,
            args_slice,
        )
    };

    let is_dynamic_cmd =
        is_quoted_all_args(effective_name) || word_has_dynamic_parts(effective_name);
    let shell_arg = if scope.in_function {
        quote! { __shell }
    } else {
        quote! { &mut __shell }
    };
    let post_sync = emit_post_call_sync(scope);

    let call = if cmd.redirects.is_empty() {
        let dyn_dispatch = if is_dynamic_cmd && !bypass_fns {
            quote! {
                if __dispatch_dynamic_fn(#shell_arg, #prog_ref, #args_slice)?.is_some() {
                    #post_sync
                } else {
                    let _ = __shell.run_command(#prog_ref, #args_slice)?;
                }
            }
        } else {
            quote! { let _ = __shell.run_command(#prog_ref, #args_slice)?; }
        };
        quote! {
            {
                #prog_setup
                #args_setup
                #dyn_dispatch
                #errexit_stmt
            }
        }
    } else {
        let (redir_bindings, redir_specs) =
            emit_redirect_specs_with_bindings(scope, &cmd.redirects);
        quote! {
            {
                #prog_setup
                #args_setup
                #(#redir_bindings)*
                let _ = __shell.run_command_with_redirects(
                    #prog_ref,
                    #args_slice,
                    &[],
                    &[#(#redir_specs),*],
                )?;
                #errexit_stmt
            }
        }
    };

    if env_stmts.is_empty() {
        call
    } else {
        quote! {
            {
                let __saved_ifs = __shell.ifs.clone();
                #(#env_stmts)*
                #call
                __shell.ifs = __saved_ifs;
            }
        }
    }
}

fn emit_redirect_specs_with_bindings(
    scope: &Scope,
    redirects: &[Redirect],
) -> (Vec<TokenStream>, Vec<TokenStream>) {
    let mut bindings = Vec::new();
    let mut specs = Vec::with_capacity(redirects.len());
    for (idx, redir) in redirects.iter().enumerate() {
        let default_fd = match redir.kind {
            RedirectKind::Input
            | RedirectKind::HereString
            | RedirectKind::HereDoc { .. }
            | RedirectKind::InputProcSubst => 0u32,
            RedirectKind::OutputTruncate
            | RedirectKind::OutputAppend
            | RedirectKind::DupOutput { .. }
            | RedirectKind::CloseFd => 1u32,
        };
        let fd = redir.fd.unwrap_or(default_fd);
        let mode = match redir.kind {
            RedirectKind::OutputTruncate => {
                let target_expr = emit_word_str_expr(scope, &redir.target);
                quote! { bash2rust_runtime::RedirectMode::OutputTruncate(#target_expr) }
            }
            RedirectKind::OutputAppend => {
                let target_expr = emit_word_str_expr(scope, &redir.target);
                quote! { bash2rust_runtime::RedirectMode::OutputAppend(#target_expr) }
            }
            RedirectKind::Input => {
                let target_expr = emit_word_str_expr(scope, &redir.target);
                quote! { bash2rust_runtime::RedirectMode::Input(#target_expr) }
            }
            RedirectKind::HereString => {
                let s_expr = emit_word_string_expr(scope, &redir.target);
                let buf_ident = format_ident!("__redir_buf_{idx}");
                bindings.push(quote! { let #buf_ident = format!("{}\n", #s_expr); });
                quote! { bash2rust_runtime::RedirectMode::InputBytes(#buf_ident.as_bytes()) }
            }
            RedirectKind::HereDoc { .. } => {
                let s_expr = emit_word_string_expr(scope, &redir.target);
                let buf_ident = format_ident!("__redir_buf_{idx}");
                bindings.push(quote! { let #buf_ident = #s_expr; });
                quote! { bash2rust_runtime::RedirectMode::InputBytes(#buf_ident.as_bytes()) }
            }
            RedirectKind::InputProcSubst => {
                let buf_ident = format_ident!("__redir_buf_{idx}");
                if let Some(inner_stmt) = &redir.proc_subst {
                    let subst_expr =
                        emit_command_subst_raw_expr(scope, inner_stmt, &redir.target.raw);
                    bindings.push(quote! { let #buf_ident = #subst_expr; });
                } else {
                    bindings.push(quote! { let #buf_ident = String::new(); });
                }
                quote! { bash2rust_runtime::RedirectMode::InputBytes(#buf_ident.as_bytes()) }
            }
            RedirectKind::DupOutput { target_fd } => {
                if fd == 1 && target_fd == 2 {
                    quote! { bash2rust_runtime::RedirectMode::DupStdoutToStderr }
                } else {
                    quote! { bash2rust_runtime::RedirectMode::DupOutput { target_fd: #target_fd } }
                }
            }
            RedirectKind::CloseFd => {
                quote! { bash2rust_runtime::RedirectMode::CloseFd }
            }
        };
        specs.push(quote! {
            bash2rust_runtime::RedirectSpec {
                fd: #fd,
                mode: #mode,
            }
        });
    }
    (bindings, specs)
}

fn emit_and_or_stmt(ctx: &CodegenCtx, scope: &mut Scope, and_or: &AndOrStmt) -> TokenStream {
    let expr = emit_and_or_bool(ctx, scope, and_or);
    quote! {
        {
            let __ao_ok = #expr;
            __shell.set_status(if __ao_ok { 0 } else { 1 });
        }
    }
}

fn emit_and_or_bool(ctx: &CodegenCtx, scope: &mut Scope, and_or: &AndOrStmt) -> TokenStream {
    let mut lhs_scope = scope.clone();
    lhs_scope.unguarded = true;
    let mut expr = emit_bool_stmt(ctx, &mut lhs_scope, &and_or.first);
    let len = and_or.rest.len();
    for (idx, item) in and_or.rest.iter().enumerate() {
        let is_last = idx + 1 == len;
        let rhs = if is_last {
            emit_bool_stmt(ctx, scope, &item.rhs)
        } else {
            let mut mid_scope = scope.clone();
            mid_scope.unguarded = true;
            emit_bool_stmt(ctx, &mut mid_scope, &item.rhs)
        };
        expr = match item.op {
            LogicalOp::And => quote! { (#expr) && (#rhs) },
            LogicalOp::Or => quote! { (#expr) || (#rhs) },
        };
    }
    expr
}

fn emit_bool_stmt(ctx: &CodegenCtx, scope: &mut Scope, stmt: &Stmt) -> TokenStream {
    match stmt {
        Stmt::Test(test_stmt) => emit_test_expr_with(scope, &test_stmt.expr),
        Stmt::AndOr(and_or) => emit_and_or_bool(ctx, scope, and_or),
        Stmt::Arith(arith) => {
            let eval_expr = emit_arith_eval_i64(scope, &arith.raw);
            quote! {
                {
                    let __arith_val = #eval_expr;
                    __shell.set_status(if __arith_val != 0 { 0 } else { 1 });
                    __arith_val != 0
                }
            }
        }
        Stmt::Command(cmd) => {
            if let Some(name_word) = &cmd.name {
                if name_word.raw == "shift" && cmd.redirects.is_empty() {
                    let shift_stmt = emit_shift_stmt(scope, &cmd.args);
                    return quote! {
                        {
                            #shift_stmt
                            __shell.last_status == 0
                        }
                    };
                }

                let (effective_name, effective_args, bypass_fns) =
                    if matches!(name_word.raw.as_str(), "command" | "builtin")
                        && !cmd.args.is_empty()
                        && !cmd.args[0].raw.starts_with('-')
                    {
                        (&cmd.args[0], &cmd.args[1..], true)
                    } else {
                        (name_word, cmd.args.as_slice(), false)
                    };

                let (args_setup, args_slice) = emit_command_args_setup(scope, effective_args);

                if !bypass_fns {
                    if let Some(fn_ident) = ctx.functions.get(&effective_name.raw) {
                        let call_block = emit_direct_fn_call(
                            scope,
                            fn_ident,
                            &args_setup,
                            &args_slice,
                            &cmd.redirects,
                        );
                        return quote! {
                            {
                                #call_block
                                __shell.last_status == 0
                            }
                        };
                    }
                }

                let (prog_setup, prog_ref, args_setup, args_slice) = if is_quoted_all_args(
                    effective_name,
                ) {
                    (
                        quote! { let __prog = args.first().copied().unwrap_or("").to_owned(); },
                        quote! { &__prog },
                        quote! {
                            #args_setup
                            let mut __full_args: Vec<String> = args.iter().skip(1).map(|s| (*s).to_owned()).collect();
                            __full_args.extend(__args);
                            let __full_arg_refs: Vec<&str> = __full_args.iter().map(String::as_str).collect();
                        },
                        quote! { &__full_arg_refs },
                    )
                } else {
                    let prog = emit_word_string_expr(scope, effective_name);
                    (
                        quote! { let __prog = #prog; },
                        quote! { &__prog },
                        args_setup,
                        args_slice,
                    )
                };

                let env_stmts: Vec<TokenStream> = cmd
                    .env_assignments
                    .iter()
                    .map(|a| emit_cmd_env_assignment(scope, a))
                    .collect();

                let is_dynamic_cmd =
                    is_quoted_all_args(effective_name) || word_has_dynamic_parts(effective_name);
                let shell_arg = if scope.in_function {
                    quote! { __shell }
                } else {
                    quote! { &mut __shell }
                };
                let post_sync = emit_post_call_sync(scope);

                let inner_bool = if cmd.redirects.is_empty() {
                    if is_dynamic_cmd && !bypass_fns {
                        quote! {
                            {
                                #prog_setup
                                #args_setup
                                if let Some(__rc) = __dispatch_dynamic_fn(#shell_arg, #prog_ref, #args_slice)? {
                                    #post_sync
                                    __rc == 0
                                } else {
                                    __shell.run_command(#prog_ref, #args_slice).is_ok_and(|__rc| __rc == 0)
                                }
                            }
                        }
                    } else {
                        quote! {
                            {
                                #prog_setup
                                #args_setup
                                __shell.run_command(#prog_ref, #args_slice).is_ok_and(|__rc| __rc == 0)
                            }
                        }
                    }
                } else {
                    let (redir_bindings, redirect_specs) =
                        emit_redirect_specs_with_bindings(scope, &cmd.redirects);
                    quote! {
                        {
                            #prog_setup
                            #args_setup
                            #(#redir_bindings)*
                            __shell
                                .run_command_with_redirects(#prog_ref, #args_slice, &[], &[#(#redirect_specs),*])
                                .is_ok_and(|__rc| __rc == 0)
                        }
                    }
                };
                if env_stmts.is_empty() {
                    inner_bool
                } else {
                    quote! {
                        {
                            let __saved_ifs = __shell.ifs.clone();
                            #(#env_stmts)*
                            let __bool_res = #inner_bool;
                            __shell.ifs = __saved_ifs;
                            __bool_res
                        }
                    }
                }
            } else {
                quote! { true }
            }
        }
        Stmt::Pipeline(pipeline) => emit_pipeline_bool(ctx, scope, pipeline),
        Stmt::Assign(a) => {
            let s = emit_assignment_with(scope, a);
            quote! {
                {
                    #s
                    __shell.last_status == 0
                }
            }
        }
        other => {
            let mut sub_scope = scope.clone();
            sub_scope.unguarded = true;
            let s = emit_stmt(ctx, &mut sub_scope, other);
            quote! {
                {
                    #s
                    true
                }
            }
        }
    }
}

fn emit_post_call_sync(scope: &Scope) -> TokenStream {
    if scope.in_function {
        return quote! {};
    }
    let syncs: Vec<TokenStream> = scope
        .vars
        .iter()
        .filter_map(|(bash_name, binding)| {
            if binding.readonly {
                return None;
            }
            match binding.ty {
                VarType::OwnedString => {
                    let ident = &binding.ident;
                    Some(quote! {
                        if let Some(__v) = __shell.get_var(#bash_name) {
                            #ident = __v.to_owned();
                        }
                    })
                }
                VarType::OwnedArray => {
                    let ident = &binding.ident;
                    Some(quote! {
                        let __arr = __shell.get_array(#bash_name);
                        if !__arr.is_empty() {
                            #ident = __arr.to_vec();
                        }
                    })
                }
                VarType::StrSlice => None,
            }
        })
        .collect();
    quote! { #(#syncs)* }
}

fn emit_pipeline_stmt(ctx: &CodegenCtx, scope: &mut Scope, pipeline: &PipelineStmt) -> TokenStream {
    let errexit_stmt = if scope.unguarded {
        quote! {}
    } else {
        quote! { __shell.check_errexit()?; }
    };

    if !pipeline_calls_defined_fn(&ctx.functions, pipeline) {
        if let Some(stages) = collect_pipeline_stages(scope, pipeline) {
            if pipeline.negated {
                return quote! {
                    let __rc = __shell.run_pipeline_inherit_status(&[#(#stages),*], __shell.pipefail)?;
                    __shell.set_status(if __rc == 0 { 1 } else { 0 });
                };
            }
            return quote! {
                let __rc = __shell.run_pipeline_inherit_status(&[#(#stages),*], __shell.pipefail)?;
                __shell.set_status(__rc);
                #errexit_stmt
            };
        }
    }

    let stage_stmts = emit_in_process_pipeline_stages(ctx, scope, pipeline);
    if pipeline.negated {
        quote! {
            {
                #stage_stmts
                __shell.set_status(if __pipe_rc == 0 { 1 } else { 0 });
            }
        }
    } else {
        quote! {
            {
                #stage_stmts
                __shell.set_status(__pipe_rc);
                #errexit_stmt
            }
        }
    }
}

fn emit_pipeline_bool(ctx: &CodegenCtx, scope: &mut Scope, pipeline: &PipelineStmt) -> TokenStream {
    if pipeline.commands.len() == 1 {
        let inner = emit_bool_stmt(ctx, scope, &pipeline.commands[0]);
        return if pipeline.negated {
            quote! {
                {
                    let __b = #inner;
                    let __neg = !__b;
                    __shell.set_status(if __neg { 0 } else { 1 });
                    __neg
                }
            }
        } else {
            inner
        };
    }

    if !pipeline_calls_defined_fn(&ctx.functions, pipeline) {
        if let Some(stages) = collect_pipeline_stages(scope, pipeline) {
            if pipeline.negated {
                return quote! {
                    !__shell
                        .run_pipeline_inherit_status(&[#(#stages),*], __shell.pipefail)
                        .is_ok_and(|__rc| __rc == 0)
                };
            }
            return quote! {
                __shell
                    .run_pipeline_inherit_status(&[#(#stages),*], __shell.pipefail)
                    .is_ok_and(|__rc| __rc == 0)
            };
        }
    }

    let stage_stmts = emit_in_process_pipeline_stages(ctx, scope, pipeline);
    if pipeline.negated {
        quote! {
            {
                #stage_stmts
                let __final_rc = if __pipe_rc == 0 { 1 } else { 0 };
                __shell.set_status(__final_rc);
                __final_rc == 0
            }
        }
    } else {
        quote! {
            {
                #stage_stmts
                __shell.set_status(__pipe_rc);
                __pipe_rc == 0
            }
        }
    }
}

fn emit_in_process_pipeline_stages(
    ctx: &CodegenCtx,
    scope: &mut Scope,
    pipeline: &PipelineStmt,
) -> TokenStream {
    let len = pipeline.commands.len();
    let mut stmts = vec![
        quote! { let mut __pipe_buf: Vec<u8> = Vec::new(); },
        quote! { let mut __pipe_rc: i32 = 0; },
    ];
    let mut stage_scope = scope.clone();
    stage_scope.unguarded = true;

    for (idx, cmd_stmt) in pipeline.commands.iter().enumerate() {
        let is_first = idx == 0;
        let is_last = idx + 1 == len;
        if !is_first {
            stmts.push(quote! { __shell.push_stdin_override(std::mem::take(&mut __pipe_buf)); });
        }
        if !is_last {
            stmts.push(quote! { __shell.push_stdout_capture(); });
        }
        let body = emit_stmt(ctx, &mut stage_scope, cmd_stmt);
        stmts.push(body);
        stmts.push(quote! {
            let __stage_rc = __shell.last_status;
            if __shell.pipefail {
                if __stage_rc != 0 {
                    __pipe_rc = __stage_rc;
                }
            } else {
                __pipe_rc = __stage_rc;
            }
        });
        if !is_last {
            stmts.push(quote! { __pipe_buf = __shell.pop_stdout_capture_raw(); });
        }
    }
    quote! { #(#stmts)* }
}

fn collect_pipeline_stages(scope: &Scope, pipeline: &PipelineStmt) -> Option<Vec<TokenStream>> {
    let mut stages = Vec::with_capacity(pipeline.commands.len());
    for cmd_stmt in &pipeline.commands {
        let Stmt::Command(cmd) = cmd_stmt else {
            return None;
        };
        let name_word = cmd.name.as_ref()?;
        let prog = emit_word_str_expr(scope, name_word);
        let args: Vec<TokenStream> = cmd
            .args
            .iter()
            .map(|w| emit_word_str_expr(scope, w))
            .collect();
        stages.push(quote! { (#prog, &[#(#args),*] as &[&str]) });
    }
    Some(stages)
}

fn emit_condition_expr(ctx: &CodegenCtx, scope: &mut Scope, block: &Block) -> TokenStream {
    if block.statements.is_empty() {
        return quote! { true };
    }
    let mut cond_scope = scope.clone();
    cond_scope.unguarded = true;
    if block.statements.len() == 1 {
        return emit_bool_stmt(ctx, &mut cond_scope, &block.statements[0]);
    }
    let stmts = emit_block_with(ctx, &mut cond_scope, block);
    quote! {
        {
            #stmts
            true
        }
    }
}

fn emit_word_string_expr(scope: &Scope, word: &Word) -> TokenStream {
    let line = word.span.map(|s| s.start.line).unwrap_or(1);
    if word.parts.is_empty() {
        let unquoted = unquote_bash_word(&word.raw);
        return quote! { String::from(#unquoted) };
    }

    if word.parts.len() == 1 {
        match &word.parts[0] {
            WordPart::Literal(lit) => return quote! { String::from(#lit) },
            WordPart::VarRef(name) => return emit_var_string_expr(scope, name, line),
            WordPart::CommandSubst(stmt) => return emit_command_subst_expr(scope, stmt, &word.raw),
            WordPart::ArithExpansion(expr) => {
                let eval_i64 = emit_arith_eval_i64(scope, expr);
                return quote! { (#eval_i64).to_string() };
            }
            WordPart::ParamExpansion { name, op } => {
                return emit_param_expansion_expr(scope, name, op, line);
            }
        }
    }

    let mut fmt_str = String::new();
    let mut fmt_args = Vec::new();
    for part in &word.parts {
        match part {
            WordPart::Literal(lit) => {
                for ch in lit.chars() {
                    match ch {
                        '{' => fmt_str.push_str("{{"),
                        '}' => fmt_str.push_str("}}"),
                        _ => fmt_str.push(ch),
                    }
                }
            }
            WordPart::VarRef(name) => {
                fmt_str.push_str("{}");
                fmt_args.push(emit_var_fmt_expr(scope, name, line));
            }
            WordPart::CommandSubst(stmt) => {
                fmt_str.push_str("{}");
                fmt_args.push(emit_command_subst_expr(scope, stmt, &word.raw));
            }
            WordPart::ArithExpansion(expr) => {
                fmt_str.push_str("{}");
                fmt_args.push(emit_arith_eval_i64(scope, expr));
            }
            WordPart::ParamExpansion { name, op } => {
                fmt_str.push_str("{}");
                fmt_args.push(emit_param_expansion_expr(scope, name, op, line));
            }
        }
    }

    quote! {
        format!(#fmt_str, #(#fmt_args),*)
    }
}

fn emit_word_str_expr(scope: &Scope, word: &Word) -> TokenStream {
    let line = word.span.map(|s| s.start.line).unwrap_or(1);
    if word.parts.is_empty() {
        let unquoted = unquote_bash_word(&word.raw);
        return quote! { #unquoted };
    }

    if word.parts.len() == 1 {
        if let WordPart::VarRef(name) = &word.parts[0] {
            return emit_var_str_expr(scope, name, line);
        }
    }

    let string_expr = emit_word_string_expr(scope, word);
    quote! { (#string_expr).as_str() }
}

fn emit_var_string_expr(scope: &Scope, name: &str, line: usize) -> TokenStream {
    if name == "?" {
        return quote! { __shell.status_string() };
    }
    if is_positional_var(name) {
        return emit_positional_string_expr(scope, name);
    }
    if let Some(binding) = scope.get(name) {
        let ident = &binding.ident;
        match binding.ty {
            VarType::OwnedString => quote! { #ident.clone() },
            VarType::OwnedArray => quote! { #ident.first().cloned().unwrap_or_default() },
            VarType::StrSlice => quote! { String::from(#ident) },
        }
    } else {
        let src = &scope.source_name;
        quote! { __shell.get_var_or_env_checked(#name, #src, #line) }
    }
}

fn emit_var_fmt_expr(scope: &Scope, name: &str, line: usize) -> TokenStream {
    if name == "?" {
        return quote! { __shell.last_status };
    }
    if is_positional_var(name) {
        return emit_positional_str_expr(scope, name);
    }
    if let Some(binding) = scope.get(name) {
        let ident = &binding.ident;
        match binding.ty {
            VarType::OwnedArray => quote! { #ident.first().map(String::as_str).unwrap_or("") },
            VarType::OwnedString | VarType::StrSlice => quote! { #ident },
        }
    } else {
        let src = &scope.source_name;
        quote! { __shell.get_var_or_env_checked(#name, #src, #line) }
    }
}

fn emit_var_str_expr(scope: &Scope, name: &str, line: usize) -> TokenStream {
    if name == "?" {
        return quote! { __shell.status_string().as_str() };
    }
    if is_positional_var(name) {
        return emit_positional_str_expr(scope, name);
    }
    if let Some(binding) = scope.get(name) {
        let ident = &binding.ident;
        match binding.ty {
            VarType::OwnedString => quote! { #ident.as_str() },
            VarType::OwnedArray => quote! { #ident.first().map(String::as_str).unwrap_or("") },
            VarType::StrSlice => quote! { #ident },
        }
    } else {
        let src = &scope.source_name;
        quote! { __shell.get_var_or_env_checked(#name, #src, #line).as_str() }
    }
}

fn emit_positional_str_expr(scope: &Scope, name: &str) -> TokenStream {
    if matches!(name, "@" | "*") {
        return quote! { args.join(" ").as_str() };
    }
    if let Ok(idx_1based) = name.parse::<usize>() {
        if idx_1based >= 1 {
            let idx0 = idx_1based - 1;
            return quote! { args.get(#idx0).copied().unwrap_or("") };
        }
    }
    let expr = emit_positional_string_expr(scope, name);
    quote! { (#expr).as_str() }
}

fn emit_positional_string_expr(_scope: &Scope, name: &str) -> TokenStream {
    if name == "#" {
        return quote! { args.len().to_string() };
    }
    if matches!(name, "@" | "*") {
        return quote! { args.join(" ") };
    }
    if let Ok(idx_1based) = name.parse::<usize>() {
        if idx_1based >= 1 {
            let idx0 = idx_1based - 1;
            return quote! { String::from(args.get(#idx0).copied().unwrap_or("")) };
        }
        return quote! { std::env::args().next().unwrap_or_default() };
    }
    quote! { bash2rust_runtime::get_env(#name) }
}

fn stmt_calls_defined_fn(functions: &HashMap<String, syn::Ident>, stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Command(cmd) => {
            if let Some(name_word) = &cmd.name {
                if !matches!(name_word.raw.as_str(), "command" | "builtin")
                    && functions.contains_key(&name_word.raw)
                {
                    return true;
                }
            }
            false
        }
        Stmt::Pipeline(p) => pipeline_calls_defined_fn(functions, p),
        Stmt::AndOr(ao) => {
            stmt_calls_defined_fn(functions, &ao.first)
                || ao
                    .rest
                    .iter()
                    .any(|r| stmt_calls_defined_fn(functions, &r.rhs))
        }
        Stmt::Block(b) => b
            .statements
            .iter()
            .any(|s| stmt_calls_defined_fn(functions, s)),
        Stmt::If(i) => {
            i.then_branch
                .statements
                .iter()
                .any(|s| stmt_calls_defined_fn(functions, s))
                || i.elif_branches.iter().any(|b| {
                    b.body
                        .statements
                        .iter()
                        .any(|s| stmt_calls_defined_fn(functions, s))
                })
                || i.else_branch.as_ref().is_some_and(|e| {
                    e.statements
                        .iter()
                        .any(|s| stmt_calls_defined_fn(functions, s))
                })
        }
        Stmt::Case(c) => c.items.iter().any(|item| {
            item.body
                .statements
                .iter()
                .any(|s| stmt_calls_defined_fn(functions, s))
        }),
        Stmt::For(f) => f
            .body
            .statements
            .iter()
            .any(|s| stmt_calls_defined_fn(functions, s)),
        Stmt::ArithFor(f) => f
            .body
            .statements
            .iter()
            .any(|s| stmt_calls_defined_fn(functions, s)),
        Stmt::Loop(l) => l
            .body
            .statements
            .iter()
            .any(|s| stmt_calls_defined_fn(functions, s)),
        _ => false,
    }
}

fn pipeline_calls_defined_fn(
    functions: &HashMap<String, syn::Ident>,
    pipeline: &PipelineStmt,
) -> bool {
    pipeline.commands.iter().any(|c| {
        stmt_uses_in_process_capture(functions, c)
            || matches!(c, Stmt::Command(cmd) if !cmd.redirects.is_empty())
    })
}

fn stmt_uses_in_process_capture(functions: &HashMap<String, syn::Ident>, stmt: &Stmt) -> bool {
    stmt_calls_defined_fn(functions, stmt)
        || matches!(
            stmt,
            Stmt::Command(cmd)
                if cmd.name.as_ref().is_some_and(|w| matches!(
                    w.raw.as_str(),
                    "command" | "type" | "umask" | "pwd" | "echo" | "cd" | "true" | "false" | ":" | "let"
                ))
        )
}

fn emit_command_subst_raw_expr(scope: &Scope, stmt: &Stmt, fallback_raw: &str) -> TokenStream {
    if stmt_uses_in_process_capture(&scope.functions, stmt) {
        let ctx = CodegenCtx {
            functions: scope.functions.clone(),
            all_functions: Vec::new(),
            source_name: scope.source_name.clone(),
        };
        let mut subst_scope = scope.clone();
        subst_scope.unguarded = true;
        let body = emit_stmt(&ctx, &mut subst_scope, stmt);
        return quote! {
            {
                __shell.push_stdout_capture();
                #body
                String::from_utf8_lossy(&__shell.pop_stdout_capture_raw()).into_owned()
            }
        };
    }

    if let Stmt::Command(cmd) = stmt {
        if let Some(name_word) = &cmd.name {
            let prog = emit_word_str_expr(scope, name_word);
            let args: Vec<TokenStream> = cmd
                .args
                .iter()
                .map(|w| emit_word_str_expr(scope, w))
                .collect();
            return quote! { bash2rust_runtime::command_subst_raw(#prog, &[#(#args),*])?.0 };
        }
    }
    emit_command_subst_expr(scope, stmt, fallback_raw)
}

fn emit_command_subst_expr(scope: &Scope, stmt: &Stmt, fallback_raw: &str) -> TokenStream {
    if stmt_uses_in_process_capture(&scope.functions, stmt) {
        let ctx = CodegenCtx {
            functions: scope.functions.clone(),
            all_functions: Vec::new(),
            source_name: scope.source_name.clone(),
        };
        let mut subst_scope = scope.clone();
        subst_scope.unguarded = true;
        let body = emit_stmt(&ctx, &mut subst_scope, stmt);
        return quote! {
            {
                __shell.push_stdout_capture();
                #body
                __shell.pop_stdout_capture()
            }
        };
    }

    match stmt {
        Stmt::Command(cmd) => {
            if let Some(name_word) = &cmd.name {
                if !cmd.redirects.is_empty() || is_quoted_all_args(name_word) {
                    let (prog_setup, prog_ref, args_setup, args_slice) = if is_quoted_all_args(
                        name_word,
                    ) {
                        let (extra_setup, _) = emit_command_args_setup(scope, &cmd.args);
                        (
                            quote! { let __prog = args.first().copied().unwrap_or("").to_owned(); },
                            quote! { &__prog },
                            quote! {
                                #extra_setup
                                let mut __full_args: Vec<String> = args.iter().skip(1).map(|s| (*s).to_owned()).collect();
                                __full_args.extend(__args);
                                let __full_arg_refs: Vec<&str> = __full_args.iter().map(String::as_str).collect();
                            },
                            quote! { &__full_arg_refs },
                        )
                    } else {
                        let prog_string = emit_word_string_expr(scope, name_word);
                        let (args_setup, args_slice) = emit_command_args_setup(scope, &cmd.args);
                        (
                            quote! { let __prog = #prog_string; },
                            quote! { &__prog },
                            args_setup,
                            args_slice,
                        )
                    };
                    let (redir_bindings, redir_specs) =
                        emit_redirect_specs_with_bindings(scope, &cmd.redirects);
                    return quote! {
                        {
                            #prog_setup
                            #args_setup
                            #(#redir_bindings)*
                            __shell.command_subst_with_redirects(#prog_ref, #args_slice, &[#(#redir_specs),*])?
                        }
                    };
                }
                let prog_string = emit_word_string_expr(scope, name_word);
                let (args_setup, args_slice) = emit_command_args_setup(scope, &cmd.args);
                if word_has_dynamic_parts(name_word) {
                    let shell_arg = if scope.in_function {
                        quote! { __shell }
                    } else {
                        quote! { &mut __shell }
                    };
                    let post_sync = emit_post_call_sync(scope);
                    return quote! {
                        {
                            let __prog = #prog_string;
                            #args_setup
                            __shell.push_stdout_capture();
                            if __dispatch_dynamic_fn(#shell_arg, &__prog, #args_slice)?.is_some() {
                                #post_sync
                                __shell.pop_stdout_capture()
                            } else {
                                let _ = __shell.pop_stdout_capture_raw();
                                __shell.command_subst_with_redirects(&__prog, #args_slice, &[])?
                            }
                        }
                    };
                }
                return quote! {
                    {
                        let __prog = #prog_string;
                        #args_setup
                        __shell.command_subst_with_redirects(&__prog, #args_slice, &[])?
                    }
                };
            }
        }
        Stmt::Pipeline(pipeline) => {
            if !pipeline_calls_defined_fn(&scope.functions, pipeline) {
                if let Some(stages) = collect_pipeline_stages(scope, pipeline) {
                    return quote! { bash2rust_runtime::run_pipeline_subst(&[#(#stages),*])? };
                }
            }
            let ctx = CodegenCtx {
                functions: scope.functions.clone(),
                all_functions: Vec::new(),
                source_name: scope.source_name.clone(),
            };
            let mut subst_scope = scope.clone();
            subst_scope.unguarded = true;
            let body = emit_stmt(&ctx, &mut subst_scope, stmt);
            return quote! {
                {
                    let __saved_cwd = std::env::current_dir().ok();
                    __shell.push_stdout_capture();
                    #body
                    let __sub_out = __shell.pop_stdout_capture();
                    if let Some(__d) = __saved_cwd {
                        let _ = std::env::set_current_dir(__d);
                    }
                    __sub_out
                }
            };
        }
        Stmt::Block(block) => {
            return emit_block_command_subst_expr(scope, block);
        }
        Stmt::AndOr(_)
        | Stmt::If(_)
        | Stmt::Case(_)
        | Stmt::For(_)
        | Stmt::ArithFor(_)
        | Stmt::Loop(_)
        | Stmt::Test(_) => {
            let ctx = CodegenCtx {
                functions: scope.functions.clone(),
                all_functions: Vec::new(),
                source_name: scope.source_name.clone(),
            };
            let mut subst_scope = scope.clone();
            subst_scope.unguarded = true;
            let body = emit_stmt(&ctx, &mut subst_scope, stmt);
            return quote! {
                {
                    let __saved_cwd = std::env::current_dir().ok();
                    __shell.push_stdout_capture();
                    #body
                    let __sub_out = __shell.pop_stdout_capture();
                    if let Some(__d) = __saved_cwd {
                        let _ = std::env::set_current_dir(__d);
                    }
                    __sub_out
                }
            };
        }
        _ => {}
    }
    quote! { String::from(#fallback_raw) }
}

fn emit_block_command_subst_expr(scope: &Scope, block: &Block) -> TokenStream {
    let mut inner_stmts = Vec::new();
    let mut ends_with_break = false;
    for s in &block.statements {
        match s {
            Stmt::Command(cmd) if cmd.name.as_ref().is_some_and(|w| w.raw == "exit") => {
                let code_expr = cmd
                    .args
                    .first()
                    .map(|w| {
                        let s = emit_word_str_expr(scope, w);
                        quote! { #s.trim().parse::<i32>().unwrap_or(0) }
                    })
                    .unwrap_or_else(|| quote! { 0i32 });
                inner_stmts.push(quote! {
                    __subst_rc = #code_expr;
                    break;
                });
                ends_with_break = true;
            }
            Stmt::Command(cmd) => {
                if let Some(name_word) = &cmd.name {
                    let prog = emit_word_str_expr(scope, name_word);
                    let args: Vec<TokenStream> = cmd
                        .args
                        .iter()
                        .map(|w| emit_word_str_expr(scope, w))
                        .collect();
                    inner_stmts.push(quote! {
                        let (__out, __rc) = bash2rust_runtime::command_subst_raw(#prog, &[#(#args),*])?;
                        __subst_out.push_str(&__out);
                        __subst_rc = __rc;
                    });
                    ends_with_break = false;
                }
            }
            Stmt::Pipeline(pipeline) => {
                if let Some(stages) = collect_pipeline_stages(scope, pipeline) {
                    inner_stmts.push(quote! {
                        let (__out, __rc) = bash2rust_runtime::run_pipeline_subst_with_status(&[#(#stages),*])?;
                        __subst_out.push_str(&__out);
                        __subst_rc = __rc;
                    });
                    ends_with_break = false;
                }
            }
            _ => {}
        }
    }

    let trailing_break = if ends_with_break {
        quote! {}
    } else {
        quote! { break; }
    };

    quote! {
        {
            let mut __subst_out = String::new();
            let mut __subst_rc: i32 = 0;
            loop {
                #(#inner_stmts)*
                #trailing_break
            }
            __shell.set_status(__subst_rc);
            __subst_out.trim_end_matches(['\r', '\n']).to_owned()
        }
    }
}

fn emit_param_expansion_expr(
    scope: &Scope,
    name: &str,
    op: &ParamExpansionOp,
    line: usize,
) -> TokenStream {
    match op {
        ParamExpansionOp::Indirect => {
            let source_name = &scope.source_name;
            if let Some(binding) = scope.get(name) {
                let ident = &binding.ident;
                match binding.ty {
                    VarType::OwnedString => quote! {
                        __shell.get_var_or_env_checked(#ident.as_str(), #source_name, #line)
                    },
                    VarType::StrSlice => quote! {
                        __shell.get_var_or_env_checked(#ident, #source_name, #line)
                    },
                    VarType::OwnedArray => quote! {
                        __shell.get_indirect_checked(#name, #source_name, #line)
                    },
                }
            } else {
                quote! { __shell.get_indirect_checked(#name, #source_name, #line) }
            }
        }
        ParamExpansionOp::Length => {
            if let Some(arr_name) = name
                .strip_suffix("[@]")
                .or_else(|| name.strip_suffix("[*]"))
            {
                if let Some(binding) = scope.get(arr_name) {
                    let ident = &binding.ident;
                    return quote! { #ident.len().to_string() };
                }
                return quote! { __shell.array_len(#arr_name).to_string() };
            }
            if let Some(binding) = scope.get(name) {
                let ident = &binding.ident;
                quote! { #ident.chars().count().to_string() }
            } else if is_positional_var(name) {
                let expr = emit_positional_string_expr(scope, name);
                quote! { #expr.chars().count().to_string() }
            } else {
                let src = &scope.source_name;
                quote! { __shell.get_var_or_env_checked(#name, #src, #line).chars().count().to_string() }
            }
        }
        ParamExpansionOp::ArrayElement { index } => {
            let idx_expr = emit_word_string_expr(scope, index);
            if let Some(binding) = scope.get(name) {
                let ident = &binding.ident;
                if binding.ty == VarType::OwnedArray {
                    return quote! {
                        {
                            let __idx = (#idx_expr).trim().parse::<usize>().unwrap_or(0);
                            #ident.get(__idx).cloned().unwrap_or_default()
                        }
                    };
                }
            }
            quote! {
                {
                    let __idx = (#idx_expr).trim().parse::<usize>().unwrap_or(0);
                    __shell.get_array_elem(#name, __idx).to_owned()
                }
            }
        }
        ParamExpansionOp::ArrayAll | ParamExpansionOp::ArrayAllIfSet => {
            quote! { __shell.get_array(#name).join(" ") }
        }
        ParamExpansionOp::ArraySlice { offset, length } => {
            let off_expr = emit_word_string_expr(scope, offset);
            let len_expr = match length {
                Some(l) => {
                    let le = emit_word_string_expr(scope, l);
                    quote! { Some((#le).trim().parse::<usize>().unwrap_or(0)) }
                }
                None => quote! { None::<usize> },
            };
            quote! {
                {
                    let __arr = __shell.get_array(#name);
                    let __off = (#off_expr).trim().parse::<usize>().unwrap_or(0).min(__arr.len());
                    let __end = match #len_expr {
                        Some(l) => __off.saturating_add(l).min(__arr.len()),
                        None => __arr.len(),
                    };
                    __arr[__off..__end].join(" ")
                }
            }
        }
        ParamExpansionOp::Trim {
            pattern,
            is_suffix,
            longest,
        } => {
            let src = &scope.source_name;
            let val_expr = if let Some(binding) = scope.get(name) {
                let ident = &binding.ident;
                quote! { #ident.as_str() }
            } else {
                quote! { __shell.get_var_or_env_checked(#name, #src, #line).as_str() }
            };
            let pat_expr = emit_word_string_expr(scope, pattern);
            quote! {
                {
                    let __pat = #pat_expr;
                    bash2rust_runtime::param_trim(#val_expr, __pat.as_str(), #is_suffix, #longest)
                }
            }
        }
        ParamExpansionOp::Replace {
            pattern,
            replacement,
            replace_all,
        } => {
            let src = &scope.source_name;
            let val_expr = if let Some(binding) = scope.get(name) {
                let ident = &binding.ident;
                quote! { #ident.as_str() }
            } else {
                quote! { __shell.get_var_or_env_checked(#name, #src, #line).as_str() }
            };
            let pat_expr = emit_word_string_expr(scope, pattern);
            let repl_expr = emit_word_string_expr(scope, replacement);
            quote! {
                {
                    let __pat = #pat_expr;
                    let __repl = #repl_expr;
                    bash2rust_runtime::param_replace(
                        #val_expr,
                        __pat.as_str(),
                        __repl.as_str(),
                        #replace_all,
                    )
                }
            }
        }
        ParamExpansionOp::Substring { offset, length } => {
            let src = &scope.source_name;
            let val_expr = if let Some(binding) = scope.get(name) {
                let ident = &binding.ident;
                quote! { #ident.as_str() }
            } else {
                quote! { __shell.get_var_or_env_checked(#name, #src, #line).as_str() }
            };
            let off_expr = emit_word_string_expr(scope, offset);
            let len_expr = match length {
                Some(l) => {
                    let le = emit_word_string_expr(scope, l);
                    quote! { Some((#le).trim().parse::<i64>().unwrap_or(0)) }
                }
                None => quote! { None::<i64> },
            };
            quote! {
                {
                    let __off = (#off_expr).trim().parse::<i64>().unwrap_or(0);
                    let __len = #len_expr;
                    bash2rust_runtime::param_substr(#val_expr, __off, __len)
                }
            }
        }
        ParamExpansionOp::UseDefault { default, colon } => {
            let def_expr = emit_word_string_expr(scope, default);
            if let Ok(pos_idx) = name.parse::<usize>() {
                if pos_idx >= 1 {
                    let zero_idx = pos_idx - 1;
                    return if *colon {
                        quote! {
                            match args.get(#zero_idx).copied() {
                                Some(__v) if !__v.is_empty() => __v.to_owned(),
                                _ => #def_expr,
                            }
                        }
                    } else {
                        quote! {
                            match args.get(#zero_idx).copied() {
                                Some(__v) => __v.to_owned(),
                                None => #def_expr,
                            }
                        }
                    };
                }
            }
            if let Some(binding) = scope.get(name) {
                let ident = &binding.ident;
                let val_expr = match binding.ty {
                    VarType::OwnedString => quote! { #ident.clone() },
                    VarType::OwnedArray => quote! { #ident.first().cloned().unwrap_or_default() },
                    VarType::StrSlice => quote! { String::from(#ident) },
                };
                if *colon {
                    quote! {
                        if #ident.is_empty() { #def_expr } else { #val_expr }
                    }
                } else {
                    val_expr
                }
            } else if *colon {
                quote! {
                    match __shell.get_var(#name).map(str::to_owned).or_else(|| std::env::var(#name).ok()) {
                        Some(__v) if !__v.is_empty() => __v,
                        _ => #def_expr,
                    }
                }
            } else {
                quote! {
                    match __shell.get_var(#name).map(str::to_owned).or_else(|| std::env::var(#name).ok()) {
                        Some(__v) => __v,
                        None => #def_expr,
                    }
                }
            }
        }
    }
}

fn try_emit_pure_arith_atom(scope: &Scope, atom: &str) -> Option<TokenStream> {
    let s = atom.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(n) = s.parse::<i64>() {
        return Some(quote! { #n });
    }
    let var_name = s.strip_prefix('$').unwrap_or(s);
    if !var_name.is_empty()
        && var_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !var_name.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        if let Some(binding) = scope.get(var_name) {
            let ident = &binding.ident;
            return match binding.ty {
                VarType::OwnedString | VarType::StrSlice => {
                    Some(quote! { #ident.trim().parse::<i64>().unwrap_or(0) })
                }
                VarType::OwnedArray => None,
            };
        }
    }
    None
}

fn try_emit_pure_arith_i64(scope: &Scope, expr: &str) -> Option<TokenStream> {
    let mut s = expr.trim();
    while s.starts_with('(') && s.ends_with(')') {
        let inner = &s[1..s.len() - 1];
        let mut depth = 0i32;
        let mut balanced = true;
        for ch in inner.chars() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth < 0 {
                        balanced = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if balanced && depth == 0 {
            s = inner.trim();
        } else {
            break;
        }
    }
    if s.is_empty()
        || s.contains(['=', '[', ']', '?', ':', ',', ';', '&', '|', '^', '~', '!'])
        || s.contains("++")
        || s.contains("--")
        || s.contains("<<")
        || s.contains(">>")
    {
        return None;
    }
    if let Some(atom) = try_emit_pure_arith_atom(scope, s) {
        return Some(atom);
    }

    const OPS: &[&str] = &["==", "!=", "<=", ">=", "<", ">", "+", "-", "*", "/", "%"];
    for &op in OPS {
        let mut matches = s.match_indices(op);
        if let (Some((idx, _)), None) = (matches.next(), matches.next()) {
            let lhs_str = s[..idx].trim();
            let rhs_str = s[idx + op.len()..].trim();
            if lhs_str.is_empty() || rhs_str.is_empty() {
                continue;
            }
            let (Some(lhs), Some(rhs)) = (
                try_emit_pure_arith_atom(scope, lhs_str),
                try_emit_pure_arith_atom(scope, rhs_str),
            ) else {
                continue;
            };
            let tokens = match op {
                "+" => quote! { (#lhs).wrapping_add(#rhs) },
                "-" => quote! { (#lhs).wrapping_sub(#rhs) },
                "*" => quote! { (#lhs).wrapping_mul(#rhs) },
                "/" => quote! { (#lhs).checked_div(#rhs).unwrap_or(0) },
                "%" => quote! { (#lhs).checked_rem(#rhs).unwrap_or(0) },
                "==" => quote! { if (#lhs) == (#rhs) { 1i64 } else { 0i64 } },
                "!=" => quote! { if (#lhs) != (#rhs) { 1i64 } else { 0i64 } },
                "<" => quote! { if (#lhs) < (#rhs) { 1i64 } else { 0i64 } },
                "<=" => quote! { if (#lhs) <= (#rhs) { 1i64 } else { 0i64 } },
                ">" => quote! { if (#lhs) > (#rhs) { 1i64 } else { 0i64 } },
                ">=" => quote! { if (#lhs) >= (#rhs) { 1i64 } else { 0i64 } },
                _ => return None,
            };
            return Some(tokens);
        }
    }
    None
}

fn emit_arith_eval_i64(scope: &Scope, expr: &str) -> TokenStream {
    if let Some(pure) = try_emit_pure_arith_i64(scope, expr) {
        return pure;
    }
    let mut pre_sync = Vec::new();
    let mut post_sync = Vec::new();
    for (bash_name, binding) in &scope.vars {
        if expr.contains(bash_name.as_str()) {
            let ident = &binding.ident;
            match binding.ty {
                VarType::OwnedString => {
                    pre_sync.push(quote! { __shell.set_var(#bash_name, #ident.clone()); });
                    post_sync.push(quote! {
                        if let Some(__v) = __shell.get_var(#bash_name) {
                            #ident = __v.to_owned();
                        }
                    });
                }
                VarType::StrSlice => {
                    pre_sync.push(quote! { __shell.set_var(#bash_name, #ident); });
                }
                VarType::OwnedArray => {
                    pre_sync.push(quote! { __shell.set_array(#bash_name, #ident.clone()); });
                }
            }
        }
    }
    quote! {
        {
            #(#pre_sync)*
            let __res = __shell.eval_arith(#expr);
            #(#post_sync)*
            __res
        }
    }
}

fn word_needs_splitting(word: &Word) -> bool {
    !word.has_quotes
        && word.parts.iter().any(|p| {
            matches!(
                p,
                WordPart::VarRef(_)
                    | WordPart::CommandSubst(_)
                    | WordPart::ArithExpansion(_)
                    | WordPart::ParamExpansion { .. }
            )
        })
}

fn word_needs_globbing(word: &Word) -> bool {
    !word.has_quotes && word.raw.contains(['*', '?', '['])
}

fn is_quoted_all_args(word: &Word) -> bool {
    word.raw == "\"$@\""
        || (word.has_quotes
            && word.parts.len() == 1
            && matches!(&word.parts[0], WordPart::VarRef(name) if name == "@"))
}

fn word_has_dynamic_parts(word: &Word) -> bool {
    !word.parts.is_empty()
        && word
            .parts
            .iter()
            .any(|p| !matches!(p, WordPart::Literal(_)))
}

fn array_expansion_part(word: &Word) -> Option<(&str, &ParamExpansionOp)> {
    if word.parts.len() == 1 {
        if let WordPart::ParamExpansion { name, op } = &word.parts[0] {
            if matches!(
                op,
                ParamExpansionOp::ArrayAll
                    | ParamExpansionOp::ArrayAllIfSet
                    | ParamExpansionOp::ArraySlice { .. }
            ) {
                return Some((name.as_str(), op));
            }
        }
    }
    None
}

fn word_needs_expansion(word: &Word) -> bool {
    word_needs_splitting(word)
        || word_needs_globbing(word)
        || is_quoted_all_args(word)
        || array_expansion_part(word).is_some()
}

fn emit_shift_stmt(scope: &Scope, args: &[Word]) -> TokenStream {
    let n_expr = if let Some(first) = args.first() {
        let s = emit_word_string_expr(scope, first);
        quote! { (#s).trim().parse::<usize>().unwrap_or(1) }
    } else {
        quote! { 1usize }
    };
    quote! {
        let __shift_n: usize = #n_expr;
        if __shift_n <= args.len() {
            args.drain(0..__shift_n);
            __shell.set_positional_args(&args);
            __shell.set_status(0);
        } else {
            __shell.set_status(1);
        }
    }
}

fn emit_command_args_setup(scope: &Scope, effective_args: &[Word]) -> (TokenStream, TokenStream) {
    if effective_args.iter().any(word_needs_expansion) {
        let expand_stmts = emit_expanded_word_vec(scope, effective_args, quote! { __args });
        (
            quote! {
                #expand_stmts
                let __arg_refs: Vec<&str> = __args.iter().map(String::as_str).collect();
            },
            quote! { &__arg_refs },
        )
    } else {
        let arg_strings: Vec<TokenStream> = effective_args
            .iter()
            .map(|w| emit_word_string_expr(scope, w))
            .collect();
        (
            quote! {
                let __args: Vec<String> = vec![#(#arg_strings),*];
                let __arg_refs: Vec<&str> = __args.iter().map(String::as_str).collect();
            },
            quote! { &__arg_refs },
        )
    }
}

fn emit_direct_fn_call(
    scope: &Scope,
    fn_ident: &syn::Ident,
    args_setup: &TokenStream,
    args_slice: &TokenStream,
    redirects: &[Redirect],
) -> TokenStream {
    let shell_arg = if scope.in_function {
        quote! { __shell }
    } else {
        quote! { &mut __shell }
    };
    let post_sync = emit_post_call_sync(scope);
    let call_stmt = if scope.unguarded {
        quote! {
            __shell.errexit_suppress += 1;
            let __fn_res = #fn_ident(#shell_arg, #args_slice);
            __shell.errexit_suppress -= 1;
            __fn_res?;
        }
    } else {
        quote! {
            #fn_ident(#shell_arg, #args_slice)?;
        }
    };
    if redirects.is_empty() {
        quote! {
            #args_setup
            #call_stmt
            #post_sync
        }
    } else {
        let (redir_bindings, redir_specs_in) = emit_redirect_specs_with_bindings(scope, redirects);
        let redir_specs_out = redir_specs_in.clone();
        quote! {
            #args_setup
            #(#redir_bindings)*
            __shell.push_stdin_from_redirects(&[#(#redir_specs_in),*])?;
            __shell.push_stdout_capture();
            #call_stmt
            let __captured = __shell.pop_stdout_capture_raw();
            __shell.apply_captured_redirects(&__captured, &[#(#redir_specs_out),*])?;
            #post_sync
        }
    }
}

fn emit_expanded_word_vec(scope: &Scope, words: &[Word], vec_ident: TokenStream) -> TokenStream {
    let mut stmts = vec![quote! { let mut #vec_ident: Vec<String> = Vec::new(); }];
    for w in words {
        if is_quoted_all_args(w) {
            stmts.push(quote! {
                #vec_ident.extend(args.iter().map(|s| (*s).to_owned()));
            });
            continue;
        }
        if let Some((arr_name, op)) = array_expansion_part(w) {
            match op {
                ParamExpansionOp::ArrayAll | ParamExpansionOp::ArrayAllIfSet => {
                    stmts.push(quote! {
                        #vec_ident.extend(__shell.get_array(#arr_name).iter().cloned());
                    });
                }
                ParamExpansionOp::ArraySlice { offset, length } => {
                    let off_expr = emit_word_string_expr(scope, offset);
                    let len_expr = match length {
                        Some(l) => {
                            let le = emit_word_string_expr(scope, l);
                            quote! { Some((#le).trim().parse::<usize>().unwrap_or(0)) }
                        }
                        None => quote! { None::<usize> },
                    };
                    stmts.push(quote! {
                        {
                            let __arr = __shell.get_array(#arr_name);
                            let __off = (#off_expr).trim().parse::<usize>().unwrap_or(0).min(__arr.len());
                            let __end = match #len_expr {
                                Some(l) => __off.saturating_add(l).min(__arr.len()),
                                None => __arr.len(),
                            };
                            #vec_ident.extend(__arr[__off..__end].iter().cloned());
                        }
                    });
                }
                _ => {}
            }
            continue;
        }
        let s_expr = emit_word_string_expr(scope, w);
        if word_needs_splitting(w) {
            stmts.push(quote! {
                let __val = #s_expr;
                for __part in bash2rust_runtime::split_words(&__val, __shell.ifs.as_deref()) {
                    #vec_ident.extend(bash2rust_runtime::expand_glob(&__part));
                }
            });
        } else if word_needs_globbing(w) {
            stmts.push(quote! {
                let __pat = #s_expr;
                #vec_ident.extend(bash2rust_runtime::expand_glob(&__pat));
            });
        } else {
            stmts.push(quote! {
                #vec_ident.push(#s_expr);
            });
        }
    }
    quote! { #(#stmts)* }
}

fn unquote_bash_word(raw: &str) -> String {
    if raw.len() >= 2
        && ((raw.starts_with('"') && raw.ends_with('"'))
            || (raw.starts_with('\'') && raw.ends_with('\'')))
    {
        raw[1..raw.len() - 1].to_owned()
    } else {
        raw.to_owned()
    }
}

fn to_snake_ident_base(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let mut cleaned: String = lower
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.starts_with(|c: char| c.is_ascii_digit()) {
        cleaned.insert(0, '_');
    }
    cleaned
}

fn safe_ident(cleaned: &str) -> syn::Ident {
    if let Ok(ident) = syn::parse_str::<syn::Ident>(cleaned) {
        return ident;
    }

    match cleaned {
        "self" | "super" | "crate" | "Self" | "_" => format_ident!("var_{}", cleaned),
        _ => format_ident!("r#{}", cleaned),
    }
}
