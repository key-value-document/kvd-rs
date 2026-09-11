//! Serializer: [`Node`] → KVD text (spec §4 grammar).
//!
//! Spelling is re-derived from the value model rather than echoed from
//! [`Scalar::raw`]: every string value is double-quoted, except the six
//! builtin type names (`int`, `str`, `bool`, `float`, `list`, `dict`) which stay
//! bare for schema documents (spec §4). Strings containing newlines use the
//! `"""` block form.

use crate::grammar::{is_builtin_type, is_key};
use crate::value::{Map, Node, Scalar, Shape};
#[cfg(not(any(test, feature = "serde")))]
#[allow(unused_imports)]
// format! resolves to this under no_std; clippy misfires on macro imports
use alloc::format;
use alloc::string::{String, ToString};
use core::fmt;
use core::fmt::Write;

/// Error produced while serializing a node tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializeError {
    message: String,
}

impl SerializeError {
    fn new(message: impl Into<String>) -> Self {
        SerializeError {
            message: message.into(),
        }
    }
}

impl fmt::Display for SerializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl core::error::Error for SerializeError {}

impl From<fmt::Error> for SerializeError {
    fn from(_: fmt::Error) -> Self {
        SerializeError::new("failed to write output")
    }
}

/// Serializes a document to KVD text. The root must be a mapping (spec §4).
///
/// Trees nesting deeper than [`crate::MAX_DEPTH`] are rejected: the
/// serializer only emits block nesting, so deeper output would not parse.
pub fn to_string(doc: &Node) -> Result<String, SerializeError> {
    let mut out = String::new();
    match doc {
        // Root keys occupy depth slot 1, matching the parser's accounting.
        Node::Map(m) => emit_map(m, 0, 1, &mut out)?,
        _ => return Err(SerializeError::new("document root must be a mapping")),
    }
    Ok(out)
}

/// Emits a mapping at the given indentation (in columns). `depth` is the
/// nesting slot of the keys in `m`.
fn emit_map(m: &Map, indent: usize, depth: usize, out: &mut String) -> Result<(), SerializeError> {
    if depth > crate::MAX_DEPTH {
        return Err(SerializeError::new(
            "nesting exceeds the maximum depth of 100",
        ));
    }
    for (key, node) in m.iter() {
        emit_pair(key, node, indent, depth, out)?;
    }
    Ok(())
}

/// Emits one `key: value` pair whose key starts at `keycol`.
fn emit_pair(
    key: &str,
    node: &Node,
    keycol: usize,
    depth: usize,
    out: &mut String,
) -> Result<(), SerializeError> {
    let prefix = format!("{}{}", " ".repeat(keycol), quote_key(key)?);
    emit_keyed_value(&prefix, node, keycol + 2, keycol, depth, out)
}

/// Emits one `key: value` pair with a preformatted header prefix
/// (e.g. `"  key"`, `"  - key"`, `"  = \"k\""`, `"  - = \"k\""`).
/// `sub_indent` is where a subtree hangs; `scalar_col` is the content
/// column for inline scalars (also the `"""` closer column).
fn emit_keyed_value(
    prefix: &str,
    node: &Node,
    sub_indent: usize,
    scalar_col: usize,
    depth: usize,
    out: &mut String,
) -> Result<(), SerializeError> {
    match node {
        Node::Map(m) => {
            if m.is_empty() {
                writeln!(out, "{prefix}: {{}}")?;
            } else {
                writeln!(out, "{prefix}:")?;
                emit_map(m, sub_indent, depth + 1, out)?;
            }
        }
        Node::Dict(m) => {
            writeln!(out, "{prefix}:")?;
            emit_dict(m, sub_indent, depth + 1, out)?;
        }
        Node::List(items) => {
            if items.is_empty() {
                writeln!(out, "{prefix}: []")?;
            } else {
                writeln!(out, "{prefix}:")?;
                emit_list(items, sub_indent, depth + 1, out)?;
            }
        }
        Node::Scalar(sc) => {
            write!(out, "{prefix}: ")?;
            write_scalar_value(sc, scalar_col, scalar_col, out)?;
            writeln!(out)?;
        }
    }
    Ok(())
}

/// Emits a dict whose `=` markers start at `indent` (spec §8.2).
/// Dict entry keys are always double-quoted, even when they look bare.
/// An empty dict emits no entries; the caller never produces one (empty
/// dicts use the `{}` literal), so this emits nothing for empty input.
fn emit_dict(m: &Map, indent: usize, depth: usize, out: &mut String) -> Result<(), SerializeError> {
    if depth > crate::MAX_DEPTH {
        return Err(SerializeError::new(
            "nesting exceeds the maximum depth of 100",
        ));
    }
    let pad = " ".repeat(indent);
    for (key, node) in m.iter() {
        emit_entry(key, node, indent, &pad, depth, out)?;
    }
    Ok(())
}

/// Emits one `= "k": value` dict entry whose marker starts at `indent`.
/// `entry_col` is the column of the entry key (content sits two past it).
fn emit_entry(
    key: &str,
    node: &Node,
    indent: usize,
    pad: &str,
    depth: usize,
    out: &mut String,
) -> Result<(), SerializeError> {
    // Dict keys are opaque and always double-quoted (spec §8.2).
    let prefix = format!("{pad}= {}", escape(key));
    // Subtrees hang off the entry marker: two past `indent` (like a bare
    // `-` item's content), not two past the key. The key is inline on the
    // marker line, so `= "k":` at indent 2 nests `- x` at indent 4.
    emit_keyed_value(&prefix, node, indent + 2, indent + 2, depth, out)
}

/// Emits a list whose `-` markers start at `indent`. `depth` is the
/// nesting slot of the items in the list.
fn emit_list(
    items: &[Node],
    indent: usize,
    depth: usize,
    out: &mut String,
) -> Result<(), SerializeError> {
    if depth > crate::MAX_DEPTH {
        return Err(SerializeError::new(
            "nesting exceeds the maximum depth of 100",
        ));
    }
    let pad = " ".repeat(indent);
    for item in items {
        match item {
            Node::Map(m) => {
                if m.is_empty() {
                    writeln!(out, "{}- {{}}", pad)?;
                    continue;
                }
                let mut iter = m.iter();
                let (first_key, first_value) = iter.next().expect("non-empty map");
                let keycol = indent + 2;
                emit_inline_pair(first_key, first_value, keycol, out, &pad, depth)?;
                for (key, node) in iter {
                    emit_pair(key, node, keycol, depth, out)?;
                }
            }
            Node::List(items2) => {
                if items2.is_empty() {
                    writeln!(out, "{}- []", pad)?;
                } else {
                    writeln!(out, "{}-", pad)?;
                    emit_list(items2, indent + 2, depth + 1, out)?;
                }
            }
            Node::Dict(m) => {
                if m.is_empty() {
                    writeln!(out, "{}- {{}}", pad)?;
                    continue;
                }
                // First entry inlined after the `-` (spec §4
                // `'- ' entry`); the rest align to its column.
                let mut iter = m.iter();
                let (first_key, first_value) = iter.next().expect("non-empty dict");
                let entry_col = indent + 2;
                emit_inline_entry(first_key, first_value, entry_col, out, &pad, depth)?;
                for (key, node) in iter {
                    emit_entry(key, node, entry_col, &" ".repeat(entry_col), depth, out)?;
                }
            }
            Node::Scalar(s) => {
                write!(out, "{}- ", pad)?;
                write_scalar_value(s, indent, indent + 2, out)?;
                writeln!(out)?;
            }
        }
    }
    Ok(())
}

/// Emits the first pair of a list-item mapping, inlined after the `-`.
fn emit_inline_pair(
    key: &str,
    node: &Node,
    keycol: usize,
    out: &mut String,
    pad: &str,
    depth: usize,
) -> Result<(), SerializeError> {
    let prefix = format!("{pad}- {}", quote_key(key)?);
    emit_keyed_value(&prefix, node, keycol + 2, keycol, depth, out)
}

/// Emits the first dict entry of a list-item dict, inlined after the `-`
/// (`- = "k": v`, spec §4). `entry_col` is the column of the entry key.
fn emit_inline_entry(
    key: &str,
    node: &Node,
    entry_col: usize,
    out: &mut String,
    pad: &str,
    depth: usize,
) -> Result<(), SerializeError> {
    let prefix = format!("{pad}- = {}", escape(key));
    // `entry_col` is the rewritten `=` marker column (two past the `-`),
    // so subtrees sit two past it — mirroring `parse_entry`.
    emit_keyed_value(&prefix, node, entry_col + 2, entry_col, depth, out)
}

/// Writes a scalar's value (after the `key: ` / `- ` prefix already
/// blocks; everything else uses its single-line spelling (spec §5).
///
/// `content_col` is the column of the key (content sits two past it).
/// `closer_col` is the column where the `"""` closer is written — for a
/// key value this is `content_col`; for a bare list item it is `content_col + 2`
/// because content and closer share the same column (spec §5).
fn format_int_favored(text: &str) -> String {
    let (sign, digits) = match text.chars().next() {
        Some('+') | Some('-') => (&text[0..1], &text[1..]),
        _ => ("", text),
    };
    let clean: String = digits.chars().filter(|c| *c != '_').collect();
    if clean.len() <= 3 {
        let mut out = String::with_capacity(sign.len() + clean.len());
        out.push_str(sign);
        out.push_str(&clean);
        return out;
    }
    let len = clean.len();
    let first = len % 3;
    let mut out = String::with_capacity(sign.len() + clean.len() + clean.len() / 3);
    out.push_str(sign);
    let mut i = 0;
    if first != 0 {
        out.push_str(&clean[0..first]);
        i = first;
        if i < len {
            out.push('_');
        }
    }
    while i < len {
        let end = (i + 3).min(len);
        out.push_str(&clean[i..end]);
        i = end;
        if i < len {
            out.push('_');
        }
    }
    out
}

fn write_scalar_value(
    s: &Scalar,
    content_col: usize,
    closer_col: usize,
    out: &mut String,
) -> Result<(), SerializeError> {
    match s.shape {
        Shape::Int => {
            write!(out, "{}", format_int_favored(&s.text))?;
        }
        Shape::Float | Shape::Bool => {
            write!(out, "{}", s.text)?;
        }
        Shape::Null => {
            write!(out, "null")?;
        }
        Shape::Str => {
            if s.text.contains('\n') {
                emit_triple(&s.text, content_col, closer_col, out)?;
            } else {
                write!(out, "{}", quote_str(&s.text))?;
            }
        }
    }
    Ok(())
}

/// Emits a `"""` block. Content lines sit two columns past `content_col`.
///
/// Two closer forms (spec §5):
///   - If `text` ends with `\n`: emit content without the trailing `\n`,
///     then `"""` standalone at `closer_col`. The parser re-adds the `\n`.
///   - Otherwise: append `"""` inline to the last content line. No `\n` added.
fn emit_triple(
    text: &str,
    content_col: usize,
    closer_col: usize,
    out: &mut String,
) -> Result<(), SerializeError> {
    writeln!(out, "\"\"\"")?;
    let pad = " ".repeat(content_col + 2);
    let closer_pad = " ".repeat(closer_col);
    let escape_line = |line: &str| -> String {
        if line.is_empty() {
            return String::new();
        }
        let leading = line.len() - line.trim_start_matches(' ').len();
        let rest = &line[leading..];
        let escaped_rest = rest.replace('\\', "\\\\");
        if leading == 0 {
            escaped_rest
        } else {
            let mut s = String::with_capacity(leading * 6 + escaped_rest.len());
            for _ in 0..leading {
                s.push_str("\\u0020");
            }
            s.push_str(&escaped_rest);
            s
        }
    };
    if let Some(body) = text.strip_suffix('\n') {
        for line in body.split('\n') {
            if line.is_empty() {
                writeln!(out)?;
            } else {
                let escaped = escape_line(line);
                writeln!(out, "{}{}", pad, escaped)?;
            }
        }
        write!(out, "{}\"\"\"", closer_pad)?;
    } else {
        let mut lines = text.split('\n').peekable();
        while let Some(line) = lines.next() {
            if lines.peek().is_some() {
                if line.is_empty() {
                    writeln!(out)?;
                } else {
                    let escaped = escape_line(line);
                    writeln!(out, "{}{}", pad, escaped)?;
                }
            } else if line.is_empty() {
                write!(out, "{}\"\"\"", closer_pad)?;
            } else {
                let escaped = escape_line(line);
                write!(out, "{}{}\"\"\"", pad, escaped)?;
            }
        }
    }
    Ok(())
}

/// Emits `s` bare or double-quoted.
///
/// Builtin type names (`int`, `str`, `bool`, `float`, `list`, `dict`) stay
/// bare — they are only valid in schema position and must not be quoted there
/// (spec §4). All other strings are double-quoted; strings containing
/// newlines use the `"""` block form.
fn quote_str(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".to_string();
    }
    if is_builtin_type(s) {
        return s.to_string();
    }
    escape(s)
}

/// Quotes `s` unless it is a valid key token (spec §4).
fn quote_key(s: &str) -> Result<String, SerializeError> {
    if s.is_empty() {
        return Err(SerializeError::new("empty key"));
    }
    if is_key(s) {
        Ok(s.to_string())
    } else {
        // Non-bare key (e.g. a Kubernetes label/annotation key such as
        // `app.kubernetes.io/name`, or a dunder key such as `"__foo__"`):
        // emit it double-quoted so it round-trips (spec §4).
        Ok(escape(s))
    }
}

/// The always-quoted spelling of `s`, with escapes applied.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                write!(out, "\\u{:04x}", c as u32).expect("writing to a String cannot fail");
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::looks_like_number;
    use crate::value::{Map, Node, Shape};

    fn doc(pairs: &[(&str, Node)]) -> Node {
        let mut m = Map::new();
        for (k, v) in pairs {
            m.insert((*k).to_string(), v.clone());
        }
        Node::map(m)
    }

    fn scalar(shape: Shape, text: &str) -> Node {
        Node::scalar(shape, text)
    }

    fn block(text: &str) -> Node {
        Node::scalar(Shape::Str, text)
    }

    #[test]
    fn scalar_spellings() {
        let d = doc(&[
            ("i", scalar(Shape::Int, "42")),
            ("f", scalar(Shape::Float, "0.75")),
            ("b", scalar(Shape::Bool, "true")),
            ("s", scalar(Shape::Str, "hello")),
        ]);
        // "hello" is not a builtin type name, so double-quoted
        assert_eq!(
            to_string(&d).unwrap(),
            "i: 42\nf: 0.75\nb: true\ns: \"hello\"\n"
        );
    }

    #[test]
    fn strings_that_look_like_shapes_are_quoted() {
        let d = doc(&[
            ("n", scalar(Shape::Str, "42")),
            ("neg", scalar(Shape::Str, "-5")),
            ("plus", scalar(Shape::Str, "+5")),
            ("sep", scalar(Shape::Str, "1_000")),
            ("t", scalar(Shape::Str, "true")),
            ("f", scalar(Shape::Str, "false")),
            ("empty", scalar(Shape::Str, "")),
        ]);
        assert_eq!(
            to_string(&d).unwrap(),
            "n: \"42\"\nneg: \"-5\"\nplus: \"+5\"\nsep: \"1_000\"\nt: \"true\"\nf: \"false\"\nempty: \"\"\n"
        );
    }

    #[test]
    fn strings_are_always_quoted() {
        let d = doc(&[
            ("w", scalar(Shape::Str, "5foo")),
            ("neg", scalar(Shape::Str, "-foo")),
            ("plus", scalar(Shape::Str, "+foo")),
            ("date", scalar(Shape::Str, "2026-08-20")),
            ("under", scalar(Shape::Str, "_x")),
        ]);
        assert_eq!(
            to_string(&d).unwrap(),
            "w: \"5foo\"\nneg: \"-foo\"\nplus: \"+foo\"\ndate: \"2026-08-20\"\nunder: \"_x\"\n"
        );
    }

    #[test]
    fn non_words_are_quoted_with_escapes() {
        let d = doc(&[
            ("sp", scalar(Shape::Str, "hello world")),
            ("hash", scalar(Shape::Str, "a#b")),
            ("quote", scalar(Shape::Str, "say \"hi\"")),
            ("back", scalar(Shape::Str, "a\\b")),
            ("tab", scalar(Shape::Str, "a\tb")),
            ("ctrl", scalar(Shape::Str, "a\u{1}b")),
        ]);
        assert_eq!(
            to_string(&d).unwrap(),
            "sp: \"hello world\"\nhash: \"a#b\"\nquote: \"say \\\"hi\\\"\"\nback: \"a\\\\b\"\ntab: \"a\\tb\"\nctrl: \"a\\u0001b\"\n"
        );
    }

    #[test]
    fn extended_strings_are_quoted() {
        // Path-like, dotted, URL-ish, and version-like strings are quoted
        // under mandatory double-quoting (spec §3 `dquote`).
        let d = doc(&[
            ("health", scalar(Shape::Str, "/health")),
            ("path", scalar(Shape::Str, "a/b/c")),
            ("dotted", scalar(Shape::Str, "foo.bar")),
            ("version", scalar(Shape::Str, "1.5.2")),
            ("url", scalar(Shape::Str, "http://x")),
            ("time", scalar(Shape::Str, "10:30")),
            ("cron", scalar(Shape::Str, "@daily")),
            ("dotfile", scalar(Shape::Str, ".env")),
            // `=` and `&` are not word characters: full query strings stay
            // quoted.
            ("q", scalar(Shape::Str, "http://x?a=1")),
        ]);
        assert_eq!(
            to_string(&d).unwrap(),
            "health: \"/health\"\npath: \"a/b/c\"\ndotted: \"foo.bar\"\nversion: \"1.5.2\"\nurl: \"http://x\"\ntime: \"10:30\"\ncron: \"@daily\"\ndotfile: \".env\"\nq: \"http://x?a=1\"\n"
        );
    }

    #[test]
    fn null_round_trips() {
        let d = doc(&[("x", scalar(Shape::Null, "null"))]);
        assert_eq!(to_string(&d).unwrap(), "x: null\n");
        let parsed = crate::deserialize::from_str("x: null").unwrap();
        match parsed.as_map().expect("map").get("x") {
            Some(Node::Scalar(s)) => assert_eq!(s.shape, Shape::Null),
            other => panic!("expected null scalar, got {other:?}"),
        }
        // "nullish" is not a builtin type name, so double-quoted
        assert_eq!(
            to_string(&doc(&[("w", scalar(Shape::Str, "nullish"))])).unwrap(),
            "w: \"nullish\"\n"
        );
    }

    #[test]
    fn nested_map_and_list() {
        let d = doc(&[(
            "a",
            Node::map({
                let mut m = Map::new();
                m.insert(
                    "b".into(),
                    Node::list(vec![scalar(Shape::Int, "1"), scalar(Shape::Str, "x")]),
                );
                m
            }),
        )]);
        assert_eq!(to_string(&d).unwrap(), "a:\n  b:\n    - 1\n    - \"x\"\n");
    }

    #[test]
    fn empty_collections_are_literals() {
        let d = doc(&[
            ("m", Node::map(Map::new())),
            ("l", Node::list(vec![])),
            (
                "lm",
                Node::list(vec![Node::map(Map::new()), Node::list(vec![])]),
            ),
        ]);
        assert_eq!(
            to_string(&d).unwrap(),
            "m: {}\nl: []\nlm:\n  - {}\n  - []\n"
        );
    }

    #[test]
    fn list_item_mapping_inlines_first_key() {
        let mut inner = Map::new();
        inner.insert("x".into(), scalar(Shape::Int, "1"));
        inner.insert("y".into(), scalar(Shape::Int, "2"));
        let d = doc(&[("l", Node::list(vec![Node::map(inner)]))]);
        assert_eq!(to_string(&d).unwrap(), "l:\n  - x: 1\n    y: 2\n");
    }

    #[test]
    fn nested_list_uses_bare_dash() {
        let d = doc(&[(
            "l",
            Node::list(vec![Node::list(vec![scalar(Shape::Int, "1")])]),
        )]);
        assert_eq!(to_string(&d).unwrap(), "l:\n  -\n    - 1\n");
    }

    #[test]
    fn triple_keep_and_strip() {
        // Trailing \n → standalone closer; no trailing \n → inline closer.
        let d = doc(&[
            ("keep", block("line1\nline2\n")),
            ("strip", block("line1\nline2")),
        ]);
        assert_eq!(
            to_string(&d).unwrap(),
            "keep: \"\"\"\n  line1\n  line2\n\"\"\"\nstrip: \"\"\"\n  line1\n  line2\"\"\"\n"
        );
    }

    #[test]
    fn triple_in_list_item() {
        let d = doc(&[("l", Node::list(vec![block("a\nb")]))]);
        assert_eq!(
            to_string(&d).unwrap(),
            "l:\n  - \"\"\"\n    a\n    b\"\"\"\n"
        );
        // Round-trips through the parser byte-for-byte.
        let text = to_string(&d).unwrap();
        let reparsed = crate::deserialize::from_str(&text).unwrap();
        assert_eq!(to_string(&reparsed).unwrap(), text);
    }

    #[test]
    fn empty_triple_in_list_item() {
        for text in ["", "\n", "a\nb", "a\nb\n"] {
            let d = doc(&[("l", Node::list(vec![block(text)]))]);
            let out = to_string(&d).unwrap();
            let reparsed = crate::deserialize::from_str(&out).unwrap();
            assert_eq!(
                to_string(&reparsed).unwrap(),
                out,
                "round-trip failed for {text:?}"
            );
            assert_eq!(
                reparsed
                    .as_map()
                    .unwrap()
                    .get("l")
                    .unwrap()
                    .as_list()
                    .unwrap()[0]
                    .as_scalar()
                    .unwrap()
                    .text,
                text
            );
        }
    }

    #[test]
    fn non_bare_keys_are_quoted() {
        // Keys that aren't valid bare keys are emitted double-quoted (spec §4),
        // so they round-trip instead of erroring.
        let out = to_string(&doc(&[("-foo", scalar(Shape::Int, "1"))])).unwrap();
        assert!(out.contains("\"-foo\": 1"));
        let out2 = to_string(&doc(&[("a.b", scalar(Shape::Int, "1"))])).unwrap();
        assert!(out2.contains("\"a.b\": 1"));
        // Valid bare keys stay unquoted.
        assert!(to_string(&doc(&[("8080", scalar(Shape::Int, "1"))])).is_ok());
        assert!(to_string(&doc(&[("2fa", scalar(Shape::Int, "1"))])).is_ok());
        // Empty keys are still rejected.
        assert!(to_string(&doc(&[("", scalar(Shape::Int, "1"))])).is_err());
    }

    #[test]
    fn root_must_be_a_mapping() {
        assert!(to_string(&Node::list(vec![])).is_err());
        assert!(to_string(&scalar(Shape::Int, "1")).is_err());
    }

    #[test]
    fn number_literal_recognition() {
        assert!(looks_like_number("42"));
        assert!(looks_like_number("-5"));
        assert!(looks_like_number("+5"));
        assert!(looks_like_number("1_000"));
        assert!(looks_like_number("45_678_112"));
        assert!(looks_like_number("0.5"));
        assert!(looks_like_number("-5.5"));
        assert!(looks_like_number("1e3"));
        assert!(looks_like_number("0.5e-3"));
        assert!(!looks_like_number("1_2"));
        assert!(!looks_like_number("1_2345"));
        assert!(!looks_like_number("1234_567"));
        assert!(!looks_like_number("5foo"));
        assert!(!looks_like_number("2026-08-20"));
        assert!(!looks_like_number("1.5.5"));
        assert!(!looks_like_number(".5"));
        assert!(!looks_like_number("1."));
        assert!(!looks_like_number("1e"));
    }

    fn deep_chain(levels: usize) -> Node {
        let mut node = scalar(Shape::Int, "0");
        for i in 0..levels {
            node = doc(&[(&format!("k{i}"), node)]);
        }
        node
    }

    #[test]
    fn err_nesting_beyond_max_depth() {
        // Programmatically-built trees deeper than MAX_DEPTH are refused:
        // the serializer only emits nesting, so the output would
        // not parse back.
        let err = to_string(&deep_chain(crate::MAX_DEPTH + 5)).unwrap_err();
        assert!(err.to_string().contains("maximum depth"));
    }

    #[test]
    fn max_depth_chain_round_trips() {
        let node = deep_chain(100); // slots 1..=100, exactly at the limit
        let out = to_string(&node).unwrap();
        assert_eq!(crate::deserialize::from_str(&out).unwrap(), node);
    }

    #[test]
    fn quote_key_quotes_non_bare() {
        assert_eq!(
            quote_key("app.kubernetes.io/name").unwrap(),
            "\"app.kubernetes.io/name\""
        );
    }

    #[test]
    fn quote_key_keeps_bare() {
        assert_eq!(quote_key("name").unwrap(), "name");
    }

    #[test]
    fn quote_key_quotes_dunder_literal() {
        // A dunder key cannot be bare; it is quoted like any non-key.
        assert_eq!(quote_key("__foo__").unwrap(), "\"__foo__\"");
    }

    #[test]
    fn quoted_dunder_key_round_trips() {
        let schema = crate::deserialize::from_str("\"__schema__\": x\n").unwrap();
        let out = to_string(&schema).unwrap();
        assert_eq!(out, "\"__schema__\": \"x\"\n");
    }

    #[test]
    fn dict_keys_always_quoted() {
        // Dict entry keys stay double-quoted even when they look bare.
        let mut m = Map::new();
        m.insert("team".into(), scalar(Shape::Str, "web"));
        m.insert("a.b/c".into(), scalar(Shape::Int, "1"));
        let d = doc(&[("labels", Node::dict(m))]);
        assert_eq!(
            to_string(&d).unwrap(),
            "labels:\n  = \"team\": \"web\"\n  = \"a.b/c\": 1\n"
        );
    }

    #[test]
    fn dict_subtrees_hang_off_marker() {
        // Value subtrees sit two past the `=` marker.
        let mut inner = Map::new();
        inner.insert("amy".into(), scalar(Shape::Str, "x"));
        let mut m = Map::new();
        m.insert(
            "team-a".into(),
            Node::list(vec![scalar(Shape::Str, "amy"), scalar(Shape::Str, "bo")]),
        );
        m.insert("plain".into(), scalar(Shape::Int, "1"));
        let d = doc(&[("groups", Node::dict(m))]);
        let out = to_string(&d).unwrap();
        assert_eq!(
            out,
            "groups:\n  = \"team-a\":\n    - \"amy\"\n    - \"bo\"\n  = \"plain\": 1\n"
        );
        // Round-trips through the parser.
        assert_eq!(crate::deserialize::from_str(&out).unwrap(), d);
    }

    #[test]
    fn dict_in_list_item_inlines_first_entry() {
        let mut m = Map::new();
        m.insert("a".into(), scalar(Shape::Int, "1"));
        m.insert("d".into(), scalar(Shape::Int, "2"));
        let d = doc(&[("items", Node::list(vec![Node::dict(m)]))]);
        let out = to_string(&d).unwrap();
        assert_eq!(out, "items:\n  - = \"a\": 1\n    = \"d\": 2\n");
        assert_eq!(crate::deserialize::from_str(&out).unwrap(), d);
    }

    #[test]
    fn serialize_error_display() {
        let e = SerializeError::new("oops");
        assert_eq!(e.to_string(), "oops");
        let _: &dyn core::error::Error = &e;
        let fmt_err: SerializeError = core::fmt::Error.into();
        assert_eq!(fmt_err.to_string(), "failed to write output");
    }

    #[test]
    fn empty_key_rejected() {
        let d = doc(&[("", scalar(Shape::Int, "1"))]);
        assert!(to_string(&d).is_err());
    }

    #[test]
    fn empty_map_in_list_uses_literal() {
        // `- {}` for an empty map item.
        let d = doc(&[("l", Node::list(vec![Node::map(Map::new())]))]);
        assert_eq!(to_string(&d).unwrap(), "l:\n  - {}\n");
    }

    #[test]
    fn triple_with_blank_and_leading_spaces() {
        // Blank lines stay blank; leading spaces are escaped.
        let d = doc(&[("k", scalar(Shape::Str, "a\n\n  indented\n"))]);
        let out = to_string(&d).unwrap();
        let reparsed = crate::deserialize::from_str(&out).unwrap();
        assert_eq!(reparsed, d);
        assert!(out.contains("\\u0020"));
    }

    #[test]
    fn triple_inline_closer_empty_last_line() {
        // Text ending without newline whose last line is empty.
        let d = doc(&[("k", scalar(Shape::Str, "a\n"))]);
        let out = to_string(&d).unwrap();
        assert_eq!(crate::deserialize::from_str(&out).unwrap(), d);
    }

    #[test]
    fn nested_dict_emits_nested_entries() {
        let mut inner = Map::new();
        inner.insert("b".into(), scalar(Shape::Int, "1"));
        let mut m = Map::new();
        m.insert("a".into(), Node::dict(inner));
        let d = doc(&[("outer", Node::dict(m))]);
        let out = to_string(&d).unwrap();
        assert_eq!(out, "outer:\n  = \"a\":\n    = \"b\": 1\n");
        assert_eq!(crate::deserialize::from_str(&out).unwrap(), d);
    }
}
