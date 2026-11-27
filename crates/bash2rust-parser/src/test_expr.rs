//! Lowering for `[ ... ]`, `test ...`, and `[[ ... ]]` expressions into [`TestExpr`].

use bash2rust_ir::{BinaryTestOp, FileTestOp, TestExpr, UnaryTestOp, Word};
use brush_parser::ast;

pub(crate) fn lower_bracket_test(
    words: &[ast::Word],
    lower_word: &dyn Fn(&ast::Word) -> Word,
) -> Option<TestExpr> {
    parse_or(words, lower_word).and_then(
        |(expr, rest)| {
            if rest.is_empty() {
                Some(expr)
            } else {
                None
            }
        },
    )
}

fn parse_or<'a>(
    words: &'a [ast::Word],
    lower_word: &dyn Fn(&ast::Word) -> Word,
) -> Option<(TestExpr, &'a [ast::Word])> {
    let (mut lhs, mut rest) = parse_and(words, lower_word)?;
    while let Some(first) = rest.first() {
        if first.value == "-o" {
            let (rhs, next_rest) = parse_and(&rest[1..], lower_word)?;
            lhs = TestExpr::Or(Box::new(lhs), Box::new(rhs));
            rest = next_rest;
        } else {
            break;
        }
    }
    Some((lhs, rest))
}

fn parse_and<'a>(
    words: &'a [ast::Word],
    lower_word: &dyn Fn(&ast::Word) -> Word,
) -> Option<(TestExpr, &'a [ast::Word])> {
    let (mut lhs, mut rest) = parse_not(words, lower_word)?;
    while let Some(first) = rest.first() {
        if first.value == "-a" {
            let (rhs, next_rest) = parse_not(&rest[1..], lower_word)?;
            lhs = TestExpr::And(Box::new(lhs), Box::new(rhs));
            rest = next_rest;
        } else {
            break;
        }
    }
    Some((lhs, rest))
}

fn parse_not<'a>(
    words: &'a [ast::Word],
    lower_word: &dyn Fn(&ast::Word) -> Word,
) -> Option<(TestExpr, &'a [ast::Word])> {
    if let Some(first) = words.first() {
        if first.value == "!" {
            let (inner, rest) = parse_not(&words[1..], lower_word)?;
            return Some((TestExpr::Not(Box::new(inner)), rest));
        }
    }
    parse_primary(words, lower_word)
}

fn parse_primary<'a>(
    words: &'a [ast::Word],
    lower_word: &dyn Fn(&ast::Word) -> Word,
) -> Option<(TestExpr, &'a [ast::Word])> {
    match words {
        [] => None,
        [op, operand, rest @ ..] if is_file_op(&op.value).is_some() => {
            let file_op = is_file_op(&op.value)?;
            Some((
                TestExpr::FileTest {
                    op: file_op,
                    path: lower_word(operand),
                },
                rest,
            ))
        }
        [op, operand, rest @ ..] if is_unary_op(&op.value).is_some() => {
            let unary_op = is_unary_op(&op.value)?;
            Some((
                TestExpr::Unary {
                    op: unary_op,
                    operand: lower_word(operand),
                },
                rest,
            ))
        }
        [lhs, op, rhs, rest @ ..] if is_binary_op(&op.value).is_some() => {
            let binary_op = is_binary_op(&op.value)?;
            Some((
                TestExpr::Binary {
                    op: binary_op,
                    lhs: lower_word(lhs),
                    rhs: lower_word(rhs),
                },
                rest,
            ))
        }
        [single, rest @ ..] => Some((
            TestExpr::Unary {
                op: UnaryTestOp::StrNonEmpty,
                operand: lower_word(single),
            },
            rest,
        )),
    }
}

fn is_file_op(op: &str) -> Option<FileTestOp> {
    match op {
        "-d" => Some(FileTestOp::Dir),
        "-f" => Some(FileTestOp::File),
        "-e" => Some(FileTestOp::Exists),
        "-r" => Some(FileTestOp::Readable),
        "-w" => Some(FileTestOp::Writable),
        "-s" => Some(FileTestOp::NonEmptyFile),
        "-L" | "-h" => Some(FileTestOp::Symlink),
        "-b" => Some(FileTestOp::BlockDevice),
        "-c" => Some(FileTestOp::CharDevice),
        "-x" => Some(FileTestOp::Executable),
        "-t" => Some(FileTestOp::FdIsTerminal),
        _ => None,
    }
}

fn is_unary_op(op: &str) -> Option<UnaryTestOp> {
    match op {
        "-z" => Some(UnaryTestOp::StrEmpty),
        "-n" => Some(UnaryTestOp::StrNonEmpty),
        _ => None,
    }
}

fn is_binary_op(op: &str) -> Option<BinaryTestOp> {
    match op {
        "=" | "==" => Some(BinaryTestOp::StrEq),
        "!=" => Some(BinaryTestOp::StrNe),
        "-eq" => Some(BinaryTestOp::IntEq),
        "-ne" => Some(BinaryTestOp::IntNe),
        "-lt" => Some(BinaryTestOp::IntLt),
        "-le" => Some(BinaryTestOp::IntLe),
        "-gt" => Some(BinaryTestOp::IntGt),
        "-ge" => Some(BinaryTestOp::IntGe),
        "=~" => Some(BinaryTestOp::RegexMatch),
        _ => None,
    }
}

pub(crate) fn lower_extended_test(
    expr: &ast::ExtendedTestExpr,
    lower_word: &dyn Fn(&ast::Word) -> Word,
) -> Option<TestExpr> {
    match expr {
        ast::ExtendedTestExpr::And(lhs, rhs) => Some(TestExpr::And(
            Box::new(lower_extended_test(lhs, lower_word)?),
            Box::new(lower_extended_test(rhs, lower_word)?),
        )),
        ast::ExtendedTestExpr::Or(lhs, rhs) => Some(TestExpr::Or(
            Box::new(lower_extended_test(lhs, lower_word)?),
            Box::new(lower_extended_test(rhs, lower_word)?),
        )),
        ast::ExtendedTestExpr::Not(inner) => Some(TestExpr::Not(Box::new(lower_extended_test(
            inner, lower_word,
        )?))),
        ast::ExtendedTestExpr::Parenthesized(inner) => lower_extended_test(inner, lower_word),
        ast::ExtendedTestExpr::UnaryTest(pred, word) => match pred {
            ast::UnaryPredicate::FileExistsAndIsDir => Some(TestExpr::FileTest {
                op: FileTestOp::Dir,
                path: lower_word(word),
            }),
            ast::UnaryPredicate::FileExistsAndIsRegularFile => Some(TestExpr::FileTest {
                op: FileTestOp::File,
                path: lower_word(word),
            }),
            ast::UnaryPredicate::FileExists => Some(TestExpr::FileTest {
                op: FileTestOp::Exists,
                path: lower_word(word),
            }),
            ast::UnaryPredicate::FileExistsAndIsReadable => Some(TestExpr::FileTest {
                op: FileTestOp::Readable,
                path: lower_word(word),
            }),
            ast::UnaryPredicate::FileExistsAndIsWritable => Some(TestExpr::FileTest {
                op: FileTestOp::Writable,
                path: lower_word(word),
            }),
            ast::UnaryPredicate::FileExistsAndIsNotZeroLength => Some(TestExpr::FileTest {
                op: FileTestOp::NonEmptyFile,
                path: lower_word(word),
            }),
            ast::UnaryPredicate::FileExistsAndIsSymlink => Some(TestExpr::FileTest {
                op: FileTestOp::Symlink,
                path: lower_word(word),
            }),
            ast::UnaryPredicate::FileExistsAndIsBlockSpecialFile => Some(TestExpr::FileTest {
                op: FileTestOp::BlockDevice,
                path: lower_word(word),
            }),
            ast::UnaryPredicate::FileExistsAndIsCharSpecialFile => Some(TestExpr::FileTest {
                op: FileTestOp::CharDevice,
                path: lower_word(word),
            }),
            ast::UnaryPredicate::FileExistsAndIsExecutable => Some(TestExpr::FileTest {
                op: FileTestOp::Executable,
                path: lower_word(word),
            }),
            ast::UnaryPredicate::FdIsOpenTerminal => Some(TestExpr::FileTest {
                op: FileTestOp::FdIsTerminal,
                path: lower_word(word),
            }),
            ast::UnaryPredicate::StringHasZeroLength => Some(TestExpr::Unary {
                op: UnaryTestOp::StrEmpty,
                operand: lower_word(word),
            }),
            ast::UnaryPredicate::StringHasNonZeroLength => Some(TestExpr::Unary {
                op: UnaryTestOp::StrNonEmpty,
                operand: lower_word(word),
            }),
            _ => None,
        },
        ast::ExtendedTestExpr::BinaryTest(pred, lhs, rhs) => {
            let lhs_word = lower_word(lhs);
            let mut rhs_word = lower_word(rhs);
            let is_b2r_regex = rhs_word.raw.contains("__B2R_REGEX__");
            if is_b2r_regex {
                rhs_word.raw = rhs_word.raw.replace("__B2R_REGEX__", "");
                for part in &mut rhs_word.parts {
                    if let bash2rust_ir::WordPart::Literal(s) = part {
                        *s = s.replace("__B2R_REGEX__", "");
                    }
                }
            }
            if matches!(pred, ast::BinaryPredicate::StringDoesNotExactlyMatchPattern) {
                if rhs_word.has_quotes {
                    return Some(TestExpr::Binary {
                        op: BinaryTestOp::StrNe,
                        lhs: lhs_word,
                        rhs: rhs_word,
                    });
                }
                return Some(TestExpr::Not(Box::new(TestExpr::Binary {
                    op: BinaryTestOp::PatternMatch,
                    lhs: lhs_word,
                    rhs: rhs_word,
                })));
            }
            let op = match pred {
                ast::BinaryPredicate::StringExactlyMatchesString => BinaryTestOp::StrEq,
                ast::BinaryPredicate::StringExactlyMatchesPattern => {
                    if rhs_word.has_quotes {
                        BinaryTestOp::StrEq
                    } else {
                        BinaryTestOp::PatternMatch
                    }
                }
                ast::BinaryPredicate::StringDoesNotExactlyMatchString => BinaryTestOp::StrNe,
                ast::BinaryPredicate::StringMatchesRegex => BinaryTestOp::RegexMatch,
                ast::BinaryPredicate::StringContainsSubstring if is_b2r_regex => {
                    BinaryTestOp::RegexMatch
                }
                ast::BinaryPredicate::ArithmeticEqualTo => BinaryTestOp::IntEq,
                ast::BinaryPredicate::ArithmeticNotEqualTo => BinaryTestOp::IntNe,
                ast::BinaryPredicate::ArithmeticLessThan => BinaryTestOp::IntLt,
                ast::BinaryPredicate::ArithmeticLessThanOrEqualTo => BinaryTestOp::IntLe,
                ast::BinaryPredicate::ArithmeticGreaterThan => BinaryTestOp::IntGt,
                ast::BinaryPredicate::ArithmeticGreaterThanOrEqualTo => BinaryTestOp::IntGe,
                _ => return None,
            };
            Some(TestExpr::Binary {
                op,
                lhs: lhs_word,
                rhs: rhs_word,
            })
        }
    }
}
