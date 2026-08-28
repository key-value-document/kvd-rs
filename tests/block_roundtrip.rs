//! Regression tests for serializer/parser round-trips of triple-quoted
//! strings in list items (the fuzzer found the dash-indentation bug).

use kvd_rs::{deserialize::from_str, serialize::to_string};

fn roundtrip(src: &str) {
    let doc = from_str(src).unwrap_or_else(|e| panic!("parse failed: {e}"));
    let out = to_string(&doc).unwrap();
    let doc2 = from_str(&out).unwrap_or_else(|e| panic!("output {out:?} does not parse: {e}"));
    assert_eq!(doc, doc2, "round-trip changed the document");
    // serialization must be a fixed point after one normalization pass
    let out2 = to_string(&doc2).unwrap();
    assert_eq!(out, out2, "serialization is not a fixed point");
}

#[test]
fn triple_in_list_item() {
    roundtrip("iMM:\n  - M: \"\"\"\n      x\n    \"\"\"\n");
    roundtrip("iMM:\n  - M: \"\"\"\n      x\n      y\n    \"\"\"\n");
}

#[test]
fn triple_in_nested_list_item() {
    roundtrip("a:\n  b:\n    - c: \"\"\"\n        x\n      \"\"\"\n");
    roundtrip("a:\n  b:\n    - c: 1\n      d: \"\"\"\n          x\n      \"\"\"\n");
}

#[test]
fn empty_triple_spelling() {
    // A zero-content triple-quoted string is the empty string.
    let doc = from_str("k: \"\"\"\n\"\"\"\n").unwrap();
    let s = doc.as_map().unwrap().get("k").unwrap().as_scalar().unwrap();
    assert_eq!(s.text, "");
}
