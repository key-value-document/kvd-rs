//! Schema verification (spec §5, §10): a separate pass over a parsed document.
//!
//! The parser is registry-free; this module checks a data [`Node`] against
//! a schema. A standalone schema document is ordinary KVD whose values are
//! builtin scalar type names (`int`, `float`, `bool`, `str`), container type
//! names (`list`, `dict`) inside a `type:` descriptor, or the `{}` / `[]`
//! literals — a bare tree mirroring the data's structure (spec §4). A
//! one-item list declares the element type for every item of the
//! corresponding data list. Descriptors may carry `optional: true`, an ignored
//! `description` string, and a `validation` block with ranges, lengths,
//! and patterns (spec §10).

mod check;
mod constraints;
mod types;
mod validate;

pub use validate::validate_schema;

use crate::value::Node;
use alloc::vec::Vec;
use core::fmt;

/// One verification failure, located by the dotted path of the offending
/// value (`app.port`, `endpoints[0].method`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Dotted path to the offending value; empty for document-level issues.
    pub path: String,
    /// Human-readable explanation.
    pub message: String,
}

impl Violation {
    /// Creates a violation at `path` with `message`.
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Violation {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

/// Everything that can go wrong in one verification call: the document,
/// schema, or types document failed to parse, or parsing succeeded but
/// verification found violations.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VerifyError {
    /// The data document did not parse.
    ParseDoc(crate::error::Error),
    /// The companion schema document did not parse.
    ParseSchema(crate::error::Error),
    /// Everything parsed; verification reported these document violations.
    Violations(Vec<Violation>),
    /// Everything parsed, but the schema itself is malformed (e.g. a quoted or
    /// numbered type leaf, a bare `list`/`dict` leaf, a descriptor missing its
    /// `type`, or an unknown type name). A malformed schema cannot be used to
    /// check a document, so no document violations are reported (spec §8.3).
    SchemaMalformed(Vec<Violation>),
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerifyError::ParseDoc(e) => write!(f, "document parse error: {e}"),
            VerifyError::ParseSchema(e) => write!(f, "schema parse error: {e}"),
            VerifyError::Violations(vs) | VerifyError::SchemaMalformed(vs) => {
                for v in vs {
                    writeln!(f, "{v}")?;
                }
                Ok(())
            }
        }
    }
}

impl core::error::Error for VerifyError {}

/// The builtin type set (spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Builtin {
    Int,
    Float,
    Bool,
    Str,
    List,
    Dict,
}

pub(crate) fn builtin(name: &str) -> Option<Builtin> {
    match name {
        "int" => Some(Builtin::Int),
        "float" => Some(Builtin::Float),
        "bool" => Some(Builtin::Bool),
        "str" => Some(Builtin::Str),
        "list" => Some(Builtin::List),
        "dict" => Some(Builtin::Dict),
        _ => None,
    }
}

/// Verifies `doc` against a schema document: a bare KVD tree
/// whose leaf values are builtin type names or the `{}`/`[]` literals
/// (spec §4).
///
/// Returns `Ok(())` when the document conforms. A malformed schema yields
/// [`VerifyError::SchemaMalformed`] (distinct from [`VerifyError::Violations`],
/// which covers a well-formed schema applied to a non-conforming document); a
/// parse failure of either input is [`VerifyError::ParseDoc`] /
/// [`VerifyError::ParseSchema`].
pub fn verify(doc: &Node, schema: &Node) -> Result<(), VerifyError> {
    // A malformed schema cannot meaningfully check a document, so validate
    // the schema first and report its problems distinctly (spec §8.3).
    let mut schema_issues = Vec::new();
    validate::validate_schema(schema, "", &mut schema_issues);
    if !schema_issues.is_empty() {
        return Err(VerifyError::SchemaMalformed(schema_issues));
    }
    let mut out = Vec::new();
    check::check(schema, doc, "", &mut out);
    if out.is_empty() {
        Ok(())
    } else {
        Err(VerifyError::Violations(out))
    }
}

/// Parses a data document and a standalone schema document from text and
/// verifies the former against the latter. The one-call form of
/// [`crate::deserialize::from_str`] + [`verify`].
pub fn verify_from_str(doc: &str, schema: &str) -> Result<(), VerifyError> {
    let d = crate::deserialize::from_str(doc).map_err(VerifyError::ParseDoc)?;
    let s = crate::deserialize::from_str(schema).map_err(VerifyError::ParseSchema)?;
    verify(&d, &s)
}
