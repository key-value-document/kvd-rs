//! Multi-line literal strings `'''...'''` (spec §2, §4).

use kvd_rs::deserialize::from_str;
use kvd_rs::serialize::to_string;

/// The text of the value at key `a` in a single-entry document.
fn text_of(doc: &str) -> String {
    let node = from_str(doc).unwrap_or_else(|e| panic!("{doc:?} fails to parse: {e}"));
    let kvd_rs::value::Node::Map(m) = &node else {
        panic!("{doc:?} is not a map: {node:?}")
    };
    match m.get("a") {
        Some(kvd_rs::value::Node::Scalar(s)) => s.text.clone(),
        other => panic!("{doc:?}: `a` is not a scalar: {other:?}"),
    }
}

#[test]
fn standalone_closer_keeps_the_trailing_newline() {
    assert_eq!(
        text_of("a: '''\n  line one\n  line two\n'''\n"),
        "line one\nline two\n"
    );
}

#[test]
fn inline_closer_drops_the_trailing_newline() {
    assert_eq!(
        text_of("a: '''\n  line one\n  line two'''\n"),
        "line one\nline two"
    );
}

#[test]
fn literal_forms_take_no_escapes() {
    // The whole point of `'''`: a backslash is a backslash, so an embedded
    // script needs no doubling (spec §2).
    assert_eq!(text_of("a: '''\n  C:\\Users\\me\n'''\n"), "C:\\Users\\me\n");
    assert_eq!(
        text_of("a: '''\n  \\n is two chars\n'''\n"),
        "\\n is two chars\n"
    );
    assert_eq!(text_of("a: '''\n  \\u0041\n'''\n"), "\\u0041\n");
    // `\"\"\"` needs doubling, which is exactly the friction `'''` removes:
    // the same logical text needs `\\` there and a plain `\` here.
    assert_eq!(text_of("a: \"\"\"\n  C:\\\\Users\n\"\"\"\n"), "C:\\Users\n");
    assert_eq!(text_of("a: '''\n  C:\\Users\n'''\n"), "C:\\Users\n");
}

#[test]
fn common_indent_is_stripped() {
    assert_eq!(
        text_of("a: '''\n    deep\n  shallow\n'''\n"),
        "  deep\nshallow\n"
    );
}

#[test]
fn blank_lines_are_kept() {
    assert_eq!(text_of("a: '''\n  one\n\n  two\n'''\n"), "one\n\ntwo\n");
}

#[test]
fn works_in_list_items_and_dict_entries() {
    // List item: content and closer sit two columns past the `- ` marker.
    let node = from_str("hooks:\n  - '''\n    #!/bin/sh\n    set -e\n    '''\n").expect("parses");
    let kvd_rs::value::Node::Map(m) = &node else {
        panic!()
    };
    let kvd_rs::value::Node::List(items) = m.get("hooks").unwrap() else {
        panic!()
    };
    match &items[0] {
        kvd_rs::value::Node::Scalar(s) => assert_eq!(s.text, "#!/bin/sh\nset -e\n"),
        other => panic!("{other:?}"),
    }

    // Dict entry.
    let node = from_str("m:\n  = \"k\": '''\n    raw \\ backslash\n    '''\n").expect("parses");
    assert!(matches!(node, kvd_rs::value::Node::Map(_)));
}

#[test]
fn quotes_of_the_other_kind_are_ordinary_content() {
    // `'''` may contain `"` freely; `"""` may contain `'` freely.
    assert_eq!(text_of("a: '''\n  say \"hello\"\n'''\n"), "say \"hello\"\n");
    assert_eq!(text_of("a: \"\"\"\n  it's fine\n\"\"\"\n"), "it's fine\n");
}

#[test]
fn empty_block() {
    assert_eq!(text_of("a: '''\n'''\n"), "");
}

#[test]
fn opener_must_be_alone_on_its_line() {
    // Maximal munch: `'''x'''` is an opener plus a stray quote, not a
    // single-line literal.
    assert!(from_str("a: '''x'''\n").is_err());
    assert!(from_str("a: ''''\n").is_err());
    assert!(from_str("a: '''\n").is_err(), "unterminated block");
}

#[test]
fn emit_always_uses_double_quotes() {
    // `'''` is an input spelling, never canonical output: emit uses a `"""`
    // block for a multi-line value and `"..."` otherwise (spec §8.2).
    let node = from_str("a: '''\n  line one\n  line two\n'''\n").expect("parses");
    let out = to_string(&node).expect("emits");
    assert_eq!(out, "a: \"\"\"\n  line one\n  line two\n\"\"\"\n");
    assert!(!out.contains('\''));

    let node = from_str("a: '''\n  one line'''\n").expect("parses");
    let out = to_string(&node).expect("emits");
    assert_eq!(out, "a: \"one line\"\n");
}

#[test]
fn roundtrips_through_emit_and_reparse() {
    let doc = "a: '''\n  one\n  two\n'''\n";
    let node = from_str(doc).expect("parses");
    let once = to_string(&node).expect("emits");
    let twice = to_string(&from_str(&once).expect("reparses")).expect("re-emits");
    assert_eq!(once, twice, "emit(parse(emit(parse(x)))) is stable");
    assert_eq!(from_str(&once).unwrap(), node);
}

#[test]
fn indented_closer_behaves_exactly_like_dquote() {
    // A closer indented past the key column is treated as an inline closer
    // (empty content), which is what `"""` already does — `'''` introduces
    // no new leniency, it inherits the existing rule (spec §4).
    for (label, doc) in [
        ("\"\"\"", "a: \"\"\"\n  one\n  \"\"\"\n"),
        ("'''", "a: '''\n  one\n  '''\n"),
    ] {
        let node = from_str(doc).unwrap_or_else(|e| panic!("{label} fails: {e}"));
        let kvd_rs::value::Node::Map(m) = &node else {
            panic!()
        };
        let kvd_rs::value::Node::Scalar(s) = m.get("a").unwrap() else {
            panic!()
        };
        assert_eq!(s.text, "one\n", "{label} closer handling differs");
    }
}
