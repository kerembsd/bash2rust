//! Statement and control-flow IR types.

use serde::{Deserialize, Serialize};

use crate::expr::{ArithExpr, Assignment, Word};
use crate::span::SourceSpan;

/// Top-level IR representation of a parsed Bash script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrProgram {
    pub source: String,
    pub statements: Vec<Stmt>,
}

/// A sequential block of statements.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Block {
    pub statements: Vec<Stmt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirects: Vec<Redirect>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_subshell: bool,
}

impl Block {
    /// Creates a new block from a list of statements.
    pub fn new(statements: Vec<Stmt>) -> Self {
        Self {
            statements,
            redirects: Vec::new(),
            is_subshell: false,
        }
    }

    /// Creates a new subshell block from a list of statements.
    pub fn subshell(statements: Vec<Stmt>) -> Self {
        Self {
            statements,
            redirects: Vec::new(),
            is_subshell: true,
        }
    }
}

/// An `if` / `elif` / `else` conditional statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IfStmt {
    pub condition: Block,
    pub then_branch: Block,
    pub elif_branches: Vec<ElifBranch>,
    pub else_branch: Option<Block>,
    pub span: Option<SourceSpan>,
}

/// A single `elif` branch in an [`IfStmt`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElifBranch {
    pub condition: Block,
    pub body: Block,
}

/// A `for var in values...; do ...; done` loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForStmt {
    pub variable: String,
    /// `None` means `for var; do ...` (iterating over `"$@"`).
    pub values: Option<Vec<Word>>,
    pub body: Block,
    pub span: Option<SourceSpan>,
}

/// A C-style arithmetic `for (( init; cond; step )); do ...; done` loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArithForStmt {
    pub initializer: Option<ArithExpr>,
    pub condition: Option<ArithExpr>,
    pub updater: Option<ArithExpr>,
    pub body: Block,
    pub span: Option<SourceSpan>,
}

/// Distinguishes `while` and `until` loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopKind {
    While,
    Until,
}

/// A `while` or `until` loop statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopStmt {
    pub loop_kind: LoopKind,
    pub condition: Block,
    pub body: Block,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirects: Vec<Redirect>,
    pub span: Option<SourceSpan>,
}

/// Kind of I/O redirection attached to a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum RedirectKind {
    OutputTruncate,
    OutputAppend,
    Input,
    HereString,
    HereDoc { strip_tabs: bool },
    InputProcSubst,
    ReadWriteProcSubst,
    DupOutput { target_fd: u32 },
    CloseFd,
}

/// An I/O redirection (`>`, `>>`, `<`, `<<<`, `<<`, `< <(cmd)`, `2>&1`, `3>&-`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Redirect {
    pub fd: Option<u32>,
    pub kind: RedirectKind,
    pub target: Word,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proc_subst: Option<Box<Stmt>>,
}

/// A Bash function definition (`name() { ... }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub body: Block,
    pub span: Option<SourceSpan>,
}

/// A simple command invocation (e.g. `echo hello`, `[ -f foo ]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandCall {
    pub name: Option<Word>,
    pub args: Vec<Word>,
    pub env_assignments: Vec<Assignment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirects: Vec<Redirect>,
    pub span: Option<SourceSpan>,
}

/// Logical connector between pipelines in an and-or list (`&&` or `||`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalOp {
    And,
    Or,
}

/// A single `&&` or `||` clause following the first pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndOrItem {
    pub op: LogicalOp,
    pub rhs: Stmt,
}

/// A boolean `&&` / `||` chain of statements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndOrStmt {
    pub first: Box<Stmt>,
    pub rest: Vec<AndOrItem>,
}

/// A pipeline of commands (`cmd1 | cmd2` or `! cmd`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineStmt {
    pub negated: bool,
    pub commands: Vec<Stmt>,
}

/// File-system predicate operator in `[ ... ]` / `test` / `[[ ... ]]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTestOp {
    Dir,
    File,
    Exists,
    Readable,
    Writable,
    NonEmptyFile,
    Symlink,
    BlockDevice,
    CharDevice,
    Executable,
    FdIsTerminal,
    OwnedByEffectiveUser,
}

/// Unary string predicate operator in `[ ... ]` / `test` / `[[ ... ]]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnaryTestOp {
    StrEmpty,
    StrNonEmpty,
}

/// Binary string or integer comparison operator in `[ ... ]` / `test` / `[[ ... ]]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryTestOp {
    StrEq,
    StrNe,
    IntEq,
    IntNe,
    IntLt,
    IntLe,
    IntGt,
    IntGe,
    RegexMatch,
    PatternMatch,
}

/// Boolean test expression lowered from `[ ... ]`, `test ...`, or `[[ ... ]]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum TestExpr {
    FileTest {
        op: FileTestOp,
        path: Word,
    },
    Unary {
        op: UnaryTestOp,
        operand: Word,
    },
    Binary {
        op: BinaryTestOp,
        lhs: Word,
        rhs: Word,
    },
    Not(Box<TestExpr>),
    And(Box<TestExpr>, Box<TestExpr>),
    Or(Box<TestExpr>, Box<TestExpr>),
}

/// A test statement (`[ ... ]`, `test ...`, or `[[ ... ]]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestStmt {
    pub expr: TestExpr,
    pub span: Option<SourceSpan>,
}

/// A single branch (`pat1 | pat2) ... ;;`) in a [`CaseStmt`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseItem {
    pub patterns: Vec<Word>,
    pub body: Block,
}

/// A `case ... in ... esac` conditional statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseStmt {
    pub value: Word,
    pub items: Vec<CaseItem>,
    pub span: Option<SourceSpan>,
}

/// A single IR statement node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Stmt {
    Assign(Assignment),
    If(IfStmt),
    Case(CaseStmt),
    For(ForStmt),
    ArithFor(ArithForStmt),
    Arith(ArithExpr),
    Loop(LoopStmt),
    Function(FunctionDef),
    Block(Block),
    Command(CommandCall),
    Test(TestStmt),
    AndOr(AndOrStmt),
    Pipeline(PipelineStmt),
    Return {
        code: Option<Word>,
        span: Option<SourceSpan>,
    },
    Source {
        path: Word,
        args: Vec<Word>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<Block>,
        span: Option<SourceSpan>,
    },
    /// Fallback node for constructs not yet lowered natively (targeted for hybrid AI translation).
    Unsupported {
        construct: String,
        raw: String,
        span: Option<SourceSpan>,
    },
    /// Raw Rust statements injected by an AI translation pass.
    RawRust {
        code: String,
        original_bash: Option<String>,
        span: Option<SourceSpan>,
    },
}
