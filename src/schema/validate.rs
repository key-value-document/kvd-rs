//! Schema structure validation (spec §5, S10).

use super::constraints::regex_for_pattern;
use super::types::descriptor;
use super::{Builtin, Violation, builtin};
use crate::value::{Map, Node, Shape, join_path};
use alloc::vec::Vec;

/// Collects structural problems in a schema tree (spec §5, §10). A well-formed
/// schema is a bare tree whose leaves are builtin type names, `{}`/`[]`
/// literals, or descriptor maps carrying a `type`. Problems found here are
/// surfaced as [`VerifyError::SchemaMalformed`], separate from document
/// violations.
///
/// Call with `path = ""` and an empty `out` at the root; recursion threads
/// the dotted path through nested nodes.
pub fn validate_schema(schema: &Node, path: &str, out: &mut Vec<Violation>) {
    match schema {
        Node::Scalar(_) => match descriptor(schema) {
            Some((name, _)) if name == "list" || name == "dict" => out.push(Violation::new(
                path,
                "`list`/`dict` may only appear in a descriptor (`type: list` / `type: dict`)",
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
                return; // `{}` leaf: any dict, well-formed.
            }
            if let Some((name, _)) = descriptor(schema) {
                // Descriptor leaf.
                validate_descriptor_schema(m, &name, path, out);
                return;
            }
            if m.get("optional").is_some()
                || m.get("description").is_some()
                || m.get("deprecated").is_some()
                || m.get("validation").is_some()
            {
                out.push(Violation::new(path, "descriptor requires a `type` key"));
                return;
            }
            for (k, sub) in m.iter() {
                validate_schema(sub, &join_path(path, k), out);
            }
        }
        Node::Dict(m) => {
            // Dict form (spec §5): any number of entries; each entry
            // value is validated as a type declaration for that key.
            for (k, sub) in m.iter() {
                validate_schema(sub, &join_path(path, k), out);
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

/// Validates a descriptor map's internal structure (spec §10).
pub(crate) fn validate_descriptor_schema(
    m: &Map,
    type_name: &str,
    path: &str,
    out: &mut Vec<Violation>,
) {
    // Unknown type already handled for scalar? For descriptor we check again.
    if builtin(type_name).is_none() {
        out.push(Violation::new(path, format!("unknown type `{type_name}`")));
        return;
    }
    // Check for unexpected keys at descriptor level.
    let allowed_descriptor_keys: &[&str] = if type_name == "list" || type_name == "dict" {
        &[
            "type",
            "optional",
            "description",
            "deprecated",
            "validation",
            "element",
        ]
    } else {
        &[
            "type",
            "optional",
            "description",
            "deprecated",
            "validation",
        ]
    };
    for (k, _) in m.iter() {
        if !allowed_descriptor_keys.contains(&k) {
            out.push(Violation::new(
                join_path(path, k),
                format!(
                    "unknown key `{k}` in descriptor (expected one of {})",
                    allowed_descriptor_keys.join(", ")
                ),
            ));
        }
    }
    // Validate `description` is a string if present (ignored by verification).
    if let Some(desc) = m.get("description") {
        match desc.as_scalar() {
            Some(s) if s.shape == Shape::Str => {}
            _ => out.push(Violation::new(
                join_path(path, "description"),
                "`description` must be a string",
            )),
        }
    }
    // Validate `deprecated` block if present (ignored by verification).
    if let Some(dnode) = m.get("deprecated") {
        match dnode.as_map() {
            Some(dmap) => {
                for (k, v) in dmap.iter() {
                    if k != "reason" && k != "since" {
                        out.push(Violation::new(
                            join_path(&join_path(path, "deprecated"), k),
                            format!(
                                "unknown key `{k}` in `deprecated` (expected one of reason, since)"
                            ),
                        ));
                        continue;
                    }
                    match v.as_scalar() {
                        Some(s) if s.shape == Shape::Str => {}
                        _ => out.push(Violation::new(
                            join_path(&join_path(path, "deprecated"), k),
                            format!("`deprecated.{k}` must be a string"),
                        )),
                    }
                }
            }
            None => {
                out.push(Violation::new(
                    join_path(path, "deprecated"),
                    "`deprecated` must be a dict",
                ));
            }
        }
    }
    // Validate `optional` is a bool if present.
    if let Some(opt) = m.get("optional") {
        match opt.as_scalar() {
            Some(s) if s.shape == Shape::Bool => {}
            _ => out.push(Violation::new(
                join_path(path, "optional"),
                "`optional` must be `true` or `false`",
            )),
        }
    }
    // Validate `element` for list/dict, ensure present and recursively valid.
    if type_name == "list" {
        match m.get("element") {
            None => {
                out.push(Violation::new(path, "type: list requires an `element` key"));
            }
            Some(e) => validate_schema(e, &join_path(path, "element"), out),
        }
    } else if type_name == "dict" {
        // `element` is optional for dict (spec §5: a `type: dict`
        // descriptor accepts any dict); when present it is validated and
        // enforced as the uniform value type.
        if let Some(e) = m.get("element") {
            validate_schema(e, &join_path(path, "element"), out);
        }
    } else if m.get("element").is_some() {
        // `element` only for list/dict
        out.push(Violation::new(
            join_path(path, "element"),
            "`element` is only valid for `type: list` / `type: dict`",
        ));
    }

    // Validate `validation` block if present.
    if let Some(vnode) = m.get("validation") {
        match vnode.as_map() {
            Some(vmap) => {
                validate_validation_block(vmap, type_name, &join_path(path, "validation"), out);
            }
            None => {
                out.push(Violation::new(
                    join_path(path, "validation"),
                    "`validation` must be a dict",
                ));
            }
        }
    }
}

/// Validates the contents of a `validation` map for a given builtin type (spec §10).
pub(crate) fn validate_validation_block(
    vmap: &Map,
    type_name: &str,
    path: &str,
    out: &mut Vec<Violation>,
) {
    let builtin = builtin(type_name).unwrap();
    let allowed: &[&str] = match builtin {
        Builtin::Int | Builtin::Float => &["min", "max", "exclusive_min", "exclusive_max"],
        Builtin::Str => &["min_len", "max_len", "pattern"],
        Builtin::List | Builtin::Dict => &["min_len", "max_len"],
        Builtin::Bool => &[],
    };
    for (k, v) in vmap.iter() {
        if !allowed.contains(&k) {
            out.push(Violation::new(
                join_path(path, k),
                format!("unknown constraint `{k}` for type `{type_name}`"),
            ));
            continue;
        }
        // Validate value shape per constraint.
        match k {
            "min" | "max" | "exclusive_min" | "exclusive_max" => {
                // Must be numeric scalar.
                match v.as_scalar() {
                    Some(s) if s.shape == Shape::Int || s.shape == Shape::Float => {
                        // For int type we expect int, for float we allow int or float.
                        // Enforce: int type requires int shape, float allows either.
                        if builtin == Builtin::Int && s.shape != Shape::Int {
                            out.push(Violation::new(
                                join_path(path, k),
                                format!("constraint `{k}` for `int` must be an int"),
                            ));
                        } else if builtin == Builtin::Float
                            && s.shape != Shape::Int
                            && s.shape != Shape::Float
                        {
                            out.push(Violation::new(
                                join_path(path, k),
                                format!("constraint `{k}` for `float` must be a number"),
                            ));
                        }
                        // Also check that numeric text parses. Floats forbid '_' (spec §2).
                        // For float, ensure it can parse as f64 without stripping.
                        if s.shape == Shape::Float {
                            if s.text.parse::<f64>().is_err() {
                                out.push(Violation::new(
                                    join_path(path, k),
                                    format!(
                                        "invalid float value `{}` for constraint `{k}`",
                                        s.text
                                    ),
                                ));
                            }
                        } else if s.shape == Shape::Int {
                            // Check int parses (allow underscores)
                            let clean = s.text.replace('_', "");
                            // Try to validate int grammar roughly; if it fails, report malformed.
                            if !crate::grammar::is_int(&clean) && !crate::grammar::is_int(&s.text) {
                                // Fallback: try to parse; if not int-like, still consider malformed.
                                // Use helper: try to see if it looks like int; if not, error.
                                // For now, if not parseable as i128, flag.
                                let tight = s.text.replace('_', "");
                                if tight.parse::<i128>().is_err() && !is_big_int(&tight) {
                                    out.push(Violation::new(
                                        join_path(path, k),
                                        format!(
                                            "invalid int value `{}` for constraint `{k}`",
                                            s.text
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                    _ => out.push(Violation::new(
                        join_path(path, k),
                        format!("constraint `{k}` must be a number"),
                    )),
                }
            }
            "min_len" | "max_len" => match v.as_scalar() {
                Some(s) if s.shape == Shape::Int => {
                    let clean = s.text.replace('_', "");
                    match clean.parse::<i64>() {
                        Ok(n) if n >= 0 => {}
                        Ok(_) => out.push(Violation::new(
                            join_path(path, k),
                            format!("constraint `{k}` must be a non-negative int"),
                        )),
                        Err(_) => out.push(Violation::new(
                            join_path(path, k),
                            format!("constraint `{k}` must be a non-negative int"),
                        )),
                    }
                }
                _ => out.push(Violation::new(
                    join_path(path, k),
                    format!("constraint `{k}` must be a non-negative int"),
                )),
            },
            "pattern" => {
                match v.as_scalar() {
                    Some(s) if s.shape == Shape::Str => {
                        // Full-match: wrapping in ^(?:...)$.
                        if regex_for_pattern(&s.text).is_err() {
                            out.push(Violation::new(
                                join_path(path, k),
                                format!("invalid pattern regex `{}`", s.text),
                            ));
                        }
                    }
                    _ => out.push(Violation::new(
                        join_path(path, k),
                        "constraint `pattern` must be a string",
                    )),
                }
            }
            _ => unreachable!(),
        }
    }
}

pub(crate) fn is_big_int(s: &str) -> bool {
    let t = s.trim_start_matches(['+', '-']);
    !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
}
