//! Schema verification (spec §5): a separate pass over a parsed document.
//!
//! The parser is registry-free; this module checks a data [`Node`] against
//! a schema. A standalone schema document is ordinary KVD whose values are
//! builtin scalar type names (`int`, `float`, `bool`, `str`), container type
//! names (`list`, `map`) inside a `type:` descriptor, or the `{}` / `[]`
//! literals — a bare tree mirroring the data's structure, with no
//! metakeys (`__schema__` belongs in data documents only, spec §4). A
//! one-item list declares the element type for every item of the
//! corresponding data list.
//!
//! v1 rules: every data key must appear in the schema and vice versa
//! (all keys required), dotted keys and nested blocks are interchangeable
//! (the parser already normalizes both to nested maps), and a `{}` leaf
//! accepts any map while a `[]` leaf accepts any list — contents unchecked.

use crate::grammar::is_type_name;
use crate::value::{Map, Node, Scalar, Shape};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

/// One verification failure, located by the dotted path of the offending
/// value (`app.port`, `endpoints[0].method`, `__schema__.pem`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Dotted path to the offending value; empty for document-level issues.
    pub path: String,
    /// Human-readable explanation.
    pub message: String,
}

impl Violation {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
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
    /// numbered type leaf, a bare `list`/`map` leaf, a descriptor missing its
    /// `type`, or an unknown type name). A malformed schema cannot be used to
    /// check a document, so no document violations are reported (spec §8.3).
    SchemaMalformed(Vec<Violation>),
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerifyError::ParseDoc(e) => write!(f, "document parse error: {e}"),
            VerifyError::ParseSchema(e) => write!(f, "schema parse error: {e}"),
            VerifyError::Violations(vs) => {
                for v in vs {
                    writeln!(f, "{v}")?;
                }
                Ok(())
            }
            VerifyError::SchemaMalformed(vs) => {
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
enum Builtin {
    Int,
    Float,
    Bool,
    Str,
    List,
    Map,
}

fn builtin(name: &str) -> Option<Builtin> {
    match name {
        "int" => Some(Builtin::Int),
        "float" => Some(Builtin::Float),
        "bool" => Some(Builtin::Bool),
        "str" => Some(Builtin::Str),
        "list" => Some(Builtin::List),
        "map" => Some(Builtin::Map),
        _ => None,
    }
}

/// Verifies `doc` against a standalone schema document: a bare KVD tree
/// whose leaf values are builtin type names or the `{}`/`[]` literals.
/// Metakeys in the schema are an error — `__schema__` belongs in data
/// documents (spec §4).
///
/// Returns `Ok(())` when the document conforms. A malformed schema yields
/// [`VerifyError::SchemaMalformed`] (distinct from [`VerifyError::Violations`],
/// which covers a well-formed schema applied to a non-conforming document); a
/// parse failure of either input is [`VerifyError::ParseDoc`] /
/// [`VerifyError::ParseSchema`].
///
/// Metakeys at the data root (`__schema__`) are excluded from the check —
/// they are not part of the data model (spec §2) — so a document with an
/// embedded schema can also be verified against an external one.
pub fn verify(doc: &Node, schema: &Node) -> Result<(), VerifyError> {
    // A malformed schema cannot meaningfully check a document, so validate
    // the schema first and report its problems distinctly (spec §8.3).
    let mut schema_issues = Vec::new();
    reject_schema_metakeys(schema, &mut schema_issues);
    if schema_issues.is_empty() {
        validate_schema(schema, "", &mut schema_issues);
    }
    if !schema_issues.is_empty() {
        return Err(VerifyError::SchemaMalformed(schema_issues));
    }
    let mut out = Vec::new();
    let data = strip_data_metakeys(doc);
    check(schema, &data, "", &mut out);
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

/// Collects structural problems in a schema tree (spec §5). A well-formed
/// schema is a bare tree whose leaves are builtin type names, `{}`/`[]`
/// literals, or descriptor maps carrying a `type`. Problems found here are
/// surfaced as [`VerifyError::SchemaMalformed`], separate from document
/// violations.
fn validate_schema(schema: &Node, path: &str, out: &mut Vec<Violation>) {
    match schema {
        Node::Scalar(_) => match descriptor(schema) {
            Some((name, _)) if name == "list" || name == "map" => out.push(Violation::new(
                path,
                "`list`/`map` may only appear in a descriptor (`type: list` / `type: map`)",
            )),
            Some((name, _)) if builtin(&name).is_none() => {
                out.push(Violation::new(path, format!("unknown type `{name}`")))
            }
            None => out.push(Violation::new(
                path,
                "schema leaf must be a type name or `{}`/`[]`",
            )),
            _ => {}
        },
        Node::Map(m) => {
            if m.is_empty() {
                return; // `{}` leaf: any map, well-formed.
            }
            if let Some((name, _)) = descriptor(schema) {
                // Descriptor leaf.
                if name == "list" {
                    match m.get("element") {
                        None => {
                            out.push(Violation::new(path, "type: list requires an `element` key"))
                        }
                        Some(e) => validate_schema(e, &join(path, "element"), out),
                    }
                } else if name == "map" {
                    // `type: map` takes no further structure.
                } else if builtin(&name).is_none() {
                    out.push(Violation::new(path, format!("unknown type `{name}`")));
                }
                return;
            }
            if m.get("optional").is_some() || m.get("validation").is_some() {
                out.push(Violation::new(path, "descriptor requires a `type` key"));
                return;
            }
            for (k, sub) in m.iter() {
                validate_schema(sub, &join(path, k), out);
            }
        }
        Node::List(items) => {
            if items.is_empty() {
                return; // `[]` leaf: any list, well-formed.
            }
            if items.len() != 1 {
                out.push(Violation::new(
                    path,
                    format!(
                        "schema list must declare exactly one element type, found {}",
                        items.len()
                    ),
                ));
                return;
            }
            validate_schema(&items[0], &format!("{path}[0]"), out);
        }
    }
}

/// Verifies `doc` against its own embedded schema — the `__schema__` entry at
/// the document root (spec §8.1). A document with no `__schema__` carries no
/// constraints and verifies successfully.
///
/// The embedded schema is checked by the same rules as a standalone one, so a
/// malformed embedded schema is reported as [`VerifyError::SchemaMalformed`]
/// rather than silently ignored.
pub fn verify_embedded(doc: &Node) -> Result<(), VerifyError> {
    let Some(schema) = doc.as_map().and_then(|m| m.get("__schema__")) else {
        return Ok(());
    };
    verify(doc, schema)
}

/// Parses a data document that may carry an embedded `__schema__` and verifies
/// it against that schema. The one-call form of
/// [`crate::deserialize::from_str`] + [`verify_embedded`].
pub fn verify_embedded_from_str(doc: &str) -> Result<(), VerifyError> {
    let d = crate::deserialize::from_str(doc).map_err(VerifyError::ParseDoc)?;
    verify_embedded(&d)
}

/// A standalone schema document must be bare: any metakey at its root is
/// a wrapped-form leftover and an error (spec §4).
fn reject_schema_metakeys(schema: &Node, out: &mut Vec<Violation>) {
    if let Some(m) = schema.as_map() {
        for (k, _) in m.iter() {
            if crate::grammar::is_metakey(k) {
                out.push(Violation::new(
                    k,
                    "metakey in a standalone schema document \
                     (`__schema__` belongs in data documents)",
                ));
            }
        }
    }
}

/// Metakeys are not data (spec §2); drop them from a data document's root
/// before checking it against an external schema.
fn strip_data_metakeys(doc: &Node) -> Node {
    match doc.as_map() {
        Some(m) if m.iter().any(|(k, _)| crate::grammar::is_metakey(k)) => {
            let mut filtered = Map::new();
            for (k, v) in m.iter() {
                if !crate::grammar::is_metakey(k) {
                    filtered.insert(k.to_string(), v.clone());
                }
            }
            Node::map(filtered)
        }
        _ => doc.clone(),
    }
}

/// Extracts an unquoted type-name word from a schema scalar (spec §5).
/// Quoted strings, numbers, and bools are errors in schema position.
/// Optionality is expressed separately with `optional: true` in a
/// descriptor block, not with a `?` suffix.
fn type_ref(s: &Scalar) -> Option<String> {
    if s.shape != Shape::Str {
        return None;
    }
    // Quoted strings are not valid type names in schema position.
    if s.raw.starts_with('"') {
        return None;
    }
    if is_type_name(&s.text) {
        Some(s.text.clone())
    } else {
        None
    }
}

/// Resolves a schema leaf to its builtin type name and optionality.
///
/// A leaf is either a bare type-name scalar (`int`) or a descriptor map
/// (`type: int`, optionally `optional: true`) — spec §5, §10. Returns
/// `None` for `{}`/`[]` leaves and for nested-schema maps/lists.
fn descriptor(schema: &Node) -> Option<(String, bool)> {
    match schema {
        Node::Scalar(s) => type_ref(s).map(|name| (name, false)),
        Node::Map(m) => {
            // Descriptor iff it carries a `type` key.
            let Node::Scalar(t) = m.get("type")? else {
                return None;
            };
            let name = type_ref(t)?;
            let optional = matches!(
                m.get("optional"),
                Some(Node::Scalar(o)) if o.shape == Shape::Bool && o.text == "true"
            );
            Some((name, optional))
        }
        _ => None,
    }
}

/// Validates `data` against a schema leaf descriptor (spec §5).
///
/// A descriptor is a map carrying a `type` key. Container types `list` and
/// `map` may carry `optional` and are dispatched to [`check_list`] /
/// [`check_map`]; scalar types defer to [`check_shape`].
fn check_descriptor(schema: &Node, data: &Node, path: &str, out: &mut Vec<Violation>) {
    let Some((name, optional)) = descriptor(schema) else {
        return;
    };
    // `null` is valid only under an optional type (spec §5).
    if matches!(data.as_scalar(), Some(sc) if sc.shape == Shape::Null) {
        if !optional {
            out.push(Violation::new(
                path,
                "null requires an optional type (`optional: true`)",
            ));
        }
        return;
    }
    match name.as_str() {
        "list" => check_list(schema, data, path, out),
        "map" => check_map(schema, data, path, out),
        _ => match builtin(&name) {
            Some(base) => check_shape(base, data, path, out),
            None => out.push(Violation::new(path, format!("unknown type `{name}`"))),
        },
    }
}

/// Verifies a `type: list` descriptor: `data` is a list whose every item
/// matches the required `element` type (spec §5). The `element` type is
/// uniform across all items.
fn check_list(schema: &Node, data: &Node, path: &str, out: &mut Vec<Violation>) {
    let Some(items) = data.as_list() else {
        out.push(Violation::new(
            path,
            format!("expected a list, found {}", kind_of(data)),
        ));
        return;
    };
    let Some(element) = schema.as_map().and_then(|m| m.get("element")) else {
        out.push(Violation::new(path, "type: list requires an `element` key"));
        return;
    };
    for (i, item) in items.iter().enumerate() {
        check(element, item, &format!("{path}[{i}]"), out);
    }
}

/// Verifies a `type: map` descriptor: `data` is a map (spec §5). Typed maps
/// are written with the nested sub-schema form, so field contents are not
/// checked here.
fn check_map(_schema: &Node, data: &Node, path: &str, out: &mut Vec<Violation>) {
    if data.as_map().is_none() {
        out.push(Violation::new(
            path,
            format!("expected a map, found {}", kind_of(data)),
        ));
    }
}

/// Recursively checks `data` against `schema`, appending violations.
fn check(schema: &Node, data: &Node, path: &str, out: &mut Vec<Violation>) {
    match schema {
        Node::Map(m) => {
            if m.is_empty() {
                // `{}` leaf: any map, contents unchecked.
                if data.as_map().is_none() {
                    out.push(Violation::new(
                        path,
                        format!("expected a map, found {}", kind_of(data)),
                    ));
                }
                return;
            }
            // Descriptor leaf? (a map with a `type` key — spec §5, §10).
            if descriptor(schema).is_some() {
                check_descriptor(schema, data, path, out);
                return;
            }
            // A map carrying `optional`/`validation` but no `type` is a
            // malformed descriptor (spec §10).
            if m.get("optional").is_some() || m.get("validation").is_some() {
                out.push(Violation::new(path, "descriptor requires a `type` key"));
                return;
            }
            let Some(dm) = data.as_map() else {
                out.push(Violation::new(
                    path,
                    format!("expected a map, found {}", kind_of(data)),
                ));
                return;
            };
            for (k, _) in dm.iter() {
                if m.get(k).is_none() {
                    out.push(Violation::new(join(path, k), "unknown key (not in schema)"));
                }
            }
            for (k, sub) in m.iter() {
                match dm.get(k) {
                    None => {
                        if !is_optional_leaf(sub) {
                            out.push(Violation::new(join(path, k), "missing key"));
                        }
                    }
                    Some(dsub) => check(sub, dsub, &join(path, k), out),
                }
            }
        }
        Node::List(items) => {
            if items.is_empty() {
                // `[]` leaf: any list, items unchecked.
                if data.as_list().is_none() {
                    out.push(Violation::new(
                        path,
                        format!("expected a list, found {}", kind_of(data)),
                    ));
                }
                return;
            }
            if items.len() != 1 {
                out.push(Violation::new(
                    path,
                    format!(
                        "schema list must declare exactly one element type, found {}",
                        items.len()
                    ),
                ));
                return;
            }
            let Some(dl) = data.as_list() else {
                out.push(Violation::new(
                    path,
                    format!("expected a list, found {}", kind_of(data)),
                ));
                return;
            };
            for (i, item) in dl.iter().enumerate() {
                check(&items[0], item, &format!("{path}[{i}]"), out);
            }
        }
        Node::Scalar(s) => {
            let Some(name) = type_ref(s) else {
                out.push(Violation::new(
                    path,
                    "schema leaf must be a type name or `{}`/`[]`",
                ));
                return;
            };
            if name == "list" || name == "map" {
                out.push(Violation::new(
                    path,
                    "`list`/`map` may only appear in a descriptor (`type: list` / `type: map`)",
                ));
                return;
            }
            check_descriptor(schema, data, path, out);
        }
    }
}

/// Checks a data node against a resolved builtin type.
fn check_shape(base: Builtin, data: &Node, path: &str, out: &mut Vec<Violation>) {
    let expected = match base {
        Builtin::Int => "int",
        Builtin::Float => "float",
        Builtin::Bool => "bool",
        Builtin::Str => "string",
        Builtin::List | Builtin::Map => {
            unreachable!("container shapes handled by check_list/check_map")
        }
    };
    let Some(s) = data.as_scalar() else {
        out.push(Violation::new(
            path,
            format!("expected {expected}, found {}", kind_of(data)),
        ));
        return;
    };
    let ok = match base {
        Builtin::Int => s.shape == Shape::Int,
        Builtin::Float => s.shape == Shape::Float,
        Builtin::Bool => s.shape == Shape::Bool,
        Builtin::Str => s.shape == Shape::Str,
        Builtin::List | Builtin::Map => {
            unreachable!("container shapes handled by check_list/check_map")
        }
    };
    if !ok {
        out.push(Violation::new(
            path,
            format!("expected {expected}, found {}", shape_name(s.shape)),
        ));
    }
}

/// Whether a schema node declares an optional leaf: a descriptor with
/// `optional: true` (spec §5). `{}`/`[]` leaves and interior nodes are
/// always required.
fn is_optional_leaf(schema: &Node) -> bool {
    matches!(descriptor(schema), Some((_, true)))
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn kind_of(node: &Node) -> &'static str {
    match node {
        Node::Scalar(_) => "a scalar",
        Node::Map(_) => "a map",
        Node::List(_) => "a list",
    }
}

fn shape_name(shape: Shape) -> &'static str {
    match shape {
        Shape::Int => "int",
        Shape::Float => "float",
        Shape::Bool => "bool",
        Shape::Str => "string",
        Shape::Null => "null",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deserialize;

    fn ok(doc: &str, schema: &str) {
        let d = deserialize::from_str(doc).expect("doc parses");
        let s = deserialize::from_str(schema).expect("schema parses");
        let r = verify(&d, &s);
        assert!(r.is_ok(), "expected no violations, got {:?}", r.err());
    }

    fn errs(doc: &str, schema: &str) -> Vec<Violation> {
        let d = deserialize::from_str(doc).expect("doc parses");
        let s = deserialize::from_str(schema).expect("schema parses");
        match verify(&d, &s).expect_err("expected violations") {
            VerifyError::Violations(v) | VerifyError::SchemaMalformed(v) => v,
            other => panic!("expected violations, got {other:?}"),
        }
    }

    #[test]
    fn scalar_types_match() {
        ok(
            "i: 42\nf: 0.75\nb: true\ns: \"hello\"\nq: \"42\"\n",
            "i: int\nf: float\nb: bool\ns: str\nq: str\n",
        );
    }

    #[test]
    fn scalar_type_mismatches() {
        assert_eq!(
            errs("a: \"hello\"\n", "a: int\n"),
            vec![Violation::new("a", "expected int, found string")]
        );
        assert_eq!(
            errs("a: 42\n", "a: bool\n"),
            vec![Violation::new("a", "expected bool, found int")]
        );
        // int is not a float: shapes are exact.
        assert_eq!(
            errs("a: 42\n", "a: float\n"),
            vec![Violation::new("a", "expected float, found int")]
        );
    }

    #[test]
    fn scalar_where_map_expected() {
        assert_eq!(
            errs("a: 1\n", "a:\n  b: int\n"),
            vec![Violation::new("a", "expected a map, found a scalar")]
        );
    }

    #[test]
    fn unknown_and_missing_keys() {
        let v = errs("a: 1\nextra: 2\n", "a: int\nmissing: int\n");
        assert!(v.contains(&Violation::new("extra", "unknown key (not in schema)")));
        assert!(v.contains(&Violation::new("missing", "missing key")));
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn optional_keys_may_be_absent() {
        // Absent under an optional descriptor: fine. Absent under `T`: missing.
        ok("a: 1\n", "a: int\nb:\n  type: int\n  optional: true\n");
        assert_eq!(
            errs("a: 1\n", "a: int\nb: int\n"),
            vec![Violation::new("b", "missing key")]
        );
    }

    #[test]
    fn optional_types_accept_null_and_base() {
        ok(
            "a: null\nb: 42\nc: null\n",
            "a:\n  type: int\n  optional: true\nb:\n  type: int\n  optional: true\nc:\n  type: str\n  optional: true\n",
        );
        // Null under a required type is an error, with or without an optional
        // descriptor elsewhere in the schema.
        assert_eq!(
            errs("a: null\n", "a: int\n"),
            vec![Violation::new(
                "a",
                "null requires an optional type (`optional: true`)"
            )]
        );
    }

    #[test]
    fn null_in_lists_requires_optional_element() {
        ok(
            "l:\n  - 1\n  - null\n",
            "l:\n  - type: int\n    optional: true\n",
        );
        assert_eq!(
            errs("l:\n  - null\n", "l:\n  - int\n"),
            vec![Violation::new(
                "l[0]",
                "null requires an optional type (`optional: true`)"
            )]
        );
    }

    #[test]
    fn null_where_map_or_list_expected_is_an_error() {
        let v = errs("a: null\n", "a:\n  b: int\n");
        assert_eq!(v.len(), 1);
        assert!(
            v[0].message.contains("expected a map"),
            "{:?}",
            v[0].message
        );
    }

    #[test]
    fn nested_paths() {
        assert_eq!(
            errs("server:\n  port: \"http\"\n", "server:\n  port: int\n"),
            vec![Violation::new("server.port", "expected int, found string")]
        );
    }

    #[test]
    fn empty_map_leaf_accepts_any_map() {
        ok("a: {}\n", "a: {}\n");
        ok("a:\n  x: 1\n  y:\n    z: \"deep\"\n", "a: {}\n");
        // ...but not a non-map.
        assert_eq!(
            errs("a: []\n", "a: {}\n"),
            vec![Violation::new("a", "expected a map, found a list")]
        );
    }

    #[test]
    fn empty_list_leaf_accepts_any_list() {
        ok("a: []\n", "a: []\n");
        ok("a:\n  - 1\n  - \"x\"\n  - {}\n", "a: []\n");
        assert_eq!(
            errs("a: {}\n", "a: []\n"),
            vec![Violation::new("a", "expected a list, found a map")]
        );
    }

    #[test]
    fn list_element_type_checks_every_item() {
        ok("ports:\n  - 80\n  - 443\n", "ports:\n  - int\n");
        assert_eq!(
            errs(
                "ports:\n  - 80\n  - \"http\"\n  - 443\n",
                "ports:\n  - int\n"
            ),
            vec![Violation::new("ports[1]", "expected int, found string")]
        );
    }

    #[test]
    fn list_of_maps_and_empty_literals() {
        ok(
            "eps:\n  - path: \"/a\"\n    port: 80\n  - path: \"/b\"\n    port: 443\n",
            "eps:\n  - path: str\n    port: int\n",
        );
        // `- {}` element type accepts any map item.
        ok("eps:\n  - k: \"v\"\n", "eps:\n  - {}\n");
    }

    #[test]
    fn type_list_descriptor_checks_items() {
        ok(
            "ports:\n  - 80\n  - 443\n",
            "ports:\n  type: list\n  element: int\n",
        );
        assert_eq!(
            errs(
                "ports:\n  - 80\n  - \"http\"\n",
                "ports:\n  type: list\n  element: int\n"
            ),
            vec![Violation::new("ports[1]", "expected int, found string")]
        );
        // Nested container element type.
        ok(
            "matrix:\n  - - 1\n    - 2\n  - - 3\n    - 4\n",
            "matrix:\n  type: list\n  element:\n    type: list\n    element: int\n",
        );
    }

    #[test]
    fn type_list_requires_element() {
        assert_eq!(
            errs("a:\n  - 1\n", "a:\n  type: list\n"),
            vec![Violation::new("a", "type: list requires an `element` key")]
        );
    }

    #[test]
    fn type_map_descriptor_accepts_any_map() {
        ok("cfg:\n  x: 1\n", "cfg:\n  type: map\n");
        assert_eq!(
            errs("cfg: 1\n", "cfg:\n  type: map\n"),
            vec![Violation::new("cfg", "expected a map, found a scalar")]
        );
    }

    #[test]
    fn optional_containers_accept_null_absence_or_empty() {
        // Optional list: absent, null, empty, or populated all pass.
        ok(
            "items: null\n",
            "items:\n  type: list\n  element: int\n  optional: true\n",
        );
        ok(
            "items: []\n",
            "items:\n  type: list\n  element: int\n  optional: true\n",
        );
        ok(
            "items:\n  - 1\n",
            "items:\n  type: list\n  element: int\n  optional: true\n",
        );
        // Required list rejects null.
        assert_eq!(
            errs("items: null\n", "items:\n  type: list\n  element: int\n"),
            vec![Violation::new(
                "items",
                "null requires an optional type (`optional: true`)"
            )]
        );
        // Optional map.
        ok("cfg: null\n", "cfg:\n  type: map\n  optional: true\n");
    }

    #[test]
    fn bare_list_map_leaf_is_an_error() {
        assert_eq!(
            errs("a: 1\n", "a: list\n"),
            vec![Violation::new(
                "a",
                "`list`/`map` may only appear in a descriptor (`type: list` / `type: map`)"
            )]
        );
        assert_eq!(
            errs("a: {}\n", "a: map\n"),
            vec![Violation::new(
                "a",
                "`list`/`map` may only appear in a descriptor (`type: list` / `type: map`)"
            )]
        );
    }

    #[test]
    fn schema_list_must_declare_one_element() {
        // `[]` leaf means any list — fine.
        ok("a:\n  - 1\n", "a: []\n");
        let s = deserialize::from_str("a:\n  - int\n  - str\n").unwrap();
        let d = deserialize::from_str("a:\n  - 1\n").unwrap();
        assert_eq!(
            verify(&d, &s).unwrap_err(),
            VerifyError::SchemaMalformed(vec![Violation::new(
                "a",
                "schema list must declare exactly one element type, found 2"
            )])
        );
    }

    #[test]
    fn unknown_type_in_schema() {
        let d = deserialize::from_str("p: 1\n").unwrap();
        let s = deserialize::from_str("p: port\n").unwrap();
        assert_eq!(
            verify(&d, &s).unwrap_err(),
            VerifyError::SchemaMalformed(vec![Violation::new("p", "unknown type `port`")])
        );
    }

    #[test]
    fn quoted_or_numbered_schema_leaves_are_errors() {
        let d = deserialize::from_str("p: 1\n").unwrap();
        for leaf in ["\"int\"", "42"] {
            let text = format!("p: {leaf}\n");
            let s = deserialize::from_str(&text).unwrap();
            assert_eq!(
                verify(&d, &s).unwrap_err(),
                VerifyError::SchemaMalformed(vec![Violation::new(
                    "p",
                    "schema leaf must be a type name or `{}`/`[]`"
                )]),
                "leaf {leaf}"
            );
        }
    }

    #[test]
    fn standalone_schema_must_be_bare() {
        let d = deserialize::from_str("p: 1\n").unwrap();

        // Wrapped form is an error outside data documents.
        let wrapped = "__schema__:\n  p: int\n";
        {
            let s = deserialize::from_str(wrapped).unwrap();
            let v = verify(&d, &s).unwrap_err();
            let violations = match v {
                VerifyError::SchemaMalformed(v) => v,
                other => panic!("expected SchemaMalformed, got {other:?}"),
            };
            assert!(
                violations
                    .iter()
                    .all(|x| x.message.contains("metakey in a standalone")),
                "{wrapped}: {violations:?}"
            );
        }
    }

    #[test]
    fn dotted_schema_matches_nested_data() {
        // Dotted spellings normalize to the same tree on both sides.
        ok(
            "server.port: 8080\nserver.host: \"localhost\"\n",
            "server.port: int\nserver.host: str\n",
        );
        ok("server:\n  port: 8080\n", "server.port: int\n");
    }

    #[test]
    fn verify_from_str_one_call() {
        assert!(verify_from_str("p: 8080\n", "p: int\n").is_ok());
        match verify_from_str("p: \"http\"\n", "p: int\n") {
            Err(VerifyError::Violations(vs)) => {
                assert_eq!(vs, vec![Violation::new("p", "expected int, found string")]);
            }
            other => panic!("expected violations, got {other:?}"),
        }
    }

    #[test]
    fn verify_from_str_reports_parse_errors() {
        // Bad document text.
        match verify_from_str("a:\n  b\n", "p: int\n") {
            Err(VerifyError::ParseDoc(_)) => {}
            other => panic!("expected ParseDoc, got {other:?}"),
        }
        // Bad schema text (non-empty flow collection is not KVD).
        match verify_from_str("a: 1\n", "s: [1]\n") {
            Err(VerifyError::ParseSchema(_)) => {}
            other => panic!("expected ParseSchema, got {other:?}"),
        }
    }

    #[test]
    fn verify_error_display() {
        let e = verify_from_str("p: \"http\"\n", "p: int\n").unwrap_err();
        assert_eq!(e.to_string(), "p: expected int, found string\n");
        let e = verify_from_str("a:\n  b\n", "p: int\n").unwrap_err();
        assert!(e.to_string().starts_with("document parse error:"));
    }

    #[test]
    fn verify_embedded_uses_root_schema() {
        let d = deserialize::from_str("__schema__:\n  port: int\nport: 8080\n").unwrap();
        assert!(verify_embedded(&d).is_ok());
        // A schema violation against the embedded schema is reported.
        let bad = deserialize::from_str("__schema__:\n  port: int\nport: \"http\"\n").unwrap();
        assert!(verify_embedded(&bad).is_err());
        // Embedded schemas with container descriptors round-trip through emit.
        let s =
            "__schema__:\n  ports:\n    type: list\n    element: int\nports:\n  - 80\n  - 443\n";
        assert!(verify_embedded_from_str(s).is_ok());
    }

    #[test]
    fn verify_embedded_absent_is_ok() {
        let d = deserialize::from_str("port: 8080\n").unwrap();
        assert!(verify_embedded(&d).is_ok());
    }

    #[test]
    fn malformed_schema_is_distinct_from_doc_violations() {
        let d = deserialize::from_str("p: 1\n").unwrap();
        // Unknown type name: the schema itself is malformed.
        let s = deserialize::from_str("p: port\n").unwrap();
        match verify(&d, &s) {
            Err(VerifyError::SchemaMalformed(v)) => {
                assert_eq!(v, vec![Violation::new("p", "unknown type `port`")])
            }
            other => panic!("expected SchemaMalformed, got {other:?}"),
        }
        // Quoted type leaf is also a malformed schema.
        let s = deserialize::from_str("p: \"int\"\n").unwrap();
        assert!(matches!(
            verify(&d, &s),
            Err(VerifyError::SchemaMalformed(_))
        ));
        // A well-formed schema with a bad document is a plain Violations.
        let s = deserialize::from_str("p: int\n").unwrap();
        let d = deserialize::from_str("p: \"http\"\n").unwrap();
        match verify(&d, &s) {
            Err(VerifyError::Violations(v)) => {
                assert_eq!(v, vec![Violation::new("p", "expected int, found string")])
            }
            other => panic!("expected Violations, got {other:?}"),
        }
    }
}
