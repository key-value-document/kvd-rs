//! Integration tests for the `serde` feature: derived types against real
//! KVD text, strict shape enforcement, enum tagging, and canonical
//! round-trips through the DOM emitter.
#![cfg(feature = "serde")]

use kvd_rs::{deserialize, from_str, serde::error::SerdeError, to_string};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Cfg {
    app: App,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct App {
    port: u16,
    host: String,
    debug: bool,
    ratio: f64,
}

const TEXT: &str = "app:\n  port: 8_080\n  host: \"cluster.local\"\n  debug: true\n  ratio: 0.75\n";

#[test]
fn readme_example() {
    let cfg: Cfg = from_str(TEXT).unwrap();
    assert_eq!(cfg.app.port, 8080);
    assert_eq!(cfg.app.host, "cluster.local");
    assert!(cfg.app.debug);
    assert!((cfg.app.ratio - 0.75).abs() < f64::EPSILON);
}

#[test]
fn derived_roundtrip_is_byte_identical() {
    let cfg: Cfg = from_str(TEXT).unwrap();
    assert_eq!(to_string(&cfg).unwrap(), TEXT);
}

#[test]
fn serialize_produces_canonical_text() {
    let cfg = Cfg {
        app: App {
            port: 8080,
            host: "cluster.local".into(),
            debug: true,
            ratio: 0.75,
        },
    };
    assert_eq!(to_string(&cfg).unwrap(), TEXT);
}

#[test]
fn shapes_are_strict() {
    // Quoted number is a string, not an int.
    let err = from_str::<Cfg>("app:\n  port: \"8080\"\n").unwrap_err();
    assert!(err.to_string().contains("invalid type: string"), "{err}");

    // Bare string where bool expected.
    let err = from_str::<Cfg>(TEXT.replace("debug: true", "debug: yes").as_str()).unwrap_err();
    assert!(err.to_string().contains("bool"), "{err}");

    // Int where float expected (no coercion).
    let err = from_str::<Cfg>(TEXT.replace("ratio: 0.75", "ratio: 1").as_str()).unwrap_err();
    assert!(err.to_string().contains("float"), "{err}");
}

#[test]
fn underscore_separators_parse() {
    #[derive(Deserialize)]
    struct Big {
        n: u64,
    }
    let big: Big = from_str("n: 1_000_000\n").unwrap();
    assert_eq!(big.n, 1_000_000);
}

#[test]
fn option_fields_accept_missing_and_null() {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Opt {
        a: Option<u16>,
        b: Option<String>,
    }

    // Missing keys deserialize as None...
    let parsed: Opt = from_str("a: 1\n").unwrap();
    assert_eq!(
        parsed,
        Opt {
            a: Some(1),
            b: None
        }
    );

    // ...and explicit null too.
    let parsed: Opt = from_str("a: null\nb: null\n").unwrap();
    assert_eq!(parsed, Opt { a: None, b: None });

    // Serializing None emits null; both forms parse back identically.
    let text = to_string(&Opt { a: None, b: None }).unwrap();
    assert_eq!(text, "a: null\nb: null\n");
}

#[test]
fn unit_enums_read_bare_strings() {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Route {
        proto: Proto,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum Proto {
        Tcp,
        Udp,
    }

    let route: Route = from_str("proto: \"Tcp\"\n").unwrap();
    assert_eq!(route.proto, Proto::Tcp);

    // Variant names are case-sensitive.
    assert!(from_str::<Route>("proto: \"tcp\"\n").is_err());

    // Serializes back to the quoted variant name.
    assert_eq!(to_string(&route).unwrap(), "proto: \"Tcp\"\n");
}

#[test]
fn externally_tagged_enums_roundtrip() {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum Value {
        Plain(String),
        Numbers(Vec<u32>),
        Point { x: i32, y: i32 },
        Nothing,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Holder {
        v: Value,
    }

    for (text, expected) in [
        ("v:\n  Plain: \"hi\"\n", Value::Plain("hi".into())),
        (
            "v:\n  Numbers:\n    - 1\n    - 2\n",
            Value::Numbers(vec![1, 2]),
        ),
        (
            "v:\n  Point:\n    x: 3\n    y: 4\n",
            Value::Point { x: 3, y: 4 },
        ),
        ("v: \"Nothing\"\n", Value::Nothing),
    ] {
        let holder: Holder = from_str(text).unwrap();
        assert_eq!(holder.v, expected, "{text}");
        assert_eq!(to_string(&holder).unwrap(), text);
    }
}

#[test]
fn nested_structs_lists_and_maps() {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Project {
        name: String,
        services: BTreeMap<String, Service>,
        tags: Vec<String>,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Service {
        image: String,
        replicas: u16,
    }

    let project = Project {
        name: "web".into(),
        services: BTreeMap::from([
            (
                "api".into(),
                Service {
                    image: "api:1".into(),
                    replicas: 2,
                },
            ),
            (
                "ui".into(),
                Service {
                    image: "ui:1".into(),
                    replicas: 1,
                },
            ),
        ]),
        tags: vec!["a".into(), "b".into()],
    };

    let text = to_string(&project).unwrap();
    // BTreeMap order is deterministic: api before ui.
    assert!(text.find("api:").unwrap() < text.find("ui:").unwrap());

    let parsed: Project = from_str(&text).unwrap();
    assert_eq!(parsed, project);
}

#[test]
fn unknown_fields_are_ignored_by_default() {
    #[derive(Debug, Deserialize)]
    struct Narrow {
        keep: u8,
    }

    let parsed: Narrow = from_str("keep: 7\ndrop: \"me\"\nextra:\n  nested: true\n").unwrap();
    assert_eq!(parsed.keep, 7);
}

#[test]
fn node_serde_preserves_semantics() {
    use kvd_rs::value::Node;

    // T -> canonical text -> DOM equals the original document's DOM.
    let cfg: Cfg = from_str(TEXT).unwrap();
    let text = to_string(&cfg).unwrap();
    assert_eq!(text, TEXT);
    let doc = deserialize::from_str(TEXT).unwrap();
    let doc2 = deserialize::from_str(&text).unwrap();
    assert_eq!(doc, doc2);

    // Node itself implements Serialize/Deserialize.
    let via_serde: Node = from_str(TEXT).unwrap();
    assert_eq!(via_serde, doc);
}

#[test]
fn floats_always_carry_a_fraction() {
    #[derive(Serialize, Deserialize)]
    #[allow(dead_code)]
    struct F {
        one: f64,
        big: f64,
    }
    let text = to_string(&F {
        one: 1.0,
        big: 1e20,
    })
    .unwrap();
    assert!(text.contains("one: 1.0\n"), "{text}");
    assert!(text.contains("big: 100000000000000000000.0\n"), "{text}");

    let back: F = from_str(&text).unwrap();
    assert_eq!(back.one, 1.0);
    assert_eq!(back.big, 1e20);
}

#[test]
fn non_finite_floats_are_rejected() {
    let err = to_string(&f64::INFINITY).unwrap_err();
    assert!(err.to_string().contains("non-finite"));
}

#[test]
fn file_and_reader_writer_roundtrip() {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Point {
        x: i32,
        y: i32,
    }

    let dir = std::env::temp_dir().join(format!("kvd-serde-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("point.kvd");

    let point = Point { x: 3, y: -4 };
    kvd_rs::to_file(&path, &point).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x: 3\ny: -4\n");
    let back: Point = kvd_rs::from_file(&path).unwrap();
    assert_eq!(back, point);

    // Reader/writer variants.
    let from_cursor: Point = kvd_rs::from_reader(std::io::Cursor::new(b"x: 1\ny: 2\n")).unwrap();
    assert_eq!(from_cursor, Point { x: 1, y: 2 });

    let mut buf = Vec::new();
    kvd_rs::to_writer(&mut buf, &point).unwrap();
    assert_eq!(String::from_utf8(buf).unwrap(), "x: 3\ny: -4\n");

    // Missing files surface as IO errors.
    let err = kvd_rs::from_file::<Point, _>(dir.join("missing.kvd")).unwrap_err();
    assert!(err.to_string().contains("No such file"), "{err}");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn parse_errors_surface_through_serde() {
    let err: SerdeError = from_str::<Cfg>("app:\n  port: 8080").unwrap_err();
    // Missing value for `port` (no trailing newline subtree) or missing
    // sibling fields — either way it must be a KVD error, not a panic.
    assert!(!err.to_string().is_empty());
}

#[test]
fn newtype_and_tuple_forms() {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Meters(u32);

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Pair(u8, String);

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Wrap {
        m: Meters,
        p: Pair,
    }

    let w: Wrap = from_str("m: 42\np:\n  - 7\n  - \"x\"\n").unwrap();
    assert_eq!(w.m, Meters(42));
    assert_eq!(w.p, Pair(7, "x".into()));
    assert_eq!(to_string(&w).unwrap(), "m: 42\np:\n  - 7\n  - \"x\"\n");
}
