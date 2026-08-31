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
        // Basic single and multi-digit without underscores.
        assert!(is_int("0"));
        assert!(is_int("-0"));
        assert!(is_int("+0"));
        assert!(is_int("42"));
        assert!(is_int("-5"));
        assert!(is_int("+5"));
        assert!(is_int("9"));
        assert!(is_int("10"));
        assert!(is_int("999"));
        assert!(is_int("1_000"));
        assert!(is_int("45_678_112"));
        // No underscore thousand grouping — plain digits valid.
        assert!(is_int("1000"));
        assert!(is_int("10000"));
        assert!(is_int("100000"));
        assert!(is_int("1000000"));
        assert!(is_int("1234567"));
        // Favored underscore grouping (1-3 digits then _000 chunks).
        assert!(is_int("10_000"));
        assert!(is_int("100_000"));
        assert!(is_int("1_000_000"));
        assert!(is_int("10_000_000"));
        assert!(is_int("100_000_000"));
        assert!(is_int("1_000_000_000"));
        assert!(is_int("999_999"));
        assert!(is_int("12_345_678"));
        // Signed with underscores.
        assert!(is_int("-1_000"));
        assert!(is_int("+1_000"));
        assert!(is_int("-1_000_000"));
        assert!(is_int("+12_345"));
        // Invalid underscore placements — must be strict thousands.
        assert!(!is_int("1_2"));
        assert!(!is_int("1_2345"));
        assert!(!is_int("1234_567"));
        assert!(!is_int("12_34"));
        assert!(!is_int("1_23"));
        assert!(!is_int("1__000"));
        assert!(!is_int("1_00"));
        assert!(!is_int("1_0000"));
        assert!(!is_int("1_00_000"));
        assert!(!is_int("100_00"));
        assert!(!is_int("1_000_00"));
        assert!(!is_int("1_000_"));
        assert!(!is_int("_1_000"));
        assert!(!is_int("1_000__000"));
        assert!(!is_int("1_000_00_000"));
        // Leading zero restrictions.
        assert!(!is_int("00"));
        assert!(!is_int("01"));
        assert!(!is_int("0_0"));
        assert!(!is_int("0_000"));
        assert!(!is_int("00_000"));
        assert!(!is_int("000"));
        assert!(!is_int("000_000"));
        // Empty / sign-only / non-numeric.
        assert!(!is_int(""));
        assert!(!is_int("+"));
        assert!(!is_int("-"));
        assert!(!is_int("_"));
        assert!(!is_int("a"));
        assert!(!is_int("1a"));
        assert!(!is_int("a1"));
        assert!(!is_int("1 000"));
        assert!(!is_int("1,000"));
        assert!(!is_int("1.0"));
        assert!(!is_int("--1"));
        assert!(!is_int("++1"));
    }

    #[test]
    fn float_recognition() {
        // Plain decimal forms.
        assert!(is_float("0.0"));
        assert!(is_float("0.00"));
        assert!(is_float("0.75"));
        assert!(is_float("1.0"));
        assert!(is_float("10.5"));
        assert!(is_float("123.456"));
        assert!(is_float("-5.5"));
        assert!(is_float("-0.5"));
        assert!(is_float("+0.5"));
        assert!(is_float("+1.0"));
        // Scientific without dot.
        assert!(is_float("1e3"));
        assert!(is_float("1E3"));
        assert!(is_float("1e+3"));
        assert!(is_float("1e-3"));
        assert!(is_float("1E+10"));
        assert!(is_float("0e10"));
        assert!(is_float("0E10"));
        assert!(is_float("0e+10"));
        assert!(is_float("0e-10"));
        assert!(is_float("123e45"));
        assert!(is_float("-10e3"));
        // Decimal + exponent.
        assert!(is_float("0.5e-3"));
        assert!(is_float("1.0e3"));
        assert!(is_float("1.5E-10"));
        assert!(is_float("123.456e78"));
        assert!(is_float("+1.5e+10"));
        assert!(is_float("-1.0e-5"));
        assert!(is_float("0.0e0"));
        // Invalid — non-numeric or malformed.
        assert!(!is_float("5foo"));
        assert!(!is_float("1.5.5"));
        assert!(!is_float(".5"));
        assert!(!is_float("1."));
        assert!(!is_float("1e"));
        assert!(!is_float("e10"));
        assert!(!is_float("1e3.5"));
        assert!(!is_float("1.0e"));
        assert!(!is_float("1.0e+"));
        assert!(!is_float("1.0E+"));
        assert!(!is_float("inf"));
        assert!(!is_float("NaN"));
        assert!(!is_float("1..0"));
        assert!(!is_float("1.0e++3"));
        assert!(!is_float("1.0e--3"));
        // Float int part must be plain (no underscores, no leading zeros).
        assert!(!is_float("1_000.5"));
        assert!(!is_float("1.00_1"));
        assert!(!is_float("00.5"));
        assert!(!is_float("01.0"));
        assert!(!is_float("1e3_000"));
        // Mantissa dot requires digits both sides.
        assert!(!is_float("0."));
        assert!(!is_float(".0"));
        assert!(!is_float(""));
        assert!(!is_float("+-1.0"));
    }

    #[test]
    fn word_and_key() {
        // Bare-key valid: single alphanumeric.
        assert!(is_key("a"));
        assert!(is_key("A"));
        assert!(is_key("0"));
        assert!(is_key("9"));
        assert!(is_key("8080"));
        assert!(is_key("2fa"));
        assert!(is_key("x"));
        assert!(is_key("ab"));
        assert!(is_key("a0"));
        // Interior dash/underscore allowed, but not leading/trailing.
        assert!(is_key("foo-bar"));
        assert!(is_key("foo_bar"));
        assert!(is_key("a-b"));
        assert!(is_key("a_b"));
        assert!(is_key("a-1"));
        assert!(is_key("a_1"));
        assert!(is_key("a-b-c"));
        assert!(is_key("a_b_c"));
        assert!(is_key("a1-b2_c3"));
        // Two-char boundary.
        assert!(is_key("ab1"));
        assert!(is_key("0a"));
        assert!(is_key("1-2")); //? Check: valid per grammar (start alnum, interior -, end alnum)
        assert!(is_key("1-2"));
        // Invalid leading/trailing.
        assert!(!is_key("-foo"));
        assert!(!is_key("_foo"));
        assert!(!is_key("foo-"));
        assert!(!is_key("foo_"));
        assert!(!is_key("-"));
        assert!(!is_key("_"));
        assert!(!is_key("_x"));
        assert!(!is_key("-1"));
        assert!(!is_key("_1"));
        // Invalid chars.
        assert!(!is_key("a.b"));
        assert!(!is_key("a b"));
        assert!(!is_key("a/b"));
        assert!(!is_key("a:b"));
        assert!(!is_key("a@b"));
        assert!(!is_key(""));
        assert!(!is_key("."));
        assert!(!is_key("foo.bar"));
        // Double dash/underscore interior is allowed (still alnum ends).
        assert!(is_key("a--b"));
        assert!(is_key("a__b"));
        assert!(is_key("a-_b"));
        // Length edge: single char already tested, empty fails.
        assert!(!is_key(""));
    }

    #[test]
    fn metakey_and_type() {
        // Metakey: __ [a-z][a-z0-9_-]* __  (len >=5)
        assert!(is_metakey("__schema__"));
        assert!(is_metakey("__my_type__"));
        assert!(is_metakey("__x__y__"));
        assert!(is_metakey("__a__")); // 5-char minimum: __ + one letter + __
        assert!(is_metakey("__ab__"));
        assert!(is_metakey("__a1__"));
        assert!(is_metakey("__a-b__"));
        assert!(is_metakey("__a_b__"));
        assert!(is_metakey("__a1-b2_c3__"));
        assert!(is_metakey("__x1__"));
        // Invalid metakey.
        assert!(!is_metakey("__Document__")); // uppercase
        assert!(!is_metakey("__A__"));
        assert!(!is_metakey("__1a__")); // must start lowercase
        assert!(!is_metakey("__-a__"));
        assert!(!is_metakey("__a__extra")); // trailing chars
        assert!(!is_metakey("___a__")); // leading extra _
        assert!(is_metakey("__a___")); // inner "a_" + trailing __ => valid
        assert!(!is_metakey("__x_")); // too short
        assert!(!is_metakey("__"));
        assert!(!is_metakey("____"));
        assert!(!is_metakey("__ab")); // missing trailing __
        assert!(!is_metakey("ab__")); // missing leading __
        assert!(!is_metakey("x__y__")); // no leading __
        assert!(is_metakey("__a__b__")); // inner "a__b" valid -> true
        assert!(!is_metakey("__A__b__")); // uppercase in inner
        assert!(!is_metakey(""));
        assert!(!is_metakey("__a-b")); // missing trailing __
        // Known metakey — only __schema__.
        assert!(is_known_metakey("__schema__"));
        assert!(!is_known_metakey("__my_type__"));
        assert!(!is_known_metakey("__a__"));
        assert!(!is_known_metakey("__SCHEMA__"));
        assert!(!is_known_metakey("schema"));
        // Type names: [a-z][a-z0-9_-]*
        assert!(is_type_name("port"));
        assert!(is_type_name("my_type"));
        assert!(is_type_name("a"));
        assert!(is_type_name("ab"));
        assert!(is_type_name("a1"));
        assert!(is_type_name("a-b_c1"));
        assert!(is_type_name("a-"));
        assert!(is_type_name("a_"));
        assert!(is_type_name("a--b"));
        assert!(is_type_name("str"));
        assert!(is_type_name("int"));
        assert!(!is_type_name("Port")); // uppercase
        assert!(!is_type_name("1port")); // digit first
        assert!(!is_type_name("")); // empty
        assert!(!is_type_name("_port"));
        assert!(!is_type_name("-port"));
        assert!(!is_type_name("a.b"));
        assert!(!is_type_name("a b"));
        // Builtin types — exactly six.
        assert!(is_builtin_type("int"));
        assert!(is_builtin_type("float"));
        assert!(is_builtin_type("bool"));
        assert!(is_builtin_type("str"));
        assert!(is_builtin_type("list"));
        assert!(is_builtin_type("map"));
        assert!(!is_builtin_type("Int"));
        assert!(!is_builtin_type("INT"));
        assert!(!is_builtin_type("ints"));
        assert!(!is_builtin_type("integer"));
        assert!(!is_builtin_type(""));
        assert!(!is_builtin_type("string"));
        // looks_like_number mirrors is_int || is_float.
        assert!(looks_like_number("42"));
        assert!(looks_like_number("1_000"));
        assert!(looks_like_number("0.75"));
        assert!(looks_like_number("1e3"));
        assert!(!looks_like_number("hello"));
        assert!(!looks_like_number("true"));
        assert!(!looks_like_number("1_2"));
        assert!(!looks_like_number("1."));
    }
}
