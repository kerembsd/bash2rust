//! Workspace-level Bash semantic compatibility harness and reliability tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bash2rust_verify::compat::{
    ai_assisted_cli_args, bash_program, collect_unsupported_constructs, compare_execution_results,
    detect_bash_version, parse_bash_version, parse_requires_bash, render_markdown_report_with_ai,
    run_ai_assisted_suite, run_compat_case, run_compatibility_suite, run_with_timeout,
    write_markdown_report_with_ai, CompatStatus,
};
use bash2rust_verify::ExecutionResult;

static TEST_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct LocalTempDir {
    path: PathBuf,
}

impl LocalTempDir {
    fn create(prefix: &str) -> Self {
        let id = TEST_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{prefix}_{}_{id}", std::process::id()));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for LocalTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize workspace root")
}

#[test]
fn run_bash_compatibility_corpus_and_generate_report() {
    let root = workspace_root();
    let corpus_dir = root.join("tests/compatibility");
    let report_path = root.join("docs/COMPATIBILITY_REPORT.md");

    let mut scripts: Vec<PathBuf> = fs::read_dir(&corpus_dir)
        .expect("read compatibility corpus dir")
        .filter_map(|e| e.ok().map(|entry| entry.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("sh"))
        .collect();
    scripts.sort();
    assert!(
        scripts.len() >= 20,
        "expected >= 20 compatibility scripts, found {}",
        scripts.len()
    );

    let bash_prog = bash_program();
    let host_bash = detect_bash_version();
    for script in &scripts {
        if let (Some(ver), Ok(contents)) = (&host_bash, fs::read_to_string(script)) {
            if let Some((req_maj, req_min)) = parse_requires_bash(&contents) {
                if (ver.major, ver.minor) < (req_maj, req_min) {
                    continue;
                }
            }
        }
        let out = Command::new(&bash_prog)
            .arg("-n")
            .arg(script)
            .output()
            .expect("run bash -n");
        assert!(
            out.status.success(),
            "bash -n failed for {}: {}",
            script.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let report = run_compatibility_suite(&corpus_dir, Duration::from_secs(5))
        .expect("compatibility suite must run cleanly");
    let ai_report = run_ai_assisted_suite(&report, Duration::from_secs(5))
        .expect("ai-assisted suite probe must not error");
    write_markdown_report_with_ai(&report, ai_report.as_ref(), &report_path)
        .expect("write COMPATIBILITY_REPORT.md");

    assert_eq!(report.total(), scripts.len());
    assert_eq!(report.timeouts(), 0, "no compatibility test may time out");
    let rendered = render_markdown_report_with_ai(&report, ai_report.as_ref());
    assert!(rendered.contains("## Deterministic Mode"));
    assert!(rendered.contains("## AI-Assisted Mode"));
    assert!(rendered.contains("| Category | Total | Passed | Failed | Pass Rate |"));
    assert!(report_path.is_file());

    println!(
        "Compatibility Baseline (Deterministic): total={}, passed={}, failed={}, compile_failures={}, semantic_mismatches={}, timeouts={}, unsupported_scripts={}",
        report.total(),
        report.passed(),
        report.failed(),
        report.compile_failures(),
        report.semantic_mismatches(),
        report.timeouts(),
        report.scripts_with_unsupported(),
    );
    if let Some(ai) = &ai_report {
        println!(
            "Compatibility (AI-Assisted inkling-2-mini @ localhost:1235): total={}, passed={}, failed={}, compile_failures={}, semantic_mismatches={}, timeouts={}",
            ai.total(),
            ai.passed(),
            ai.failed(),
            ai.compile_failures(),
            ai.semantic_mismatches(),
            ai.timeouts(),
        );
    } else {
        println!(
            "Compatibility (AI-Assisted): http://localhost:1235/v1 unavailable (skipped offline)"
        );
    }
}

#[test]
fn harness_builds_exact_ai_assisted_cli_args() {
    let args = ai_assisted_cli_args(Path::new(
        "tests/compatibility/14_subshell_var_isolation.sh",
    ));
    assert_eq!(
        args,
        vec![
            "--file",
            "tests/compatibility/14_subshell_var_isolation.sh",
            "--emit",
            "rust",
            "--ai",
            "openai",
            "--openai-url",
            "http://localhost:1235/v1",
            "--ai-model",
            "inkling-2-mini",
        ]
    );
}

#[test]
fn harness_strict_comparison_rejects_stdout_stderr_and_exit_code_mismatches() {
    let base = ExecutionResult {
        stdout: "ok\n".to_owned(),
        stderr: String::new(),
        exit_code: 0,
    };

    assert_eq!(
        compare_execution_results(base.clone(), base.clone()),
        CompatStatus::Pass
    );

    let diff_stdout = ExecutionResult {
        stdout: "different\n".to_owned(),
        ..base.clone()
    };
    assert!(matches!(
        compare_execution_results(base.clone(), diff_stdout),
        CompatStatus::SemanticMismatch { .. }
    ));

    let diff_stderr = ExecutionResult {
        stderr: "err\n".to_owned(),
        ..base.clone()
    };
    assert!(matches!(
        compare_execution_results(base.clone(), diff_stderr),
        CompatStatus::SemanticMismatch { .. }
    ));

    let diff_rc = ExecutionResult {
        exit_code: 2,
        ..base.clone()
    };
    assert!(matches!(
        compare_execution_results(base, diff_rc),
        CompatStatus::SemanticMismatch { .. }
    ));
}

#[test]
fn harness_distinguishes_compile_failure_from_semantic_mismatch() {
    let dir = LocalTempDir::create("compat_compile_fail");
    let bad_script = dir.path.join("01_bad_syntax.sh");
    fs::write(&bad_script, "#!/usr/bin/env bash\nif then fi\n").expect("write bad script");

    let case = run_compat_case(&bad_script, Duration::from_secs(5)).expect("run case");
    assert!(matches!(
        case.status,
        CompatStatus::CompileFailure { ref stage, .. } if stage == "parser"
    ));
}

#[test]
fn harness_enforces_timeout_and_kills_hanging_process() {
    let dir = LocalTempDir::create("compat_timeout");
    let res = run_with_timeout(
        "bash",
        &["-c", "sleep 5"],
        &dir.path,
        Duration::from_millis(150),
        "bash",
    )
    .expect("timeout invocation");

    assert!(matches!(
        res,
        Err(CompatStatus::Timeout { ref target, .. }) if target == "bash"
    ));
}

#[test]
fn harness_isolates_working_directories_and_detects_unsupported_constructs() {
    let dir = LocalTempDir::create("compat_iso");
    let script = dir.path.join("14_subshell_and_eval.sh");
    fs::write(
        &script,
        "#!/usr/bin/env bash\nprintf \"hi\\n\" > ./marker.txt\ncoproc { printf \"sub\\n\"; }\neval \"echo dyn\"\n",
    )
    .expect("write script");

    let output = bash2rust_core::compile_file_to_ir(&script).expect("parse ir");
    let unsupported = collect_unsupported_constructs(&output.ir);
    assert!(
        unsupported.iter().any(|u| u.contains("compound_command")),
        "expected subshell unsupported entry, got {unsupported:?}"
    );
    assert!(
        !unsupported.iter().any(|u| u.contains("eval")),
        "eval should lower natively, got {unsupported:?}"
    );

    let _ = run_compat_case(&script, Duration::from_secs(5)).expect("run isolated case");
    assert!(
        !dir.path.join("marker.txt").exists(),
        "marker.txt must not leak into script source dir"
    );
}

#[test]
fn harness_parses_bash_versions_correctly() {
    let mac = parse_bash_version("GNU bash, version 3.2.57(1)-release (arm64-apple-darwin24)")
        .expect("parse macOS bash");
    assert_eq!((mac.major, mac.minor, mac.patch), (3, 2, 57));
    assert!(mac.meets_minimum((3, 2)));
    assert!(!mac.meets_minimum((4, 0)));

    let linux = parse_bash_version("GNU bash, version 5.2.21(1)-release (x86_64-pc-linux-gnu)")
        .expect("parse Linux bash");
    assert_eq!((linux.major, linux.minor, linux.patch), (5, 2, 21));
    assert!(linux.meets_minimum((4, 0)));
}

#[test]
fn harness_skips_scripts_requiring_newer_bash() {
    let dir = LocalTempDir::create("bash2rust_requires_bash");
    let script = dir.path.join("99_future_bash.sh");
    fs::write(
        &script,
        "#!/usr/bin/env bash\n# requires-bash: 99.0\nprintf \"never\\n\"\n",
    )
    .expect("write script");

    let res = run_compat_case(&script, Duration::from_secs(5)).expect("run skipped case");
    assert!(
        matches!(res.status, CompatStatus::Skipped { .. }),
        "expected Skipped status, got {:?}",
        res.status
    );
}
