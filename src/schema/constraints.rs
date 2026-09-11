//! Validation constraint enforcement (spec §10).
//!
//! String length is Unicode scalar count, list/dict length is element count.

use super::{Builtin, Violation};
use crate::value::{Node, Shape};
use alloc::vec::Vec;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Global cache for compiled regexes: anchored pattern to Regex.
static REGEX_CACHE: OnceLock<Mutex<HashMap<String, regex::Regex>>> = OnceLock::new();

/// Maximum compiled patterns retained.
const REGEX_CACHE_CAP: usize = 64;

pub(crate) fn regex_for_pattern(pattern: &str) -> Result<regex::Regex, regex::Error> {
    let anchored = format!("^(?:{pattern})$");
    let cache = REGEX_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let map = cache.lock().unwrap();
        if let Some(re) = map.get(&anchored) {
            return Ok(re.clone());
        }
    }
    let re = regex::Regex::new(&anchored)?;
    let mut map = cache.lock().unwrap();
    if map.len() >= REGEX_CACHE_CAP {
        map.clear();
    }
    map.insert(anchored, re.clone());
    Ok(re)
}

// Validation constraint enforcement (spec §10)
//
// String length is Unicode scalar count (`chars().count()`), not bytes;
// `list`/`dict` length is element/entry count. This matches the spec table
// “string length ≥ min_len” and “collection length (key count)”.

pub(crate) fn check_validation_for_scalar(
    schema: &Node,
    data: &Node,
    builtin: Builtin,
    path: &str,
    out: &mut Vec<Violation>,
) {
    let Some(s) = data.as_scalar() else {
        return;
    };
    let Some(vmap) = schema
        .as_map()
        .and_then(|m| m.get("validation"))
        .and_then(|n| n.as_map())
    else {
        return;
    };
    match builtin {
        Builtin::Int => {
            for (k, v) in vmap.iter() {
                let bound = match v.as_scalar() {
                    Some(b) => b.text.clone(),
                    None => continue,
                };
                check_numeric_bound(cmp_int(&s.text, &bound), k, &s.text, &bound, path, out);
            }
        }
        Builtin::Float => {
            // Float grammar forbids '_' (spec §2). For int-shaped bounds on a
            // float type, underscores are allowed (e.g. `min: 1_000` for float),
            // so strip underscores only for Int-shaped bounds.
            let data_f: f64 = s.text.parse().unwrap_or(f64::NAN);
            if data_f.is_nan() {
                return;
            }
            for (k, v) in vmap.iter() {
                let bound_sc = match v.as_scalar() {
                    Some(b) => b,
                    None => continue,
                };
                let bound = bound_sc.text.clone();
                let bound_f: f64 = if bound_sc.shape == Shape::Int {
                    bound.replace('_', "").parse().unwrap_or(f64::NAN)
                } else {
                    bound.parse().unwrap_or(f64::NAN)
                };
                if bound_f.is_nan() {
                    continue;
                }
                check_numeric_bound(
                    data_f
                        .partial_cmp(&bound_f)
                        .unwrap_or(core::cmp::Ordering::Equal),
                    k,
                    &s.text,
                    &bound,
                    path,
                    out,
                );
            }
        }
        Builtin::Str => {
            let len = s.text.chars().count() as i64;
            for (k, v) in vmap.iter() {
                match k {
                    "min_len" | "max_len" => {
                        check_len_bound(len, k, v, path, out);
                    }
                    "pattern" => {
                        let pat = match v.as_scalar() {
                            Some(sc) => sc.text.clone(),
                            None => continue,
                        };
                        match regex_for_pattern(&pat) {
                            Ok(re) => {
                                if !re.is_match(&s.text) {
                                    out.push(Violation::new(
                                        path,
                                        format!(
                                            "value \"{}\" does not match pattern \"{}\"",
                                            s.text, pat
                                        ),
                                    ));
                                }
                            }
                            Err(_) => {
                                out.push(Violation::new(
                                    path,
                                    format!("invalid pattern \"{}\"", pat),
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn check_validation_for_list_or_dict(
    schema: &Node,
    data: &Node,
    builtin: Builtin,
    path: &str,
    out: &mut Vec<Violation>,
) {
    let Some(vmap) = schema
        .as_map()
        .and_then(|m| m.get("validation"))
        .and_then(|n| n.as_map())
    else {
        return;
    };
    let len: i64 = match builtin {
        Builtin::List => data.as_list().map(|l| l.len() as i64).unwrap_or(0),
        Builtin::Dict => data.as_keyed().map(|m| m.len() as i64).unwrap_or(0),
        _ => return,
    };
    for (k, v) in vmap.iter() {
        if k == "min_len" || k == "max_len" {
            check_len_bound(len, k, v, path, out);
        }
    }
}

/// Checks one numeric bound (`min`/`max`/`exclusive_min`/`exclusive_max`)
/// against an [`Ordering`], pushing a violation on failure. Both int
/// ([`cmp_int`]) and float (`partial_cmp`) comparisons funnel through here
/// so the four messages live in one place.
pub(crate) fn check_numeric_bound(
    ord: core::cmp::Ordering,
    k: &str,
    value_text: &str,
    bound_text: &str,
    path: &str,
    out: &mut Vec<Violation>,
) {
    use core::cmp::Ordering::{Greater, Less};
    let fail = match k {
        "min" => ord == Less,
        "max" => ord == Greater,
        "exclusive_min" => ord != Greater,
        "exclusive_max" => ord != Less,
        _ => return,
    };
    if !fail {
        return;
    }
    let msg = match k {
        "min" => format!("value {value_text} is less than min {bound_text}"),
        "max" => format!("value {value_text} exceeds max {bound_text}"),
        "exclusive_min" => {
            format!("value {value_text} must be greater than exclusive_min {bound_text}")
        }
        _ => format!("value {value_text} must be less than exclusive_max {bound_text}"),
    };
    out.push(Violation::new(path, msg));
}

/// Checks one `min_len`/`max_len` bound against `len`, pushing a violation
/// on failure. Malformed (negative/unparseable) bounds are skipped here;
/// the schema-shape pass reports them as malformed.
pub(crate) fn check_len_bound(len: i64, k: &str, v: &Node, path: &str, out: &mut Vec<Violation>) {
    let bound: i64 = v
        .as_scalar()
        .map(|sc| sc.text.replace('_', "").parse::<i64>().unwrap_or(-1))
        .unwrap_or(-1);
    if bound < 0 {
        return;
    }
    if k == "min_len" && len < bound {
        out.push(Violation::new(
            path,
            format!("length {len} is less than min_len {bound}"),
        ));
    } else if k == "max_len" && len > bound {
        out.push(Violation::new(
            path,
            format!("length {len} exceeds max_len {bound}"),
        ));
    }
}

/// Compare two int literal texts (may contain '_' and sign) as integers.
/// Returns Ordering.
pub(crate) fn cmp_int(a: &str, b: &str) -> core::cmp::Ordering {
    let a_clean = a.replace('_', "");
    let b_clean = b.replace('_', "");
    // Normalize sign and digits
    let (a_neg, a_digits) = split_sign(&a_clean);
    let (b_neg, b_digits) = split_sign(&b_clean);
    let a_norm = normalize_digits(a_digits);
    let b_norm = normalize_digits(b_digits);
    // Both zero? Treat -0 == 0
    let a_is_zero = a_norm == "0";
    let b_is_zero = b_norm == "0";
    let a_neg = if a_is_zero { false } else { a_neg };
    let b_neg = if b_is_zero { false } else { b_neg };
    match (a_neg, b_neg) {
        (true, false) => return core::cmp::Ordering::Less,
        (false, true) => return core::cmp::Ordering::Greater,
        (true, true) => {
            // both negative: larger magnitude is smaller
            return cmp_abs(b_norm, a_norm);
        }
        (false, false) => {}
    }
    cmp_abs(a_norm, b_norm)
}

fn split_sign(s: &str) -> (bool, &str) {
    if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = s.strip_prefix('+') {
        (false, rest)
    } else {
        (false, s)
    }
}

fn normalize_digits(s: &str) -> &str {
    let trimmed = s.trim_start_matches('0');
    if trimmed.is_empty() { "0" } else { trimmed }
}

fn cmp_abs(a: &str, b: &str) -> core::cmp::Ordering {
    if a.len() != b.len() {
        return a.len().cmp(&b.len());
    }
    a.cmp(b)
}
