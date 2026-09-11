//! Document against schema checking (spec §5).

use super::constraints::{check_validation_for_list_or_dict, check_validation_for_scalar};
use super::types::{descriptor, is_optional_leaf, kind_of, shape_name, type_ref};
use super::{Builtin, Violation, builtin};
use crate::value::{Node, Shape, join_path};
use alloc::vec::Vec;

/// Validates `data` against a schema leaf descriptor (spec §5, §10).
///
/// A descriptor is a map carrying a `type` key. Container types `list` and
/// `dict` may carry `optional` and are dispatched to [`check_list`] /
/// [`check_dict`]; scalar types defer to [`check_shape`] followed by constraint
/// checks.
pub(crate) fn check_descriptor(schema: &Node, data: &Node, path: &str, out: &mut Vec<Violation>) {
    let Some((name, optional)) = descriptor(schema) else {
        return;
    };
    // `null` is valid only under an optional type (spec §5). Constraints skipped.
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
        "dict" => check_dict(schema, data, path, out),
        _ => match builtin(&name) {
            Some(base) => {
                let before = out.len();
                check_shape(base, data, path, out);
                // Only apply constraints if shape matched (no new violations).
                if out.len() == before {
                    check_validation_for_scalar(schema, data, base, path, out);
                }
            }
            None => out.push(Violation::new(path, format!("unknown type `{name}`"))),
        },
    }
}

/// Verifies a `type: list` descriptor: `data` is a list whose every item
/// matches the required `element` type (spec §5). The `element` type is
/// uniform across all items. Also enforces `validation: {min_len,max_len}`.
pub(crate) fn check_list(schema: &Node, data: &Node, path: &str, out: &mut Vec<Violation>) {
    let Some(items) = data.as_list() else {
        out.push(Violation::new(
            path,
            format!("expected a list, found {}", kind_of(data)),
        ));
        return;
    };
    // Length constraints first (spec §10).
    check_validation_for_list_or_dict(schema, data, Builtin::List, path, out);
    let Some(element) = schema.as_map().and_then(|m| m.get("element")) else {
        out.push(Violation::new(path, "type: list requires an `element` key"));
        return;
    };
    for (i, item) in items.iter().enumerate() {
        check(element, item, &format!("{path}[{i}]"), out);
    }
}

/// Verifies a `type: dict` descriptor: `data` is a keyed collection (spec
/// §5). Without `element`, any dict passes; with `element`, every value
/// must match it (managed opt-in). Enforces length constraints.
pub(crate) fn check_dict(schema: &Node, data: &Node, path: &str, out: &mut Vec<Violation>) {
    if data.as_keyed().is_none() {
        out.push(Violation::new(
            path,
            format!("expected a dict, found {}", kind_of(data)),
        ));
        return;
    };
    check_validation_for_list_or_dict(schema, data, Builtin::Dict, path, out);
    // A `type: dict` descriptor may carry an `element` type (spec §10);
    // every dict value must match it. Without `element`, any dict passes.
    if let Some(element) = schema.as_keyed().and_then(|m| m.get("element")) {
        let Some(dm) = data.as_keyed() else { return };
        for (k, v) in dm.iter() {
            check(element, v, &join_path(path, k), out);
        }
    }
}

/// Recursively checks `data` against `schema`, appending violations.
pub(crate) fn check(schema: &Node, data: &Node, path: &str, out: &mut Vec<Violation>) {
    match schema {
        Node::Map(m) => {
            if m.is_empty() {
                // `{}` leaf: any keyed collection, contents unchecked.
                if data.as_keyed().is_none() {
                    out.push(Violation::new(
                        path,
                        format!("expected a dict, found {}", kind_of(data)),
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
            if m.get("optional").is_some()
                || m.get("description").is_some()
                || m.get("deprecated").is_some()
                || m.get("validation").is_some()
            {
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
                    out.push(Violation::new(
                        join_path(path, k),
                        "unknown key (not in schema)",
                    ));
                }
            }
            for (k, sub) in m.iter() {
                match dm.get(k) {
                    None => {
                        if !is_optional_leaf(sub) {
                            out.push(Violation::new(join_path(path, k), "missing key"));
                        }
                    }
                    Some(dsub) => check(sub, dsub, &join_path(path, k), out),
                }
            }
        }
        Node::Dict(m) => {
            // Dict form (spec §5): the data value must be a dict.
            // Declared keys present in the data are checked against
            // their type; declared keys may be absent (optional) and
            // undeclared data keys pass unchecked (any type).
            let Some(dm) = data.as_dict() else {
                out.push(Violation::new(
                    path,
                    format!("expected a dict, found {}", kind_of(data)),
                ));
                return;
            };
            for (k, sub) in m.iter() {
                if let Some(v) = dm.get(k) {
                    check(sub, v, &join_path(path, k), out);
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
            if name == "list" || name == "dict" {
                out.push(Violation::new(
                    path,
                    "`list`/`dict` may only appear in a descriptor (`type: list` / `type: dict`)",
                ));
                return;
            }
            check_descriptor(schema, data, path, out);
        }
    }
}

/// Checks a data node against a resolved builtin type.
pub(crate) fn check_shape(base: Builtin, data: &Node, path: &str, out: &mut Vec<Violation>) {
    let expected = match base {
        Builtin::Int => "int",
        Builtin::Float => "float",
        Builtin::Bool => "bool",
        Builtin::Str => "string",
        Builtin::List | Builtin::Dict => {
            unreachable!("container shapes handled by check_list/check_dict")
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
        Builtin::List | Builtin::Dict => {
            unreachable!("container shapes handled by check_list/check_dict")
        }
    };
    if !ok {
        out.push(Violation::new(
            path,
            format!("expected {expected}, found {}", shape_name(s.shape)),
        ));
    }
}
