//! Integration tests for Phase 9: AI Self-Healing compilation loop.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use bash2rust_ai::AiProvider;
use bash2rust_verify::{compile_check_rust_source, compile_file_with_ai_self_healing, VerifyError};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[derive(Clone, Copy)]
enum MockMode {
    AlwaysValid,
    RustcErrorThenHeal,
    SynErrorThenHeal,
    AlwaysBroken,
    UnsafeShell,
}

struct MockAiProvider {
    mode: MockMode,
    translate_calls: AtomicUsize,
    repair_calls: AtomicUsize,
    captured_errors: Mutex<Vec<String>>,
}

impl MockAiProvider {
    fn new(mode: MockMode) -> Self {
        Self {
            mode,
            translate_calls: AtomicUsize::new(0),
            repair_calls: AtomicUsize::new(0),
            captured_errors: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl AiProvider for MockAiProvider {
    async fn translate_unsupported(&self, raw_bash: &str) -> anyhow::Result<String> {
        self.translate_calls.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            MockMode::AlwaysValid => Ok(format!("let _translated = {raw_bash:?};")),
            MockMode::RustcErrorThenHeal | MockMode::AlwaysBroken => {
                Ok("let broken: i32 = \"not_an_int\";".to_owned())
            }
            MockMode::SynErrorThenHeal => Ok("let x = ;".to_owned()),
            MockMode::UnsafeShell => Ok(
                "std::process::Command::new(\"sh\").arg(\"-c\").arg(\"id\").status()?;".to_owned(),
            ),
        }
    }

    async fn repair_rust(
        &self,
        raw_bash: &str,
        _broken_rust: &str,
        compiler_error: &str,
    ) -> anyhow::Result<String> {
        self.repair_calls.fetch_add(1, Ordering::SeqCst);
        self.captured_errors
            .lock()
            .expect("lock errors")
            .push(compiler_error.to_owned());

        match self.mode {
            MockMode::RustcErrorThenHeal | MockMode::SynErrorThenHeal => {
                Ok(format!("let _healed = {raw_bash:?};"))
            }
            MockMode::AlwaysBroken => Ok("let still_broken: i32 = \"still_bad\";".to_owned()),
            MockMode::AlwaysValid => Ok(format!("let _translated = {raw_bash:?};")),
            MockMode::UnsafeShell => Ok(
                "std::process::Command::new(\"sh\").arg(\"-c\").arg(\"id\").status()?;".to_owned(),
            ),
        }
    }
}

#[tokio::test]
async fn self_healing_happy_path_succeeds_on_attempt_1() {
    let provider = MockAiProvider::new(MockMode::AlwaysValid);
    let report =
        compile_file_with_ai_self_healing(fixture_path("18_complex_unsupported.sh"), &provider)
            .await
            .expect("happy path");

    assert_eq!(report.attempts, 1);
    assert!(!report.healed);
    assert!(report.initial_error.is_none());
    assert!(report.translated_nodes >= 2);
    assert_eq!(provider.repair_calls.load(Ordering::SeqCst), 0);
    compile_check_rust_source(&report.rust_code).expect("rustc metadata check");
}

#[tokio::test]
async fn self_healing_recovers_from_real_rustc_type_error() {
    let provider = MockAiProvider::new(MockMode::RustcErrorThenHeal);
    let report =
        compile_file_with_ai_self_healing(fixture_path("18_complex_unsupported.sh"), &provider)
            .await
            .expect("rustc error should self-heal on attempt 2");

    assert_eq!(report.attempts, 2);
    assert!(report.healed);
    let initial_err = report.initial_error.as_deref().expect("initial_error");
    assert!(
        initial_err.contains("E0308") || initial_err.contains("mismatched types"),
        "expected real rustc E0308 diagnostic, got:\n{initial_err}"
    );
    let errors = provider.captured_errors.lock().expect("lock errors");
    assert!(errors.len() >= 2);
    assert!(errors[0].contains("E0308") || errors[0].contains("mismatched types"));
    compile_check_rust_source(&report.rust_code).expect("healed code compiles");
}

#[tokio::test]
async fn self_healing_recovers_from_syn_syntax_error() {
    let provider = MockAiProvider::new(MockMode::SynErrorThenHeal);
    let report =
        compile_file_with_ai_self_healing(fixture_path("18_complex_unsupported.sh"), &provider)
            .await
            .expect("syn error should self-heal on attempt 2");

    assert_eq!(report.attempts, 2);
    assert!(report.healed);
    assert!(report.initial_error.is_some());
    assert!(provider.repair_calls.load(Ordering::SeqCst) >= 2);
    compile_check_rust_source(&report.rust_code).expect("healed code compiles");
}

#[tokio::test]
async fn self_healing_enforces_single_retry_cap() {
    let provider = MockAiProvider::new(MockMode::AlwaysBroken);
    let err =
        compile_file_with_ai_self_healing(fixture_path("18_complex_unsupported.sh"), &provider)
            .await
            .expect_err("persistent error must fail after 1 retry");

    assert!(matches!(err, VerifyError::RustcFailed(_)));
    // 2 unsupported nodes in 18_complex_unsupported.sh -> 2 translate + 2 repair calls max.
    assert_eq!(provider.translate_calls.load(Ordering::SeqCst), 2);
    assert_eq!(provider.repair_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn self_healing_rejects_unsafe_or_shell_spawning_ai_output_before_rustc() {
    let provider = MockAiProvider::new(MockMode::UnsafeShell);
    let err =
        compile_file_with_ai_self_healing(fixture_path("18_complex_unsupported.sh"), &provider)
            .await
            .expect_err("unsafe/shell-spawning AI output must be rejected immediately");

    assert!(matches!(err, VerifyError::Ai(_)));
    assert!(
        err.to_string().contains("security filter"),
        "unexpected error: {err}"
    );
    assert_eq!(provider.translate_calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider.repair_calls.load(Ordering::SeqCst), 0);
}
