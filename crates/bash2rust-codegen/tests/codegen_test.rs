//! Unit and integration tests for `bash2rust-codegen`.

use std::path::{Path, PathBuf};

use bash2rust_codegen::{
    emit_assignment, emit_for_stmt, emit_if_stmt, emit_loop_stmt, generate_rust,
    parse_rust_snippet, CodegenError,
};
use bash2rust_ir::{
    ArrayItem, AssignTarget, AssignValue, Assignment, Block, CommandCall, ElifBranch, ForStmt,
    IfStmt, IrProgram, LoopKind, LoopStmt, Stmt, VarQualifier, Word,
};
use bash2rust_parser::{parse_file_to_ir, parse_str_to_ir};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn word(raw: &str) -> Word {
    Word {
        raw: raw.to_owned(),
        span: None,
        parts: vec![],
        has_quotes: false,
        part_quoted: vec![],
    }
}

#[test]
fn emits_let_mut_for_basic_assignment() {
    let ir = parse_str_to_ir("name=\"bash2rust\"\n", "inline.sh").expect("parse");
    let rust_code = generate_rust(&ir).expect("generate rust");
    assert!(
        rust_code.contains("let mut name = String::from(\"bash2rust\");"),
        "unexpected generated Rust:\n{rust_code}"
    );
    syn::parse_file(&rust_code).expect("valid syn::File");
}

#[test]
fn emits_assignment_variants_and_escapes_keywords() {
    let assign_kw = Assignment {
        target: AssignTarget::Variable {
            name: "type".to_owned(),
        },
        value: AssignValue::Scalar(word("'release'")),
        append: false,
        qualifier: VarQualifier::Global,
        span: None,
    };
    let tokens = emit_assignment(&assign_kw).to_string();
    assert!(tokens.contains("let mut r#type = String :: from (\"release\")"));

    let assign_arr = Assignment {
        target: AssignTarget::Variable {
            name: "items".to_owned(),
        },
        value: AssignValue::Array(vec![
            ArrayItem {
                key: None,
                value: word("\"a\""),
            },
            ArrayItem {
                key: None,
                value: word("\"b\""),
            },
        ]),
        append: false,
        qualifier: VarQualifier::Global,
        span: None,
    };
    let arr_tokens = emit_assignment(&assign_arr).to_string();
    assert!(arr_tokens
        .contains("let mut items = vec ! [String :: from (\"a\") , String :: from (\"b\")]"));
}

#[test]
fn emits_if_elif_else_valid_rust() {
    let if_stmt = IfStmt {
        condition: Block::new(vec![Stmt::Command(CommandCall {
            name: Some(word("true")),
            args: vec![],
            env_assignments: vec![],
            redirects: vec![],
            span: None,
        })]),
        then_branch: Block::new(vec![Stmt::Assign(Assignment {
            target: AssignTarget::Variable {
                name: "x".to_owned(),
            },
            value: AssignValue::Scalar(word("1")),
            append: false,
            qualifier: VarQualifier::Global,
            span: None,
        })]),
        elif_branches: vec![ElifBranch {
            condition: Block::new(vec![]),
            body: Block::new(vec![]),
        }],
        else_branch: Some(Block::new(vec![])),
        span: None,
    };

    let tokens = emit_if_stmt(&if_stmt);
    let wrapped = format!("fn main() {{ {tokens} }}");
    syn::parse_file(&wrapped).expect("IfStmt must parse as valid Rust");
}

#[test]
fn emits_for_and_loop_and_unsupported_valid_rust() {
    let for_stmt = ForStmt {
        variable: "target".to_owned(),
        values: Some(vec![word("linux"), word("macos")]),
        body: Block::new(vec![]),
        span: None,
    };
    let loop_stmt = LoopStmt {
        loop_kind: LoopKind::While,
        condition: Block::new(vec![]),
        body: Block::new(vec![]),
        redirects: Vec::new(),
        span: None,
    };
    let program = IrProgram {
        source: "test.sh".to_owned(),
        statements: vec![
            Stmt::For(for_stmt.clone()),
            Stmt::Loop(loop_stmt.clone()),
            Stmt::Unsupported {
                construct: "function_definition".to_owned(),
                raw: "foo() { :; }".to_owned(),
                span: None,
            },
        ],
    };

    assert!(emit_for_stmt(&for_stmt)
        .to_string()
        .contains("for target in [\"linux\" , \"macos\"]"));
    assert!(emit_loop_stmt(&loop_stmt)
        .to_string()
        .contains("while true"));

    let rust_code = generate_rust(&program).expect("generate rust");
    syn::parse_file(&rust_code).expect("valid syn::File");
}

#[test]
fn emits_runtime_calls_for_subst_pipelines_and_tests() {
    let ir_subst = parse_str_to_ir("today=$(date +%Y)\n", "subst.sh").expect("parse");
    let rust_subst = generate_rust(&ir_subst).expect("gen");
    assert!(
        rust_subst.contains("command_subst_with_redirects")
            && rust_subst.contains("\"date\"")
            && rust_subst.contains("\"+%Y\""),
        "unexpected:\n{rust_subst}"
    );

    let ir_pipe = parse_str_to_ir("printf hi | tr a-z A-Z\n", "pipe.sh").expect("parse");
    let rust_pipe = generate_rust(&ir_pipe).expect("gen");
    assert!(
        rust_pipe.contains("run_pipeline"),
        "unexpected:\n{rust_pipe}"
    );

    let ir_test =
        parse_str_to_ir("if [ -d /var/log ]; then echo ok; fi\n", "test.sh").expect("parse");
    let rust_test = generate_rust(&ir_test).expect("gen");
    assert!(
        rust_test.contains("if bash2rust_runtime::test_dir(\"/var/log\")"),
        "unexpected:\n{rust_test}"
    );

    let ir_and_or = parse_str_to_ir(
        "if [ ! -f /a ] && [ -e /b ]; then echo ok; fi\n",
        "and_or.sh",
    )
    .expect("parse");
    let rust_and_or = generate_rust(&ir_and_or).expect("gen");
    assert!(
        rust_and_or.contains(
            "if (!(bash2rust_runtime::test_file(\"/a\"))) && (bash2rust_runtime::test_exists(\"/b\"))"
        ),
        "unexpected:\n{rust_and_or}"
    );
}

#[test]
fn emits_string_interpolation_functions_and_redirections() {
    let ir_interp = parse_str_to_ir(
        "log_dir=\"/var/log\"\nmsg=\"Path: ${log_dir}\"\ngreeting=\"Hello $USER\"\nlit='Hello $USER'\n",
        "interp.sh",
    )
    .expect("parse interp");
    let rust_interp = generate_rust(&ir_interp).expect("gen interp");
    assert!(
        rust_interp.contains("format!(\"Path: {}\", log_dir)"),
        "expected Path interpolation in:\n{rust_interp}"
    );
    assert!(
        rust_interp.contains("format!(")
            && rust_interp.contains("\"Hello {}\"")
            && rust_interp.contains("__shell.get_var_or_env_checked(\"USER\", \"interp.sh\""),
        "expected Hello $USER env interpolation in:\n{rust_interp}"
    );
    assert!(
        rust_interp.contains("let mut lit = String::from(\"Hello $USER\");"),
        "expected non-interpolated single quotes in:\n{rust_interp}"
    );

    let ir_fn = parse_file_to_ir(fixture_path("16_functions_local.sh")).expect("parse 16");
    let rust_fn = generate_rust(&ir_fn).expect("gen 16");
    assert!(
        rust_fn.contains("fn greet_user(")
            && rust_fn.contains("__shell: &mut bash2rust_runtime::ShellState")
            && rust_fn.contains("args: &[&str]"),
        "expected hoisted greet_user fn in:\n{rust_fn}"
    );
    assert!(
        rust_fn.contains("greet_user(&mut __shell, &__arg_refs)?;"),
        "expected direct fn call in:\n{rust_fn}"
    );

    let ir_redir = parse_file_to_ir(fixture_path("17_redirections.sh")).expect("parse 17");
    let rust_redir = generate_rust(&ir_redir).expect("gen 17");
    assert!(
        rust_redir.contains("run_command_with_redirects"),
        "expected run_command_with_redirects in:\n{rust_redir}"
    );

    let ir_for_args =
        parse_str_to_ir("for arg; do echo \"$arg\"; done\n", "for_args.sh").expect("parse");
    let rust_for_args = generate_rust(&ir_for_args).expect("gen for_args");
    assert!(
        rust_for_args.contains("for arg in std::env::args().skip(1)"),
        "expected env::args loop in:\n{rust_for_args}"
    );
    assert!(
        rust_for_args.contains("arg.clone()"),
        "expected arg.clone() for OwnedString loop var in:\n{rust_for_args}"
    );
    syn::parse_file(&rust_for_args).expect("valid syn::File");
}

#[test]
fn raw_rust_injection_and_validation() {
    let stmts = parse_rust_snippet("let a = 1; let b = a + 2;").expect("bare stmts");
    assert_eq!(stmts.len(), 2);

    let main_stmts =
        parse_rust_snippet("fn main() { println!(\"wrapped\"); }").expect("fn main wrapper");
    assert_eq!(main_stmts.len(), 1);

    let err = parse_rust_snippet("let x = ;")
        .err()
        .expect("invalid rust should fail");
    assert!(matches!(err, CodegenError::InvalidAiRust { .. }));

    let program = IrProgram {
        source: "raw.sh".to_owned(),
        statements: vec![Stmt::RawRust {
            code: "let injected = 42;".to_owned(),
            original_bash: Some("eval \"injected=42\"".to_owned()),
            span: None,
        }],
    };
    let rust_code = generate_rust(&program).expect("generate raw rust");
    assert!(rust_code.contains("let injected = 42;"));
    syn::parse_file(&rust_code).expect("valid syn::File");

    let wrapped_program = IrProgram {
        source: "wrapped.sh".to_owned(),
        statements: vec![Stmt::RawRust {
            code: "fn main() { println!(\"wrapped\"); }".to_owned(),
            original_bash: None,
            span: None,
        }],
    };
    let wrapped_rust = generate_rust(&wrapped_program).expect("generate wrapped raw rust");
    assert!(wrapped_rust.contains("println!(\"wrapped\")"));
    assert_eq!(
        wrapped_rust.matches("fn main").count(),
        1,
        "must unwrap inner fn main:\n{wrapped_rust}"
    );
    syn::parse_file(&wrapped_rust).expect("valid syn::File");
}

#[test]
fn generates_valid_rust_for_all_17_fixtures() {
    let fixtures = [
        "01_variables_basic.sh",
        "02_variables_qualifiers.sh",
        "03_arrays.sh",
        "04_if_basic.sh",
        "05_if_elif_else.sh",
        "06_nested_conditionals_and_or.sh",
        "07_for_wordlist.sh",
        "08_for_arithmetic.sh",
        "09_while_until.sh",
        "10_combined_pipeline.sh",
        "11_command_substitution.sh",
        "12_pipelines.sh",
        "13_file_checks.sh",
        "14_sysadmin_automation.sh",
        "15_string_interpolation.sh",
        "16_functions_local.sh",
        "17_redirections.sh",
    ];

    for name in fixtures {
        let ir = parse_file_to_ir(fixture_path(name)).unwrap_or_else(|e| {
            panic!("failed to parse fixture {name}: {e}");
        });
        let rust_code = generate_rust(&ir).unwrap_or_else(|e| {
            panic!("failed to generate Rust for fixture {name}: {e}");
        });
        assert!(
            rust_code.contains("fn main() -> Result<(), Box<dyn std::error::Error>>"),
            "missing Result main() in {name}:\n{rust_code}"
        );
        syn::parse_file(&rust_code).unwrap_or_else(|e| {
            panic!("generated Rust for {name} failed syn::parse_file: {e}\n{rust_code}");
        });
    }
}

#[test]
fn emits_valid_rust_for_case_and_parameter_expansion() {
    let src = r#"
val="main.rs"
printf "%s %s %s\n" "${UNSET_VAR:-fallback}" "${val-alt}" "${#val}"
case "$val" in
    *.rs|*.sh)
        kind="code"
        ;;
    *)
        kind="other"
        ;;
esac
printf "%s\n" "$kind"
"#;
    let ir = parse_str_to_ir(src, "case_param.sh").expect("parse");
    let rust_code = generate_rust(&ir).expect("generate rust");
    assert!(
        rust_code.contains("bash2rust_runtime::pattern_match"),
        "expected pattern_match in:\n{rust_code}"
    );
    assert!(
        rust_code.contains(".chars().count().to_string()"),
        "expected length expansion in:\n{rust_code}"
    );
    syn::parse_file(&rust_code).expect("valid syn::File");
}
