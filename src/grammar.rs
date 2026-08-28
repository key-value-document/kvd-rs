//! Token predicates for the KVD grammar (spec §3).
//!
//! These classify a string slice against the format's token grammar. They
//! are shared by the serializer (which re-derives spelling) and the parser
//! (which classifies tokens). They do not allocate and never fail.

/// `[A-Za-z0-9] | [A-Za-z0-9] [A-Za-z0-9_-]* [A-Za-z0-9]` (spec §3 `key`).
pub fn is_key(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    let mut last = None;
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return false;
        }
        last = Some(c);
    }
    last.is_none_or(|c| c.is_ascii_alphanumeric())
}

/// `__ [a-z] [a-z0-9_-]* __` (spec §3 `metakey`).
pub fn is_metakey(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 5 {
        return false;
    }
    if &b[0..2] != b"__" || &b[b.len() - 2..] != b"__" {
        return false;
    }
    let inner = &b[2..b.len() - 2];
    inner[0].is_ascii_lowercase()
        && inner
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
}

/// The metakey defined by the spec (§3).
pub fn is_known_metakey(s: &str) -> bool {
    matches!(s, "__schema__")
}

/// `[a-z] [a-z0-9_-]*` (spec §3 `type`; forced lowercase).
pub fn is_type_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// The six builtin type names (`int`, `float`, `bool`, `str`, `list`,
/// `map`) — spec §5. These are the only bare words permitted in value
/// position; they are meaningful only in schema position. Optionality is
/// expressed with `optional: true` in a descriptor block.
///
/// The serializer leaves these bare (never double-quoted) so that schema
/// documents round-trip through `emit` → `parse` → `verify`. A data string
/// whose text equals one of these names is also emitted bare; re-parsing it
/// yields an equal `Str` scalar, so the value round-trips.
pub fn is_builtin_type(s: &str) -> bool {
    matches!(s, "int" | "float" | "bool" | "str" | "list" | "map")
}

/// True when `s` spells an int or float literal (spec §3 `int`/`float`).
pub fn looks_like_number(s: &str) -> bool {
    is_int(s) || is_float(s)
}

/// `[+-]?0 | [+-]?[1-9][0-9]* | [+-]?[1-9][0-9]{0,2}(_[0-9]{3})+` (spec §3).
pub fn is_int(s: &str) -> bool {
    let bytes = strip_sign(s).as_bytes();
    if bytes.is_empty() {
        return false;
    }
    if bytes[0] == b'0' {
        return bytes.len() == 1;
    }
    if !(b'1'..=b'9').contains(&bytes[0]) {
        return false;
    }
    if !bytes.contains(&b'_') {
        return bytes.iter().all(|b| b.is_ascii_digit());
    }
    // Strict thousands groups: 1-3 digits, then `_` + exactly 3 digits.
    let mut i = 0;
    let mut run = 0;
    while i < bytes.len() {
        if bytes[i] == b'_' {
            if run == 0 || run > 3 {
                return false;
            }
            if i + 3 >= bytes.len() {
                return false;
            }
            if !bytes[i + 1..i + 4].iter().all(|b| b.is_ascii_digit()) {
                return false;
            }
            i += 4;
            run = 3;
            if i < bytes.len() && bytes[i] != b'_' {
                return false;
            }
            continue;
        }
        if !bytes[i].is_ascii_digit() {
            return false;
        }
        run += 1;
        i += 1;
    }
    run <= 3
}

/// `[+-]?0\.[0-9]+([eE][+-]?[0-9]+)? | [+-]?[1-9][0-9]*\.[0-9]+([eE][+-]?[0-9]+)? |
/// [+-]?[1-9][0-9]*[eE][+-]?[0-9]+ | [+-]?0[eE][+-]?[0-9]+` (spec §3).
pub fn is_float(s: &str) -> bool {
    let bytes = strip_sign(s).as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let exp_pos = bytes.iter().position(|b| *b == b'e' || *b == b'E');
    let (mantissa, exp) = match exp_pos {
        Some(i) => (&bytes[..i], Some(&bytes[i + 1..])),
        None => (bytes, None),
    };
    if let Some(e) = exp {
        let e = strip_sign_bytes(e);
        if e.is_empty() || !e.iter().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    match mantissa.iter().position(|b| *b == b'.') {
        Some(dot) => {
            let int_part = &mantissa[..dot];
            let frac_part = &mantissa[dot + 1..];
            if frac_part.is_empty() || !frac_part.iter().all(|b| b.is_ascii_digit()) {
                return false;
            }
            is_plain_int(int_part)
        }
        None => {
            if exp.is_none() {
                return false;
            }
            is_plain_int(mantissa)
        }
    }
}

/// `0 | [1-9][0-9]*` — the integer part of a float.
fn is_plain_int(bytes: &[u8]) -> bool {
    match bytes {
        [b'0'] => true,
        [first, rest @ ..] => {
            (b'1'..=b'9').contains(first) && rest.iter().all(|b| b.is_ascii_digit())
        }
        _ => false,
    }
}

fn strip_sign(s: &str) -> &str {
    s.strip_prefix('+')
        .or_else(|| s.strip_prefix('-'))
        .unwrap_or(s)
}

fn strip_sign_bytes(b: &[u8]) -> &[u8] {
    match b.first() {
        Some(b'+') | Some(b'-') => &b[1..],
        _ => b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_recognition() {
        assert!(is_int("42"));
        assert!(is_int("-5"));
        assert!(is_int("+5"));
        assert!(is_int("1_000"));
        assert!(is_int("45_678_112"));
        assert!(!is_int("1_2"));
        assert!(!is_int("1_2345"));
        assert!(!is_int("1234_567"));
        assert!(!is_int("00"));
        assert!(!is_int("0_000"));
    }

    #[test]
    fn float_recognition() {
        assert!(is_float("0.75"));
        assert!(is_float("-5.5"));
        assert!(is_float("1e3"));
        assert!(is_float("0.5e-3"));
        assert!(!is_float("5foo"));
        assert!(!is_float("1.5.5"));
        assert!(!is_float(".5"));
        assert!(!is_float("1."));
        assert!(!is_float("1e"));
    }

    #[test]
    fn word_and_key() {
        assert!(is_key("8080"));
        assert!(is_key("2fa"));
        assert!(!is_key("-foo"));
        assert!(!is_key("foo-"));
        assert!(!is_key("_x"));
        assert!(!is_key("a.b"));
    }

    #[test]
    fn metakey_and_type() {
        assert!(is_metakey("__schema__"));
        assert!(is_metakey("__my_type__"));
        assert!(is_metakey("__x__y__"));
        assert!(is_metakey("__a__")); // 5-char minimum: __ + one letter + __
        assert!(!is_metakey("__Document__"));
        assert!(!is_metakey("__x_"));
        assert!(!is_metakey("x__y__"));
        assert!(is_type_name("port"));
        assert!(is_type_name("my_type"));
        assert!(!is_type_name("Port"));
        assert!(!is_type_name("1port"));
    }
}
