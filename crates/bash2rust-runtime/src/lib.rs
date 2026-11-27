//! Runtime compatibility layer for generated `bash2rust` Rust programs.
//!
//! All process execution uses `std::process::Command` directly without invoking `sh -c`.

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Write};
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use thiserror::Error;

mod source_interp;

static SIGINT_PENDING: AtomicBool = AtomicBool::new(false);
static SIGTERM_PENDING: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn handle_sigint(_: libc::c_int) {
    SIGINT_PENDING.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
extern "C" fn handle_sigterm(_: libc::c_int) {
    SIGTERM_PENDING.store(true, Ordering::SeqCst);
}

/// Callback type for invoking generated shell functions by name from the runtime.
pub type FnDispatcher =
    fn(&mut ShellState, &str, &[&str]) -> Result<Option<i32>, Box<dyn std::error::Error>>;

/// Snapshot of scalar, array, and nameref variables for subshell isolation.
pub type VarsSnapshot = (
    HashMap<String, String>,
    HashMap<String, Vec<String>>,
    HashMap<String, String>,
);

/// Errors returned by `bash2rust-runtime` helpers.
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("failed to execute `{program}`: {source}")]
    Io {
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("command `{program}` exited with status {code:?}: {stderr}")]
    CommandFailed {
        program: String,
        code: Option<i32>,
        stderr: String,
    },

    #[error("output from `{program}` was not valid UTF-8: {source}")]
    InvalidUtf8 {
        program: String,
        #[source]
        source: std::string::FromUtf8Error,
    },

    #[error("cannot execute an empty pipeline")]
    EmptyPipeline,

    #[error("subshell exited with status {0}")]
    SubshellExit(i32),
}

/// Convenience result alias for runtime operations.
pub type RuntimeResult<T> = Result<T, RuntimeError>;

/// Captured output and exit status of a command or pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub success: bool,
}

/// Runs a command directly and returns its [`ExitStatus`].
pub fn run_command(program: &str, args: &[&str]) -> RuntimeResult<ExitStatus> {
    Command::new(program)
        .args(args)
        .status()
        .map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })
}

/// Runs a command with extra environment variables and returns its [`ExitStatus`].
pub fn run_command_env(
    program: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> RuntimeResult<ExitStatus> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.status().map_err(|source| RuntimeError::Io {
        program: program.to_owned(),
        source,
    })
}

/// Runs a command and returns `true` if it exited with status 0.
pub fn exec_ok(program: &str, args: &[&str]) -> bool {
    match program {
        ":" | "true" => true,
        "false" => false,
        _ => run_command(program, args).is_ok_and(|s| s.success()),
    }
}

/// Returns an environment variable's value, or an empty string if unset (matching Bash `$VAR`).
pub fn get_env(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

/// Tracks mutable shell execution state (`$?`, `set -euo pipefail`, `IFS`, global vars/arrays, capture stack) in generated binaries.
#[derive(Debug, Clone)]
pub struct ShellState {
    pub last_status: i32,
    pub errexit: bool,
    pub errexit_suppress: usize,
    pub subshell_depth: usize,
    pub nounset: bool,
    pub pipefail: bool,
    pub ifs: Option<String>,
    pub vars: HashMap<String, String>,
    pub arrays: HashMap<String, Vec<String>>,
    pub namerefs: HashMap<String, String>,
    pub source_stack: Vec<std::path::PathBuf>,
    pub sourced_functions: HashMap<String, Vec<String>>,
    pub stdout_capture_stack: Vec<Vec<u8>>,
    pub stdin_override: Option<Vec<u8>>,
    pub positional_args: Vec<String>,
    pub fn_dispatcher: Option<FnDispatcher>,
    pub exit_trap: Option<String>,
    pub int_trap: Option<String>,
    pub term_trap: Option<String>,
    pub err_trap: Option<String>,
    start_instant: Instant,
    seconds_offset: i64,
    random_state: Cell<u64>,
}

impl Default for ShellState {
    fn default() -> Self {
        let seed = (std::process::id() as u64)
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        Self {
            last_status: 0,
            errexit: false,
            errexit_suppress: 0,
            subshell_depth: 0,
            nounset: false,
            pipefail: false,
            ifs: None,
            vars: HashMap::new(),
            arrays: HashMap::new(),
            namerefs: HashMap::new(),
            source_stack: Vec::new(),
            sourced_functions: HashMap::new(),
            stdout_capture_stack: Vec::new(),
            stdin_override: None,
            positional_args: Vec::new(),
            fn_dispatcher: None,
            exit_trap: None,
            int_trap: None,
            term_trap: None,
            err_trap: None,
            start_instant: Instant::now(),
            seconds_offset: 0,
            random_state: Cell::new(seed),
        }
    }
}

impl ShellState {
    /// Registers the generated dynamic function dispatcher for trap callbacks.
    pub fn set_fn_dispatcher(&mut self, dispatcher: FnDispatcher) {
        self.fn_dispatcher = Some(dispatcher);
    }

    /// Snapshot of scalar, array, and nameref shell variables for subshell isolation.
    pub fn vars_snapshot(&self) -> VarsSnapshot {
        (
            self.vars.clone(),
            self.arrays.clone(),
            self.namerefs.clone(),
        )
    }

    /// Restores scalar, array, and nameref shell variables after a subshell finishes.
    pub fn restore_vars(&mut self, snap: VarsSnapshot) {
        self.vars = snap.0;
        self.arrays = snap.1;
        self.namerefs = snap.2;
    }

    /// Returns `true` if any namerefs (`declare -n` / `local -n`) are currently active.
    pub fn has_namerefs(&self) -> bool {
        !self.namerefs.is_empty()
    }

    /// Resolves a variable name through `namerefs`, bounded against cycles.
    pub fn resolve_nameref<'a>(&'a self, name: &'a str) -> &'a str {
        let mut cur = name;
        for _ in 0..32 {
            match self.namerefs.get(cur) {
                Some(next) if next != cur => cur = next.as_str(),
                _ => break,
            }
        }
        cur
    }

    /// Resolves an indirect variable expansion (`${!name}`).
    pub fn get_indirect(&self, name: &str) -> Option<String> {
        let target = self
            .get_var(name)
            .map(str::to_owned)
            .or_else(|| std::env::var(name).ok())?;
        if target.is_empty() {
            return None;
        }
        self.get_var(&target)
            .map(str::to_owned)
            .or_else(|| std::env::var(&target).ok())
            .or_else(|| self.resolve_special_var(&target, 1))
    }

    /// Resolves an indirect variable expansion (`${!name}`), honoring `set -u`.
    pub fn get_indirect_checked(&self, name: &str, source: &str, line: usize) -> String {
        let target = self.get_var_or_env_checked(name, source, line);
        if target.is_empty() {
            if self.nounset {
                eprintln!("{source}: line {line}: {name}: invalid indirect expansion");
                std::process::exit(1);
            }
            return String::new();
        }
        self.get_var_or_env_checked(&target, source, line)
    }

    /// Enters a subshell scope.
    pub fn enter_subshell(&mut self) {
        self.subshell_depth += 1;
    }

    /// Leaves a subshell scope.
    pub fn leave_subshell(&mut self) {
        self.subshell_depth = self.subshell_depth.saturating_sub(1);
    }

    /// Returns `true` if currently executing inside a subshell.
    pub fn in_subshell(&self) -> bool {
        self.subshell_depth > 0
    }

    /// Runs the registered `EXIT` trap at most once, if any.
    pub fn run_exit_trap(&mut self) {
        if let (Some(handler), Some(dispatch)) = (self.exit_trap.take(), self.fn_dispatcher) {
            let fn_name = handler.split_whitespace().next().unwrap_or("");
            if !fn_name.is_empty() {
                let _ = dispatch(self, fn_name, &[]);
            }
        }
    }

    /// Checks whether `SIGINT` or `SIGTERM` was received and invokes the corresponding trap.
    pub fn check_signals(&mut self) {
        if SIGINT_PENDING.swap(false, Ordering::SeqCst) {
            if let (Some(handler), Some(dispatch)) = (self.int_trap.clone(), self.fn_dispatcher) {
                let fn_name = handler.split_whitespace().next().unwrap_or("");
                if !fn_name.is_empty() {
                    let _ = dispatch(self, fn_name, &[]);
                }
            }
        }
        if SIGTERM_PENDING.swap(false, Ordering::SeqCst) {
            if let (Some(handler), Some(dispatch)) = (self.term_trap.clone(), self.fn_dispatcher) {
                let fn_name = handler.split_whitespace().next().unwrap_or("");
                if !fn_name.is_empty() {
                    let _ = dispatch(self, fn_name, &[]);
                }
            }
        }
    }
    /// Updates and returns `last_status`.
    pub fn set_status(&mut self, code: i32) -> i32 {
        self.last_status = code;
        code
    }

    /// Returns `$?` formatted as a decimal string.
    pub fn status_string(&self) -> String {
        self.last_status.to_string()
    }

    /// Sets a global shell variable.
    pub fn set_var(&mut self, name: &str, val: impl Into<String>) {
        let target = self.resolve_nameref(name).to_owned();
        let val = val.into();
        if target == "SECONDS" {
            self.seconds_offset = val.trim().parse::<i64>().unwrap_or(0);
            self.start_instant = Instant::now();
            return;
        }
        if target == "RANDOM" {
            let seed = val.trim().parse::<u64>().unwrap_or(0);
            self.random_state.set(seed);
            return;
        }
        self.vars.insert(target, val);
    }

    /// Gets a global shell variable if set.
    pub fn get_var(&self, name: &str) -> Option<&str> {
        let target = self.resolve_nameref(name);
        self.vars.get(target).map(String::as_str)
    }

    /// Resolves Bash built-in special variables (`EUID`, `UID`, `PPID`, `HOSTNAME`, `OSTYPE`, `RANDOM`, `SECONDS`, `LINENO`).
    pub fn resolve_special_var(&self, name: &str, line: usize) -> Option<String> {
        match name {
            "EUID" => {
                #[cfg(unix)]
                {
                    // SAFETY: `geteuid` is always safe to call.
                    Some(unsafe { libc::geteuid() }.to_string())
                }
                #[cfg(not(unix))]
                {
                    Some("1000".to_owned())
                }
            }
            "UID" => {
                #[cfg(unix)]
                {
                    // SAFETY: `getuid` is always safe to call.
                    Some(unsafe { libc::getuid() }.to_string())
                }
                #[cfg(not(unix))]
                {
                    Some("1000".to_owned())
                }
            }
            "PPID" => {
                #[cfg(unix)]
                {
                    Some(std::os::unix::process::parent_id().to_string())
                }
                #[cfg(not(unix))]
                {
                    Some("0".to_owned())
                }
            }
            "HOSTNAME" => {
                #[cfg(unix)]
                {
                    let mut buf = [0u8; 256];
                    // SAFETY: `buf` is valid writable memory of length `buf.len()`.
                    let rc = unsafe {
                        libc::gethostname(buf.as_mut_ptr().cast::<libc::c_char>(), buf.len())
                    };
                    if rc == 0 {
                        let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                        let host = String::from_utf8_lossy(&buf[..nul]).into_owned();
                        if !host.is_empty() {
                            return Some(host);
                        }
                    }
                    Some("localhost".to_owned())
                }
                #[cfg(not(unix))]
                {
                    Some("localhost".to_owned())
                }
            }
            "OSTYPE" => {
                if cfg!(target_os = "macos") {
                    Some("darwin".to_owned())
                } else if cfg!(target_os = "linux") {
                    Some("linux-gnu".to_owned())
                } else {
                    Some(std::env::consts::OS.to_owned())
                }
            }
            "RANDOM" => {
                let next = self
                    .random_state
                    .get()
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                self.random_state.set(next);
                Some(((next >> 16) % 32768).to_string())
            }
            "SECONDS" => {
                let elapsed = self.start_instant.elapsed().as_secs() as i64;
                Some((self.seconds_offset + elapsed).max(0).to_string())
            }
            "LINENO" => Some(line.to_string()),
            _ => None,
        }
    }

    /// Reads a global shell variable or environment variable (or Bash special variable), enforcing `set -u` (`nounset`).
    pub fn get_var_or_env_checked(&self, name: &str, source: &str, line: usize) -> String {
        let target = self.resolve_nameref(name);
        if let Some(v) = self.vars.get(target) {
            return v.clone();
        }
        if let Ok(v) = std::env::var(target) {
            return v;
        }
        if let Some(v) = self.resolve_special_var(target, line) {
            return v;
        }
        self.get_env_checked(target, source, line)
    }

    /// Appends `suffix` to a global shell variable.
    pub fn append_var(&mut self, name: &str, suffix: &str) {
        let target = self.resolve_nameref(name).to_owned();
        self.vars.entry(target).or_default().push_str(suffix);
    }

    /// Unsets a global shell variable/array (and clears `ifs` if `name == "IFS"`).
    pub fn unset_var(&mut self, name: &str) {
        let target = self.resolve_nameref(name).to_owned();
        if target == "IFS" {
            self.ifs = None;
        }
        self.vars.remove(&target);
        self.arrays.remove(&target);
    }

    /// Sets a global shell array.
    pub fn set_array(&mut self, name: &str, items: Vec<String>) {
        let target = self.resolve_nameref(name).to_owned();
        self.arrays.insert(target, items);
    }

    /// Returns a slice of a global shell array (empty slice if unset).
    pub fn get_array(&self, name: &str) -> &[String] {
        let target = self.resolve_nameref(name);
        self.arrays.get(target).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Returns an element of a global shell array (or `""` if out of bounds).
    pub fn get_array_elem(&self, name: &str, idx: usize) -> &str {
        let target = self.resolve_nameref(name);
        self.arrays
            .get(target)
            .and_then(|v| v.get(idx))
            .map(String::as_str)
            .unwrap_or("")
    }

    /// Sets an element of a global shell array, growing with empty strings if needed.
    pub fn set_array_elem(&mut self, name: &str, idx: usize, val: impl Into<String>) {
        let target = self.resolve_nameref(name).to_owned();
        let arr = self.arrays.entry(target).or_default();
        if idx >= arr.len() {
            arr.resize(idx + 1, String::new());
        }
        arr[idx] = val.into();
    }

    /// Appends items to a global shell array.
    pub fn append_array(&mut self, name: &str, items: Vec<String>) {
        let target = self.resolve_nameref(name).to_owned();
        self.arrays.entry(target).or_default().extend(items);
    }

    /// Returns the length of a global shell array.
    pub fn array_len(&self, name: &str) -> usize {
        let target = self.resolve_nameref(name);
        self.arrays.get(target).map_or(0, Vec::len)
    }

    /// Evaluates a Bash integer arithmetic expression (`$(( ... ))`, `(( ... ))`, `for (( ... ))`).
    pub fn eval_arith(&mut self, expr: &str) -> i64 {
        let trimmed = expr.trim();
        if trimmed.is_empty() {
            return 0;
        }
        let mut parser = ArithParser::new(trimmed, self);
        parser.parse_comma()
    }

    /// Pushes a new in-process stdout capture buffer onto the stack (for `$(func)` / pipelines).
    pub fn push_stdout_capture(&mut self) {
        self.stdout_capture_stack.push(Vec::new());
    }

    /// Pops the top stdout capture buffer and trims trailing `\n` / `\r\n` like POSIX `$(...)`.
    pub fn pop_stdout_capture(&mut self) -> String {
        let buf = self.stdout_capture_stack.pop().unwrap_or_default();
        String::from_utf8_lossy(&buf)
            .trim_end_matches(['\r', '\n'])
            .to_owned()
    }

    /// Pops the top stdout capture buffer as raw bytes (for in-process pipeline stages).
    pub fn pop_stdout_capture_raw(&mut self) -> Vec<u8> {
        self.stdout_capture_stack.pop().unwrap_or_default()
    }

    /// Sets the active `stdin_override` buffer for the next command/stage.
    pub fn push_stdin_override(&mut self, data: Vec<u8>) {
        self.stdin_override = Some(data);
    }

    /// Takes and clears the active `stdin_override` buffer.
    pub fn take_stdin_override(&mut self) -> Option<Vec<u8>> {
        self.stdin_override.take()
    }

    /// Sets or clears the active `stdin_override` buffer.
    pub fn set_stdin_override(&mut self, data: Option<Vec<u8>>) {
        self.stdin_override = data;
    }

    /// Updates the active positional arguments (`$1`..`$N`, `$@`, `$#`).
    pub fn set_positional_args(&mut self, args: &[&str]) {
        self.positional_args = args.iter().map(|s| (*s).to_owned()).collect();
    }

    /// Reads any `fd == 0` input redirection into `stdin_override` for in-process function calls.
    pub fn push_stdin_from_redirects(
        &mut self,
        redirects: &[RedirectSpec<'_>],
    ) -> RuntimeResult<()> {
        for spec in redirects {
            if spec.fd == 0 {
                match spec.mode {
                    RedirectMode::Input(path) => {
                        let bytes = fs::read(path).map_err(|source| RuntimeError::Io {
                            program: path.to_owned(),
                            source,
                        })?;
                        self.push_stdin_override(bytes);
                    }
                    RedirectMode::InputBytes(bytes) => {
                        self.push_stdin_override(bytes.to_vec());
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Routes captured stdout from an in-process function call according to `redirects`.
    pub fn apply_captured_redirects(
        &mut self,
        captured: &[u8],
        redirects: &[RedirectSpec<'_>],
    ) -> RuntimeResult<()> {
        let mut fd1_mode: Option<RedirectMode<'_>> = None;
        for spec in redirects {
            if spec.fd == 1 {
                fd1_mode = Some(spec.mode);
            }
        }
        match fd1_mode {
            Some(RedirectMode::OutputTruncate(path)) => {
                fs::write(path, captured).map_err(|source| RuntimeError::Io {
                    program: path.to_owned(),
                    source,
                })?;
            }
            Some(RedirectMode::OutputAppend(path)) => {
                let mut f = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|source| RuntimeError::Io {
                        program: path.to_owned(),
                        source,
                    })?;
                f.write_all(captured).map_err(|source| RuntimeError::Io {
                    program: path.to_owned(),
                    source,
                })?;
            }
            Some(RedirectMode::DupStdoutToStderr)
            | Some(RedirectMode::DupOutput { target_fd: 2 }) => {
                let mut err = io::stderr().lock();
                let _ = err.write_all(captured);
                let _ = err.flush();
            }
            _ => {
                if let Some(top) = self.stdout_capture_stack.last_mut() {
                    top.extend_from_slice(captured);
                } else if !captured.is_empty() {
                    let mut out = io::stdout().lock();
                    let _ = out.write_all(captured);
                    let _ = out.flush();
                }
            }
        }
        Ok(())
    }

    /// Executes `program` with `args`, honoring active `stdout_capture_stack` and `stdin_override`.
    pub fn run_command(&mut self, program: &str, args: &[&str]) -> RuntimeResult<i32> {
        self.run_command_env(program, args, &[])
    }

    /// Executes `program` with `args` and `env`, honoring active `stdout_capture_stack` and `stdin_override`.
    pub fn run_command_env(
        &mut self,
        program: &str,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> RuntimeResult<i32> {
        self.run_command_with_redirects(program, args, env, &[])
    }

    /// Executes `program` with `args`, `env`, and `redirects`, honoring active `stdout_capture_stack` and `stdin_override`.
    pub fn run_command_with_redirects(
        &mut self,
        program: &str,
        args: &[&str],
        env: &[(&str, &str)],
        redirects: &[RedirectSpec<'_>],
    ) -> RuntimeResult<i32> {
        if let Some(rc) = self.try_run_builtin(program, args, env, redirects)? {
            return Ok(self.set_status(rc));
        }
        if redirects.is_empty() {
            if let Some(res) = self.call_sourced_fn(program, args) {
                let rc = res?;
                return Ok(self.set_status(rc));
            }
        }

        let mut stdin_data = self.stdin_override.take();
        let capture_stdout =
            !self.stdout_capture_stack.is_empty() && !redirects.iter().any(|r| r.fd == 1);

        if stdin_data.is_none() && !capture_stdout {
            let status = if redirects.is_empty() {
                run_command_env(program, args, env)?
            } else {
                run_command_with_redirects(program, args, env, redirects)?
            };
            let rc = Self::exit_code_from_status(&status);
            self.set_status(rc);
            self.check_signals();
            return Ok(self.last_status);
        }

        let mut cmd = Command::new(program);
        cmd.args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }

        if stdin_data.is_some() {
            cmd.stdin(Stdio::piped());
        }
        if capture_stdout {
            cmd.stdout(Stdio::piped());
        }

        let mut stdout_file: Option<File> = None;
        let mut stderr_file: Option<File> = None;
        let mut dup_1_to_2 = false;
        let mut dup_2_to_1 = false;

        for redir in redirects {
            match redir.mode {
                RedirectMode::Input(path) if redir.fd == 0 => {
                    stdin_data = None;
                    let f = File::open(path).map_err(|source| RuntimeError::Io {
                        program: program.to_owned(),
                        source,
                    })?;
                    cmd.stdin(Stdio::from(f));
                }
                RedirectMode::InputBytes(bytes) if redir.fd == 0 => {
                    stdin_data = Some(bytes.to_vec());
                    cmd.stdin(Stdio::piped());
                }
                RedirectMode::OutputTruncate(path) => {
                    let f = open_output_truncate(path)?;
                    if redir.fd == 2 {
                        let clone = f.try_clone().map_err(|source| RuntimeError::Io {
                            program: program.to_owned(),
                            source,
                        })?;
                        stderr_file = Some(clone);
                        cmd.stderr(Stdio::from(f));
                    } else {
                        let clone = f.try_clone().map_err(|source| RuntimeError::Io {
                            program: program.to_owned(),
                            source,
                        })?;
                        stdout_file = Some(clone);
                        cmd.stdout(Stdio::from(f));
                    }
                }
                RedirectMode::OutputAppend(path) => {
                    let f = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .map_err(|source| RuntimeError::Io {
                            program: program.to_owned(),
                            source,
                        })?;
                    if redir.fd == 2 {
                        let clone = f.try_clone().map_err(|source| RuntimeError::Io {
                            program: program.to_owned(),
                            source,
                        })?;
                        stderr_file = Some(clone);
                        cmd.stderr(Stdio::from(f));
                    } else {
                        let clone = f.try_clone().map_err(|source| RuntimeError::Io {
                            program: program.to_owned(),
                            source,
                        })?;
                        stdout_file = Some(clone);
                        cmd.stdout(Stdio::from(f));
                    }
                }
                RedirectMode::DupOutput { target_fd } => {
                    if redir.fd == 2 && target_fd == 1 {
                        dup_2_to_1 = true;
                    } else if redir.fd == 1 && target_fd == 2 {
                        dup_1_to_2 = true;
                    }
                }
                RedirectMode::DupStdoutToStderr => {
                    dup_1_to_2 = true;
                }
                RedirectMode::Input(_) | RedirectMode::InputBytes(_) | RedirectMode::CloseFd => {}
            }
        }

        if dup_2_to_1 {
            if let Some(ref f) = stdout_file {
                let clone = f.try_clone().map_err(|source| RuntimeError::Io {
                    program: program.to_owned(),
                    source,
                })?;
                cmd.stderr(Stdio::from(clone));
            }
        }
        if dup_1_to_2 {
            if let Some(ref f) = stderr_file {
                let clone = f.try_clone().map_err(|source| RuntimeError::Io {
                    program: program.to_owned(),
                    source,
                })?;
                cmd.stdout(Stdio::from(clone));
            } else {
                let (reader, writer) = io::pipe().map_err(|source| RuntimeError::Io {
                    program: program.to_owned(),
                    source,
                })?;
                cmd.stdout(Stdio::from(writer));
                drop(reader);
            }
        }

        let mut child = cmd.spawn().map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;

        let writer_handle = stdin_data.and_then(|data| {
            child.stdin.take().map(|mut child_stdin| {
                std::thread::spawn(move || {
                    let _ = child_stdin.write_all(&data);
                })
            })
        });

        let output = child
            .wait_with_output()
            .map_err(|source| RuntimeError::Io {
                program: program.to_owned(),
                source,
            })?;

        if let Some(h) = writer_handle {
            let _ = h.join();
        }
        if capture_stdout {
            if let Some(top) = self.stdout_capture_stack.last_mut() {
                top.extend_from_slice(&output.stdout);
            }
        }
        let rc = Self::exit_code_from_status(&output.status);
        self.set_status(rc);
        self.check_signals();
        Ok(self.last_status)
    }

    /// Executes an external pipeline, honoring active `stdout_capture_stack` and `stdin_override`.
    pub fn run_pipeline_inherit_status(
        &mut self,
        stages: &[(&str, &[&str])],
        pipefail: bool,
    ) -> RuntimeResult<i32> {
        let stdin_data = self.stdin_override.take();
        let capture_stdout = !self.stdout_capture_stack.is_empty();

        if stdin_data.is_none() && !capture_stdout {
            let rc = run_pipeline_inherit_status(stages, pipefail)?;
            return Ok(self.set_status(rc));
        }

        if stages.is_empty() {
            return Ok(self.set_status(0));
        }

        if stages.len() == 1 {
            if let Some(rc) = self.try_run_builtin(stages[0].0, stages[0].1, &[], &[])? {
                return Ok(self.set_status(rc));
            }
        }

        let mut cur_input = stdin_data;
        let mut final_rc = 0i32;
        for (idx, (prog, args)) in stages.iter().enumerate() {
            let is_last = idx + 1 == stages.len();
            if let Some(data) = cur_input.take() {
                self.stdin_override = Some(data);
            }
            if !is_last || capture_stdout {
                self.push_stdout_capture();
            }
            let rc = self.run_command(prog, args)?;
            if pipefail {
                if rc != 0 {
                    final_rc = rc;
                }
            } else {
                final_rc = rc;
            }
            if !is_last {
                cur_input = Some(self.pop_stdout_capture_raw());
            } else if capture_stdout {
                let out = self.pop_stdout_capture_raw();
                if let Some(top) = self.stdout_capture_stack.last_mut() {
                    top.extend_from_slice(&out);
                }
            }
        }
        Ok(self.set_status(final_rc))
    }

    /// Converts an [`ExitStatus`] to a Bash-compatible exit code (`128 + signal` on Unix signals).
    pub fn exit_code_from_status(status: &ExitStatus) -> i32 {
        if let Some(code) = status.code() {
            return code;
        }
        #[cfg(unix)]
        if let Some(sig) = status.signal() {
            return 128 + sig;
        }
        1
    }

    /// Exits immediately with `last_status` if `errexit` (`set -e`) is enabled and `last_status != 0`.
    pub fn check_errexit(&mut self) -> RuntimeResult<()> {
        self.check_signals();
        if self.errexit && self.errexit_suppress == 0 && self.last_status != 0 {
            let code = self.last_status;
            if self.subshell_depth > 0 {
                return Err(RuntimeError::SubshellExit(code));
            }
            self.run_exit_trap();
            std::process::exit(code);
        }
        Ok(())
    }

    /// Reads an environment variable, exiting with Bash's `unbound variable` error if `nounset` (`set -u`) is enabled.
    pub fn get_env_checked(&self, name: &str, source: &str, line: usize) -> String {
        if let Ok(v) = std::env::var(name) {
            return v;
        }
        if self.nounset {
            eprintln!("{source}: line {line}: {name}: unbound variable");
            std::process::exit(1);
        }
        String::new()
    }

    /// Applies `set` builtin flags (`-e`/`+e`, `-u`/`+u`, `-o pipefail`/`+o pipefail`, and combined short flags like `-euo pipefail`).
    pub fn apply_set_flags(&mut self, args: &[&str]) {
        let mut i = 0usize;
        while i < args.len() {
            let arg = args[i];
            if arg == "--" {
                break;
            }
            if let Some(rest) = arg.strip_prefix('-').or_else(|| arg.strip_prefix('+')) {
                let enable = arg.starts_with('-');
                if rest.is_empty() {
                    i += 1;
                    continue;
                }
                for ch in rest.chars() {
                    match ch {
                        'e' => self.errexit = enable,
                        'u' => self.nounset = enable,
                        'o' if i + 1 < args.len() => {
                            i += 1;
                            match args[i] {
                                "errexit" => self.errexit = enable,
                                "nounset" => self.nounset = enable,
                                "pipefail" => self.pipefail = enable,
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
            i += 1;
        }
    }

    fn emit_builtin_output(
        &mut self,
        text: &str,
        redirects: &[RedirectSpec<'_>],
    ) -> RuntimeResult<()> {
        let mut fd1_mode: Option<RedirectMode<'_>> = None;
        for spec in redirects {
            if spec.fd == 1 {
                fd1_mode = Some(spec.mode);
            }
        }
        match fd1_mode {
            Some(RedirectMode::OutputTruncate(path)) => {
                fs::write(path, text.as_bytes()).map_err(|source| RuntimeError::Io {
                    program: path.to_owned(),
                    source,
                })?;
            }
            Some(RedirectMode::OutputAppend(path)) => {
                let mut f = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|source| RuntimeError::Io {
                        program: path.to_owned(),
                        source,
                    })?;
                f.write_all(text.as_bytes())
                    .map_err(|source| RuntimeError::Io {
                        program: path.to_owned(),
                        source,
                    })?;
            }
            Some(RedirectMode::DupStdoutToStderr) => {
                eprint!("{text}");
                let _ = io::stderr().flush();
            }
            _ => {
                if let Some(top) = self.stdout_capture_stack.last_mut() {
                    top.extend_from_slice(text.as_bytes());
                } else if !text.is_empty() {
                    print!("{text}");
                    let _ = io::stdout().flush();
                }
            }
        }
        Ok(())
    }

    fn emit_builtin_stderr(&mut self, text: &str, redirects: &[RedirectSpec<'_>]) {
        let mut fd2_mode: Option<RedirectMode<'_>> = None;
        for spec in redirects {
            if spec.fd == 2 {
                fd2_mode = Some(spec.mode);
            }
        }
        match fd2_mode {
            Some(RedirectMode::OutputTruncate(path)) => {
                let _ = fs::write(path, text.as_bytes());
            }
            Some(RedirectMode::OutputAppend(path)) => {
                if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
                    let _ = f.write_all(text.as_bytes());
                }
            }
            Some(RedirectMode::DupOutput { target_fd: 1 }) => {
                // Route stderr to fd 1's destination (e.g. `&>/dev/null`).
                let mut fd1_mode: Option<RedirectMode<'_>> = None;
                for spec in redirects {
                    if spec.fd == 1 {
                        fd1_mode = Some(spec.mode);
                    }
                }
                match fd1_mode {
                    Some(RedirectMode::OutputTruncate(path)) => {
                        let _ = fs::write(path, text.as_bytes());
                    }
                    Some(RedirectMode::OutputAppend(path)) => {
                        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
                            let _ = f.write_all(text.as_bytes());
                        }
                    }
                    _ => {
                        if let Some(top) = self.stdout_capture_stack.last_mut() {
                            top.extend_from_slice(text.as_bytes());
                        } else if !text.is_empty() {
                            print!("{text}");
                            let _ = io::stdout().flush();
                        }
                    }
                }
            }
            _ => {
                eprint!("{text}");
                let _ = io::stderr().flush();
            }
        }
    }

    fn try_run_builtin(
        &mut self,
        program: &str,
        args: &[&str],
        env: &[(&str, &str)],
        redirects: &[RedirectSpec<'_>],
    ) -> RuntimeResult<Option<i32>> {
        match program {
            ":" | "true" => {
                self.emit_builtin_output("", redirects)?;
                Ok(Some(0))
            }
            "false" => {
                self.emit_builtin_output("", redirects)?;
                Ok(Some(1))
            }
            "umask" => Ok(Some(self.builtin_umask(args, redirects)?)),
            "cd" => Ok(Some(self.builtin_cd(args, redirects)?)),
            "pwd" => {
                let cwd = std::env::current_dir().map_err(|source| RuntimeError::Io {
                    program: "pwd".to_owned(),
                    source,
                })?;
                self.emit_builtin_output(&format!("{}\n", cwd.display()), redirects)?;
                Ok(Some(0))
            }
            "command" => self.builtin_command(args, env, redirects),
            "builtin" => {
                if args.is_empty() {
                    return Ok(Some(0));
                }
                let inner = args[0];
                if let Some(rc) = self.try_run_builtin(inner, &args[1..], env, redirects)? {
                    Ok(Some(rc))
                } else {
                    eprintln!("bash: builtin: {inner}: not a shell builtin");
                    Ok(Some(1))
                }
            }
            "type" => Ok(Some(self.builtin_type(args, redirects)?)),
            "let" => Ok(Some(self.builtin_let(args))),
            "declare" | "typeset" | "export" | "readonly" | "local" => {
                self.builtin_declare(program, args);
                self.emit_builtin_output("", redirects)?;
                Ok(Some(0))
            }
            "caller" => Ok(Some(1)),
            "exit" => {
                let code = args
                    .first()
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(self.last_status);
                if self.subshell_depth > 0 {
                    return Err(RuntimeError::SubshellExit(code));
                }
                self.run_exit_trap();
                std::process::exit(code);
            }
            "trap" => {
                if args.is_empty() {
                    return Ok(Some(0));
                }
                let handler = args[0];
                let clear = handler == "-";
                for &sig in &args[1..] {
                    match sig {
                        "EXIT" | "0" => {
                            self.exit_trap = if clear {
                                None
                            } else {
                                Some(handler.to_owned())
                            };
                        }
                        "INT" | "SIGINT" | "2" => {
                            self.int_trap = if clear {
                                None
                            } else {
                                Some(handler.to_owned())
                            };
                            #[cfg(unix)]
                            unsafe {
                                let func = if clear {
                                    libc::SIG_DFL
                                } else {
                                    handle_sigint as *const () as usize
                                };
                                libc::signal(libc::SIGINT, func);
                            }
                        }
                        "TERM" | "SIGTERM" | "15" => {
                            self.term_trap = if clear {
                                None
                            } else {
                                Some(handler.to_owned())
                            };
                            #[cfg(unix)]
                            unsafe {
                                let func = if clear {
                                    libc::SIG_DFL
                                } else {
                                    handle_sigterm as *const () as usize
                                };
                                libc::signal(libc::SIGTERM, func);
                            }
                        }
                        "ERR" => {
                            self.err_trap = if clear {
                                None
                            } else {
                                Some(handler.to_owned())
                            };
                        }
                        _ => {}
                    }
                }
                Ok(Some(0))
            }
            "read" => Ok(Some(self.builtin_read(args, redirects)?)),
            "echo" => Ok(Some(self.builtin_echo(args, redirects)?)),
            "unset" => {
                let mut unset_nameref = false;
                for &arg in args {
                    if arg == "-n" {
                        unset_nameref = true;
                        continue;
                    }
                    if !arg.starts_with('-') {
                        if unset_nameref {
                            self.namerefs.remove(arg);
                        } else {
                            self.unset_var(arg);
                        }
                    }
                }
                self.emit_builtin_output("", redirects)?;
                Ok(Some(0))
            }
            "hash" | "ulimit" | "shopt" | "enable" | "wait" | "jobs" | "times" | "dirs"
            | "shift" | "break" | "continue" | "exec" | "mapfile" | "readarray" | "pushd"
            | "popd" | "getopts" | "source" | "." | "alias" | "unalias" | "bg" | "fg"
            | "disown" => {
                self.emit_builtin_output("", redirects)?;
                Ok(Some(0))
            }
            _ => Ok(None),
        }
    }

    fn builtin_umask(
        &mut self,
        args: &[&str],
        redirects: &[RedirectSpec<'_>],
    ) -> RuntimeResult<i32> {
        let mut print_reusable = false;
        let mut symbolic = false;
        let mut mode_arg: Option<&str> = None;
        let mut end_of_opts = false;

        for &arg in args {
            if !end_of_opts && arg == "--" {
                end_of_opts = true;
                continue;
            }
            if !end_of_opts && arg.starts_with('-') && arg.len() > 1 {
                for ch in arg[1..].chars() {
                    match ch {
                        'p' => print_reusable = true,
                        'S' => symbolic = true,
                        _ => {
                            self.emit_builtin_stderr(
                                &format!("bash: umask: -{ch}: invalid option\n"),
                                redirects,
                            );
                            return Ok(2);
                        }
                    }
                }
            } else if mode_arg.is_none() {
                mode_arg = Some(arg);
            }
        }

        if let Some(spec) = mode_arg {
            if spec.chars().all(|c| c.is_ascii_digit()) && !spec.is_empty() {
                if !spec.chars().all(|c| ('0'..='7').contains(&c)) {
                    self.emit_builtin_stderr(
                        &format!("bash: umask: {spec}: octal number out of range\n"),
                        redirects,
                    );
                    return Ok(1);
                }
                match u32::from_str_radix(spec, 8) {
                    Ok(val) => {
                        set_current_umask(val & 0o777);
                        self.emit_builtin_output("", redirects)?;
                        return Ok(0);
                    }
                    Err(_) => {
                        self.emit_builtin_stderr(
                            &format!("bash: umask: {spec}: octal number out of range\n"),
                            redirects,
                        );
                        return Ok(1);
                    }
                }
            }
            let cur = current_umask();
            if let Some(new_mask) = apply_symbolic_umask(cur, spec) {
                set_current_umask(new_mask);
                self.emit_builtin_output("", redirects)?;
                return Ok(0);
            }
            self.emit_builtin_stderr(
                &format!("bash: umask: `{spec}': invalid symbolic mode\n"),
                redirects,
            );
            return Ok(1);
        }

        let mask = current_umask();
        let line = if symbolic {
            let sym = format_symbolic_umask(mask);
            if print_reusable {
                format!("umask -S {sym}\n")
            } else {
                format!("{sym}\n")
            }
        } else if print_reusable {
            format!("umask {mask:04o}\n")
        } else {
            format!("{mask:04o}\n")
        };
        self.emit_builtin_output(&line, redirects)?;
        Ok(0)
    }

    fn builtin_cd(&mut self, args: &[&str], redirects: &[RedirectSpec<'_>]) -> RuntimeResult<i32> {
        let mut target: Option<&str> = None;
        let mut end_of_opts = false;
        for &arg in args {
            if !end_of_opts && arg == "--" {
                end_of_opts = true;
                continue;
            }
            if !end_of_opts && (arg == "-L" || arg == "-P") {
                continue;
            }
            target = Some(arg);
            break;
        }
        let print_dest = target == Some("-");
        let dest = match target {
            None => std::env::var("HOME").unwrap_or_else(|_| "/".to_owned()),
            Some("-") => match std::env::var("OLDPWD") {
                Ok(p) if !p.is_empty() => p,
                _ => {
                    eprintln!("bash: cd: OLDPWD not set");
                    return Ok(1);
                }
            },
            Some(p) => p.to_owned(),
        };
        let old_pwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        if let Err(err) = std::env::set_current_dir(&dest) {
            eprintln!("bash: cd: {dest}: {err}");
            return Ok(1);
        }
        let new_pwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or(dest);
        unsafe {
            std::env::set_var("OLDPWD", &old_pwd);
            std::env::set_var("PWD", &new_pwd);
        }
        self.vars.insert("OLDPWD".to_owned(), old_pwd);
        self.vars.insert("PWD".to_owned(), new_pwd.clone());
        if print_dest {
            self.emit_builtin_output(&format!("{new_pwd}\n"), redirects)?;
        } else {
            self.emit_builtin_output("", redirects)?;
        }
        Ok(0)
    }

    fn builtin_command(
        &mut self,
        args: &[&str],
        env: &[(&str, &str)],
        redirects: &[RedirectSpec<'_>],
    ) -> RuntimeResult<Option<i32>> {
        let mut mode_v = false;
        let mut mode_cap_v = false;
        let mut use_default_path = false;
        let mut idx = 0usize;
        while idx < args.len() {
            let arg = args[idx];
            if arg == "--" {
                idx += 1;
                break;
            }
            if !arg.starts_with('-') || arg.len() == 1 {
                break;
            }
            for ch in arg[1..].chars() {
                match ch {
                    'v' => {
                        mode_v = true;
                        mode_cap_v = false;
                    }
                    'V' => {
                        mode_cap_v = true;
                        mode_v = false;
                    }
                    'p' => use_default_path = true,
                    _ => {
                        self.emit_builtin_stderr(
                            &format!("bash: command: -{ch}: invalid option\n"),
                            redirects,
                        );
                        return Ok(Some(2));
                    }
                }
            }
            idx += 1;
        }
        let rest = &args[idx..];
        let shell_path = self.vars.get("PATH").cloned();
        if mode_v || mode_cap_v {
            let mut all_found = true;
            let mut out = String::new();
            for &name in rest {
                if is_shell_builtin(name) {
                    if mode_cap_v {
                        out.push_str(&format!("{name} is a shell builtin\n"));
                    } else {
                        out.push_str(&format!("{name}\n"));
                    }
                } else if let Some(path) =
                    find_in_path_with(name, shell_path.as_deref(), use_default_path)
                {
                    if mode_cap_v {
                        out.push_str(&format!("{name} is {path}\n"));
                    } else {
                        out.push_str(&format!("{path}\n"));
                    }
                } else {
                    if mode_cap_v {
                        self.emit_builtin_stderr(
                            &format!("bash: command: {name}: not found\n"),
                            redirects,
                        );
                    }
                    all_found = false;
                }
            }
            self.emit_builtin_output(&out, redirects)?;
            return Ok(Some(if all_found { 0 } else { 1 }));
        }
        if rest.is_empty() {
            return Ok(Some(0));
        }
        let prog = rest[0];
        let prog_args = &rest[1..];
        if let Some(rc) = self.try_run_builtin(prog, prog_args, env, redirects)? {
            return Ok(Some(rc));
        }
        let status = if redirects.is_empty() {
            run_command_env(prog, prog_args, env)?
        } else {
            run_command_with_redirects(prog, prog_args, env, redirects)?
        };
        Ok(Some(Self::exit_code_from_status(&status)))
    }

    fn builtin_type(
        &mut self,
        args: &[&str],
        redirects: &[RedirectSpec<'_>],
    ) -> RuntimeResult<i32> {
        let mut type_only = false;
        let mut path_only = false;
        let mut force_path = false;
        let mut idx = 0usize;
        while idx < args.len() {
            let arg = args[idx];
            if arg == "--" {
                idx += 1;
                break;
            }
            if arg.starts_with('-') && arg.len() > 1 {
                for ch in arg[1..].chars() {
                    match ch {
                        't' => type_only = true,
                        'p' => path_only = true,
                        'P' => {
                            path_only = true;
                            force_path = true;
                        }
                        _ => {}
                    }
                }
                idx += 1;
            } else {
                break;
            }
        }
        let shell_path = self.vars.get("PATH").cloned();
        let mut all_found = true;
        let mut out = String::new();
        for &name in &args[idx..] {
            if !force_path && is_shell_builtin(name) {
                if path_only {
                    // `type -p <builtin>` prints nothing and exits 0 in Bash.
                } else if type_only {
                    out.push_str("builtin\n");
                } else {
                    out.push_str(&format!("{name} is a shell builtin\n"));
                }
            } else if let Some(path) = find_in_path_with(name, shell_path.as_deref(), false) {
                if path_only {
                    out.push_str(&format!("{path}\n"));
                } else if type_only {
                    out.push_str("file\n");
                } else {
                    out.push_str(&format!("{name} is {path}\n"));
                }
            } else {
                if !type_only && !path_only {
                    self.emit_builtin_stderr(
                        &format!("bash: type: {name}: not found\n"),
                        redirects,
                    );
                }
                all_found = false;
            }
        }
        self.emit_builtin_output(&out, redirects)?;
        Ok(if all_found { 0 } else { 1 })
    }

    fn builtin_let(&mut self, args: &[&str]) -> i32 {
        if args.is_empty() {
            return 1;
        }
        let mut last_val = 0i64;
        for &expr in args {
            let trimmed = expr.trim();
            if let Some((lhs, rhs)) = trimmed.split_once('=') {
                let lhs_trimmed = lhs.trim();
                let rhs_val = rhs.trim().parse::<i64>().unwrap_or(0);
                let new_val = if let Some(var) = lhs_trimmed.strip_suffix('+') {
                    let cur = self
                        .get_var(var.trim())
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(0);
                    cur + rhs_val
                } else if let Some(var) = lhs_trimmed.strip_suffix('-') {
                    let cur = self
                        .get_var(var.trim())
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(0);
                    cur - rhs_val
                } else {
                    rhs_val
                };
                let var_name = lhs_trimmed.trim_end_matches(['+', '-']).trim();
                self.set_var(var_name, new_val.to_string());
                last_val = new_val;
            } else {
                last_val = trimmed.parse::<i64>().unwrap_or(0);
            }
        }
        if last_val == 0 {
            1
        } else {
            0
        }
    }

    fn builtin_declare(&mut self, cmd: &str, args: &[&str]) {
        let mut export_var = cmd == "export";
        let mut is_array = false;
        let mut is_nameref = false;
        for &arg in args {
            if (arg.starts_with('-') || arg.starts_with('+')) && arg.len() > 1 {
                if arg.contains('x') && arg.starts_with('-') {
                    export_var = true;
                }
                if arg.contains('a') && arg.starts_with('-') {
                    is_array = true;
                }
                if arg.contains('n') && arg.starts_with('-') {
                    is_nameref = true;
                }
                if arg.contains('n') && arg.starts_with('+') {
                    is_nameref = false;
                }
                continue;
            }
            if is_nameref {
                if let Some((ref_name, target)) = arg.split_once('=') {
                    if !target.is_empty() {
                        self.namerefs.insert(ref_name.to_owned(), target.to_owned());
                    }
                }
                continue;
            }
            if let Some((raw_name, val)) = arg.split_once('=') {
                let append = raw_name.ends_with('+');
                let name = raw_name.strip_suffix('+').unwrap_or(raw_name);
                let trimmed = val.trim();
                if (is_array || trimmed.starts_with('('))
                    && trimmed.starts_with('(')
                    && trimmed.ends_with(')')
                {
                    let inner = trimmed[1..trimmed.len() - 1].trim();
                    let items: Vec<String> = if inner.is_empty() {
                        Vec::new()
                    } else {
                        split_words(inner, None)
                    };
                    if append {
                        self.append_array(name, items);
                    } else {
                        self.set_array(name, items);
                    }
                } else if append {
                    self.append_var(name, val);
                } else {
                    self.set_var(name, val);
                    if export_var {
                        let target = self.resolve_nameref(name).to_owned();
                        unsafe {
                            std::env::set_var(target, val);
                        }
                    }
                }
            } else if is_array {
                self.arrays.entry(arg.to_owned()).or_default();
            } else if export_var {
                if let Some(val) = self.vars.get(arg).cloned() {
                    unsafe {
                        std::env::set_var(arg, val);
                    }
                }
            }
        }
    }

    fn builtin_echo(
        &mut self,
        args: &[&str],
        redirects: &[RedirectSpec<'_>],
    ) -> RuntimeResult<i32> {
        let mut no_newline = false;
        let mut interpret_escapes = false;
        let mut idx = 0usize;
        while idx < args.len() {
            let arg = args[idx];
            if arg.starts_with('-')
                && arg.len() > 1
                && arg[1..].chars().all(|c| matches!(c, 'n' | 'e' | 'E'))
            {
                for ch in arg[1..].chars() {
                    match ch {
                        'n' => no_newline = true,
                        'e' => interpret_escapes = true,
                        'E' => interpret_escapes = false,
                        _ => {}
                    }
                }
                idx += 1;
            } else {
                break;
            }
        }
        let joined = args[idx..].join(" ");
        let mut out = if interpret_escapes {
            let mut res = String::with_capacity(joined.len());
            let mut chars = joined.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    match chars.next() {
                        Some('n') => res.push('\n'),
                        Some('t') => res.push('\t'),
                        Some('r') => res.push('\r'),
                        Some('\\') => res.push('\\'),
                        Some('a') => res.push('\x07'),
                        Some('b') => res.push('\x08'),
                        Some('f') => res.push('\x0c'),
                        Some('v') => res.push('\x0b'),
                        Some('c') => {
                            no_newline = true;
                            break;
                        }
                        Some(other) => {
                            res.push('\\');
                            res.push(other);
                        }
                        None => res.push('\\'),
                    }
                } else {
                    res.push(c);
                }
            }
            res
        } else {
            joined
        };
        if !no_newline {
            out.push('\n');
        }
        self.emit_builtin_output(&out, redirects)?;
        Ok(0)
    }

    fn builtin_read(
        &mut self,
        args: &[&str],
        redirects: &[RedirectSpec<'_>],
    ) -> RuntimeResult<i32> {
        let mut raw_mode = false;
        let mut prompt: Option<&str> = None;
        let mut nchars: Option<usize> = None;
        let mut delim: u8 = b'\n';
        let mut array_var: Option<&str> = None;
        let mut var_names: Vec<&str> = Vec::new();
        let mut idx = 0usize;
        let mut end_of_opts = false;

        while idx < args.len() {
            let arg = args[idx];
            if !end_of_opts && arg == "--" {
                end_of_opts = true;
                idx += 1;
                continue;
            }
            if !end_of_opts && arg.starts_with('-') && arg.len() > 1 {
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut c_idx = 0usize;
                while c_idx < chars.len() {
                    match chars[c_idx] {
                        'r' | 's' => {}
                        'p' => {
                            if c_idx + 1 == chars.len() && idx + 1 < args.len() {
                                idx += 1;
                                prompt = Some(args[idx]);
                            }
                        }
                        'n' | 'N' => {
                            if c_idx + 1 == chars.len() && idx + 1 < args.len() {
                                idx += 1;
                                nchars = args[idx].parse::<usize>().ok();
                            }
                        }
                        'd' => {
                            if c_idx + 1 == chars.len() && idx + 1 < args.len() {
                                idx += 1;
                                delim = args[idx].as_bytes().first().copied().unwrap_or(0);
                            }
                        }
                        'a' => {
                            if c_idx + 1 == chars.len() && idx + 1 < args.len() {
                                idx += 1;
                                array_var = Some(args[idx]);
                            }
                        }
                        't' | 'u' if c_idx + 1 == chars.len() && idx + 1 < args.len() => {
                            idx += 1;
                        }
                        _ => {}
                    }
                    if chars[c_idx] == 'r' {
                        raw_mode = true;
                    }
                    c_idx += 1;
                }
            } else {
                var_names.push(arg);
            }
            idx += 1;
        }

        if let Some(p) = prompt {
            self.emit_builtin_stderr(p, redirects);
        }

        // Read from input redirect `< file` / `<<<` / `<<`, else `stdin_override`, else `io::stdin()`.
        let mut input_file_buf: Option<Vec<u8>> = None;
        for spec in redirects {
            if spec.fd == 0 {
                match spec.mode {
                    RedirectMode::Input(path) => {
                        input_file_buf = fs::read(path).ok();
                    }
                    RedirectMode::InputBytes(bytes) => {
                        input_file_buf = Some(bytes.to_vec());
                    }
                    _ => {}
                }
            }
        }

        let mut line_bytes = Vec::new();
        let got_eof: bool;

        if let Some(file_bytes) = input_file_buf {
            if file_bytes.is_empty() {
                got_eof = true;
            } else {
                let end = file_bytes
                    .iter()
                    .position(|&b| b == delim)
                    .unwrap_or(file_bytes.len());
                let take_len = nchars.map_or(end, |n| end.min(n));
                line_bytes.extend_from_slice(&file_bytes[..take_len]);
                got_eof = false;
            }
        } else if let Some(mut buf) = self.stdin_override.take() {
            if buf.is_empty() {
                got_eof = true;
            } else if let Some(n) = nchars {
                let take = n.min(buf.len());
                line_bytes.extend_from_slice(&buf[..take]);
                let rem = buf.split_off(take);
                self.stdin_override = Some(rem);
                got_eof = false;
            } else if let Some(pos) = buf.iter().position(|&b| b == delim) {
                line_bytes.extend_from_slice(&buf[..pos]);
                let rem = buf.split_off(pos + 1);
                self.stdin_override = Some(rem);
                got_eof = false;
            } else {
                line_bytes = buf;
                self.stdin_override = Some(Vec::new());
                got_eof = true;
            }
        } else {
            use std::io::Read;
            let stdin = io::stdin();
            let mut handle = stdin.lock();
            let mut byte = [0u8; 1];
            let mut read_any = false;
            let mut hit_delim = false;
            loop {
                if let Some(n) = nchars {
                    if line_bytes.len() >= n {
                        break;
                    }
                }
                match handle.read(&mut byte) {
                    Ok(0) => break,
                    Ok(_) => {
                        read_any = true;
                        if nchars.is_none() && byte[0] == delim {
                            hit_delim = true;
                            break;
                        }
                        line_bytes.push(byte[0]);
                    }
                    Err(_) => break,
                }
            }
            got_eof = !read_any || (!hit_delim && nchars.is_none() && line_bytes.is_empty());
            if read_any && !hit_delim && nchars.is_none() && !line_bytes.is_empty() {
                // Partial last line without trailing newline: Bash assigns vars and returns 1.
                let line = String::from_utf8_lossy(&line_bytes).into_owned();
                self.assign_read_vars(&line, raw_mode, array_var, &var_names);
                return Ok(1);
            }
        }

        let line = String::from_utf8_lossy(&line_bytes)
            .trim_end_matches('\r')
            .to_owned();
        self.assign_read_vars(&line, raw_mode, array_var, &var_names);
        Ok(if got_eof { 1 } else { 0 })
    }

    fn assign_read_vars(
        &mut self,
        line: &str,
        raw_mode: bool,
        array_var: Option<&str>,
        var_names: &[&str],
    ) {
        let processed = if raw_mode {
            line.to_owned()
        } else {
            let mut out = String::with_capacity(line.len());
            let mut chars = line.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                } else {
                    out.push(c);
                }
            }
            out
        };

        if let Some(arr) = array_var {
            let fields = split_words(&processed, self.ifs.as_deref());
            self.set_array(arr, fields);
            return;
        }

        if var_names.is_empty() {
            self.set_var("REPLY", processed);
            return;
        }

        if var_names.len() == 1 {
            let ifs = self.ifs.clone().unwrap_or_else(|| " \t\n".to_owned());
            let trimmed = processed.trim_matches(|c| ifs.contains(c));
            self.set_var(var_names[0], trimmed);
            return;
        }

        let ifs = self.ifs.clone().unwrap_or_else(|| " \t\n".to_owned());
        let mut rest = processed.trim_start_matches(|c| ifs.contains(c));
        for (i, &name) in var_names.iter().enumerate() {
            if i + 1 == var_names.len() {
                let tail = rest.trim_end_matches(|c| ifs.contains(c));
                self.set_var(name, tail);
            } else if let Some(pos) = rest.find(|c| ifs.contains(c)) {
                self.set_var(name, &rest[..pos]);
                rest = rest[pos..].trim_start_matches(|c| ifs.contains(c));
            } else {
                self.set_var(name, rest);
                rest = "";
            }
        }
    }
}

fn current_umask() -> u32 {
    #[cfg(unix)]
    // SAFETY: `umask` is always safe to call; we immediately restore the prior mask.
    unsafe {
        let m = libc::umask(0);
        libc::umask(m);
        #[allow(clippy::unnecessary_cast)]
        {
            (m as u32) & 0o777
        }
    }
    #[cfg(not(unix))]
    {
        0o022
    }
}

fn set_current_umask(mask: u32) {
    #[cfg(unix)]
    // SAFETY: `umask` is always safe to call with a valid `mode_t` mask.
    unsafe {
        libc::umask((mask & 0o777) as libc::mode_t);
    }
}

fn format_symbolic_umask(mask: u32) -> String {
    let perms = !mask & 0o777;
    let triad = |bits: u32| -> String {
        let mut s = String::new();
        if bits & 4 != 0 {
            s.push('r');
        }
        if bits & 2 != 0 {
            s.push('w');
        }
        if bits & 1 != 0 {
            s.push('x');
        }
        s
    };
    format!(
        "u={},g={},o={}",
        triad((perms >> 6) & 7),
        triad((perms >> 3) & 7),
        triad(perms & 7)
    )
}

fn apply_symbolic_umask(current_mask: u32, spec: &str) -> Option<u32> {
    let mut perms = !current_mask & 0o777;
    for clause in spec.split(',') {
        if clause.is_empty() {
            return None;
        }
        let bytes = clause.as_bytes();
        let mut idx = 0usize;
        let mut who_mask = 0u32;
        while idx < bytes.len() {
            match bytes[idx] {
                b'u' => who_mask |= 0o700,
                b'g' => who_mask |= 0o070,
                b'o' => who_mask |= 0o007,
                b'a' => who_mask |= 0o777,
                _ => break,
            }
            idx += 1;
        }
        if who_mask == 0 {
            who_mask = 0o777;
        }
        if idx >= bytes.len() {
            return None;
        }
        while idx < bytes.len() {
            let op = bytes[idx];
            if !matches!(op, b'=' | b'+' | b'-') {
                return None;
            }
            idx += 1;
            let mut rwx = 0u32;
            while idx < bytes.len() {
                match bytes[idx] {
                    b'r' => rwx |= 0o444,
                    b'w' => rwx |= 0o222,
                    b'x' => rwx |= 0o111,
                    b'=' | b'+' | b'-' => break,
                    _ => return None,
                }
                idx += 1;
            }
            let bits = rwx & who_mask;
            match op {
                b'=' => perms = (perms & !who_mask) | bits,
                b'+' => perms |= bits,
                b'-' => perms &= !bits,
                _ => unreachable!(),
            }
        }
    }
    Some(!perms & 0o777)
}

/// Returns `true` if `name` is a POSIX/Bash shell builtin recognized by `bash2rust-runtime`.
pub fn is_shell_builtin(name: &str) -> bool {
    matches!(
        name,
        ":" | "true"
            | "false"
            | "umask"
            | "cd"
            | "pwd"
            | "command"
            | "builtin"
            | "type"
            | "let"
            | "declare"
            | "typeset"
            | "export"
            | "readonly"
            | "local"
            | "shift"
            | "break"
            | "continue"
            | "return"
            | "exit"
            | "set"
            | "unset"
            | "echo"
            | "printf"
            | "read"
            | "test"
            | "["
            | "eval"
            | "exec"
            | "source"
            | "."
            | "trap"
            | "hash"
            | "ulimit"
            | "shopt"
            | "caller"
            | "enable"
            | "wait"
            | "jobs"
            | "times"
            | "dirs"
            | "mapfile"
            | "readarray"
            | "pushd"
            | "popd"
            | "getopts"
            | "alias"
            | "unalias"
            | "bg"
            | "fg"
            | "disown"
    )
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    if let Some(s) = path.to_str() {
        if let Ok(c_path) = CString::new(s) {
            // SAFETY: `c_path` is NUL-terminated.
            return unsafe { libc::access(c_path.as_ptr(), libc::X_OK) } == 0;
        }
    }
    true
}

fn find_in_path_with(
    name: &str,
    shell_path: Option<&str>,
    use_default_path: bool,
) -> Option<String> {
    const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
    if name.contains('/') {
        let p = Path::new(name);
        return if is_executable_file(p) {
            Some(name.to_owned())
        } else {
            None
        };
    }
    let env_path;
    let search_path = if use_default_path {
        DEFAULT_PATH
    } else if let Some(p) = shell_path.filter(|s| !s.is_empty()) {
        p
    } else {
        env_path = std::env::var("PATH").unwrap_or_default();
        if env_path.is_empty() {
            DEFAULT_PATH
        } else {
            &env_path
        }
    };
    for dir in std::env::split_paths(search_path) {
        let candidate = dir.join(name);
        if is_executable_file(&candidate) {
            return Some(candidate.display().to_string());
        }
    }
    None
}

/// Splits an unquoted expansion value according to POSIX 2.6.5 field-splitting rules.
///
/// - `ifs = None` uses the default `" \t\n"`.
/// - `ifs = Some("")` performs no splitting (empty input yields `vec![]`, non-empty yields `vec![value.to_owned()]`).
/// - IFS whitespace (`' '`, `'\t'`, `'\n'` present in `ifs`) trims leading/trailing runs and collapses consecutive delimiters.
/// - Non-whitespace IFS characters preserve empty fields between consecutive delimiters.
pub fn split_words(value: &str, ifs: Option<&str>) -> Vec<String> {
    let ifs = ifs.unwrap_or(" \t\n");
    if ifs.is_empty() {
        return if value.is_empty() {
            Vec::new()
        } else {
            vec![value.to_owned()]
        };
    }

    let is_ifs_ws = |c: char| matches!(c, ' ' | '\t' | '\n') && ifs.contains(c);
    let is_ifs_non_ws = |c: char| !matches!(c, ' ' | '\t' | '\n') && ifs.contains(c);

    let mut result = Vec::new();
    let mut rest = value.trim_start_matches(is_ifs_ws);

    while !rest.is_empty() {
        let end = rest
            .find(|c| is_ifs_ws(c) || is_ifs_non_ws(c))
            .unwrap_or(rest.len());
        result.push(rest[..end].to_owned());
        rest = &rest[end..];
        rest = rest.trim_start_matches(is_ifs_ws);
        if let Some(first) = rest.chars().next() {
            if is_ifs_non_ws(first) {
                rest = &rest[first.len_utf8()..];
                rest = rest.trim_start_matches(is_ifs_ws);
            }
        }
    }

    result
}

/// Expands a pathname glob pattern (`*`, `?`, `[...]`) following POSIX filename expansion rules.
///
/// - Preserves directory prefixes (e.g. `./*.txt` -> `./a1.txt`, `sub/*.txt` -> `sub/a.txt`).
/// - Hides dotfiles unless the filename pattern explicitly begins with `.`.
/// - Sorts matches lexicographically (`LC_ALL=C` / UTF-8 byte order).
/// - Returns `vec![pattern.to_owned()]` if no metacharacters are present or no files match.
pub fn expand_glob(pattern: &str) -> Vec<String> {
    let (dir_str, prefix, file_pat) = match pattern.rfind('/') {
        Some(pos) => {
            let dir = if pos == 0 { "/" } else { &pattern[..pos] };
            (dir, &pattern[..=pos], &pattern[pos + 1..])
        }
        None => (".", "", pattern),
    };

    if !file_pat.contains(['*', '?', '[']) {
        return vec![pattern.to_owned()];
    }

    let Ok(entries) = std::fs::read_dir(dir_str) else {
        return vec![pattern.to_owned()];
    };

    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let name_os = entry.file_name();
        let Some(name) = name_os.to_str() else {
            continue;
        };
        if glob_match(file_pat, name) {
            matches.push(format!("{prefix}{name}"));
        }
    }

    if matches.is_empty() {
        return vec![pattern.to_owned()];
    }

    matches.sort();
    matches
}

/// Strips shortest/longest prefix or suffix matching glob `pat` from `val` (`#`, `##`, `%`, `%%`).
pub fn param_trim(val: &str, pat: &str, is_suffix: bool, longest: bool) -> String {
    if pat.is_empty() {
        return val.to_owned();
    }
    let mut boundaries: Vec<usize> = val.char_indices().map(|(i, _)| i).collect();
    boundaries.push(val.len());
    if !is_suffix {
        if longest {
            for &b in boundaries.iter().rev() {
                if pattern_match(pat, &val[..b]) {
                    return val[b..].to_owned();
                }
            }
        } else {
            for &b in &boundaries {
                if pattern_match(pat, &val[..b]) {
                    return val[b..].to_owned();
                }
            }
        }
    } else if longest {
        for &b in &boundaries {
            if pattern_match(pat, &val[b..]) {
                return val[..b].to_owned();
            }
        }
    } else {
        for &b in boundaries.iter().rev() {
            if pattern_match(pat, &val[b..]) {
                return val[..b].to_owned();
            }
        }
    }
    val.to_owned()
}

/// Replaces first or all occurrences of glob `pat` in `val` with `repl` (`/` and `//`).
pub fn param_replace(val: &str, pat: &str, repl: &str, replace_all: bool) -> String {
    if pat.is_empty() {
        return val.to_owned();
    }
    if !pat.contains(['*', '?', '[']) {
        return if replace_all {
            val.replace(pat, repl)
        } else {
            val.replacen(pat, repl, 1)
        };
    }
    let mut out = String::with_capacity(val.len());
    let mut pos = 0usize;
    while pos <= val.len() {
        let mut matched_end: Option<usize> = None;
        for end in (pos + 1..=val.len()).rev() {
            if val.is_char_boundary(end) && pattern_match(pat, &val[pos..end]) {
                matched_end = Some(end);
                break;
            }
        }
        if let Some(end) = matched_end {
            out.push_str(repl);
            pos = end;
            if !replace_all {
                out.push_str(&val[pos..]);
                break;
            }
        } else if pos < val.len() {
            let ch = val[pos..].chars().next().expect("valid char");
            out.push(ch);
            pos += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

/// Extracts a character substring `${var:offset[:len]}`.
pub fn param_substr(val: &str, offset: i64, len: Option<i64>) -> String {
    let chars: Vec<char> = val.chars().collect();
    let total = chars.len() as i64;
    let start = if offset < 0 {
        (total + offset).max(0) as usize
    } else {
        (offset as usize).min(chars.len())
    };
    match len {
        Some(l) if l < 0 => {
            let end = (total + l).max(0) as usize;
            if end <= start {
                String::new()
            } else {
                chars[start..end].iter().collect()
            }
        }
        Some(l) => {
            let end = start.saturating_add(l as usize).min(chars.len());
            chars[start..end].iter().collect()
        }
        None => chars[start..].iter().collect(),
    }
}

/// Matches `text` against a Bash `case` glob pattern (`*`, `?`, `[...]`) without dotfile hiding.
pub fn pattern_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_match_chars(&pat, &txt)
}

fn glob_match(pattern: &str, name: &str) -> bool {
    if name.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = name.chars().collect();
    glob_match_chars(&pat, &txt)
}

fn glob_match_chars(pat: &[char], txt: &[char]) -> bool {
    let mut pi = 0usize;
    let mut ti = 0usize;
    let mut star_pi: Option<usize> = None;
    let mut star_ti: usize = 0;

    while ti < txt.len() {
        if pi < pat.len() && pat[pi] == '*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if pi < pat.len() && pat[pi] == '?' {
            pi += 1;
            ti += 1;
        } else if pi < pat.len() && pat[pi] == '[' {
            if let Some((matched, next_pi)) = match_char_class(&pat[pi + 1..], txt[ti]) {
                if matched {
                    pi += 1 + next_pi;
                    ti += 1;
                    continue;
                }
            } else if pat[pi] == txt[ti] {
                pi += 1;
                ti += 1;
                continue;
            }
            if let Some(sp) = star_pi {
                pi = sp + 1;
                star_ti += 1;
                ti = star_ti;
            } else {
                return false;
            }
        } else if pi < pat.len() && pat[pi] == txt[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }

    pi == pat.len()
}

fn match_char_class(rest: &[char], ch: char) -> Option<(bool, usize)> {
    if rest.is_empty() {
        return None;
    }
    let mut i = 0usize;
    let negated = if rest[i] == '!' || rest[i] == '^' {
        i += 1;
        true
    } else {
        false
    };

    let mut close = None;
    let mut j = i;
    if j < rest.len() && rest[j] == ']' {
        j += 1;
    }
    while j < rest.len() {
        if rest[j] == '[' && j + 1 < rest.len() && rest[j + 1] == ':' {
            if let Some(rel_end) = rest[j + 2..].windows(2).position(|w| w == [':', ']']) {
                j += 2 + rel_end + 2;
                continue;
            }
        }
        if rest[j] == ']' {
            close = Some(j);
            break;
        }
        j += 1;
    }
    let close_idx = close?;

    let mut matched = false;
    while i < close_idx {
        if rest[i] == '[' && i + 1 < close_idx && rest[i + 1] == ':' {
            if let Some(rel_end) = rest[i + 2..close_idx]
                .windows(2)
                .position(|w| w == [':', ']'])
            {
                let cls: String = rest[i + 2..i + 2 + rel_end].iter().collect();
                let hit = match cls.as_str() {
                    "space" => ch.is_whitespace(),
                    "blank" => ch == ' ' || ch == '\t',
                    "alpha" => ch.is_ascii_alphabetic(),
                    "digit" => ch.is_ascii_digit(),
                    "alnum" => ch.is_ascii_alphanumeric(),
                    "upper" => ch.is_ascii_uppercase(),
                    "lower" => ch.is_ascii_lowercase(),
                    "punct" => ch.is_ascii_punctuation(),
                    "print" => ch.is_ascii_graphic() || ch == ' ',
                    _ => false,
                };
                if hit {
                    matched = true;
                }
                i += 2 + rel_end + 2;
                continue;
            }
        }
        if i + 2 < close_idx && rest[i + 1] == '-' {
            let start = rest[i];
            let end = rest[i + 2];
            if start <= ch && ch <= end {
                matched = true;
            }
            i += 3;
        } else {
            if rest[i] == ch {
                matched = true;
            }
            i += 1;
        }
    }

    Some((matched ^ negated, close_idx + 1))
}

/// I/O redirection target mode for [`RedirectSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectMode<'a> {
    OutputTruncate(&'a str),
    OutputAppend(&'a str),
    Input(&'a str),
    InputBytes(&'a [u8]),
    DupOutput { target_fd: u32 },
    DupStdoutToStderr,
    CloseFd,
}

fn open_output_truncate(path: &str) -> RuntimeResult<File> {
    if path == "/dev/tty" {
        OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|source| RuntimeError::Io {
                program: path.to_owned(),
                source,
            })
    } else {
        File::create(path).map_err(|source| RuntimeError::Io {
            program: path.to_owned(),
            source,
        })
    }
}

/// A single file-descriptor redirection applied to a spawned command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedirectSpec<'a> {
    pub fd: u32,
    pub mode: RedirectMode<'a>,
}

/// Executes a command with explicit stdin/stdout/stderr redirections (`>`, `>>`, `<`, `2>&1`).
pub fn run_command_with_redirects(
    program: &str,
    args: &[&str],
    env: &[(&str, &str)],
    redirects: &[RedirectSpec<'_>],
) -> RuntimeResult<ExitStatus> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }

    let mut stdout_file: Option<File> = None;
    let mut stderr_file: Option<File> = None;
    let mut dup_stderr_to_stdout = false;
    let mut dup_stdout_to_stderr = false;
    let mut input_bytes: Option<&[u8]> = None;

    for spec in redirects {
        match (spec.fd, spec.mode) {
            (0, RedirectMode::Input(path)) => {
                let file = File::open(path).map_err(|source| RuntimeError::Io {
                    program: path.to_owned(),
                    source,
                })?;
                cmd.stdin(Stdio::from(file));
                input_bytes = None;
            }
            (0, RedirectMode::InputBytes(bytes)) => {
                input_bytes = Some(bytes);
                cmd.stdin(Stdio::piped());
            }
            (1, RedirectMode::OutputTruncate(path)) => {
                let file = open_output_truncate(path)?;
                let cloned = file.try_clone().map_err(|source| RuntimeError::Io {
                    program: path.to_owned(),
                    source,
                })?;
                stdout_file = Some(cloned);
                cmd.stdout(Stdio::from(file));
            }
            (1, RedirectMode::OutputAppend(path)) => {
                let file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|source| RuntimeError::Io {
                        program: path.to_owned(),
                        source,
                    })?;
                let cloned = file.try_clone().map_err(|source| RuntimeError::Io {
                    program: path.to_owned(),
                    source,
                })?;
                stdout_file = Some(cloned);
                cmd.stdout(Stdio::from(file));
            }
            (2, RedirectMode::OutputTruncate(path)) => {
                let file = open_output_truncate(path)?;
                let cloned = file.try_clone().map_err(|source| RuntimeError::Io {
                    program: path.to_owned(),
                    source,
                })?;
                stderr_file = Some(cloned);
                cmd.stderr(Stdio::from(file));
            }
            (2, RedirectMode::OutputAppend(path)) => {
                let file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|source| RuntimeError::Io {
                        program: path.to_owned(),
                        source,
                    })?;
                let cloned = file.try_clone().map_err(|source| RuntimeError::Io {
                    program: path.to_owned(),
                    source,
                })?;
                stderr_file = Some(cloned);
                cmd.stderr(Stdio::from(file));
            }
            (1, RedirectMode::DupStdoutToStderr | RedirectMode::DupOutput { target_fd: 2 }) => {
                if let Some(ref f) = stderr_file {
                    let cloned = f.try_clone().map_err(|source| RuntimeError::Io {
                        program: program.to_owned(),
                        source,
                    })?;
                    cmd.stdout(Stdio::from(cloned));
                } else {
                    dup_stdout_to_stderr = true;
                }
            }
            (2, RedirectMode::DupOutput { target_fd: 1 }) => {
                if let Some(ref f) = stdout_file {
                    let cloned = f.try_clone().map_err(|source| RuntimeError::Io {
                        program: program.to_owned(),
                        source,
                    })?;
                    cmd.stderr(Stdio::from(cloned));
                } else {
                    dup_stderr_to_stdout = true;
                }
            }
            _ => {}
        }
    }

    if dup_stdout_to_stderr {
        let (mut reader, writer) = io::pipe().map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;
        cmd.stdout(Stdio::from(writer));

        let mut child = cmd.spawn().map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;
        drop(cmd);

        let copy_res = io::copy(&mut reader, &mut io::stderr());
        let status = child.wait().map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;
        copy_res.map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;
        return Ok(status);
    }

    if dup_stderr_to_stdout {
        let (mut reader, writer) = io::pipe().map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;
        let stderr_writer = writer.try_clone().map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;
        cmd.stdout(Stdio::from(writer));
        cmd.stderr(Stdio::from(stderr_writer));

        let mut child = cmd.spawn().map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;
        drop(cmd);

        let copy_res = io::copy(&mut reader, &mut io::stdout());
        let status = child.wait().map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;
        copy_res.map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;
        return Ok(status);
    }

    if let Some(bytes) = input_bytes {
        let mut child = cmd.spawn().map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(bytes);
        }
        return child.wait().map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        });
    }

    cmd.status().map_err(|source| RuntimeError::Io {
        program: program.to_owned(),
        source,
    })
}

/// Executes a command and returns its raw UTF-8 stdout (preserving trailing newlines) and exit code.
pub fn command_subst_raw(program: &str, args: &[&str]) -> RuntimeResult<(String, i32)> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;

    let stdout = String::from_utf8(output.stdout).map_err(|source| RuntimeError::InvalidUtf8 {
        program: program.to_owned(),
        source,
    })?;
    Ok((stdout, ShellState::exit_code_from_status(&output.status)))
}

/// Executes a command and returns its trimmed UTF-8 stdout and exit code (Bash `$(...)`).
pub fn command_subst_with_status(program: &str, args: &[&str]) -> RuntimeResult<(String, i32)> {
    let (stdout, code) = command_subst_raw(program, args)?;
    Ok((trim_trailing_newlines(&stdout).to_owned(), code))
}

/// Executes a command and returns its UTF-8 stdout with trailing newlines stripped (Bash `$(...)`).
pub fn command_subst(program: &str, args: &[&str]) -> RuntimeResult<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;

    if !output.status.success() {
        return Err(RuntimeError::CommandFailed {
            program: program.to_owned(),
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let stdout = String::from_utf8(output.stdout).map_err(|source| RuntimeError::InvalidUtf8 {
        program: program.to_owned(),
        source,
    })?;
    Ok(trim_trailing_newlines(&stdout).to_owned())
}

impl ShellState {
    /// Executes `program` for command substitution `$(...)` with explicit redirections
    /// (including `3>&1 1>/dev/tty 2>&3 3>&-` fd-swapping for `dialog`/`whiptail` menus)
    /// and records the child's exit code in `self.last_status`.
    pub fn command_subst_with_redirects(
        &mut self,
        program: &str,
        args: &[&str],
        redirects: &[RedirectSpec<'_>],
    ) -> RuntimeResult<String> {
        let mut fd3_to_stdout = false;
        let mut fd2_to_fd3 = false;
        let mut fd1_path: Option<(&str, bool)> = None;
        let mut fd0_path: Option<&str> = None;
        let mut fd0_bytes: Option<&[u8]> = None;

        for spec in redirects {
            match (spec.fd, spec.mode) {
                (0, RedirectMode::Input(path)) => {
                    fd0_path = Some(path);
                    fd0_bytes = None;
                }
                (0, RedirectMode::InputBytes(bytes)) => {
                    fd0_bytes = Some(bytes);
                    fd0_path = None;
                }
                (1, RedirectMode::OutputTruncate(path)) => fd1_path = Some((path, false)),
                (1, RedirectMode::OutputAppend(path)) => fd1_path = Some((path, true)),
                (3, RedirectMode::DupOutput { target_fd: 1 }) => fd3_to_stdout = true,
                (2, RedirectMode::DupOutput { target_fd: 3 }) => fd2_to_fd3 = true,
                _ => {}
            }
        }

        let capture_stderr = fd3_to_stdout && fd2_to_fd3;
        let mut cmd = Command::new(program);
        cmd.args(args);

        if let Some(path) = fd0_path {
            let file = File::open(path).map_err(|source| RuntimeError::Io {
                program: path.to_owned(),
                source,
            })?;
            cmd.stdin(Stdio::from(file));
        } else if fd0_bytes.is_some() {
            cmd.stdin(Stdio::piped());
        } else if matches!(program, "dialog" | "whiptail") {
            if let Ok(tty) = File::open("/dev/tty") {
                cmd.stdin(Stdio::from(tty));
            }
        }

        if capture_stderr {
            if let Some((path, append)) = fd1_path {
                let file = if append {
                    OpenOptions::new().create(true).append(true).open(path)
                } else if path == "/dev/tty" {
                    OpenOptions::new().write(true).open(path)
                } else {
                    File::create(path)
                }
                .map_err(|source| RuntimeError::Io {
                    program: path.to_owned(),
                    source,
                })?;
                cmd.stdout(Stdio::from(file));
            } else {
                cmd.stdout(Stdio::inherit());
            }
            cmd.stderr(Stdio::piped());
        } else {
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::inherit());
        }

        let mut child = cmd.spawn().map_err(|source| RuntimeError::Io {
            program: program.to_owned(),
            source,
        })?;
        if let Some(bytes) = fd0_bytes {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(bytes);
            }
        }
        let output = child
            .wait_with_output()
            .map_err(|source| RuntimeError::Io {
                program: program.to_owned(),
                source,
            })?;

        let rc = Self::exit_code_from_status(&output.status);
        self.set_status(rc);
        let captured = if capture_stderr {
            output.stderr
        } else {
            output.stdout
        };
        let text = String::from_utf8(captured).map_err(|source| RuntimeError::InvalidUtf8 {
            program: program.to_owned(),
            source,
        })?;
        Ok(trim_trailing_newlines(&text).to_owned())
    }
}

/// Executes a multi-stage pipeline (`stages[0] | stages[1] | ...`) and reaps all child processes.
pub fn run_pipeline(stages: &[(&str, &[&str])]) -> RuntimeResult<CommandOutput> {
    if stages.is_empty() {
        return Err(RuntimeError::EmptyPipeline);
    }

    let mut children: Vec<(String, Child)> = Vec::with_capacity(stages.len());
    let mut prev_stdout: Option<std::process::ChildStdout> = None;

    for (idx, &(program, args)) in stages.iter().enumerate() {
        let mut cmd = Command::new(program);
        cmd.args(args);

        if let Some(stdout) = prev_stdout.take() {
            cmd.stdin(Stdio::from(stdout));
        }

        let is_last = idx + 1 == stages.len();
        cmd.stdout(Stdio::piped());
        if is_last {
            cmd.stderr(Stdio::piped());
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(source) => {
                for (_, mut earlier) in children {
                    let _ = earlier.kill();
                    let _ = earlier.wait();
                }
                return Err(RuntimeError::Io {
                    program: program.to_owned(),
                    source,
                });
            }
        };

        if !is_last {
            prev_stdout = child.stdout.take();
        }
        children.push((program.to_owned(), child));
    }

    let (last_prog, last_child) = children.pop().expect("non-empty pipeline");
    let last_res = last_child
        .wait_with_output()
        .map_err(|source| RuntimeError::Io {
            program: last_prog.clone(),
            source,
        });

    // Unconditionally reap all upstream stage processes even if a wait fails.
    let mut wait_err: Option<RuntimeError> = None;
    for (prog, mut child) in children {
        if let Err(source) = child.wait() {
            if wait_err.is_none() {
                wait_err = Some(RuntimeError::Io {
                    program: prog,
                    source,
                });
            }
        }
    }

    let last_out = last_res?;
    if let Some(err) = wait_err {
        return Err(err);
    }

    let stdout =
        String::from_utf8(last_out.stdout).map_err(|source| RuntimeError::InvalidUtf8 {
            program: last_prog,
            source,
        })?;
    let stderr = String::from_utf8_lossy(&last_out.stderr).into_owned();

    Ok(CommandOutput {
        stdout,
        stderr,
        exit_code: last_out.status.code().unwrap_or(-1),
        success: last_out.status.success(),
    })
}

/// Executes a pipeline and returns its stdout (with trailing newlines stripped) and exit code.
pub fn run_pipeline_subst_with_status(stages: &[(&str, &[&str])]) -> RuntimeResult<(String, i32)> {
    let out = run_pipeline(stages)?;
    Ok((
        trim_trailing_newlines(&out.stdout).to_owned(),
        out.exit_code,
    ))
}

/// Executes a pipeline and returns its stdout with trailing newlines stripped (`$(cmd1 | cmd2)`).
pub fn run_pipeline_subst(stages: &[(&str, &[&str])]) -> RuntimeResult<String> {
    let out = run_pipeline(stages)?;
    if !out.success {
        let last_prog = stages.last().map(|(p, _)| *p).unwrap_or("<pipeline>");
        return Err(RuntimeError::CommandFailed {
            program: last_prog.to_owned(),
            code: Some(out.exit_code),
            stderr: out.stderr,
        });
    }
    Ok(trim_trailing_newlines(&out.stdout).to_owned())
}

/// Executes a pipeline and returns `true` if the final stage exited with status 0.
pub fn run_pipeline_ok(stages: &[(&str, &[&str])]) -> bool {
    run_pipeline(stages).is_ok_and(|out| out.success)
}

/// Executes a multi-stage pipeline where the final stage inherits stdout/stderr (for top-level pipelines).
pub fn run_pipeline_inherit(stages: &[(&str, &[&str])]) -> RuntimeResult<ExitStatus> {
    if stages.is_empty() {
        return Err(RuntimeError::EmptyPipeline);
    }

    let mut children: Vec<(String, Child)> = Vec::with_capacity(stages.len());
    let mut prev_stdout: Option<std::process::ChildStdout> = None;

    for (idx, &(program, args)) in stages.iter().enumerate() {
        let mut cmd = Command::new(program);
        cmd.args(args);

        if let Some(stdout) = prev_stdout.take() {
            cmd.stdin(Stdio::from(stdout));
        }

        let is_last = idx + 1 == stages.len();
        if !is_last {
            cmd.stdout(Stdio::piped());
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(source) => {
                for (_, mut earlier) in children {
                    let _ = earlier.kill();
                    let _ = earlier.wait();
                }
                return Err(RuntimeError::Io {
                    program: program.to_owned(),
                    source,
                });
            }
        };

        if !is_last {
            prev_stdout = child.stdout.take();
        }
        children.push((program.to_owned(), child));
    }

    let (last_prog, mut last_child) = children.pop().expect("non-empty pipeline");
    let last_res = last_child.wait().map_err(|source| RuntimeError::Io {
        program: last_prog,
        source,
    });

    let mut wait_err: Option<RuntimeError> = None;
    for (prog, mut child) in children {
        if let Err(source) = child.wait() {
            if wait_err.is_none() {
                wait_err = Some(RuntimeError::Io {
                    program: prog,
                    source,
                });
            }
        }
    }

    let status = last_res?;
    if let Some(err) = wait_err {
        return Err(err);
    }
    Ok(status)
}

/// Executes a multi-stage pipeline with inherited stdout/stderr, reaping all stages and returning
/// the Bash-compatible pipeline exit code (rightmost non-zero if `pipefail`, else last stage).
pub fn run_pipeline_inherit_status(
    stages: &[(&str, &[&str])],
    pipefail: bool,
) -> RuntimeResult<i32> {
    if stages.is_empty() {
        return Err(RuntimeError::EmptyPipeline);
    }

    let mut children: Vec<(String, Child)> = Vec::with_capacity(stages.len());
    let mut prev_stdout: Option<std::process::ChildStdout> = None;

    for (idx, &(program, args)) in stages.iter().enumerate() {
        let mut cmd = Command::new(program);
        cmd.args(args);

        if let Some(stdout) = prev_stdout.take() {
            cmd.stdin(Stdio::from(stdout));
        }

        let is_last = idx + 1 == stages.len();
        if !is_last {
            cmd.stdout(Stdio::piped());
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(source) => {
                for (_, mut earlier) in children {
                    let _ = earlier.kill();
                    let _ = earlier.wait();
                }
                return Err(RuntimeError::Io {
                    program: program.to_owned(),
                    source,
                });
            }
        };

        if !is_last {
            prev_stdout = child.stdout.take();
        }
        children.push((program.to_owned(), child));
    }

    let mut codes = Vec::with_capacity(children.len());
    let mut wait_err: Option<RuntimeError> = None;
    for (prog, mut child) in children {
        match child.wait() {
            Ok(status) => codes.push(ShellState::exit_code_from_status(&status)),
            Err(source) => {
                if wait_err.is_none() {
                    wait_err = Some(RuntimeError::Io {
                        program: prog,
                        source,
                    });
                }
            }
        }
    }

    if let Some(err) = wait_err {
        return Err(err);
    }

    if pipefail {
        Ok(codes.iter().copied().rfind(|&c| c != 0).unwrap_or(0))
    } else {
        Ok(codes.last().copied().unwrap_or(0))
    }
}

/// Equivalent to Bash `[ -d path ]`.
pub fn test_dir(path: &str) -> bool {
    Path::new(path).is_dir()
}

/// Equivalent to Bash `[ -f path ]`.
pub fn test_file(path: &str) -> bool {
    Path::new(path).is_file()
}

/// Equivalent to Bash `[ -e path ]`.
pub fn test_exists(path: &str) -> bool {
    Path::new(path).exists()
}

/// Equivalent to Bash `[ -r path ]`.
pub fn test_readable(path: &str) -> bool {
    #[cfg(unix)]
    if let Ok(c_path) = CString::new(path) {
        // SAFETY: `c_path` is NUL-terminated.
        return unsafe { libc::access(c_path.as_ptr(), libc::R_OK) } == 0;
    }
    fs::File::open(path).is_ok()
}

/// Equivalent to Bash `[ -w path ]`.
pub fn test_writable(path: &str) -> bool {
    #[cfg(unix)]
    if let Ok(c_path) = CString::new(path) {
        // SAFETY: `c_path` is NUL-terminated.
        return unsafe { libc::access(c_path.as_ptr(), libc::W_OK) } == 0;
    }
    fs::metadata(path).is_ok_and(|m| !m.permissions().readonly())
}

/// Equivalent to Bash `[ -s path ]`.
pub fn test_non_empty_file(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
}

/// Equivalent to Bash `[ -L path ]` / `[ -h path ]`.
pub fn test_symlink(path: &str) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// Equivalent to Bash `[[ -b path ]]`.
pub fn test_block_device(path: &str) -> bool {
    #[cfg(unix)]
    {
        fs::metadata(path).is_ok_and(|m| m.file_type().is_block_device())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// Equivalent to Bash `[[ -c path ]]`.
pub fn test_char_device(path: &str) -> bool {
    #[cfg(unix)]
    {
        fs::metadata(path).is_ok_and(|m| m.file_type().is_char_device())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// Equivalent to Bash `[[ -x path ]]`.
pub fn test_executable(path: &str) -> bool {
    #[cfg(unix)]
    {
        fs::metadata(path).is_ok_and(|m| (m.permissions().mode() & 0o111) != 0)
    }
    #[cfg(not(unix))]
    {
        Path::new(path).exists()
    }
}

/// Equivalent to Bash `[[ -t fd ]]`.
pub fn test_fd_terminal(fd_str: &str) -> bool {
    match fd_str.trim().parse::<i32>() {
        Ok(0) => io::stdin().is_terminal(),
        Ok(1) => io::stdout().is_terminal(),
        Ok(2) => io::stderr().is_terminal(),
        _ => false,
    }
}

/// Equivalent to Bash `[[ text =~ pattern ]]`.
pub fn test_regex_match(text: &str, pattern: &str) -> bool {
    regex::Regex::new(pattern).is_ok_and(|re| re.is_match(text))
}

/// Equivalent to Bash `[ -n s ]`.
pub fn test_str_non_empty(s: &str) -> bool {
    !s.is_empty()
}

/// Equivalent to Bash `[ -z s ]`.
pub fn test_str_empty(s: &str) -> bool {
    s.is_empty()
}

/// Equivalent to Bash `[ a = b ]` / `[ a == b ]`.
pub fn test_str_eq(a: &str, b: &str) -> bool {
    a == b
}

/// Equivalent to Bash `[ a != b ]`.
pub fn test_str_ne(a: &str, b: &str) -> bool {
    a != b
}

/// Equivalent to Bash `[ a -eq b ]`.
pub fn test_int_eq(a: &str, b: &str) -> bool {
    parse_int_pair(a, b).is_some_and(|(x, y)| x == y)
}

/// Equivalent to Bash `[ a -ne b ]`.
pub fn test_int_ne(a: &str, b: &str) -> bool {
    parse_int_pair(a, b).is_some_and(|(x, y)| x != y)
}

/// Equivalent to Bash `[ a -lt b ]`.
pub fn test_int_lt(a: &str, b: &str) -> bool {
    parse_int_pair(a, b).is_some_and(|(x, y)| x < y)
}

/// Equivalent to Bash `[ a -le b ]`.
pub fn test_int_le(a: &str, b: &str) -> bool {
    parse_int_pair(a, b).is_some_and(|(x, y)| x <= y)
}

/// Equivalent to Bash `[ a -gt b ]`.
pub fn test_int_gt(a: &str, b: &str) -> bool {
    parse_int_pair(a, b).is_some_and(|(x, y)| x > y)
}

/// Equivalent to Bash `[ a -ge b ]`.
pub fn test_int_ge(a: &str, b: &str) -> bool {
    parse_int_pair(a, b).is_some_and(|(x, y)| x >= y)
}

fn parse_int_pair(a: &str, b: &str) -> Option<(i64, i64)> {
    let x = a.trim().parse::<i64>().ok()?;
    let y = b.trim().parse::<i64>().ok()?;
    Some((x, y))
}

fn trim_trailing_newlines(s: &str) -> &str {
    s.trim_end_matches(['\r', '\n'])
}

struct ArithParser<'a> {
    src: &'a [u8],
    pos: usize,
    shell: &'a mut ShellState,
}

impl<'a> ArithParser<'a> {
    fn new(input: &'a str, shell: &'a mut ShellState) -> Self {
        Self {
            src: input.as_bytes(),
            pos: 0,
            shell,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn starts_with(&mut self, pat: &[u8]) -> bool {
        self.skip_ws();
        self.src[self.pos..].starts_with(pat)
    }

    fn eat(&mut self, pat: &[u8]) -> bool {
        if self.starts_with(pat) {
            self.pos += pat.len();
            true
        } else {
            false
        }
    }

    fn read_var_i64(&self, name: &str) -> i64 {
        let raw = self
            .shell
            .vars
            .get(name)
            .cloned()
            .or_else(|| std::env::var(name).ok())
            .unwrap_or_default();
        parse_int_str(&raw)
    }

    fn write_var_i64(&mut self, name: &str, val: i64) -> i64 {
        self.shell.vars.insert(name.to_owned(), val.to_string());
        val
    }

    fn parse_comma(&mut self) -> i64 {
        let mut val = self.parse_assign();
        while self.eat(b",") {
            val = self.parse_assign();
        }
        val
    }

    fn parse_assign(&mut self) -> i64 {
        self.skip_ws();
        let saved = self.pos;
        if let Some(name) = self.try_parse_bare_ident() {
            let op = if self.eat(b"=") && !self.src.get(self.pos).is_some_and(|&b| b == b'=') {
                Some("=")
            } else if self.eat(b"+=") {
                Some("+=")
            } else if self.eat(b"-=") {
                Some("-=")
            } else if self.eat(b"*=") {
                Some("*=")
            } else if self.eat(b"/=") {
                Some("/=")
            } else if self.eat(b"%=") {
                Some("%=")
            } else {
                None
            };
            if let Some(op) = op {
                let rhs = self.parse_assign();
                let cur = self.read_var_i64(&name);
                let next = match op {
                    "=" => rhs,
                    "+=" => cur.wrapping_add(rhs),
                    "-=" => cur.wrapping_sub(rhs),
                    "*=" => cur.wrapping_mul(rhs),
                    "/=" => {
                        if rhs == 0 {
                            0
                        } else {
                            cur.wrapping_div(rhs)
                        }
                    }
                    "%=" => {
                        if rhs == 0 {
                            0
                        } else {
                            cur.wrapping_rem(rhs)
                        }
                    }
                    _ => rhs,
                };
                return self.write_var_i64(&name, next);
            }
        }
        self.pos = saved;
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> i64 {
        let cond = self.parse_log_or();
        if self.eat(b"?") {
            let then_v = self.parse_comma();
            let _ = self.eat(b":");
            let else_v = self.parse_ternary();
            if cond != 0 {
                then_v
            } else {
                else_v
            }
        } else {
            cond
        }
    }

    fn parse_log_or(&mut self) -> i64 {
        let mut lhs = self.parse_log_and();
        while self.eat(b"||") {
            let rhs = self.parse_log_and();
            lhs = i64::from(lhs != 0 || rhs != 0);
        }
        lhs
    }

    fn parse_log_and(&mut self) -> i64 {
        let mut lhs = self.parse_bit_or();
        while self.eat(b"&&") {
            let rhs = self.parse_bit_or();
            lhs = i64::from(lhs != 0 && rhs != 0);
        }
        lhs
    }

    fn parse_bit_or(&mut self) -> i64 {
        let mut lhs = self.parse_bit_xor();
        while self.starts_with(b"|") && !self.starts_with(b"||") {
            self.pos += 1;
            let rhs = self.parse_bit_xor();
            lhs |= rhs;
        }
        lhs
    }

    fn parse_bit_xor(&mut self) -> i64 {
        let mut lhs = self.parse_bit_and();
        while self.eat(b"^") {
            let rhs = self.parse_bit_and();
            lhs ^= rhs;
        }
        lhs
    }

    fn parse_bit_and(&mut self) -> i64 {
        let mut lhs = self.parse_eq();
        while self.starts_with(b"&") && !self.starts_with(b"&&") {
            self.pos += 1;
            let rhs = self.parse_eq();
            lhs &= rhs;
        }
        lhs
    }

    fn parse_eq(&mut self) -> i64 {
        let mut lhs = self.parse_rel();
        loop {
            if self.eat(b"==") {
                let rhs = self.parse_rel();
                lhs = i64::from(lhs == rhs);
            } else if self.eat(b"!=") {
                let rhs = self.parse_rel();
                lhs = i64::from(lhs != rhs);
            } else {
                break;
            }
        }
        lhs
    }

    fn parse_rel(&mut self) -> i64 {
        let mut lhs = self.parse_shift();
        loop {
            if self.eat(b"<=") {
                let rhs = self.parse_shift();
                lhs = i64::from(lhs <= rhs);
            } else if self.eat(b">=") {
                let rhs = self.parse_shift();
                lhs = i64::from(lhs >= rhs);
            } else if self.starts_with(b"<") && !self.starts_with(b"<<") {
                self.pos += 1;
                let rhs = self.parse_shift();
                lhs = i64::from(lhs < rhs);
            } else if self.starts_with(b">") && !self.starts_with(b">>") {
                self.pos += 1;
                let rhs = self.parse_shift();
                lhs = i64::from(lhs > rhs);
            } else {
                break;
            }
        }
        lhs
    }

    fn parse_shift(&mut self) -> i64 {
        let mut lhs = self.parse_add();
        loop {
            if self.eat(b"<<") {
                let rhs = self.parse_add();
                lhs = lhs.wrapping_shl((rhs & 63) as u32);
            } else if self.eat(b">>") {
                let rhs = self.parse_add();
                lhs = lhs.wrapping_shr((rhs & 63) as u32);
            } else {
                break;
            }
        }
        lhs
    }

    fn parse_add(&mut self) -> i64 {
        let mut lhs = self.parse_mul();
        loop {
            if self.starts_with(b"+") && !self.starts_with(b"++") && !self.starts_with(b"+=") {
                self.pos += 1;
                let rhs = self.parse_mul();
                lhs = lhs.wrapping_add(rhs);
            } else if self.starts_with(b"-") && !self.starts_with(b"--") && !self.starts_with(b"-=")
            {
                self.pos += 1;
                let rhs = self.parse_mul();
                lhs = lhs.wrapping_sub(rhs);
            } else {
                break;
            }
        }
        lhs
    }

    fn parse_mul(&mut self) -> i64 {
        let mut lhs = self.parse_unary();
        loop {
            if self.starts_with(b"*") && !self.starts_with(b"*=") {
                self.pos += 1;
                let rhs = self.parse_unary();
                lhs = lhs.wrapping_mul(rhs);
            } else if self.starts_with(b"/") && !self.starts_with(b"/=") {
                self.pos += 1;
                let rhs = self.parse_unary();
                lhs = if rhs == 0 { 0 } else { lhs.wrapping_div(rhs) };
            } else if self.starts_with(b"%") && !self.starts_with(b"%=") {
                self.pos += 1;
                let rhs = self.parse_unary();
                lhs = if rhs == 0 { 0 } else { lhs.wrapping_rem(rhs) };
            } else {
                break;
            }
        }
        lhs
    }

    fn parse_unary(&mut self) -> i64 {
        if self.eat(b"++") {
            if let Some(name) = self.try_parse_bare_ident() {
                let next = self.read_var_i64(&name).wrapping_add(1);
                return self.write_var_i64(&name, next);
            }
            return 0;
        }
        if self.eat(b"--") {
            if let Some(name) = self.try_parse_bare_ident() {
                let next = self.read_var_i64(&name).wrapping_sub(1);
                return self.write_var_i64(&name, next);
            }
            return 0;
        }
        if self.eat(b"+") {
            return self.parse_unary();
        }
        if self.eat(b"-") {
            return self.parse_unary().wrapping_neg();
        }
        if self.eat(b"!") {
            return i64::from(self.parse_unary() == 0);
        }
        if self.eat(b"~") {
            return !self.parse_unary();
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> i64 {
        if self.eat(b"(") {
            let val = self.parse_comma();
            let _ = self.eat(b")");
            return val;
        }

        self.skip_ws();
        if self.pos >= self.src.len() {
            return 0;
        }

        // `${#arr[@]}` / `${#arr[*]}` / `${#var}` / `${var}`
        if self.src[self.pos..].starts_with(b"${") {
            self.pos += 2;
            if let Some(rel_end) = self.src[self.pos..].iter().position(|&b| b == b'}') {
                let inner = std::str::from_utf8(&self.src[self.pos..self.pos + rel_end])
                    .unwrap_or("")
                    .trim();
                self.pos += rel_end + 1;
                if let Some(rest) = inner.strip_prefix('#') {
                    if let Some(arr_name) = rest
                        .strip_suffix("[@]")
                        .or_else(|| rest.strip_suffix("[*]"))
                    {
                        return self.shell.array_len(arr_name.trim()) as i64;
                    }
                    let len = self
                        .shell
                        .vars
                        .get(rest)
                        .cloned()
                        .or_else(|| std::env::var(rest).ok())
                        .map_or(0, |v| v.chars().count());
                    return len as i64;
                }
                return self.read_var_i64(inner);
            }
            return 0;
        }

        // `$var`
        if self.src[self.pos] == b'$' {
            self.pos += 1;
            if let Some(name) = self.try_parse_bare_ident() {
                return self.read_var_i64(&name);
            }
            return 0;
        }

        // Integer literal (hex `0x...` or decimal)
        if self.src[self.pos].is_ascii_digit() {
            let start = self.pos;
            while self.pos < self.src.len()
                && (self.src[self.pos].is_ascii_alphanumeric() || self.src[self.pos] == b'_')
            {
                self.pos += 1;
            }
            let tok = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("0");
            return parse_int_str(tok);
        }

        // Bare identifier (with optional `[idx]` array subscript or postfix `++` / `--`)
        if let Some(name) = self.try_parse_bare_ident() {
            if self.eat(b"[") {
                let idx_val = self.parse_comma();
                let _ = self.eat(b"]");
                if idx_val < 0 {
                    return 0;
                }
                return self
                    .shell
                    .get_array_elem(&name, idx_val as usize)
                    .trim()
                    .parse::<i64>()
                    .unwrap_or(0);
            }
            let cur = self.read_var_i64(&name);
            if self.eat(b"++") {
                self.write_var_i64(&name, cur.wrapping_add(1));
            } else if self.eat(b"--") {
                self.write_var_i64(&name, cur.wrapping_sub(1));
            }
            return cur;
        }

        self.pos += 1;
        0
    }

    fn try_parse_bare_ident(&mut self) -> Option<String> {
        self.skip_ws();
        let start = self.pos;
        if start < self.src.len()
            && (self.src[start].is_ascii_alphabetic() || self.src[start] == b'_')
        {
            self.pos += 1;
            while self.pos < self.src.len()
                && (self.src[self.pos].is_ascii_alphanumeric() || self.src[self.pos] == b'_')
            {
                self.pos += 1;
            }
            let name = std::str::from_utf8(&self.src[start..self.pos]).ok()?;
            return Some(name.to_owned());
        }
        None
    }
}

fn parse_int_str(s: &str) -> i64 {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).unwrap_or(0)
    } else {
        t.parse::<i64>().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_subst_captures_and_trims() {
        let out = command_subst("printf", &["hello\n\n"]).expect("printf");
        assert_eq!(out, "hello");
    }

    #[test]
    fn run_pipeline_pipes_and_reaps() {
        let out = run_pipeline(&[("printf", &["hello\n"]), ("tr", &["a-z", "A-Z"])])
            .expect("pipeline should succeed");
        assert!(out.success);
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, "HELLO\n");

        let subst = run_pipeline_subst(&[("printf", &["world\n"]), ("tr", &["a-z", "A-Z"])])
            .expect("pipeline subst");
        assert_eq!(subst, "WORLD");
        assert!(run_pipeline_ok(&[("true", &[])]));
    }

    #[test]
    fn run_command_and_exec_ok() {
        let status = run_command("true", &[]).expect("run true");
        assert!(status.success());
        assert!(exec_ok("true", &[]));
        assert!(!exec_ok("false", &[]));
    }

    #[test]
    fn file_and_dir_tests() {
        let dir = std::env::temp_dir();
        let dir_str = dir.to_str().expect("temp_dir utf8");
        assert!(test_dir(dir_str));
        assert!(test_exists(dir_str));
        assert!(!test_file(dir_str));

        let file_path = dir.join("bash2rust_runtime_test_file.txt");
        fs::write(&file_path, "data").expect("write temp file");
        let file_str = file_path.to_str().expect("file utf8");

        assert!(test_file(file_str));
        assert!(test_exists(file_str));
        assert!(test_non_empty_file(file_str));
        assert!(test_readable(file_str));
        assert!(!test_dir(file_str));

        let _ = fs::remove_file(&file_path);
        assert!(!test_exists(file_str));
    }

    #[test]
    fn string_and_int_tests() {
        assert!(test_str_non_empty("abc"));
        assert!(test_str_empty(""));
        assert!(test_str_eq("a", "a"));
        assert!(test_str_ne("a", "b"));
        assert!(test_int_eq("42", "42"));
        assert!(test_int_lt("3", "10"));
        assert!(test_int_ge("10", "10"));
        assert!(!test_int_eq("abc", "10"));
    }

    #[test]
    fn get_env_and_redirections_work() {
        assert_eq!(get_env("BASH2RUST_DEFINITELY_UNSET_VAR_12345"), "");
        std::env::set_var("BASH2RUST_TEST_ENV_VAR", "hello_env");
        assert_eq!(get_env("BASH2RUST_TEST_ENV_VAR"), "hello_env");
        std::env::remove_var("BASH2RUST_TEST_ENV_VAR");

        let dir = std::env::temp_dir();
        let out_path = dir.join("bash2rust_redir_out.txt");
        let out_str = out_path.to_str().expect("utf8");

        // Truncate `>`
        let status1 = run_command_with_redirects(
            "printf",
            &["line1\n"],
            &[],
            &[RedirectSpec {
                fd: 1,
                mode: RedirectMode::OutputTruncate(out_str),
            }],
        )
        .expect("redirect >");
        assert!(status1.success());

        // Append `>>`
        let status2 = run_command_with_redirects(
            "printf",
            &["line2\n"],
            &[],
            &[RedirectSpec {
                fd: 1,
                mode: RedirectMode::OutputAppend(out_str),
            }],
        )
        .expect("redirect >>");
        assert!(status2.success());

        let contents = fs::read_to_string(&out_path).expect("read out");
        assert_eq!(contents, "line1\nline2\n");

        // Input `<` + `2>&1` into a second file
        let out2_path = dir.join("bash2rust_redir_in_out.txt");
        let out2_str = out2_path.to_str().expect("utf8");
        let status3 = run_command_with_redirects(
            "cat",
            &[],
            &[],
            &[
                RedirectSpec {
                    fd: 0,
                    mode: RedirectMode::Input(out_str),
                },
                RedirectSpec {
                    fd: 1,
                    mode: RedirectMode::OutputTruncate(out2_str),
                },
                RedirectSpec {
                    fd: 2,
                    mode: RedirectMode::DupOutput { target_fd: 1 },
                },
            ],
        )
        .expect("redirect < and 2>&1");
        assert!(status3.success());
        assert_eq!(
            fs::read_to_string(&out2_path).expect("read out2"),
            "line1\nline2\n"
        );

        let _ = fs::remove_file(&out_path);
        let _ = fs::remove_file(&out2_path);

        // `2>&1` without stdout redirected to a file (exercises `io::pipe()` branch).
        let status4 = run_command_with_redirects(
            "true",
            &[],
            &[],
            &[RedirectSpec {
                fd: 2,
                mode: RedirectMode::DupOutput { target_fd: 1 },
            }],
        )
        .expect("redirect 2>&1 pipe");
        assert!(status4.success());
    }

    #[test]
    fn split_words_posix_ifs_rules() {
        assert_eq!(
            split_words("alpha beta gamma", None),
            vec!["alpha", "beta", "gamma"]
        );
        assert_eq!(split_words("", None), Vec::<String>::new());
        assert_eq!(split_words("   \t\n  ", None), Vec::<String>::new());
        assert_eq!(split_words("   a   b   ", None), vec!["a", "b"]);
        assert_eq!(
            split_words("one:two::four", Some(":")),
            vec!["one", "two", "", "four"]
        );
        assert_eq!(split_words("a : b", Some(": ")), vec!["a", "b"]);
        assert_eq!(split_words("a:b", Some("")), vec!["a:b"]);
        assert_eq!(split_words("", Some("")), Vec::<String>::new());
    }

    #[test]
    fn expand_glob_posix_rules() {
        let dir = std::env::temp_dir().join(format!(
            "bash2rust_glob_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("mkdir");
        for name in [
            "a1.txt",
            "a2.txt",
            "b10.txt",
            "file1.dat",
            "file2.dat",
            "file9.dat",
            ".hidden.dat",
        ] {
            fs::write(dir.join(name), "").expect("touch");
        }
        let d = dir.to_str().expect("utf8");

        assert_eq!(
            expand_glob(&format!("{d}/a?.txt")),
            vec![format!("{d}/a1.txt"), format!("{d}/a2.txt")]
        );
        assert_eq!(
            expand_glob(&format!("{d}/*.txt")),
            vec![
                format!("{d}/a1.txt"),
                format!("{d}/a2.txt"),
                format!("{d}/b10.txt")
            ]
        );
        assert_eq!(
            expand_glob(&format!("{d}/file[12].dat")),
            vec![format!("{d}/file1.dat"), format!("{d}/file2.dat")]
        );
        assert_eq!(
            expand_glob(&format!("{d}/file[!19].dat")),
            vec![format!("{d}/file2.dat")]
        );
        assert_eq!(
            expand_glob(&format!("{d}/*.dat")),
            vec![
                format!("{d}/file1.dat"),
                format!("{d}/file2.dat"),
                format!("{d}/file9.dat")
            ]
        );
        assert_eq!(
            expand_glob(&format!("{d}/.*.dat")),
            vec![format!("{d}/.hidden.dat")]
        );
        assert_eq!(
            expand_glob(&format!("{d}/no_such_*.xyz")),
            vec![format!("{d}/no_such_*.xyz")]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn shell_state_apply_set_flags_works() {
        let mut shell = ShellState::default();
        assert!(!shell.errexit && !shell.nounset && !shell.pipefail);

        shell.apply_set_flags(&["-euo", "pipefail"]);
        assert!(shell.errexit && shell.nounset && shell.pipefail);

        shell.apply_set_flags(&["+e", "+o", "pipefail"]);
        assert!(!shell.errexit && shell.nounset && !shell.pipefail);

        shell.apply_set_flags(&["-o", "errexit", "+u"]);
        assert!(shell.errexit && !shell.nounset && !shell.pipefail);
    }

    #[test]
    fn pattern_match_case_rules() {
        assert!(pattern_match("*.rs", "main.rs"));
        assert!(!pattern_match("*.rs", "main.sh"));
        assert!(pattern_match("*", ".hidden"));
        assert!(pattern_match("err_*", "err_timeout"));
        assert!(pattern_match("file[12].dat", "file2.dat"));
        assert!(!pattern_match("file[12].dat", "file3.dat"));
        assert!(pattern_match("a?c", "abc"));
    }

    #[test]
    fn shell_state_arith_and_var_array_helpers_work() {
        let mut sh = ShellState::default();
        sh.set_var("x", "10");
        sh.set_var("y", "3");
        assert_eq!(sh.eval_arith("x * 2 + y"), 23);
        assert_eq!(sh.eval_arith("0x10 + 5"), 21);
        assert_eq!(sh.eval_arith("x += 5, ++x"), 16);
        assert_eq!(sh.get_var("x"), Some("16"));
        assert_eq!(sh.eval_arith("y > 2 ? 42 : 99"), 42);
        assert_eq!(sh.eval_arith("10 / 0"), 0);

        sh.set_array("arr", vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]);
        assert_eq!(sh.eval_arith("${#arr[@]}"), 3);
        assert_eq!(sh.get_array_elem("arr", 1), "b");
        sh.set_array_elem("arr", 4, "e");
        assert_eq!(sh.array_len("arr"), 5);
        assert_eq!(sh.get_array_elem("arr", 3), "");
    }

    #[test]
    fn extended_test_helpers_work() {
        assert!(test_regex_match("err_404_not_found", r"^err_[0-9]+"));
        assert!(!test_regex_match("ok_200", r"^err_[0-9]+"));
        let _ = test_fd_terminal("1");
        assert!(!test_fd_terminal("99"));
    }

    #[test]
    fn shell_state_capture_and_stdin_override_work() {
        let mut sh = ShellState::default();
        sh.push_stdout_capture();
        let rc = sh
            .run_command("printf", &["hello\nworld\n"])
            .expect("capture printf");
        assert_eq!(rc, 0);
        assert_eq!(sh.pop_stdout_capture(), "hello\nworld");

        sh.push_stdin_override(b"alpha\nbeta\n".to_vec());
        sh.push_stdout_capture();
        let rc2 = sh
            .run_command("tr", &["a-z", "A-Z"])
            .expect("stdin_override tr");
        assert_eq!(rc2, 0);
        assert_eq!(sh.pop_stdout_capture(), "ALPHA\nBETA");
    }

    #[test]
    fn builtin_umask_octal_symbolic_and_invalid() {
        let saved = current_umask();
        let mut sh = ShellState::default();

        assert_eq!(sh.run_command("umask", &["027"]).unwrap(), 0);
        sh.push_stdout_capture();
        assert_eq!(sh.run_command("umask", &[]).unwrap(), 0);
        assert_eq!(sh.pop_stdout_capture(), "0027");

        sh.push_stdout_capture();
        assert_eq!(sh.run_command("umask", &["-p"]).unwrap(), 0);
        assert_eq!(sh.pop_stdout_capture(), "umask 0027");

        sh.push_stdout_capture();
        assert_eq!(sh.run_command("umask", &["-S"]).unwrap(), 0);
        assert_eq!(sh.pop_stdout_capture(), "u=rwx,g=rx,o=");

        assert_eq!(sh.run_command("umask", &["u=rwx,g=rx,o=rx"]).unwrap(), 0);
        sh.push_stdout_capture();
        assert_eq!(sh.run_command("umask", &[]).unwrap(), 0);
        assert_eq!(sh.pop_stdout_capture(), "0022");

        assert_eq!(sh.run_command("umask", &["0999"]).unwrap(), 1);

        set_current_umask(saved);
    }

    #[test]
    fn builtin_dispatch_colon_true_false_command_and_type() {
        let mut sh = ShellState::default();
        assert_eq!(sh.run_command(":", &["ignored"]).unwrap(), 0);
        assert_eq!(sh.run_command("true", &[]).unwrap(), 0);
        assert_eq!(sh.run_command("false", &[]).unwrap(), 1);
        assert!(exec_ok(":", &[]));
        assert!(exec_ok("true", &[]));
        assert!(!exec_ok("false", &[]));

        sh.push_stdout_capture();
        assert_eq!(sh.run_command("command", &["-v", "umask"]).unwrap(), 0);
        assert_eq!(sh.pop_stdout_capture(), "umask");

        sh.push_stdout_capture();
        assert_eq!(sh.run_command("command", &["-V", "true"]).unwrap(), 0);
        assert_eq!(sh.pop_stdout_capture(), "true is a shell builtin");

        assert_eq!(
            sh.run_command("command", &["-v", "__nonexistent_cmd_xyz__"])
                .unwrap(),
            1
        );

        sh.push_stdout_capture();
        assert_eq!(sh.run_command("type", &["-t", "umask"]).unwrap(), 0);
        assert_eq!(sh.pop_stdout_capture(), "builtin");

        assert_eq!(sh.run_command("let", &["x=42"]).unwrap(), 0);
        assert_eq!(sh.get_var("x"), Some("42"));
        assert_eq!(sh.run_command("let", &["x=0"]).unwrap(), 1);
    }

    #[test]
    fn builtin_read_stdin_override_and_multi_vars() {
        let mut sh = ShellState::default();
        sh.push_stdin_override(b"alpha beta gamma\nsecond\n".to_vec());

        assert_eq!(sh.run_command("read", &["-r", "a", "b"]).unwrap(), 0);
        assert_eq!(sh.get_var("a"), Some("alpha"));
        assert_eq!(sh.get_var("b"), Some("beta gamma"));

        assert_eq!(sh.run_command("read", &["-r"]).unwrap(), 0);
        assert_eq!(sh.get_var("REPLY"), Some("second"));

        assert_eq!(sh.run_command("read", &["-r", "eof_var"]).unwrap(), 1);
        assert_eq!(sh.get_var("eof_var"), Some(""));
    }

    #[test]
    fn special_vars_and_fd_swap_capture_work() {
        let mut sh = ShellState {
            nounset: true,
            ..ShellState::default()
        };
        assert!(!sh.get_var_or_env_checked("EUID", "t.sh", 1).is_empty());
        assert!(!sh.get_var_or_env_checked("UID", "t.sh", 2).is_empty());
        assert!(!sh.get_var_or_env_checked("PPID", "t.sh", 3).is_empty());
        assert!(!sh.get_var_or_env_checked("HOSTNAME", "t.sh", 4).is_empty());
        assert!(!sh.get_var_or_env_checked("OSTYPE", "t.sh", 5).is_empty());
        let r: u32 = sh
            .get_var_or_env_checked("RANDOM", "t.sh", 6)
            .parse()
            .unwrap();
        assert!(r < 32768);
        assert_eq!(sh.get_var_or_env_checked("LINENO", "t.sh", 42), "42");
        sh.set_var("SECONDS", "10");
        let s: i64 = sh
            .get_var_or_env_checked("SECONDS", "t.sh", 7)
            .parse()
            .unwrap();
        assert!(s >= 10);

        let out = sh
            .command_subst_with_redirects(
                "sh",
                &["-c", "echo stdout_msg; echo stderr_msg >&2; exit 3"],
                &[
                    RedirectSpec {
                        fd: 3,
                        mode: RedirectMode::DupOutput { target_fd: 1 },
                    },
                    RedirectSpec {
                        fd: 1,
                        mode: RedirectMode::OutputTruncate("/dev/null"),
                    },
                    RedirectSpec {
                        fd: 2,
                        mode: RedirectMode::DupOutput { target_fd: 3 },
                    },
                    RedirectSpec {
                        fd: 3,
                        mode: RedirectMode::CloseFd,
                    },
                ],
            )
            .unwrap();
        assert_eq!(out, "stderr_msg");
        assert_eq!(sh.last_status, 3);
    }

    #[test]
    fn source_file_circular_guard_and_nameref_declare_n() {
        let dir = std::env::temp_dir().join(format!("bash2rust_src_cyc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.sh");
        let b = dir.join("b.sh");
        std::fs::write(&a, format!("A=1\nsource {}\n", b.display())).unwrap();
        std::fs::write(&b, format!("B=2\nsource {}\n", a.display())).unwrap();

        let mut sh = ShellState::default();
        let rc = sh.source_file(a.to_str().unwrap(), &[]).unwrap();
        assert_eq!(rc, 1);
        assert_eq!(sh.get_var("A"), Some("1"));
        assert_eq!(sh.get_var("B"), Some("2"));
        assert!(sh.source_stack.is_empty());
        let _ = std::fs::remove_dir_all(&dir);

        sh.set_var("orig", "init");
        assert_eq!(sh.run_command("declare", &["-n", "r=orig"]).unwrap(), 0);
        assert_eq!(sh.get_var("r"), Some("init"));
        sh.set_var("r", "updated");
        assert_eq!(sh.get_var("orig"), Some("updated"));
        assert_eq!(sh.run_command("unset", &["-n", "r"]).unwrap(), 0);
        assert_eq!(sh.get_var("orig"), Some("updated"));
    }
}
