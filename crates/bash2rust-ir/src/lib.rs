//! Intermediate representation (IR) for `bash2rust`.

pub mod expr;
pub mod span;
pub mod stmt;

pub use expr::{
    ArithExpr, ArrayItem, AssignTarget, AssignValue, Assignment, ParamExpansionOp, VarQualifier,
    Word, WordPart,
};
pub use span::{SourcePos, SourceSpan};
pub use stmt::{
    AndOrItem, AndOrStmt, ArithForStmt, BinaryTestOp, Block, CaseItem, CaseStmt, CommandCall,
    ElifBranch, FileTestOp, ForStmt, FunctionDef, IfStmt, IrProgram, LogicalOp, LoopKind, LoopStmt,
    PipelineStmt, Redirect, RedirectKind, Stmt, TestExpr, TestStmt, UnaryTestOp,
};
