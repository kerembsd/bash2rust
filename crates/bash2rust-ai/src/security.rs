//! AST-based security deny-list for AI-generated Rust snippets.

use std::fmt;

use syn::visit::{self, Visit};
use syn::{
    Abi, Expr, ExprCall, ExprLit, ExprMethodCall, ImplItemFn, ItemFn, ItemForeignMod, ItemImpl,
    ItemTrait, Lit, Path, TraitItemFn, UseTree,
};
use thiserror::Error;

/// A specific security policy violation detected in AI-generated Rust code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityViolation {
    pub rule: &'static str,
    pub detail: String,
}

impl fmt::Display for SecurityViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.rule, self.detail)
    }
}

/// Error returned when AI-generated Rust fails syntax parsing or AST security validation.
#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("AI-generated Rust could not be parsed by syn: {0}")]
    ParseError(String),

    #[error("AI-generated Rust rejected by AST security filter: {0}")]
    Rejected(SecurityViolation),
}

/// Validates AI-generated Rust code against the AST security deny-list.
///
/// Accepts either a full `syn::File` or statement sequence wrapped in `{ ... }`.
/// Rejects unparseable syntax, `unsafe` / FFI `extern`, `transmute` / `libc`,
/// arbitrary shell execution (`sh`, `bash`, `eval`, `-c`), and absolute-path
/// destructive filesystem operations (`remove_dir_all`, `remove_dir`, `remove_file`, `rm`).
pub fn validate_ai_rust(code: &str) -> Result<(), SecurityError> {
    if let Ok(file) = syn::parse_file(code) {
        let mut visitor = SecurityVisitor::default();
        visitor.visit_file(&file);
        return visitor.into_result();
    }

    let wrapped = format!("{{\n{code}\n}}");
    match syn::parse_str::<syn::Block>(&wrapped) {
        Ok(block) => {
            let mut visitor = SecurityVisitor::default();
            visitor.visit_block(&block);
            visitor.into_result()
        }
        Err(err) => Err(SecurityError::ParseError(err.to_string())),
    }
}

#[derive(Default)]
struct SecurityVisitor {
    violation: Option<SecurityViolation>,
}

impl SecurityVisitor {
    fn record(&mut self, rule: &'static str, detail: impl Into<String>) {
        if self.violation.is_none() {
            self.violation = Some(SecurityViolation {
                rule,
                detail: detail.into(),
            });
        }
    }

    fn into_result(self) -> Result<(), SecurityError> {
        match self.violation {
            Some(v) => Err(SecurityError::Rejected(v)),
            None => Ok(()),
        }
    }
}

fn is_shell_binary(s: &str) -> bool {
    matches!(
        s.trim(),
        "sh" | "/bin/sh"
            | "/usr/bin/sh"
            | "bash"
            | "/bin/bash"
            | "/usr/bin/bash"
            | "zsh"
            | "/bin/zsh"
            | "/usr/bin/zsh"
            | "dash"
            | "/bin/dash"
            | "ksh"
            | "/bin/ksh"
            | "eval"
    )
}

fn is_absolute_path(s: &str) -> bool {
    if s.starts_with('/') {
        return true;
    }
    let b = s.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

fn extract_str_literals(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(lit), ..
        }) => out.push(lit.value()),
        Expr::Reference(r) => extract_str_literals(&r.expr, out),
        Expr::Array(arr) => {
            for elem in &arr.elems {
                extract_str_literals(elem, out);
            }
        }
        Expr::Tuple(t) => {
            for elem in &t.elems {
                extract_str_literals(elem, out);
            }
        }
        Expr::Call(call) => {
            for arg in &call.args {
                extract_str_literals(arg, out);
            }
        }
        Expr::MethodCall(mc) => {
            extract_str_literals(&mc.receiver, out);
            for arg in &mc.args {
                extract_str_literals(arg, out);
            }
        }
        Expr::Paren(p) => extract_str_literals(&p.expr, out),
        Expr::Group(g) => extract_str_literals(&g.expr, out),
        _ => {}
    }
}

fn path_last_ident(path: &Path) -> Option<String> {
    path.segments.last().map(|s| s.ident.to_string())
}

fn path_tail_two(path: &Path) -> Option<(String, String)> {
    let len = path.segments.len();
    if len >= 2 {
        Some((
            path.segments[len - 2].ident.to_string(),
            path.segments[len - 1].ident.to_string(),
        ))
    } else {
        None
    }
}

fn chain_uses_rm(expr: &Expr) -> bool {
    match expr {
        Expr::Call(call) => {
            if let Expr::Path(ep) = &*call.func {
                if path_tail_two(&ep.path).is_some_and(|(a, b)| a == "Command" && b == "new") {
                    if let Some(first) = call.args.first() {
                        let mut lits = Vec::new();
                        extract_str_literals(first, &mut lits);
                        return lits.iter().any(|s| s == "rm" || s == "/bin/rm");
                    }
                }
            }
            false
        }
        Expr::MethodCall(mc) => chain_uses_rm(&mc.receiver),
        Expr::Reference(r) => chain_uses_rm(&r.expr),
        _ => false,
    }
}

impl<'ast> Visit<'ast> for SecurityVisitor {
    fn visit_expr_unsafe(&mut self, i: &'ast syn::ExprUnsafe) {
        self.record("unsafe", "unsafe blocks are forbidden in AI-generated Rust");
        visit::visit_expr_unsafe(self, i);
    }

    fn visit_item_fn(&mut self, i: &'ast ItemFn) {
        if i.sig.unsafety.is_some() {
            self.record(
                "unsafe",
                format!("unsafe fn `{}` is forbidden", i.sig.ident),
            );
        }
        visit::visit_item_fn(self, i);
    }

    fn visit_impl_item_fn(&mut self, i: &'ast ImplItemFn) {
        if i.sig.unsafety.is_some() {
            self.record(
                "unsafe",
                format!("unsafe method `{}` is forbidden", i.sig.ident),
            );
        }
        visit::visit_impl_item_fn(self, i);
    }

    fn visit_trait_item_fn(&mut self, i: &'ast TraitItemFn) {
        if i.sig.unsafety.is_some() {
            self.record(
                "unsafe",
                format!("unsafe trait fn `{}` is forbidden", i.sig.ident),
            );
        }
        visit::visit_trait_item_fn(self, i);
    }

    fn visit_item_impl(&mut self, i: &'ast ItemImpl) {
        if i.unsafety.is_some() {
            self.record("unsafe", "unsafe impl blocks are forbidden");
        }
        visit::visit_item_impl(self, i);
    }

    fn visit_item_trait(&mut self, i: &'ast ItemTrait) {
        if i.unsafety.is_some() {
            self.record("unsafe", format!("unsafe trait `{}` is forbidden", i.ident));
        }
        visit::visit_item_trait(self, i);
    }

    fn visit_item_foreign_mod(&mut self, i: &'ast ItemForeignMod) {
        self.record("ffi", "FFI extern blocks are forbidden");
        visit::visit_item_foreign_mod(self, i);
    }

    fn visit_abi(&mut self, i: &'ast Abi) {
        self.record("ffi", "extern ABI declarations are forbidden");
        visit::visit_abi(self, i);
    }

    fn visit_path(&mut self, i: &'ast Path) {
        for seg in &i.segments {
            let ident = seg.ident.to_string();
            if ident == "transmute" || ident == "transmute_copy" {
                self.record("transmute", format!("`{ident}` is forbidden"));
            } else if ident == "libc" {
                self.record("libc", "`libc` usage is forbidden");
            }
        }
        visit::visit_path(self, i);
    }

    fn visit_use_tree(&mut self, i: &'ast UseTree) {
        match i {
            UseTree::Path(p) => {
                let ident = p.ident.to_string();
                if ident == "libc" || ident == "transmute" || ident == "transmute_copy" {
                    self.record("import", format!("importing `{ident}` is forbidden"));
                }
            }
            UseTree::Name(n) => {
                let ident = n.ident.to_string();
                if ident == "libc" || ident == "transmute" || ident == "transmute_copy" {
                    self.record("import", format!("importing `{ident}` is forbidden"));
                }
            }
            _ => {}
        }
        visit::visit_use_tree(self, i);
    }

    fn visit_expr_call(&mut self, i: &'ast ExprCall) {
        if let Expr::Path(ep) = &*i.func {
            let last = path_last_ident(&ep.path);
            let tail2 = path_tail_two(&ep.path);

            // Command::new("sh" | "bash" | "eval" | ...)
            if tail2
                .as_ref()
                .is_some_and(|(a, b)| a == "Command" && b == "new")
            {
                if let Some(first) = i.args.first() {
                    let mut lits = Vec::new();
                    extract_str_literals(first, &mut lits);
                    for s in lits {
                        if is_shell_binary(&s) {
                            self.record(
                                "shell_exec",
                                format!(
                                    "arbitrary shell execution `Command::new({s:?})` is forbidden"
                                ),
                            );
                        }
                    }
                }
            }

            // bash2rust_runtime command and pipeline helpers
            if matches!(
                last.as_deref(),
                Some(
                    "run_command"
                        | "run_command_env"
                        | "exec_ok"
                        | "command_subst"
                        | "command_subst_raw"
                        | "command_subst_with_status"
                        | "run_command_with_redirects"
                        | "run_pipeline"
                        | "run_pipeline_subst"
                        | "run_pipeline_subst_with_status"
                        | "run_pipeline_ok"
                        | "run_pipeline_inherit"
                        | "run_pipeline_inherit_status"
                )
            ) {
                let mut all_lits = Vec::new();
                for arg in &i.args {
                    extract_str_literals(arg, &mut all_lits);
                }
                for s in &all_lits {
                    if is_shell_binary(s) {
                        self.record(
                            "shell_exec",
                            format!(
                                "arbitrary shell execution `{s}` via runtime helper is forbidden"
                            ),
                        );
                    }
                    if s == "-c" {
                        self.record(
                            "shell_exec",
                            "shell `-c` argument in runtime command helper is forbidden",
                        );
                    }
                }
                if all_lits.iter().any(|s| s == "rm" || s == "/bin/rm")
                    && all_lits.iter().any(|s| is_absolute_path(s))
                {
                    self.record(
                        "destructive_fs",
                        "runtime `rm` invocation targeting an absolute path is forbidden",
                    );
                }
            }

            // std::fs::{remove_dir_all, remove_dir, remove_file} with absolute path
            if matches!(
                last.as_deref(),
                Some("remove_dir_all" | "remove_dir" | "remove_file")
            ) {
                if let Some(first) = i.args.first() {
                    let mut lits = Vec::new();
                    extract_str_literals(first, &mut lits);
                    for s in lits {
                        if is_absolute_path(&s) {
                            self.record(
                                "destructive_fs",
                                format!(
                                    "`{}` on absolute path `{s}` is forbidden",
                                    last.as_deref().unwrap_or("remove")
                                ),
                            );
                        }
                    }
                }
            }
        }
        visit::visit_expr_call(self, i);
    }

    fn visit_expr_method_call(&mut self, i: &'ast ExprMethodCall) {
        let method = i.method.to_string();
        if method == "arg" || method == "args" {
            let mut lits = Vec::new();
            for arg in &i.args {
                extract_str_literals(arg, &mut lits);
            }
            if lits.iter().any(|s| s == "-c") {
                self.record(
                    "shell_exec",
                    "passing `-c` to a command invocation is forbidden",
                );
            }
            if chain_uses_rm(&i.receiver) && lits.iter().any(|s| is_absolute_path(s)) {
                self.record(
                    "destructive_fs",
                    "`Command::new(\"rm\")` targeting an absolute path is forbidden",
                );
            }
        }
        visit::visit_expr_method_call(self, i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_safe_rust_snippets() {
        let safe = r#"
            let mut x = String::from("ok");
            let status = bash2rust_runtime::run_command("echo", &[&x])?;
            let _ = std::fs::remove_file("relative_tmp.txt");
            println!("{x} {:?}", status);
        "#;
        validate_ai_rust(safe).expect("safe Rust must pass");
    }

    #[test]
    fn rejects_unparseable_rust() {
        let err = validate_ai_rust("let x = ;").expect_err("must reject invalid syntax");
        assert!(matches!(err, SecurityError::ParseError(_)));
    }

    #[test]
    fn rejects_unsafe_and_ffi() {
        assert!(validate_ai_rust("unsafe { let _ = 1; }").is_err());
        assert!(validate_ai_rust("unsafe fn bad() {}").is_err());
        assert!(validate_ai_rust("unsafe trait Bad {}").is_err());
        assert!(validate_ai_rust("extern \"C\" { fn puts(s: *const i8); }").is_err());
    }

    #[test]
    fn rejects_transmute_and_libc() {
        assert!(validate_ai_rust("let _x: i32 = std::mem::transmute(1u32);").is_err());
        assert!(validate_ai_rust("use libc::getpid;").is_err());
    }

    #[test]
    fn rejects_shell_execution() {
        assert!(validate_ai_rust(
            "std::process::Command::new(\"sh\").arg(\"-c\").arg(\"id\").status()?;"
        )
        .is_err());
        assert!(validate_ai_rust("std::process::Command::new(\"/bin/bash\").status()?;").is_err());
        assert!(
            validate_ai_rust("std::process::Command::new(String::from(\"sh\")).status()?;")
                .is_err()
        );
        assert!(
            validate_ai_rust("bash2rust_runtime::run_command(\"bash\", &[\"-c\", \"ls\"])?;")
                .is_err()
        );
        assert!(
            validate_ai_rust("bash2rust_runtime::run_command_env(\"sh\", &[], &[])?;").is_err()
        );
        assert!(
            validate_ai_rust("bash2rust_runtime::command_subst(\"eval\", &[\"echo 1\"])?;")
                .is_err()
        );
        assert!(validate_ai_rust(
            "bash2rust_runtime::run_pipeline(&[(\"sh\", &[\"-c\", \"id\"])])?;"
        )
        .is_err());
    }

    #[test]
    fn rejects_destructive_absolute_fs_ops() {
        assert!(validate_ai_rust("std::fs::remove_dir_all(\"/etc\")?;").is_err());
        assert!(
            validate_ai_rust("std::fs::remove_dir_all(std::path::Path::new(\"/etc\"))?;").is_err()
        );
        assert!(validate_ai_rust("std::fs::remove_file(\"/tmp/secret\")?;").is_err());
        assert!(validate_ai_rust("std::fs::remove_file(\"/tmp/secret\".to_string())?;").is_err());
        assert!(validate_ai_rust("std::fs::remove_dir(\"C:\\\\Windows\")?;").is_err());
        assert!(validate_ai_rust(
            "std::process::Command::new(\"rm\").arg(\"-rf\").arg(\"/var/log\").status()?;"
        )
        .is_err());
        assert!(
            validate_ai_rust("bash2rust_runtime::run_command(\"rm\", &[\"-rf\", \"/etc\"])?;")
                .is_err()
        );
    }
}
