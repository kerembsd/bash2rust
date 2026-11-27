//! End-to-end CLI tests for `bash2rust`, including `--migrate` Cargo project generation.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn create(prefix: &str) -> Self {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("{prefix}_{pid}_{id}"));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn assert_migrated_project_matches_bash(fixture_name: &str, expected_pkg_name: &str) {
    let fixture = fixture_path(fixture_name);
    let tmp = TempDirGuard::create("bash2rust_migrate_e2e");
    let project_dir = tmp.path.join("migrated");
    let target_dir = tmp.path.join("cargo_target");

    let bin = env!("CARGO_BIN_EXE_bash2rust");
    let migrate_out = Command::new(bin)
        .arg("--file")
        .arg(&fixture)
        .arg("--migrate")
        .arg(&project_dir)
        .output()
        .expect("run bash2rust --migrate");
    assert!(
        migrate_out.status.success(),
        "bash2rust --migrate failed: {}",
        String::from_utf8_lossy(&migrate_out.stderr)
    );

    let cargo_toml = project_dir.join("Cargo.toml");
    let main_rs = project_dir.join("src/main.rs");
    assert!(cargo_toml.is_file(), "missing Cargo.toml");
    assert!(main_rs.is_file(), "missing src/main.rs");

    let toml_text = fs::read_to_string(&cargo_toml).expect("read Cargo.toml");
    assert!(
        toml_text.contains(&format!("name = \"{expected_pkg_name}\"")),
        "unexpected Cargo.toml:\n{toml_text}"
    );
    assert!(toml_text.contains("bash2rust-runtime"));
    assert!(toml_text.contains("[workspace]"));

    let cargo_bin = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let rust_run = Command::new(&cargo_bin)
        .current_dir(&project_dir)
        .env("CARGO_TARGET_DIR", &target_dir)
        .args(["run", "--quiet"])
        .output()
        .expect("cargo run migrated project");
    assert!(
        rust_run.status.success(),
        "migrated project failed to run: {}",
        String::from_utf8_lossy(&rust_run.stderr)
    );

    let bash_run = Command::new("bash")
        .arg(&fixture)
        .output()
        .expect("run bash fixture");
    assert_eq!(
        rust_run.stdout, bash_run.stdout,
        "stdout mismatch for {fixture_name}"
    );
    assert_eq!(
        rust_run.status.code(),
        bash_run.status.code(),
        "exit code mismatch for {fixture_name}"
    );
}

#[test]
fn migrate_14_sysadmin_automation_builds_and_matches_bash() {
    assert_migrated_project_matches_bash(
        "14_sysadmin_automation.sh",
        "script_14_sysadmin_automation",
    );
}

#[test]
fn migrate_16_functions_local_builds_and_matches_bash() {
    assert_migrated_project_matches_bash("16_functions_local.sh", "script_16_functions_local");
}

#[test]
fn cli_default_ir_and_emit_rust_modes_work() {
    let bin = env!("CARGO_BIN_EXE_bash2rust");
    let fixture = fixture_path("01_variables_basic.sh");

    let ir_out = Command::new(bin)
        .arg("--file")
        .arg(&fixture)
        .output()
        .expect("run default IR mode");
    assert!(ir_out.status.success());
    let ir_stdout = String::from_utf8_lossy(&ir_out.stdout);
    assert!(ir_stdout.contains("IrProgram"));

    let rust_out = Command::new(bin)
        .arg("--file")
        .arg(&fixture)
        .args(["--emit", "rust"])
        .output()
        .expect("run --emit rust mode");
    assert!(rust_out.status.success());
    let rust_stdout = String::from_utf8_lossy(&rust_out.stdout);
    assert!(rust_stdout.contains("fn main()"));
}

#[test]
fn cli_emit_rust_resolves_static_source() {
    let bin = env!("CARGO_BIN_EXE_bash2rust");
    let dir = std::env::temp_dir().join(format!("bash2rust_cli_src_{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    let lib = dir.join("lib.sh");
    let main = dir.join("main.sh");
    fs::write(&lib, "STATIC_LIB_VAR=\"hello_static\"\n").expect("write lib.sh");
    fs::write(
        &main,
        "#!/usr/bin/env bash\nsource ./lib.sh\necho \"$STATIC_LIB_VAR\"\n",
    )
    .expect("write main.sh");

    let rust_out = Command::new(bin)
        .arg("--file")
        .arg(&main)
        .args(["--emit", "rust"])
        .output()
        .expect("run --emit rust with static source");
    let _ = fs::remove_dir_all(&dir);
    assert!(rust_out.status.success());
    let rust_stdout = String::from_utf8_lossy(&rust_out.stdout);
    assert!(
        rust_stdout.contains("hello_static") && !rust_stdout.contains("source_file("),
        "expected static source inlining, got:\n{rust_stdout}"
    );
}
