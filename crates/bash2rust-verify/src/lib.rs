//! Differential verification and AI self-healing engine for `bash2rust`.

pub mod ai_heal;
pub mod compat;

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use thiserror::Error;

pub use ai_heal::{
    compile_file_with_ai_self_healing, compile_ir_with_ai_self_healing,
    compile_str_with_ai_self_healing, SelfHealReport,
};

/// Captured stdout, stderr, and exit code from executing a script or binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Side-by-side comparison of `bash` vs compiled Rust execution for a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DifferentialReport {
    pub script_path: PathBuf,
    pub bash: ExecutionResult,
    pub rust: ExecutionResult,
}

impl DifferentialReport {
    /// Returns `true` if `stdout`, `stderr`, and `exit_code` all match identically.
    pub fn matches(&self) -> bool {
        self.bash == self.rust
    }
}

impl fmt::Display for DifferentialReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Differential mismatch for `{}`:",
            self.script_path.display()
        )?;
        writeln!(
            f,
            "  exit_code: bash={} vs rust={}",
            self.bash.exit_code, self.rust.exit_code
        )?;
        if self.bash.stdout != self.rust.stdout {
            writeln!(f, "  stdout (bash): {:?}", self.bash.stdout)?;
            writeln!(f, "  stdout (rust): {:?}", self.rust.stdout)?;
        }
        if self.bash.stderr != self.rust.stderr {
            writeln!(f, "  stderr (bash): {:?}", self.bash.stderr)?;
            writeln!(f, "  stderr (rust): {:?}", self.rust.stderr)?;
        }
        Ok(())
    }
}

/// Errors produced by the verification and self-healing engine.
#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("compilation failed: {0}")]
    Compile(#[from] bash2rust_core::CompileError),

    #[error("AI provider failed: {0}")]
    Ai(#[from] anyhow::Error),

    #[error("failed to locate or build `bash2rust_runtime` artifact: {0}")]
    RuntimeArtifact(String),

    #[error("rustc failed to compile generated Rust:\n{0}")]
    RustcFailed(String),

    #[error("{0}")]
    Mismatch(Box<DifferentialReport>),
}

struct RuntimeArtifacts {
    rlib_path: PathBuf,
    dep_dirs: Vec<PathBuf>,
}

static RUNTIME_ARTIFACTS: OnceLock<Result<RuntimeArtifacts, String>> = OnceLock::new();
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn create(prefix: &str) -> Result<Self, std::io::Error> {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("{prefix}_{pid}_{id}"));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn ensure_runtime_artifacts() -> Result<&'static RuntimeArtifacts, VerifyError> {
    let res = RUNTIME_ARTIFACTS.get_or_init(|| {
        let root = workspace_root();
        let rlib = root.join("target/debug/libbash2rust_runtime.rlib");

        let cargo_bin = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let status = Command::new(&cargo_bin)
            .current_dir(&root)
            .args(["build", "-p", "bash2rust-runtime"])
            .status()
            .map_err(|e| format!("failed to invoke `{cargo_bin}`: {e}"))?;
        if !status.success() {
            return Err(format!(
                "`cargo build -p bash2rust-runtime` exited with {status}"
            ));
        }

        if !rlib.exists() {
            return Err(format!(
                "expected `{}` to exist after build",
                rlib.display()
            ));
        }

        let mut dep_dirs = Vec::new();
        let deps = root.join("target/debug/deps");
        if deps.is_dir() {
            dep_dirs.push(deps);
        }
        for sub in [
            "thiserror",
            "thiserror-impl",
            "regex",
            "regex-automata",
            "regex-syntax",
            "aho-corasick",
            "memchr",
            "libc",
        ] {
            let base = root.join("target/debug/build").join(sub);
            if let Ok(entries) = fs::read_dir(&base) {
                for entry in entries.flatten() {
                    let out = entry.path().join("out");
                    if out.is_dir() {
                        dep_dirs.push(out);
                    }
                }
            }
        }

        Ok(RuntimeArtifacts {
            rlib_path: rlib,
            dep_dirs,
        })
    });

    res.as_ref()
        .map_err(|msg| VerifyError::RuntimeArtifact(msg.clone()))
}

fn configure_rustc_cmd(artifacts: &RuntimeArtifacts) -> Command {
    let mut rustc = Command::new("rustc");
    rustc
        .arg("--edition")
        .arg("2021")
        .arg("--crate-type")
        .arg("bin")
        .arg("--extern")
        .arg(format!(
            "bash2rust_runtime={}",
            artifacts.rlib_path.display()
        ));
    for dir in &artifacts.dep_dirs {
        rustc.arg("-L").arg(format!("dependency={}", dir.display()));
    }
    rustc
}

/// Type-checks and syntax-checks a generated Rust source string using `rustc --emit=metadata`
/// linked against `bash2rust_runtime` inside an isolated RAII temp directory.
pub fn compile_check_rust_source(rust_source: &str) -> Result<(), VerifyError> {
    let artifacts = ensure_runtime_artifacts()?;
    let tmp = TempDirGuard::create("bash2rust_check")?;
    let src_file = tmp.path.join("main.rs");
    let meta_file = tmp.path.join("app.rmeta");
    fs::write(&src_file, rust_source)?;

    let mut rustc = configure_rustc_cmd(artifacts);
    rustc
        .arg("--emit=metadata")
        .arg("-o")
        .arg(&meta_file)
        .arg(&src_file);

    let out = rustc.output()?;
    if !out.status.success() {
        return Err(VerifyError::RustcFailed(
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ));
    }
    Ok(())
}

/// Executes a Bash script using the system `bash` interpreter and captures its output.
pub fn run_bash_script(script_path: impl AsRef<Path>) -> Result<ExecutionResult, VerifyError> {
    let output = Command::new("bash").arg(script_path.as_ref()).output()?;
    Ok(ExecutionResult {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

/// Compiles a Bash script to Rust, builds it with `rustc`, runs the binary, and captures its output.
pub fn compile_and_run_rust(script_path: impl AsRef<Path>) -> Result<ExecutionResult, VerifyError> {
    let rust_source = bash2rust_core::compile_file_to_rust(script_path.as_ref())?;
    let artifacts = ensure_runtime_artifacts()?;

    let tmp = TempDirGuard::create("bash2rust_verify")?;
    let src_file = tmp.path.join("main.rs");
    let bin_file = tmp.path.join("app");
    fs::write(&src_file, &rust_source)?;

    let mut rustc = configure_rustc_cmd(artifacts);
    rustc.arg("-o").arg(&bin_file).arg(&src_file);

    let compile_out = rustc.output()?;
    if !compile_out.status.success() {
        return Err(VerifyError::RustcFailed(
            String::from_utf8_lossy(&compile_out.stderr).into_owned(),
        ));
    }

    let run_out = Command::new(&bin_file).output()?;
    Ok(ExecutionResult {
        stdout: String::from_utf8_lossy(&run_out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&run_out.stderr).into_owned(),
        exit_code: run_out.status.code().unwrap_or(-1),
    })
}

/// Runs both `bash` and the compiled Rust binary for `script_path` and returns a [`DifferentialReport`].
pub fn verify_differential(
    script_path: impl AsRef<Path>,
) -> Result<DifferentialReport, VerifyError> {
    let path = script_path.as_ref();
    let bash = run_bash_script(path)?;
    let rust = compile_and_run_rust(path)?;
    Ok(DifferentialReport {
        script_path: path.to_path_buf(),
        bash,
        rust,
    })
}

/// Verifies that `bash` and the compiled Rust binary produce identical stdout, stderr, and exit code.
pub fn assert_differential_match(
    script_path: impl AsRef<Path>,
) -> Result<DifferentialReport, VerifyError> {
    let report = verify_differential(script_path)?;
    if report.matches() {
        Ok(report)
    } else {
        Err(VerifyError::Mismatch(Box::new(report)))
    }
}
