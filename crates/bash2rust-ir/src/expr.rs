//! Expression, word, and assignment IR types.

use serde::{Deserialize, Serialize};

use crate::span::SourceSpan;
use crate::stmt::Stmt;

/// Operation for a `${...}` parameter expansion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ParamExpansionOp {
    Length,
    UseDefault {
        default: Word,
        colon: bool,
    },
    ArrayElement {
        index: Box<Word>,
    },
    ArrayAll,
    ArrayAllIfSet,
    ArraySlice {
        offset: Box<Word>,
        length: Option<Box<Word>>,
    },
    Trim {
        pattern: Box<Word>,
        is_suffix: bool,
        longest: bool,
    },
    Replace {
        pattern: Box<Word>,
        replacement: Box<Word>,
        replace_all: bool,
    },
    Substring {
        offset: Box<Word>,
        length: Option<Box<Word>>,
    },
}

/// A structured piece of a [`Word`], supporting variable interpolation and `$(...)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum WordPart {
    Literal(String),
    VarRef(String),
    CommandSubst(Box<Stmt>),
    ArithExpansion(String),
    ParamExpansion { name: String, op: ParamExpansionOp },
}

/// A Bash word token with its raw text, optional source span, and optional structured parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Word {
    pub raw: String,
    pub span: Option<SourceSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<WordPart>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_quotes: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub part_quoted: Vec<bool>,
}

/// An unexpanded arithmetic expression (e.g. inside `(( ... ))`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArithExpr {
    pub raw: String,
}

/// Scope/storage qualifier for a variable declaration or assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VarQualifier {
    #[default]
    Global,
    Local,
    Export,
    Readonly,
}

/// Target of a variable assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AssignTarget {
    Variable { name: String },
    ArrayElement { name: String, index: String },
}

/// A single element in a Bash array literal `( [k]=v ... )`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrayItem {
    pub key: Option<Word>,
    pub value: Word,
}

/// Value assigned in a variable assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum AssignValue {
    Scalar(Word),
    Array(Vec<ArrayItem>),
}

/// A Bash variable declaration or assignment (`x=1`, `x+=2`, `local x=1`, `export PATH=...`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub target: AssignTarget,
    pub value: AssignValue,
    pub append: bool,
    pub qualifier: VarQualifier,
    pub span: Option<SourceSpan>,
}
