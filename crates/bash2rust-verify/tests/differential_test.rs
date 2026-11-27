//! Differential tests comparing `bash` execution against compiled `bash2rust` binaries.

use std::path::{Path, PathBuf};

use bash2rust_verify::{
    assert_differential_match, DifferentialReport, ExecutionResult, VerifyError,
};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn differential_01_variables_basic() {
    assert_differential_match(fixture_path("01_variables_basic.sh")).expect("01 match");
}

#[test]
fn differential_04_if_basic() {
    assert_differential_match(fixture_path("04_if_basic.sh")).expect("04 match");
}

#[test]
fn differential_05_if_elif_else() {
    assert_differential_match(fixture_path("05_if_elif_else.sh")).expect("05 match");
}

#[test]
fn differential_06_nested_conditionals_and_or() {
    assert_differential_match(fixture_path("06_nested_conditionals_and_or.sh")).expect("06 match");
}

#[test]
fn differential_07_for_wordlist() {
    assert_differential_match(fixture_path("07_for_wordlist.sh")).expect("07 match");
}

#[test]
fn differential_11_command_substitution() {
    assert_differential_match(fixture_path("11_command_substitution.sh")).expect("11 match");
}

#[test]
fn differential_12_pipelines() {
    assert_differential_match(fixture_path("12_pipelines.sh")).expect("12 match");
}

#[test]
fn differential_13_file_checks() {
    assert_differential_match(fixture_path("13_file_checks.sh")).expect("13 match");
}

#[test]
fn differential_14_sysadmin_automation() {
    assert_differential_match(fixture_path("14_sysadmin_automation.sh")).expect("14 match");
}

#[test]
fn differential_15_string_interpolation() {
    assert_differential_match(fixture_path("15_string_interpolation.sh")).expect("15 match");
}

#[test]
fn differential_16_functions_local() {
    assert_differential_match(fixture_path("16_functions_local.sh")).expect("16 match");
}

#[test]
fn differential_17_redirections() {
    assert_differential_match(fixture_path("17_redirections.sh")).expect("17 match");
}

#[test]
fn differential_report_detects_and_formats_mismatch() {
    let report = DifferentialReport {
        script_path: PathBuf::from("example.sh"),
        bash: ExecutionResult {
            stdout: "hello\n".to_owned(),
            stderr: String::new(),
            exit_code: 0,
        },
        rust: ExecutionResult {
            stdout: "world\n".to_owned(),
            stderr: "warn\n".to_owned(),
            exit_code: 1,
        },
    };
    assert!(!report.matches());
    let rendered = report.to_string();
    assert!(rendered.contains("exit_code: bash=0 vs rust=1"));
    assert!(rendered.contains("stdout (bash): \"hello\\n\""));
    assert!(rendered.contains("stdout (rust): \"world\\n\""));
    assert!(rendered.contains("stderr (rust): \"warn\\n\""));

    let err = VerifyError::Mismatch(Box::new(report));
    assert!(err.to_string().contains("Differential mismatch"));
}
