//! Schema verification integration tests: data against schema checks.

use kvd_rs::deserialize;
use kvd_rs::schema::{VerifyError, Violation, verify, verify_from_str};

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
fn scalar_where_dict_expected() {
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
fn null_where_dict_or_list_expected_is_an_error() {
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
fn empty_dict_leaf_accepts_any_dict() {
    ok("a: {}\n", "a: {}\n");
    ok("a:\n  x: 1\n  y:\n    z: \"deep\"\n", "a: {}\n");
    // ...but not a non-map.
    assert_eq!(
        errs("a: []\n", "a: {}\n"),
        vec![Violation::new("a", "expected a dict, found a list")]
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
fn list_of_dicts_and_empty_literals() {
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
fn type_dict_descriptor_accepts_any_dict() {
    ok("cfg:\n  x: 1\n", "cfg:\n  type: dict\n");
    ok("cfg:\n  = \"k\": 1\n", "cfg:\n  type: dict\n");
    assert_eq!(
        errs("cfg: 1\n", "cfg:\n  type: dict\n"),
        vec![Violation::new("cfg", "expected a dict, found a scalar")]
    );
}

#[test]
fn dict_declared_keys_checked_undeclared_pass() {
    // Declared keys present in the data are checked; undeclared data
    // keys pass with any type; declared keys may be absent.
    ok(
        "metrics:\n  = \"errors/total\": 3\n  = \"another\": 1.2\n",
        "metrics:\n  = \"errors/total\": int\n  = \"another\": float\n",
    );
    // Wrong type on a declared key fails.
    assert_eq!(
        errs(
            "metrics:\n  = \"errors/total\": \"x\"\n",
            "metrics:\n  = \"errors/total\": int\n",
        ),
        vec![Violation::new(
            "metrics.errors/total",
            "expected int, found string"
        )]
    );
    // Undeclared data key with any type passes.
    ok(
        "metrics:\n  = \"errors/total\": 3\n  = \"whatever\": \"x\"\n",
        "metrics:\n  = \"errors/total\": int\n",
    );
    // Declared key absent from data is fine (optional).
    ok(
        "metrics:\n  = \"other\": 1.2\n",
        "metrics:\n  = \"errors/total\": int\n",
    );
    // Data must still be a dict, not node prefixes.
    assert_eq!(
        errs("m:\n  a: 1\n", "m:\n  = \"example\": int\n"),
        vec![Violation::new("m", "expected a dict, found a map")]
    );
    // Nested declared types work.
    ok(
        "groups:\n  = \"team-a\":\n    - \"amy\"\n",
        "groups:\n  = \"team-a\":\n    - str\n",
    );
    assert_eq!(
        errs(
            "groups:\n  = \"team-a\":\n    - 1\n",
            "groups:\n  = \"team-a\":\n    - str\n",
        ),
        vec![Violation::new(
            "groups.team-a[0]",
            "expected string, found int"
        )]
    );
}

#[test]
fn schema_dict_entry_types_validated() {
    // Unknown type in a declared entry is malformed schema.
    let s = deserialize::from_str("m:\n  = \"a\": port\n").unwrap();
    let d = deserialize::from_str("m:\n  = \"a\": 1\n").unwrap();
    assert_eq!(
        verify(&d, &s).unwrap_err(),
        VerifyError::SchemaMalformed(vec![Violation::new("m.a", "unknown type `port`")])
    );
}

#[test]
fn dict_element_descriptor_checks_values() {
    ok("m:\n  = \"a\": 1\n", "m:\n  type: dict\n  element: int\n");
    assert_eq!(
        errs(
            "m:\n  = \"a\": \"x\"\n",
            "m:\n  type: dict\n  element: int\n",
        ),
        vec![Violation::new("m.a", "expected int, found string")]
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
    ok("cfg: null\n", "cfg:\n  type: dict\n  optional: true\n");
}

#[test]
fn bare_list_dict_leaf_is_an_error() {
    assert_eq!(
        errs("a: 1\n", "a: list\n"),
        vec![Violation::new(
            "a",
            "`list`/`dict` may only appear in a descriptor (`type: list` / `type: dict`)"
        )]
    );
    assert_eq!(
        errs("a: {}\n", "a: dict\n"),
        vec![Violation::new(
            "a",
            "`list`/`dict` may only appear in a descriptor (`type: list` / `type: dict`)"
        )]
    );
}

#[test]
fn schema_list_must_declare_one_element() {
    // `[]` leaf means any list.
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
fn dunder_keys_verify_like_any_key() {
    // No metakeys: a quoted dunder key is an ordinary key on both sides.
    ok("\"__schema__\": x\n", "\"__schema__\": str\n");
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
fn bare_dunder_key_is_parse_error() {
    // Bare `__name__` cannot match the key grammar (leading/trailing `_`).
    assert!(deserialize::from_str("__schema__:\n  p: int\n").is_err());
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

// §10 Validation constraints
#[test]
fn int_validation_ranges() {
    ok(
        "a: 5\n",
        "a:\n  type: int\n  validation:\n    min: 0\n    max: 10\n",
    );
    assert!(
        errs("a: -1\n", "a:\n  type: int\n  validation:\n    min: 0\n")[0]
            .message
            .contains("less than min")
    );
    assert!(
        errs("a: 11\n", "a:\n  type: int\n  validation:\n    max: 10\n")[0]
            .message
            .contains("exceeds max")
    );
    ok(
        "a: 5\n",
        "a:\n  type: int\n  validation:\n    exclusive_min: 4\n    exclusive_max: 6\n",
    );
    assert!(
        errs(
            "a: 4\n",
            "a:\n  type: int\n  validation:\n    exclusive_min: 4\n"
        )[0]
        .message
        .contains("greater than exclusive_min")
    );
    assert!(
        errs(
            "a: 6\n",
            "a:\n  type: int\n  validation:\n    exclusive_max: 6\n"
        )[0]
        .message
        .contains("less than exclusive_max")
    );
    // underscore and big-int
    ok(
        "a: 1_000\n",
        "a:\n  type: int\n  validation:\n    min: 999\n",
    );
    assert!(
        errs(
            "a: 1_000\n",
            "a:\n  type: int\n  validation:\n    max: 999\n"
        )[0]
        .message
        .contains("exceeds max")
    );
    ok(
        "a: 99999999999999999999\n",
        "a:\n  type: int\n  validation:\n    min: 99999999999999999998\n",
    );
    assert!(
        errs(
            "a: 99999999999999999999\n",
            "a:\n  type: int\n  validation:\n    max: 99999999999999999998\n"
        )[0]
        .message
        .contains("exceeds max")
    );
    ok(
        "a: -5\n",
        "a:\n  type: int\n  validation:\n    min: -10\n    max: 0\n",
    );
    assert!(
        errs("a: -15\n", "a:\n  type: int\n  validation:\n    min: -10\n")[0]
            .message
            .contains("less than min")
    );
}

#[test]
fn float_validation_ranges() {
    ok(
        "a: 1.5\n",
        "a:\n  type: float\n  validation:\n    min: 0.5\n    max: 2.5\n",
    );
    assert!(
        errs(
            "a: 0.4\n",
            "a:\n  type: float\n  validation:\n    min: 0.5\n"
        )[0]
        .message
        .contains("less than min")
    );
    assert!(
        errs(
            "a: 3.0\n",
            "a:\n  type: float\n  validation:\n    max: 2.5\n"
        )[0]
        .message
        .contains("exceeds max")
    );
    ok(
        "a: 1.0\n",
        "a:\n  type: float\n  validation:\n    exclusive_min: 0.5\n    exclusive_max: 1.5\n",
    );
    assert!(
        errs(
            "a: 0.5\n",
            "a:\n  type: float\n  validation:\n    exclusive_min: 0.5\n"
        )[0]
        .message
        .contains("greater than exclusive_min")
    );
    // int-shaped bound on float is allowed
    ok("a: 1.5\n", "a:\n  type: float\n  validation:\n    min: 1\n");
}

#[test]
fn str_validation_lengths_and_pattern() {
    ok(
        "a: \"hello\"\n",
        "a:\n  type: str\n  validation:\n    min_len: 3\n    max_len: 10\n",
    );
    assert!(
        errs(
            "a: \"hi\"\n",
            "a:\n  type: str\n  validation:\n    min_len: 3\n"
        )[0]
        .message
        .contains("less than min_len")
    );
    assert!(
        errs(
            "a: \"hello world long\"\n",
            "a:\n  type: str\n  validation:\n    max_len: 5\n"
        )[0]
        .message
        .contains("exceeds max_len")
    );
    // Unicode scalar count (é = 1 char, 2 bytes)
    ok(
        "a: \"é\"\n",
        "a:\n  type: str\n  validation:\n    min_len: 1\n    max_len: 1\n",
    );
    assert!(
        errs(
            "a: \"é\"\n",
            "a:\n  type: str\n  validation:\n    max_len: 0\n"
        )[0]
        .message
        .contains("exceeds max_len")
    );
    // pattern full-match
    ok(
        "a: \"abc123\"\n",
        "a:\n  type: str\n  validation:\n    pattern: \"^[a-z]+[0-9]+$\"\n",
    );
    assert!(
        errs(
            "a: \"ABC\"\n",
            "a:\n  type: str\n  validation:\n    pattern: \"^[a-z]+$\"\n"
        )[0]
        .message
        .contains("does not match pattern")
    );
    // "foo" must not match "foobar" (full-match)
    assert!(
        errs(
            "a: \"foobar\"\n",
            "a:\n  type: str\n  validation:\n    pattern: \"foo\"\n"
        )[0]
        .message
        .contains("does not match pattern")
    );
    ok(
        "a: \"foo\"\n",
        "a:\n  type: str\n  validation:\n    pattern: \"foo\"\n",
    );
    ok(
        "a: \"a-b_c\"\n",
        "a:\n  type: str\n  validation:\n    pattern: \"^[a-z][a-z0-9_-]*$\"\n",
    );
}

#[test]
fn list_and_dict_validation_lengths() {
    ok(
        "a:\n  - 1\n  - 2\n",
        "a:\n  type: list\n  element: int\n  validation:\n    min_len: 1\n    max_len: 3\n",
    );
    assert!(
        errs(
            "a: []\n",
            "a:\n  type: list\n  element: int\n  validation:\n    min_len: 1\n"
        )[0]
        .message
        .contains("less than min_len")
    );
    assert!(
        errs(
            "a:\n  - 1\n  - 2\n  - 3\n  - 4\n",
            "a:\n  type: list\n  element: int\n  validation:\n    max_len: 3\n"
        )[0]
        .message
        .contains("exceeds max_len")
    );
    ok(
        "a:\n  x: 1\n",
        "a:\n  type: dict\n  validation:\n    min_len: 1\n",
    );
    assert!(
        errs(
            "a: {}\n",
            "a:\n  type: dict\n  validation:\n    min_len: 1\n"
        )[0]
        .message
        .contains("less than min_len")
    );
    assert!(
        errs(
            "a:\n  x: 1\n  y: 2\n",
            "a:\n  type: dict\n  validation:\n    max_len: 1\n"
        )[0]
        .message
        .contains("exceeds max_len")
    );
}

#[test]
fn validation_skipped_for_null_and_absent() {
    ok(
        "a: null\n",
        "a:\n  type: int\n  optional: true\n  validation:\n    min: 0\n",
    );
    ok(
        "a: null\n",
        "a:\n  type: list\n  element: int\n  optional: true\n  validation:\n    min_len: 10\n",
    );
    ok(
        "a: null\n",
        "a:\n  type: dict\n  optional: true\n  validation:\n    min_len: 1\n",
    );
    // absent optional with validation
    ok(
        "a: 1\n",
        "a: int\nb:\n  type: str\n  optional: true\n  validation:\n    min_len: 1\n",
    );
    // list element validation with null skipping
    ok(
        "a:\n  - 1\n  - null\n",
        "a:\n  type: list\n  element:\n    type: int\n    optional: true\n    validation:\n      min: 0\n",
    );
}

#[test]
fn list_element_validation() {
    ok(
        "a:\n  - \"abc\"\n  - \"def\"\n",
        "a:\n  type: list\n  element:\n    type: str\n    validation:\n      pattern: \"^[a-z]+$\"\n",
    );
    assert!(errs(
        "a:\n  - \"ABC\"\n",
        "a:\n  type: list\n  element:\n    type: str\n    validation:\n      pattern: \"^[a-z]+$\"\n"
    )[0]
        .message
        .contains("does not match pattern"));
    ok(
        "a:\n  - 5\n  - 6\n",
        "a:\n  type: list\n  element:\n    type: int\n    validation:\n      min: 0\n      max: 10\n",
    );
    assert!(
        errs(
            "a:\n  - 11\n",
            "a:\n  type: list\n  element:\n    type: int\n    validation:\n      max: 10\n"
        )[0]
        .message
        .contains("exceeds max")
    );
}

#[test]
fn validation_unknown_constraint_is_schema_malformed() {
    let d = deserialize::from_str("a: 1\n").unwrap();
    for (schema, expect) in [
        (
            "a:\n  type: int\n  validation:\n    pattern: \"foo\"\n",
            "unknown constraint",
        ),
        (
            "a:\n  type: str\n  validation:\n    min: 0\n",
            "unknown constraint",
        ),
        (
            "a:\n  type: bool\n  validation:\n    min_len: 1\n",
            "unknown constraint",
        ),
        (
            "a:\n  type: list\n  element: int\n  validation:\n    pattern: \"foo\"\n",
            "unknown constraint",
        ),
    ] {
        let s = deserialize::from_str(schema).unwrap();
        match verify(&d, &s) {
            Err(VerifyError::SchemaMalformed(v)) => {
                assert!(
                    v.iter().any(|x| x.message.contains(expect)),
                    "expected '{expect}' in {v:?} for schema {schema}"
                );
            }
            other => panic!("expected SchemaMalformed for {schema}, got {other:?}"),
        }
    }
}

#[test]
fn validation_schema_malformed_cases() {
    let d = deserialize::from_str("a: 1\n").unwrap();
    // invalid regex
    let s = deserialize::from_str("a:\n  type: str\n  validation:\n    pattern: \"[\"\n").unwrap();
    assert!(matches!(
        verify(&d, &s),
        Err(VerifyError::SchemaMalformed(_))
    ));
    // RE2 dialect: look-around not supported
    let s = deserialize::from_str("a:\n  type: str\n  validation:\n    pattern: \"(?<=a)b\"\n")
        .unwrap();
    match verify(&d, &s) {
        Err(VerifyError::SchemaMalformed(v)) => {
            assert!(v.iter().any(|x| x.message.contains("invalid pattern")));
        }
        other => panic!("expected SchemaMalformed for look-around, got {other:?}"),
    }
    // validation not a map
    let s = deserialize::from_str("a:\n  type: int\n  validation: 0\n").unwrap();
    assert!(matches!(
        verify(&d, &s),
        Err(VerifyError::SchemaMalformed(_))
    ));
    // descriptor requires type
    let s = deserialize::from_str("a:\n  validation:\n    min: 0\n").unwrap();
    assert!(matches!(
        verify(&d, &s),
        Err(VerifyError::SchemaMalformed(_))
    ));
    // extra descriptor key
    let s = deserialize::from_str("a:\n  type: int\n  foo: bar\n").unwrap();
    match verify(&d, &s) {
        Err(VerifyError::SchemaMalformed(v)) => {
            assert!(v.iter().any(|x| x.message.contains("unknown key")));
        }
        other => panic!("expected SchemaMalformed for extra key, got {other:?}"),
    }
    // optional not bool
    let s = deserialize::from_str("a:\n  type: int\n  optional: \"true\"\n").unwrap();
    assert!(matches!(
        verify(&d, &s),
        Err(VerifyError::SchemaMalformed(_))
    ));
    // element only for list
    let s = deserialize::from_str("a:\n  type: int\n  element: int\n").unwrap();
    assert!(matches!(
        verify(&d, &s),
        Err(VerifyError::SchemaMalformed(_))
    ));
}

#[test]
fn violation_and_error_display() {
    // Empty-path violation prints the message alone.
    assert_eq!(Violation::new("", "oops").to_string(), "oops");
    assert_eq!(Violation::new("a", "bad").to_string(), "a: bad");
    // ParseSchema display path.
    let e = verify_from_str("a: 1\n", "a:\n  b\n").unwrap_err();
    assert!(e.to_string().starts_with("schema parse error:"));
    let _: &dyn core::error::Error = &e;
}

#[test]
fn regex_cache_evicts_when_full() {
    // Fill the cache past its cap with distinct valid patterns.
    for i in 0..80 {
        let pat = format!("x{i}y");
        let schema = format!("a:\n  type: str\n  validation:\n    pattern: \"{pat}\"\n");
        let d = deserialize::from_str(format!("a: \"x{i}y\"\n").as_str()).unwrap();
        let s = deserialize::from_str(&schema).unwrap();
        assert!(verify(&d, &s).is_ok(), "pattern {pat} should verify");
    }
    // Cache still serves: repeat a pattern.
    let d = deserialize::from_str("a: \"x\"\n").unwrap();
    let s = deserialize::from_str("a:\n  type: str\n  validation:\n    pattern: \"x+\"\n").unwrap();
    assert!(verify(&d, &s).is_ok());
}

#[test]
fn descriptor_and_deprecated_shape_errors() {
    let d = deserialize::from_str("a: 1\n").unwrap();
    // description must be a string.
    let s = deserialize::from_str("a:\n  type: int\n  description: 1\n").unwrap();
    assert!(matches!(
        verify(&d, &s),
        Err(VerifyError::SchemaMalformed(_))
    ));
    // deprecated must be a dict.
    let s = deserialize::from_str("a:\n  type: int\n  deprecated: yes\n").unwrap();
    assert!(matches!(
        verify(&d, &s),
        Err(VerifyError::SchemaMalformed(_))
    ));
    // deprecated with unknown key.
    let s = deserialize::from_str("a:\n  type: int\n  deprecated:\n    bogus: \"x\"\n").unwrap();
    assert!(matches!(
        verify(&d, &s),
        Err(VerifyError::SchemaMalformed(_))
    ));
    // deprecated value must be a string.
    let s = deserialize::from_str("a:\n  type: int\n  deprecated:\n    reason: 1\n").unwrap();
    assert!(matches!(
        verify(&d, &s),
        Err(VerifyError::SchemaMalformed(_))
    ));
    // constraint value of the wrong shape.
    let s = deserialize::from_str("a:\n  type: int\n  validation:\n    min: \"x\"\n").unwrap();
    assert!(matches!(
        verify(&d, &s),
        Err(VerifyError::SchemaMalformed(_))
    ));
    // float bound on int type.
    let s = deserialize::from_str("a:\n  type: int\n  validation:\n    min: 0.5\n").unwrap();
    assert!(matches!(
        verify(&d, &s),
        Err(VerifyError::SchemaMalformed(_))
    ));
    // pattern must be a string.
    let s = deserialize::from_str("a:\n  type: str\n  validation:\n    pattern: 1\n").unwrap();
    assert!(matches!(
        verify(&d, &s),
        Err(VerifyError::SchemaMalformed(_))
    ));
    // min_len must be a non-negative int.
    let s = deserialize::from_str("a:\n  type: str\n  validation:\n    min_len: -1\n").unwrap();
    assert!(matches!(
        verify(&d, &s),
        Err(VerifyError::SchemaMalformed(_))
    ));
    // huge int bounds compare correctly.
    let d = deserialize::from_str("a: 99999999999999999999\n").unwrap();
    let s =
        deserialize::from_str("a:\n  type: int\n  validation:\n    min: 99999999999999999998\n")
            .unwrap();
    assert!(verify(&d, &s).is_ok());
}
