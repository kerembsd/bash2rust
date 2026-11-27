//! Bash compatibility test harness and Markdown report generator.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use bash2rust_ir::{
    AssignValue, Block, IrProgram, ParamExpansionOp, Stmt, TestExpr, Word, WordPart,
};

use crate::{
    configure_rustc_cmd, ensure_runtime_artifacts, workspace_root, ExecutionResult, TempDirGuard,
    VerifyError,
};

/// Default OpenAI-compatible URL used by the AI-assisted compatibility harness.
pub const DEFAULT_COMPAT_OPENAI_URL: &str = "http://localhost:1235/v1";
/// Default model name used by the AI-assisted compatibility harness.
pub const DEFAULT_COMPAT_AI_MODEL: &str = "inkling-2-mini";
/// Minimum supported Bash version (`major.minor`) for the compatibility corpus.
pub const MIN_SUPPORTED_BASH_VERSION: (u32, u32) = (3, 2);

/// Parsed host `bash --version` metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BashVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub raw: String,
}

impl BashVersion {
    /// Returns `true` if `self` is at least `(min.0, min.1)`.
    pub fn meets_minimum(&self, min: (u32, u32)) -> bool {
        (self.major, self.minor) >= min
    }
}

/// Parses `bash --version` output (e.g. `GNU bash, version 3.2.57(1)-release` or `5.2.21`).
pub fn parse_bash_version(version_output: &str) -> Option<BashVersion> {
    let first_line = version_output.lines().next()?.trim();
    let after_ver = first_line
        .split("version ")
        .nth(1)
        .unwrap_or(first_line)
        .split_whitespace()
        .next()?;
    let numeric = after_ver.split('(').next().unwrap_or(after_ver);
    let mut parts = numeric.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    let patch = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    Some(BashVersion {
        major,
        minor,
        patch,
        raw: first_line.to_owned(),
    })
}

/// Returns the `bash` executable configured via `BASH2RUST_BASH`, or resolves `"bash"` from `PATH`.
pub fn bash_program() -> String {
    if let Ok(v) = std::env::var("BASH2RUST_BASH") {
        if !v.trim().is_empty() {
            return v;
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("bash");
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    "bash".to_owned()
}

/// Detects the host `bash` version by running `$BASH2RUST_BASH --version` (or `bash --version`).
pub fn detect_bash_version() -> Option<BashVersion> {
    let bash = bash_program();
    let output = Command::new(&bash).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_bash_version(&stdout)
}

pub fn parse_requires_bash(script_contents: &str) -> Option<(u32, u32)> {
    for line in script_contents.lines().take(16) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed
            .strip_prefix("# requires-bash:")
            .or_else(|| trimmed.strip_prefix("#requires-bash:"))
        {
            let ver = rest.trim();
            let mut parts = ver.split('.');
            if let (Some(maj), Some(min)) = (
                parts.next().and_then(|s| s.parse::<u32>().ok()),
                parts.next().and_then(|s| s.parse::<u32>().ok()),
            ) {
                return Some((maj, min));
            }
        }
    }
    None
}

static CLI_BINARY: OnceLock<Result<PathBuf, String>> = OnceLock::new();

/// Semantic category in the Bash compatibility corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompatCategory {
    WordSplitting,
    Quoting,
    Globbing,
    StrictMode,
    Subshells,
    Pipelines,
    CommandSubstitution,
    RedirectionsStderr,
    CaseAndParamExpansion,
    ArithmeticAndCFor,
    ExtendedTests,
    FunctionScoping,
}

impl CompatCategory {
    /// Ordered list of all 12 compatibility categories.
    pub const ALL: [Self; 12] = [
        Self::WordSplitting,
        Self::Quoting,
        Self::Globbing,
        Self::StrictMode,
        Self::Subshells,
        Self::Pipelines,
        Self::CommandSubstitution,
        Self::RedirectionsStderr,
        Self::CaseAndParamExpansion,
        Self::ArithmeticAndCFor,
        Self::ExtendedTests,
        Self::FunctionScoping,
    ];

    /// Infers the category from a compatibility fixture filename.
    pub fn from_filename(filename: &str) -> Self {
        if filename.contains("word_splitting") {
            Self::WordSplitting
        } else if filename.contains("quoting") {
            Self::Quoting
        } else if filename.contains("globbing") {
            Self::Globbing
        } else if filename.contains("strict_")
            || filename.contains("umask")
            || filename.contains("special_vars")
        {
            Self::StrictMode
        } else if filename.contains("subshell") {
            Self::Subshells
        } else if filename.contains("pipeline") {
            Self::Pipelines
        } else if filename.contains("cmdsubst") {
            Self::CommandSubstitution
        } else if filename.contains("case_")
            || filename.contains("parameter_expansion")
            || filename.contains("param_exp")
        {
            Self::CaseAndParamExpansion
        } else if filename.contains("arithmetic") || filename.contains("arith") {
            Self::ArithmeticAndCFor
        } else if filename.contains("extended_test") {
            Self::ExtendedTests
        } else if filename.contains("function_") || filename.contains("fn_dispatch") {
            Self::FunctionScoping
        } else {
            Self::RedirectionsStderr
        }
    }

    /// Returns the exact human-readable category header.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WordSplitting => "Word Splitting",
            Self::Quoting => "Quoting",
            Self::Globbing => "Globbing",
            Self::StrictMode => "set -euo pipefail",
            Self::Subshells => "Subshells",
            Self::Pipelines => "Pipelines",
            Self::CommandSubstitution => "Command Substitution",
            Self::RedirectionsStderr => "Redirections & Stderr",
            Self::CaseAndParamExpansion => "Case & Parameter Expansion",
            Self::ArithmeticAndCFor => "Arithmetic & C-Style Loops",
            Self::ExtendedTests => "Extended Tests",
            Self::FunctionScoping => "Function Scoping",
        }
    }
}

impl fmt::Display for CompatCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Execution mode under test (kept strictly separate in reports).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompatMode {
    Deterministic,
    AiAssisted,
}

impl CompatMode {
    /// Human-readable mode label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deterministic => "Deterministic",
            Self::AiAssisted => "AI-Assisted",
        }
    }
}

/// Outcome of a single compatibility test case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatStatus {
    Pass,
    CompileFailure {
        stage: String,
        message: String,
    },
    SemanticMismatch {
        bash: ExecutionResult,
        rust: ExecutionResult,
        summary: String,
    },
    Timeout {
        target: String,
        timeout_secs: u64,
    },
    Skipped {
        reason: String,
    },
}

/// Full result record for one compatibility script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatCaseResult {
    pub name: String,
    pub path: PathBuf,
    pub category: CompatCategory,
    pub mode: CompatMode,
    pub status: CompatStatus,
    pub unsupported_constructs: Vec<String>,
}

/// Aggregated results across an entire compatibility corpus run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatSuiteReport {
    pub mode: CompatMode,
    pub cases: Vec<CompatCaseResult>,
}

impl CompatSuiteReport {
    pub fn total(&self) -> usize {
        self.cases.len()
    }

    pub fn passed(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| matches!(c.status, CompatStatus::Pass))
            .count()
    }

    pub fn skipped(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| matches!(c.status, CompatStatus::Skipped { .. }))
            .count()
    }

    pub fn failed(&self) -> usize {
        self.total() - self.passed() - self.skipped()
    }

    pub fn compile_failures(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| matches!(c.status, CompatStatus::CompileFailure { .. }))
            .count()
    }

    pub fn semantic_mismatches(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| matches!(c.status, CompatStatus::SemanticMismatch { .. }))
            .count()
    }

    pub fn timeouts(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| matches!(c.status, CompatStatus::Timeout { .. }))
            .count()
    }

    pub fn scripts_with_unsupported(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| !c.unsupported_constructs.is_empty())
            .count()
    }
}

/// Collects all [`Stmt::Unsupported`] nodes in `program` so none are silently skipped.
pub fn collect_unsupported_constructs(program: &IrProgram) -> Vec<String> {
    let mut out = Vec::new();
    collect_from_stmts(&program.statements, &mut out);
    out
}

fn collect_from_block(block: &Block, out: &mut Vec<String>) {
    collect_from_stmts(&block.statements, out);
}

fn collect_from_stmts(stmts: &[Stmt], out: &mut Vec<String>) {
    for stmt in stmts {
        collect_from_stmt(stmt, out);
    }
}

fn collect_from_word(word: &Word, out: &mut Vec<String>) {
    for part in &word.parts {
        match part {
            WordPart::CommandSubst(inner) => collect_from_stmt(inner, out),
            WordPart::ParamExpansion {
                op:
                    ParamExpansionOp::UseDefault { default, .. }
                    | ParamExpansionOp::AssignDefault { default, .. },
                ..
            } => collect_from_word(default, out),
            WordPart::ParamExpansion {
                op: ParamExpansionOp::UseAlt { alt, .. },
                ..
            } => collect_from_word(alt, out),
            WordPart::ParamExpansion {
                op: ParamExpansionOp::ArrayElement { index },
                ..
            } => collect_from_word(index, out),
            WordPart::ParamExpansion {
                op: ParamExpansionOp::ArrayElemTrim { index, pattern, .. },
                ..
            } => {
                collect_from_word(index, out);
                collect_from_word(pattern, out);
            }
            WordPart::ParamExpansion {
                op:
                    ParamExpansionOp::ArrayElemReplace {
                        index,
                        pattern,
                        replacement,
                        ..
                    },
                ..
            } => {
                collect_from_word(index, out);
                collect_from_word(pattern, out);
                collect_from_word(replacement, out);
            }
            WordPart::ParamExpansion {
                op:
                    ParamExpansionOp::ArrayElemSubstring {
                        index,
                        offset,
                        length,
                    },
                ..
            } => {
                collect_from_word(index, out);
                collect_from_word(offset, out);
                if let Some(len) = length {
                    collect_from_word(len, out);
                }
            }
            WordPart::ParamExpansion {
                op: ParamExpansionOp::Trim { pattern, .. },
                ..
            } => collect_from_word(pattern, out),
            WordPart::ParamExpansion {
                op:
                    ParamExpansionOp::Replace {
                        pattern,
                        replacement,
                        ..
                    }
                    | ParamExpansionOp::ArrayReplace {
                        pattern,
                        replacement,
                        ..
                    },
                ..
            } => {
                collect_from_word(pattern, out);
                collect_from_word(replacement, out);
            }
            WordPart::ParamExpansion {
                op: ParamExpansionOp::CaseModify { pattern, .. },
                ..
            } => {
                if let Some(pat) = pattern {
                    collect_from_word(pat, out);
                }
            }
            WordPart::ParamExpansion {
                op:
                    ParamExpansionOp::Substring { offset, length }
                    | ParamExpansionOp::ArraySlice { offset, length },
                ..
            } => {
                collect_from_word(offset, out);
                if let Some(len) = length {
                    collect_from_word(len, out);
                }
            }
            WordPart::Literal(_)
            | WordPart::VarRef(_)
            | WordPart::ArithExpansion(_)
            | WordPart::ParamExpansion {
                op:
                    ParamExpansionOp::Length
                    | ParamExpansionOp::Indirect
                    | ParamExpansionOp::ArrayAll
                    | ParamExpansionOp::ArrayAllIfSet
                    | ParamExpansionOp::ArrayKeys,
                ..
            } => {}
        }
    }
}

fn collect_from_test_expr(expr: &TestExpr, out: &mut Vec<String>) {
    match expr {
        TestExpr::FileTest { path, .. } => collect_from_word(path, out),
        TestExpr::Unary { operand, .. } => collect_from_word(operand, out),
        TestExpr::Binary { lhs, rhs, .. } => {
            collect_from_word(lhs, out);
            collect_from_word(rhs, out);
        }
        TestExpr::Not(inner) => collect_from_test_expr(inner, out),
        TestExpr::And(l, r) | TestExpr::Or(l, r) => {
            collect_from_test_expr(l, out);
            collect_from_test_expr(r, out);
        }
    }
}

fn collect_from_stmt(stmt: &Stmt, out: &mut Vec<String>) {
    match stmt {
        Stmt::Unsupported { construct, raw, .. } => {
            let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
            let preview = truncate_preview(&compact, 80);
            out.push(format!("{construct}: `{preview}`"));
        }
        Stmt::Assign(a) => match &a.value {
            AssignValue::Scalar(w) => collect_from_word(w, out),
            AssignValue::Array(items) => {
                for item in items {
                    collect_from_word(&item.value, out);
                }
            }
        },
        Stmt::If(i) => {
            collect_from_block(&i.condition, out);
            collect_from_block(&i.then_branch, out);
            for b in &i.elif_branches {
                collect_from_block(&b.condition, out);
                collect_from_block(&b.body, out);
            }
            if let Some(e) = &i.else_branch {
                collect_from_block(e, out);
            }
        }
        Stmt::Case(c) => {
            collect_from_word(&c.value, out);
            for item in &c.items {
                for pat in &item.patterns {
                    collect_from_word(pat, out);
                }
                collect_from_block(&item.body, out);
            }
        }
        Stmt::For(f) => {
            if let Some(vals) = &f.values {
                for w in vals {
                    collect_from_word(w, out);
                }
            }
            collect_from_block(&f.body, out);
        }
        Stmt::ArithFor(f) => collect_from_block(&f.body, out),
        Stmt::Arith(_) => {}
        Stmt::Return { code, .. } => {
            if let Some(w) = code {
                collect_from_word(w, out);
            }
        }
        Stmt::Loop(l) => {
            collect_from_block(&l.condition, out);
            collect_from_block(&l.body, out);
        }
        Stmt::Function(f) => collect_from_block(&f.body, out),
        Stmt::Block(b) => collect_from_block(b, out),
        Stmt::Source {
            path, args, body, ..
        } => {
            collect_from_word(path, out);
            for arg in args {
                collect_from_word(arg, out);
            }
            if let Some(b) = body {
                collect_from_block(b, out);
            }
        }
        Stmt::Command(c) => {
            if let Some(name) = &c.name {
                collect_from_word(name, out);
            }
            for arg in &c.args {
                collect_from_word(arg, out);
            }
            for r in &c.redirects {
                collect_from_word(&r.target, out);
            }
        }
        Stmt::Test(t) => collect_from_test_expr(&t.expr, out),
        Stmt::AndOr(ao) => {
            collect_from_stmt(&ao.first, out);
            for item in &ao.rest {
                collect_from_stmt(&item.rhs, out);
            }
        }
        Stmt::Pipeline(p) => collect_from_stmts(&p.commands, out),
        Stmt::RawRust { .. } => {}
    }
}

/// Runs `program` inside an isolated `workdir` with a sanitized deterministic environment
/// and a hard `timeout`, reaping the child process if the timeout expires.
pub fn run_with_timeout(
    program: &str,
    args: &[&str],
    workdir: &Path,
    timeout: Duration,
    target_label: &str,
) -> Result<Result<ExecutionResult, CompatStatus>, VerifyError> {
    run_with_timeout_raw(program, args, workdir, timeout, target_label, true)
}

fn run_with_timeout_raw(
    program: &str,
    args: &[&str],
    workdir: &Path,
    timeout: Duration,
    target_label: &str,
    sanitized_env: bool,
) -> Result<Result<ExecutionResult, CompatStatus>, VerifyError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if sanitized_env {
        cmd.env_clear()
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", workdir)
            .env("TMPDIR", workdir);
    }

    let mut child = cmd.spawn()?;
    let mut stdout_pipe = child.stdout.take().expect("piped stdout");
    let mut stderr_pipe = child.stderr.take().expect("piped stderr");

    let stdout_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let stderr_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    let mut timed_out = false;
    let exit_status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(10));
    };

    let stdout_bytes = stdout_handle.join().unwrap_or_default();
    let stderr_bytes = stderr_handle.join().unwrap_or_default();

    if timed_out {
        return Ok(Err(CompatStatus::Timeout {
            target: target_label.to_owned(),
            timeout_secs: timeout.as_secs().max(1),
        }));
    }

    let status = exit_status.expect("child exited without timeout");
    Ok(Ok(ExecutionResult {
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        exit_code: status.code().unwrap_or(-1),
    }))
}

/// Strictly compares `bash` and `rust` [`ExecutionResult`]s (`stdout`, `stderr`, and `exit_code`).
pub fn compare_execution_results(bash: ExecutionResult, rust: ExecutionResult) -> CompatStatus {
    if bash == rust {
        return CompatStatus::Pass;
    }

    let mut diffs = Vec::new();
    if bash.exit_code != rust.exit_code {
        diffs.push(format!(
            "exit_code: bash={} vs rust={}",
            bash.exit_code, rust.exit_code
        ));
    }
    if bash.stdout != rust.stdout {
        diffs.push(format!(
            "stdout: bash={:?} vs rust={:?}",
            truncate_preview(&bash.stdout, 80),
            truncate_preview(&rust.stdout, 80)
        ));
    }
    if bash.stderr != rust.stderr {
        diffs.push(format!(
            "stderr: bash={:?} vs rust={:?}",
            truncate_preview(&bash.stderr, 80),
            truncate_preview(&rust.stderr, 80)
        ));
    }

    CompatStatus::SemanticMismatch {
        bash,
        rust,
        summary: diffs.join("; "),
    }
}

fn truncate_preview(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let taken: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        let keep = max_chars.saturating_sub(3);
        let head: String = taken.chars().take(keep).collect();
        format!("{head}...")
    } else {
        taken
    }
}

/// Executes a single compatibility script in deterministic mode inside isolated temp directories.
pub fn run_compat_case(
    script_path: &Path,
    timeout: Duration,
) -> Result<CompatCaseResult, VerifyError> {
    let abs_script = script_path
        .canonicalize()
        .unwrap_or_else(|_| script_path.to_path_buf());
    let name = script_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown.sh")
        .to_owned();
    let category = CompatCategory::from_filename(&name);

    if let Ok(script_src) = fs::read_to_string(&abs_script) {
        if let Some((req_maj, req_min)) = parse_requires_bash(&script_src) {
            if let Some(host) = detect_bash_version() {
                if !host.meets_minimum((req_maj, req_min)) {
                    let reason = format!(
                        "requires bash >= {req_maj}.{req_min}, host is {}.{}",
                        host.major, host.minor
                    );
                    eprintln!("warning: skipping {name}: {reason}");
                    return Ok(CompatCaseResult {
                        name,
                        path: script_path.to_path_buf(),
                        category,
                        mode: CompatMode::Deterministic,
                        status: CompatStatus::Skipped { reason },
                        unsupported_constructs: Vec::new(),
                    });
                }
            }
        }
    }

    let bash_workdir = TempDirGuard::create("bash2rust_compat_bash")?;
    let bash_arg = abs_script.to_string_lossy().into_owned();
    let bash_prog = bash_program();
    let bash_res = match run_with_timeout(
        &bash_prog,
        &[&bash_arg],
        &bash_workdir.path,
        timeout,
        "bash",
    )? {
        Ok(res) => res,
        Err(timeout_status) => {
            return Ok(CompatCaseResult {
                name,
                path: script_path.to_path_buf(),
                category,
                mode: CompatMode::Deterministic,
                status: timeout_status,
                unsupported_constructs: Vec::new(),
            });
        }
    };

    let compiled_ir = match bash2rust_core::compile_file_to_ir(&abs_script) {
        Ok(out) => out.ir,
        Err(err) => {
            return Ok(CompatCaseResult {
                name,
                path: script_path.to_path_buf(),
                category,
                mode: CompatMode::Deterministic,
                status: CompatStatus::CompileFailure {
                    stage: "parser".to_owned(),
                    message: err.to_string(),
                },
                unsupported_constructs: Vec::new(),
            });
        }
    };

    let unsupported_constructs = collect_unsupported_constructs(&compiled_ir);

    let rust_source = match bash2rust_core::generate_rust_from_ir(&compiled_ir) {
        Ok(src) => src,
        Err(err) => {
            return Ok(CompatCaseResult {
                name,
                path: script_path.to_path_buf(),
                category,
                mode: CompatMode::Deterministic,
                status: CompatStatus::CompileFailure {
                    stage: "codegen".to_owned(),
                    message: err.to_string(),
                },
                unsupported_constructs,
            });
        }
    };

    let artifacts = ensure_runtime_artifacts()?;
    let bin_dir = TempDirGuard::create("bash2rust_compat_bin")?;
    let src_file = bin_dir.path.join("main.rs");
    let bin_file = bin_dir.path.join("app");
    fs::write(&src_file, &rust_source)?;

    let mut rustc = configure_rustc_cmd(artifacts);
    rustc.arg("-o").arg(&bin_file).arg(&src_file);
    let compile_out = rustc.output()?;
    if !compile_out.status.success() {
        let stderr = String::from_utf8_lossy(&compile_out.stderr).into_owned();
        return Ok(CompatCaseResult {
            name,
            path: script_path.to_path_buf(),
            category,
            mode: CompatMode::Deterministic,
            status: CompatStatus::CompileFailure {
                stage: "rustc".to_owned(),
                message: stderr,
            },
            unsupported_constructs,
        });
    }

    let rust_workdir = TempDirGuard::create("bash2rust_compat_rust")?;
    let bin_str = bin_file.to_string_lossy().into_owned();
    let status = match run_with_timeout(&bin_str, &[], &rust_workdir.path, timeout, "rust")? {
        Ok(rust_res) => compare_execution_results(bash_res, rust_res),
        Err(timeout_status) => timeout_status,
    };

    Ok(CompatCaseResult {
        name,
        path: script_path.to_path_buf(),
        category,
        mode: CompatMode::Deterministic,
        status,
        unsupported_constructs,
    })
}

/// Discovers and runs all `*.sh` scripts in `corpus_dir` (sorted) without aborting on failures.
pub fn run_compatibility_suite(
    corpus_dir: &Path,
    timeout: Duration,
) -> Result<CompatSuiteReport, VerifyError> {
    let mut scripts: Vec<PathBuf> = fs::read_dir(corpus_dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("sh"))
        .collect();
    scripts.sort();

    let mut cases = Vec::with_capacity(scripts.len());
    for script in scripts {
        cases.push(run_compat_case(&script, timeout)?);
    }

    Ok(CompatSuiteReport {
        mode: CompatMode::Deterministic,
        cases,
    })
}

/// Builds the exact `bash2rust` CLI arguments for AI-assisted compatibility runs.
pub fn ai_assisted_cli_args(script_path: &Path) -> Vec<String> {
    vec![
        "--file".to_owned(),
        script_path.to_string_lossy().into_owned(),
        "--emit".to_owned(),
        "rust".to_owned(),
        "--ai".to_owned(),
        "openai".to_owned(),
        "--openai-url".to_owned(),
        DEFAULT_COMPAT_OPENAI_URL.to_owned(),
        "--ai-model".to_owned(),
        DEFAULT_COMPAT_AI_MODEL.to_owned(),
    ]
}

/// Probes `127.0.0.1:1235` with a 250ms timeout to check if the local AI server is reachable.
pub fn is_local_ai_server_reachable() -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], 1235));
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

fn ensure_cli_binary() -> Result<&'static PathBuf, VerifyError> {
    let res = CLI_BINARY.get_or_init(|| {
        let root = workspace_root();
        let bin = root.join("target/debug/bash2rust");
        if !bin.exists() {
            let cargo_bin = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
            let status = Command::new(&cargo_bin)
                .current_dir(&root)
                .args(["build", "-p", "bash2rust-cli"])
                .status()
                .map_err(|e| format!("failed to invoke `{cargo_bin}`: {e}"))?;
            if !status.success() {
                return Err(format!(
                    "`cargo build -p bash2rust-cli` exited with {status}"
                ));
            }
        }
        if !bin.exists() {
            return Err(format!("expected `{}` to exist after build", bin.display()));
        }
        Ok(bin)
    });

    res.as_ref()
        .map_err(|msg| VerifyError::RuntimeArtifact(msg.clone()))
}

/// Runs the AI-assisted pass on a single compatibility script that contains unsupported constructs.
pub fn run_ai_assisted_case(
    script_path: &Path,
    unsupported_constructs: Vec<String>,
    exec_timeout: Duration,
) -> Result<CompatCaseResult, VerifyError> {
    let abs_script = script_path
        .canonicalize()
        .unwrap_or_else(|_| script_path.to_path_buf());
    let name = script_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown.sh")
        .to_owned();
    let category = CompatCategory::from_filename(&name);

    let bash_workdir = TempDirGuard::create("bash2rust_ai_compat_bash")?;
    let bash_arg = abs_script.to_string_lossy().into_owned();
    let bash_res = match run_with_timeout(
        "bash",
        &[&bash_arg],
        &bash_workdir.path,
        exec_timeout,
        "bash",
    )? {
        Ok(res) => res,
        Err(timeout_status) => {
            return Ok(CompatCaseResult {
                name,
                path: script_path.to_path_buf(),
                category,
                mode: CompatMode::AiAssisted,
                status: timeout_status,
                unsupported_constructs,
            });
        }
    };

    let cli_bin = ensure_cli_binary()?;
    let cli_args = ai_assisted_cli_args(&abs_script);
    let cli_arg_refs: Vec<&str> = cli_args.iter().map(String::as_str).collect();
    let cli_bin_str = cli_bin.to_string_lossy().into_owned();
    let cli_res = run_with_timeout_raw(
        &cli_bin_str,
        &cli_arg_refs,
        &workspace_root(),
        Duration::from_secs(60),
        "ai-cli",
        false,
    )?;
    let cli_out = match cli_res {
        Ok(out) => out,
        Err(timeout_status) => {
            return Ok(CompatCaseResult {
                name,
                path: script_path.to_path_buf(),
                category,
                mode: CompatMode::AiAssisted,
                status: timeout_status,
                unsupported_constructs,
            });
        }
    };
    if cli_out.exit_code != 0 {
        return Ok(CompatCaseResult {
            name,
            path: script_path.to_path_buf(),
            category,
            mode: CompatMode::AiAssisted,
            status: CompatStatus::CompileFailure {
                stage: "ai-cli".to_owned(),
                message: cli_out.stderr,
            },
            unsupported_constructs,
        });
    }

    let rust_source = cli_out.stdout;
    let artifacts = ensure_runtime_artifacts()?;
    let bin_dir = TempDirGuard::create("bash2rust_ai_compat_bin")?;
    let src_file = bin_dir.path.join("main.rs");
    let bin_file = bin_dir.path.join("app");
    fs::write(&src_file, &rust_source)?;

    let mut rustc = configure_rustc_cmd(artifacts);
    rustc.arg("-o").arg(&bin_file).arg(&src_file);
    let compile_out = rustc.output()?;
    if !compile_out.status.success() {
        return Ok(CompatCaseResult {
            name,
            path: script_path.to_path_buf(),
            category,
            mode: CompatMode::AiAssisted,
            status: CompatStatus::CompileFailure {
                stage: "rustc".to_owned(),
                message: String::from_utf8_lossy(&compile_out.stderr).into_owned(),
            },
            unsupported_constructs,
        });
    }

    let rust_workdir = TempDirGuard::create("bash2rust_ai_compat_rust")?;
    let bin_str = bin_file.to_string_lossy().into_owned();
    let status = match run_with_timeout(&bin_str, &[], &rust_workdir.path, exec_timeout, "rust")? {
        Ok(rust_res) => compare_execution_results(bash_res, rust_res),
        Err(timeout_status) => timeout_status,
    };

    Ok(CompatCaseResult {
        name,
        path: script_path.to_path_buf(),
        category,
        mode: CompatMode::AiAssisted,
        status,
        unsupported_constructs,
    })
}

/// Runs AI-assisted compatibility tests on all scripts with unsupported constructs if `127.0.0.1:1235` is reachable.
pub fn run_ai_assisted_suite(
    det_report: &CompatSuiteReport,
    exec_timeout: Duration,
) -> Result<Option<CompatSuiteReport>, VerifyError> {
    if !is_local_ai_server_reachable() {
        return Ok(None);
    }

    let mut cases = Vec::new();
    for case in &det_report.cases {
        if !case.unsupported_constructs.is_empty() {
            cases.push(run_ai_assisted_case(
                &case.path,
                case.unsupported_constructs.clone(),
                exec_timeout,
            )?);
        }
    }

    Ok(Some(CompatSuiteReport {
        mode: CompatMode::AiAssisted,
        cases,
    }))
}

/// Renders a Markdown compatibility report with summary metrics, category table, and per-case details.
pub fn render_markdown_report(report: &CompatSuiteReport) -> String {
    render_markdown_report_with_ai(report, None)
}

/// Renders a Markdown compatibility report including an optional [`CompatMode::AiAssisted`] section.
pub fn render_markdown_report_with_ai(
    report: &CompatSuiteReport,
    ai_report: Option<&CompatSuiteReport>,
) -> String {
    let mut out = String::new();
    let total = report.total();
    let passed = report.passed();
    let skipped = report.skipped();
    let failed = report.failed();
    let pass_pct = if total == 0 {
        0.0
    } else {
        (passed as f64 / total as f64) * 100.0
    };
    let host_bash = detect_bash_version()
        .map(|v| format!("{}.{}.{} (`{}`)", v.major, v.minor, v.patch, v.raw))
        .unwrap_or_else(|| "unknown".to_owned());

    out.push_str("# `bash2rust` Bash Compatibility Report\n\n");
    out.push_str(
        "> **Scope Disclaimer:** All pass rates and metrics below apply strictly to the tested compatibility corpus (`tests/compatibility/`) under isolated deterministic execution, **not** universal Bash compatibility.\n\n",
    );
    out.push_str(&format!(
        "- **Host Bash Version:** {host_bash}\n\
         - **Minimum Supported Bash Version:** `{}.{}`\n\n",
        MIN_SUPPORTED_BASH_VERSION.0, MIN_SUPPORTED_BASH_VERSION.1,
    ));

    out.push_str("## Deterministic Mode\n\n");
    out.push_str(&format!(
        "- **Total tests:** {total}\n\
         - **Passed:** {passed} ({pass_pct:.1}%)\n\
         - **Skipped:** {skipped}\n\
         - **Failed:** {failed}\n\
         - **Compilation Failures:** {}\n\
         - **Semantic Mismatches:** {}\n\
         - **Timeouts:** {}\n\
         - **Scripts with Unsupported Syntax:** {}\n\n",
        report.compile_failures(),
        report.semantic_mismatches(),
        report.timeouts(),
        report.scripts_with_unsupported(),
    ));

    out.push_str("| Category | Total | Passed | Failed | Pass Rate |\n");
    out.push_str("| :--- | :---: | :---: | :---: | :---: |\n");

    let mut grouped: BTreeMap<CompatCategory, Vec<&CompatCaseResult>> = BTreeMap::new();
    for cat in CompatCategory::ALL {
        grouped.insert(cat, Vec::new());
    }
    for case in &report.cases {
        grouped.entry(case.category).or_default().push(case);
    }

    for cat in CompatCategory::ALL {
        let items = &grouped[&cat];
        let cat_total = items.len();
        let cat_pass = items
            .iter()
            .filter(|c| matches!(c.status, CompatStatus::Pass))
            .count();
        let cat_skip = items
            .iter()
            .filter(|c| matches!(c.status, CompatStatus::Skipped { .. }))
            .count();
        let cat_fail = cat_total - cat_pass - cat_skip;
        let rate = if cat_total == 0 {
            0.0
        } else {
            (cat_pass as f64 / cat_total as f64) * 100.0
        };
        let fixtures = items
            .iter()
            .map(|c| format!("`{}`", c.name))
            .collect::<Vec<_>>()
            .join(", ");
        let cat_label = if fixtures.is_empty() {
            cat.as_str().to_owned()
        } else {
            format!("{} ({fixtures})", cat.as_str())
        };
        out.push_str(&format!(
            "| {cat_label} | {cat_total} | {cat_pass} | {cat_fail} | {rate:.1}% |\n"
        ));
    }

    out.push_str("\n## AI-Assisted Mode\n\n");
    if let Some(ai) = ai_report {
        let ai_total = ai.total();
        let ai_passed = ai.passed();
        let ai_failed = ai.failed();
        let ai_pct = if ai_total == 0 {
            0.0
        } else {
            (ai_passed as f64 / ai_total as f64) * 100.0
        };
        out.push_str(&format!(
            "- **Endpoint:** `{DEFAULT_COMPAT_OPENAI_URL}` (`--ai openai --ai-model {DEFAULT_COMPAT_AI_MODEL}`)\n\
             - **Total scripts with unsupported syntax:** {ai_total}\n\
             - **Passed:** {ai_passed} ({ai_pct:.1}%)\n\
             - **Failed:** {ai_failed}\n\
             - **Compilation Failures:** {}\n\
             - **Semantic Mismatches:** {}\n\
             - **Timeouts:** {}\n\n",
            ai.compile_failures(),
            ai.semantic_mismatches(),
            ai.timeouts(),
        ));
        for case in &ai.cases {
            let status_label = match &case.status {
                CompatStatus::Pass => "PASS".to_owned(),
                CompatStatus::CompileFailure { stage, message } => {
                    let first_line = message.lines().next().unwrap_or("compile error");
                    format!(
                        "COMPILE FAILURE ({stage}): {}",
                        truncate_preview(first_line, 120)
                    )
                }
                CompatStatus::SemanticMismatch { summary, .. } => {
                    format!("SEMANTIC MISMATCH: {summary}")
                }
                CompatStatus::Timeout {
                    target,
                    timeout_secs,
                } => {
                    format!("TIMEOUT ({target} exceeded {timeout_secs}s)")
                }
                CompatStatus::Skipped { reason } => {
                    format!("SKIPPED ({reason})")
                }
            };
            out.push_str(&format!("- `{}` — **{}**\n", case.name, status_label));
        }
        out.push('\n');
    } else {
        out.push_str(&format!(
            "- **Status:** `{DEFAULT_COMPAT_OPENAI_URL}` (`{DEFAULT_COMPAT_AI_MODEL}`) was unavailable in this environment (skipped offline).\n\
             - AI-assisted translation (`--ai openai --openai-url {DEFAULT_COMPAT_OPENAI_URL} --ai-model {DEFAULT_COMPAT_AI_MODEL}`) is tracked separately from deterministic compiler parity.\n\n",
        ));
    }

    out.push_str("## Detailed Per-Category Results\n\n");
    for cat in CompatCategory::ALL {
        out.push_str(&format!("### {}\n\n", cat.as_str()));
        for case in &grouped[&cat] {
            let status_label = match &case.status {
                CompatStatus::Pass => "PASS".to_owned(),
                CompatStatus::CompileFailure { stage, message } => {
                    let first_line = message.lines().next().unwrap_or("compile error");
                    format!(
                        "COMPILE FAILURE ({stage}): {}",
                        truncate_preview(first_line, 120)
                    )
                }
                CompatStatus::SemanticMismatch { summary, .. } => {
                    format!("SEMANTIC MISMATCH: {summary}")
                }
                CompatStatus::Timeout {
                    target,
                    timeout_secs,
                } => {
                    format!("TIMEOUT ({target} exceeded {timeout_secs}s)")
                }
                CompatStatus::Skipped { reason } => {
                    format!("SKIPPED ({reason})")
                }
            };
            let unsup = if case.unsupported_constructs.is_empty() {
                "none".to_owned()
            } else {
                case.unsupported_constructs.join(", ")
            };
            out.push_str(&format!(
                "- `{}` — **{}** (unsupported: {})\n",
                case.name, status_label, unsup
            ));
        }
        out.push('\n');
    }

    out
}

/// Writes [`render_markdown_report`] to `output_path`.
pub fn write_markdown_report(
    report: &CompatSuiteReport,
    output_path: &Path,
) -> Result<(), VerifyError> {
    write_markdown_report_with_ai(report, None, output_path)
}

/// Writes [`render_markdown_report_with_ai`] to `output_path`.
pub fn write_markdown_report_with_ai(
    report: &CompatSuiteReport,
    ai_report: Option<&CompatSuiteReport>,
    output_path: &Path,
) -> Result<(), VerifyError> {
    let markdown = render_markdown_report_with_ai(report, ai_report);
    fs::write(output_path, markdown)?;
    Ok(())
}
