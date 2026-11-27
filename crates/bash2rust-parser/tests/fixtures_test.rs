//! Integration tests verifying `brush-parser` parsing and IR lowering across all 14 fixtures.

use std::path::{Path, PathBuf};

use bash2rust_ir::{
    AssignTarget, AssignValue, FileTestOp, IrProgram, LogicalOp, LoopKind, RedirectKind, Stmt,
    TestExpr, VarQualifier, WordPart,
};
use bash2rust_parser::{parse_file_to_ir, parse_str_to_ir};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn fixture_01_variables_basic() {
    let ir = parse_file_to_ir(fixture_path("01_variables_basic.sh")).expect("parse 01");
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a)
            if matches!(&a.target, AssignTarget::Variable { name } if name == "name")
                && a.qualifier == VarQualifier::Global
                && !a.append
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Command(c) if c.name.as_ref().is_some_and(|w| w.raw == "echo")
    )));
}

#[test]
fn fixture_02_variables_qualifiers() {
    let ir = parse_file_to_ir(fixture_path("02_variables_qualifiers.sh")).expect("parse 02");
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a)
            if matches!(&a.target, AssignTarget::Variable { name } if name == "APP_ENV")
                && a.qualifier == VarQualifier::Export
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a)
            if matches!(&a.target, AssignTarget::Variable { name } if name == "MAX_RETRIES")
                && a.qualifier == VarQualifier::Readonly
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a)
            if matches!(&a.target, AssignTarget::Variable { name } if name == "path_list")
                && a.append
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Function(f) if f.name == "greet"
    )));
}

#[test]
fn fixture_03_arrays() {
    let ir = parse_file_to_ir(fixture_path("03_arrays.sh")).expect("parse 03");
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a)
            if matches!(&a.target, AssignTarget::Variable { name } if name == "fruits")
                && matches!(&a.value, AssignValue::Array(items) if items.len() == 3)
                && !a.append
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a)
            if matches!(&a.target, AssignTarget::ArrayElement { name, index } if name == "fruits" && index == "1")
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a)
            if matches!(&a.target, AssignTarget::Variable { name } if name == "fruits")
                && matches!(&a.value, AssignValue::Array(items) if items.len() == 2)
                && a.append
    )));
}

#[test]
fn fixture_04_if_basic() {
    let ir = parse_file_to_ir(fixture_path("04_if_basic.sh")).expect("parse 04");
    let ifs: Vec<_> = ir
        .statements
        .iter()
        .filter_map(|s| match s {
            Stmt::If(if_stmt) => Some(if_stmt),
            _ => None,
        })
        .collect();
    assert_eq!(ifs.len(), 2);
    assert!(ifs[0].else_branch.is_none());
    assert!(ifs[1].else_branch.is_some());
}

#[test]
fn fixture_05_if_elif_else() {
    let ir = parse_file_to_ir(fixture_path("05_if_elif_else.sh")).expect("parse 05");
    let if_stmt = ir
        .statements
        .iter()
        .find_map(|s| match s {
            Stmt::If(i) => Some(i),
            _ => None,
        })
        .expect("expected IfStmt");
    assert_eq!(if_stmt.elif_branches.len(), 2);
    assert!(if_stmt.else_branch.is_some());
}

#[test]
fn fixture_06_nested_conditionals_and_or() {
    let ir = parse_file_to_ir(fixture_path("06_nested_conditionals_and_or.sh")).expect("parse 06");
    let outer_if = ir
        .statements
        .iter()
        .find_map(|s| match s {
            Stmt::If(i) => Some(i),
            _ => None,
        })
        .expect("expected outer IfStmt");
    assert!(outer_if
        .then_branch
        .statements
        .iter()
        .any(|s| matches!(s, Stmt::If(inner) if inner.else_branch.is_some())));

    let and_or = ir
        .statements
        .iter()
        .find_map(|s| match s {
            Stmt::AndOr(ao) => Some(ao),
            _ => None,
        })
        .expect("expected AndOrStmt");
    assert_eq!(and_or.rest.len(), 2);
    assert_eq!(and_or.rest[0].op, LogicalOp::And);
    assert_eq!(and_or.rest[1].op, LogicalOp::Or);
}

#[test]
fn fixture_07_for_wordlist() {
    let ir = parse_file_to_ir(fixture_path("07_for_wordlist.sh")).expect("parse 07");
    let for_stmt = ir
        .statements
        .iter()
        .find_map(|s| match s {
            Stmt::For(f) => Some(f),
            _ => None,
        })
        .expect("expected ForStmt");
    assert_eq!(for_stmt.variable, "target");
    let values = for_stmt.values.as_ref().expect("expected word list");
    assert_eq!(values.len(), 3);
    assert_eq!(values[0].raw, "linux");
}

#[test]
fn fixture_08_for_arithmetic() {
    let ir = parse_file_to_ir(fixture_path("08_for_arithmetic.sh")).expect("parse 08");
    let arith_for = ir
        .statements
        .iter()
        .find_map(|s| match s {
            Stmt::ArithFor(f) => Some(f),
            _ => None,
        })
        .expect("expected ArithForStmt");
    assert!(arith_for.initializer.is_some());
    assert!(arith_for.condition.is_some());
    assert!(arith_for.updater.is_some());
    assert!(!arith_for.body.statements.is_empty());
}

#[test]
fn fixture_09_while_until() {
    let ir = parse_file_to_ir(fixture_path("09_while_until.sh")).expect("parse 09");
    let loops: Vec<_> = ir
        .statements
        .iter()
        .filter_map(|s| match s {
            Stmt::Loop(l) => Some(l),
            _ => None,
        })
        .collect();
    assert_eq!(loops.len(), 2);
    assert_eq!(loops[0].loop_kind, LoopKind::While);
    assert_eq!(loops[1].loop_kind, LoopKind::Until);
}

#[test]
fn fixture_10_combined_pipeline() {
    let ir = parse_file_to_ir(fixture_path("10_combined_pipeline.sh")).expect("parse 10");
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a) if a.qualifier == VarQualifier::Export
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a) if a.qualifier == VarQualifier::Readonly
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::For(f) if f.body.statements.iter().any(|inner| matches!(inner, Stmt::If(_)))
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Pipeline(p) if !p.negated && p.commands.len() == 2
    )));
}

#[test]
fn fixture_11_command_substitution() {
    let ir = parse_file_to_ir(fixture_path("11_command_substitution.sh")).expect("parse 11");
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a)
            if matches!(&a.target, AssignTarget::Variable { name } if name == "today")
                && matches!(&a.value, AssignValue::Scalar(w) if matches!(w.parts.as_slice(), [WordPart::CommandSubst(_)]))
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a)
            if matches!(&a.target, AssignTarget::Variable { name } if name == "host_banner")
                && matches!(&a.value, AssignValue::Scalar(w) if matches!(w.parts.as_slice(), [WordPart::Literal(_), WordPart::CommandSubst(_)]))
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a)
            if matches!(&a.target, AssignTarget::Variable { name } if name == "upper_msg")
                && matches!(&a.value, AssignValue::Scalar(w) if matches!(w.parts.as_slice(), [WordPart::CommandSubst(inner)] if matches!(inner.as_ref(), Stmt::Pipeline(_))))
    )));
}

#[test]
fn fixture_12_pipelines() {
    let ir = parse_file_to_ir(fixture_path("12_pipelines.sh")).expect("parse 12");
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Pipeline(p) if p.commands.len() == 3
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::If(i) if matches!(i.condition.statements.as_slice(), [Stmt::Pipeline(p)] if p.commands.len() == 2)
    )));
}

#[test]
fn fixture_13_file_checks() {
    let ir = parse_file_to_ir(fixture_path("13_file_checks.sh")).expect("parse 13");
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::If(i) if matches!(
            i.condition.statements.as_slice(),
            [Stmt::Test(t)] if matches!(&t.expr, TestExpr::FileTest { op: FileTestOp::Dir, .. })
        )
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::If(i) if matches!(
            i.condition.statements.as_slice(),
            [Stmt::AndOr(ao)] if ao.rest.len() == 1 && ao.rest[0].op == LogicalOp::And
        )
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::If(i) if matches!(
            i.condition.statements.as_slice(),
            [Stmt::Test(t)] if matches!(&t.expr, TestExpr::And(_, _))
        )
    )));
}

#[test]
fn fixture_14_sysadmin_automation() {
    let ir = parse_file_to_ir(fixture_path("14_sysadmin_automation.sh")).expect("parse 14");
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a) if matches!(&a.target, AssignTarget::Variable { name } if name == "current_host")
    )));
    assert!(ir.statements.iter().any(|s| matches!(s, Stmt::If(_))));
    assert!(ir.statements.iter().any(|s| matches!(s, Stmt::For(_))));
}

#[test]
fn fixture_15_string_interpolation() {
    let ir = parse_file_to_ir(fixture_path("15_string_interpolation.sh")).expect("parse 15");
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a)
            if matches!(&a.target, AssignTarget::Variable { name } if name == "msg")
                && matches!(&a.value, AssignValue::Scalar(w) if matches!(
                    w.parts.as_slice(),
                    [WordPart::Literal(l), WordPart::VarRef(v)] if l == "Path: " && v == "log_dir"
                ))
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Assign(a)
            if matches!(&a.target, AssignTarget::Variable { name } if name == "literal_only")
                && matches!(&a.value, AssignValue::Scalar(w) if w.parts.is_empty())
    )));
}

#[test]
fn fixture_16_functions_local() {
    let ir = parse_file_to_ir(fixture_path("16_functions_local.sh")).expect("parse 16");
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Function(f)
            if f.name == "greet_user"
                && f.body.statements.iter().any(|inner| matches!(
                    inner,
                    Stmt::Assign(a) if a.qualifier == VarQualifier::Local
                ))
    )));
}

#[test]
fn fixture_17_redirections() {
    let ir = parse_file_to_ir(fixture_path("17_redirections.sh")).expect("parse 17");
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Command(c) if c.redirects.iter().any(|r| r.kind == RedirectKind::OutputTruncate)
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Command(c) if c.redirects.iter().any(|r| r.kind == RedirectKind::OutputAppend)
    )));
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Command(c)
            if c.redirects.iter().any(|r| r.kind == RedirectKind::Input)
                && c.redirects.iter().any(|r| matches!(r.kind, RedirectKind::DupOutput { target_fd: 1 }))
    )));
}

#[test]
fn eval_and_trap_lower_natively() {
    let ir = parse_str_to_ir(
        "eval \"echo hi\"\ntrap \"rm -f /tmp/x\" EXIT\n",
        "builtins.sh",
    )
    .expect("parse eval and trap");
    assert_eq!(ir.statements.len(), 2);
    assert!(matches!(
        &ir.statements[0],
        Stmt::Command(cmd) if cmd.name.as_ref().is_some_and(|w| w.raw == "eval")
    ));
    assert!(matches!(
        &ir.statements[1],
        Stmt::Command(cmd) if cmd.name.as_ref().is_some_and(|w| w.raw == "trap")
    ));
}

#[test]
fn fixture_18_complex_unsupported() {
    let ir = parse_file_to_ir(fixture_path("18_complex_unsupported.sh")).expect("parse 18");
    assert!(ir.statements.iter().any(|s| matches!(
        s,
        Stmt::Command(cmd) if cmd.name.as_ref().is_some_and(|w| w.raw == "trap")
    )));
    let unsupported_count = ir
        .statements
        .iter()
        .filter(|s| {
            matches!(
                s,
                Stmt::Unsupported { construct, .. } if construct == "compound_command"
            )
        })
        .count();
    assert_eq!(unsupported_count, 2);
}

#[test]
fn case_and_param_expansion_lower_natively_without_unsupported() {
    let src = r#"
x=""
printf "%s %s\n" "${x:-def}" "${#x}"
case "$x" in
    a|b) printf "ab\n" ;;
    *.rs) printf "rs\n" ;;
    *) printf "other\n" ;;
esac
"#;
    let ir = parse_str_to_ir(src, "case_param.sh").expect("parse case and param expansion");
    assert!(
        !ir.statements
            .iter()
            .any(|s| matches!(s, Stmt::Unsupported { .. })),
        "expected zero Stmt::Unsupported, got: {ir:#?}"
    );
    assert!(ir.statements.iter().any(|s| matches!(s, Stmt::Case(_))));
}

#[test]
fn all_fixtures_roundtrip_serde_json() {
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
        "18_complex_unsupported.sh",
    ];

    for name in fixtures {
        let ir = parse_file_to_ir(fixture_path(name)).unwrap_or_else(|e| {
            panic!("failed to parse fixture {name}: {e}");
        });
        let json = serde_json::to_string(&ir).unwrap_or_else(|e| {
            panic!("failed to serialize IR for {name}: {e}");
        });
        let decoded: IrProgram = serde_json::from_str(&json).unwrap_or_else(|e| {
            panic!("failed to deserialize IR for {name}: {e}");
        });
        assert_eq!(ir, decoded, "JSON roundtrip mismatch for {name}");
    }
}
