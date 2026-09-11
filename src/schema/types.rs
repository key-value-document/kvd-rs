//! Schema leaf types: builtin names, descriptors, small helpers.

use crate::grammar::is_type_name;
use crate::value::{Node, Scalar, Shape};
use alloc::string::String;

/// Extracts an unquoted type-name word from a schema scalar (spec §5).
/// Quoted strings, numbers, and bools are errors in schema position.
/// Optionality is expressed separately with `optional: true` in a
/// descriptor block, not with a `?` suffix.
pub(crate) fn type_ref(s: &Scalar) -> Option<String> {
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
pub(crate) fn descriptor(schema: &Node) -> Option<(String, bool)> {
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

/// Whether a schema node declares an optional leaf: a descriptor with
/// `optional: true` (spec §5). `{}`/`[]` leaves and interior nodes are
/// always required.
pub(crate) fn is_optional_leaf(schema: &Node) -> bool {
    matches!(descriptor(schema), Some((_, true)))
}

pub(crate) fn kind_of(node: &Node) -> &'static str {
    match node {
        Node::Scalar(_) => "a scalar",
        Node::Map(_) => "a map",
        Node::Dict(_) => "a dict",
        Node::List(_) => "a list",
    }
}

pub(crate) fn shape_name(shape: Shape) -> &'static str {
    match shape {
        Shape::Int => "int",
        Shape::Float => "float",
        Shape::Bool => "bool",
        Shape::Str => "string",
        Shape::Null => "null",
    }
}
