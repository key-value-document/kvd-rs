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

#[test]
fn scalar_type_mismatches_error() {
    // Every describe() arm: wrong shape for the target type.
    let err: SerdeError =
        from_str::<Cfg>("app:\n  port: \"8080\"\n  host: \"h\"\n  debug: true\n  ratio: 0.5\n")
            .unwrap_err();
    assert!(err.to_string().contains("int"), "{err}");
    let err: SerdeError =
        from_str::<Cfg>("app:\n  port: 1\n  host: 2\n  debug: true\n  ratio: 0.5\n").unwrap_err();
    assert!(err.to_string().contains("string"), "{err}");
    let err: SerdeError =
        from_str::<Cfg>("app:\n  port: 1\n  host: \"h\"\n  debug: yes\n  ratio: 0.5\n")
            .unwrap_err();
    assert!(err.to_string().contains("bool"), "{err}");
    let err: SerdeError =
        from_str::<Cfg>("app:\n  port: 1\n  host: \"h\"\n  debug: true\n  ratio: \"x\"\n")
            .unwrap_err();
    assert!(err.to_string().contains("float"), "{err}");
}

#[test]
fn int_out_of_range_errors() {
    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Small {
        n: i8,
    }
    // 999 does not fit in i8.
    let err: SerdeError = from_str::<Small>("n: 999\n").unwrap_err();
    assert!(!err.to_string().is_empty());
    // Huge literal beyond u64.
    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Big {
        n: u64,
    }
    let err: SerdeError = from_str::<Big>("n: 99999999999999999999999\n").unwrap_err();
    assert!(err.to_string().contains("64 bits"), "{err}");
    // Negative into unsigned.
    let err: SerdeError = from_str::<Big>("n: -5\n").unwrap_err();
    assert!(!err.to_string().is_empty());
}

#[test]
fn char_and_unit_types() {
    #[derive(Debug, PartialEq, Deserialize)]
    struct C {
        c: char,
    }
    let v: C = from_str("c: \"x\"\n").unwrap();
    assert_eq!(v.c, 'x');
    assert!(from_str::<C>("c: \"xy\"\n").is_err());
    assert!(from_str::<C>("c: 1\n").is_err());

    #[derive(Debug, PartialEq, Deserialize)]
    struct U {
        u: (),
    }
    let v: U = from_str("u: null\n").unwrap();
    assert_eq!(v.u, ());
    assert!(from_str::<U>("u: 1\n").is_err());
}

#[test]
fn bytes_as_int_lists() {
    // Root must be a map; wrap in a key.
    let mut m =
        from_str::<std::collections::BTreeMap<String, Vec<u8>>>("v:\n  - 1\n  - 2\n  - 255\n")
            .unwrap();
    assert_eq!(m.remove("v").unwrap(), vec![1u8, 2, 255]);
    // Serialize bytes back through a wrapper (root must be a mapping).
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct WB {
        v: Vec<u8>,
    }
    let w = WB { v: vec![1u8, 2, 3] };
    let text = to_string(&w).unwrap();
    let back: WB = from_str(&text).unwrap();
    assert_eq!(back, w);
}

#[test]
fn enum_error_paths() {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum E {
        A,
        B(String),
    }
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct H {
        e: E,
    }
    // Empty map is not an enum.
    assert!(from_str::<H>("e: {}\n").is_err());
    // Multi-entry map is not an externally tagged enum.
    assert!(from_str::<H>("e:\n  A: 1\n  B: 2\n").is_err());
    // Scalar int is not an enum.
    assert!(from_str::<H>("e: 1\n").is_err());
    // Unit variant where newtype expected.
    assert!(from_str::<H>("e: \"B\"\n").is_err());
    // Struct variant paths.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum S {
        P { x: i32, y: i32 },
        T(u8, String),
    }
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct HS {
        s: S,
    }
    let v: HS = from_str("s:\n  P:\n    x: 1\n    y: 2\n").unwrap();
    assert_eq!(v.s, S::P { x: 1, y: 2 });
    let v: HS = from_str("s:\n  T:\n    - 7\n    - \"x\"\n").unwrap();
    assert_eq!(v.s, S::T(7, "x".into()));
    // Unit variant from null payload.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum N {
        Nothing,
    }
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct HN {
        n: N,
    }
    let v: HN = from_str("n:\n  Nothing: null\n").unwrap();
    assert_eq!(v.n, N::Nothing);
    assert!(from_str::<HN>("n:\n  Nothing: 1\n").is_err());
}

#[test]
fn node_serialize_out_of_range() {
    use kvd_rs::value::{Node, Shape};
    // Integer text beyond u64 fails when the target is a serde number.
    let n = Node::scalar(Shape::Int, "99999999999999999999999");
    let v: Result<u64, SerdeError> = u64::deserialize(n.into_deserializer());
    use serde::de::IntoDeserializer;
    assert!(v.is_err());
    let _ = n;
}

#[test]
fn serde_error_conversions() {
    use kvd_rs::serde::error::SerdeError;
    // From io error.
    let io = std::io::Error::other("boom");
    let e = SerdeError::from(io);
    assert!(e.to_string().contains("boom"));
    // From KVD parse error.
    let perr = kvd_rs::deserialize::from_str("a: \"x").unwrap_err();
    let e = SerdeError::from(perr);
    assert!(!e.to_string().is_empty());
    // From serialize error (empty key).
    let serr = kvd_rs::serialize::to_string(&kvd_rs::value::Node::map({
        let mut m = kvd_rs::value::Map::new();
        m.insert(
            "".into(),
            kvd_rs::value::Node::scalar(kvd_rs::value::Shape::Int, "1"),
        );
        m
    }))
    .unwrap_err();
    let e = SerdeError::from(serr);
    assert!(!e.to_string().is_empty());
    // float_text rejects non-finite.
    assert!(kvd_rs::to_string(&f64::NAN).is_err());
    assert!(kvd_rs::to_string(&f64::INFINITY).is_err());
}

#[test]
fn map_key_must_be_string() {
    // Non-string map keys fail.
    let mut m = std::collections::BTreeMap::new();
    m.insert(1u8, "x");
    assert!(to_string(&m).is_err());
}

#[test]
fn tuple_struct_and_newtype_roundtrip() {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct TS(u8, String, bool);
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct W {
        t: TS,
    }
    let v = from_str::<W>("t:\n  - 7\n  - \"x\"\n  - true\n").unwrap().t;
    assert_eq!(v, TS(7, "x".into(), true));
    // Serialize back through wrapper.
    let w = W {
        t: TS(7, "x".into(), true),
    };
    assert_eq!(to_string(&w).unwrap(), "t:\n  - 7\n  - \"x\"\n  - true\n");
}

#[test]
fn all_int_widths_roundtrip() {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Widths {
        a: i8,
        b: i16,
        c: i32,
        d: i64,
        e: u8,
        f: u16,
        g: u32,
        h: u64,
        i: f32,
        j: char,
    }
    let w = Widths {
        a: -8,
        b: -300,
        c: -70000,
        d: -5,
        e: 250,
        f: 60000,
        g: 4000000000,
        h: 18000000000000000000,
        i: 1.5,
        j: 'z',
    };
    let text = to_string(&w).unwrap();
    let back: Widths = from_str(&text).unwrap();
    assert_eq!(back, w);
    // Wrong shapes error for each width.
    assert!(from_str::<Widths>(&text.replace("a: -8", "a: \"x\"")).is_err());
    assert!(from_str::<Widths>(&text.replace("j: \"z\"", "j: 1")).is_err());
}

#[test]
fn serde_wrong_shape_errors() {
    // bool target from int.
    assert!(from_str::<bool>("1\n").is_err());
    // seq target from scalar.
    assert!(from_str::<Vec<u8>>("1\n").is_err());
    // map target from scalar.
    assert!(from_str::<BTreeMap<String, u8>>("1\n").is_err());
    // f32 via f64 path with bad float text.
    assert!(from_str::<f64>("\"x\"\n").is_err());
    // unit struct.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Marker;
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct WM {
        m: Marker,
    }
    assert_eq!(from_str::<WM>("m: null\n").unwrap().m, Marker);
    // newtype struct.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct N(u32);
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct WN {
        n: N,
    }
    assert_eq!(from_str::<WN>("n: 5\n").unwrap().n, N(5));
}

#[test]
fn node_visitor_paths() {
    use kvd_rs::value::Node;
    use serde::de::{IntoDeserializer, value::BorrowedStrDeserializer};
    // Node::deserialize via a foreign deserializer (str).
    let n = Node::deserialize(BorrowedStrDeserializer::<SerdeError>::new("hi")).unwrap();
    assert_eq!(n.as_scalar().unwrap().text, "hi");
    // Via bool / numbers / unit / seq / map.
    use serde::de::value::{BoolDeserializer, I64Deserializer};
    let n = Node::deserialize(BoolDeserializer::<SerdeError>::new(true)).unwrap();
    assert_eq!(n.as_scalar().unwrap().text, "true");
    let n = Node::deserialize(I64Deserializer::<SerdeError>::new(-3)).unwrap();
    assert_eq!(n.as_scalar().unwrap().text, "-3");
    // into_deserializer on &Node.
    let node = kvd_rs::deserialize::from_str("a: 1\n").unwrap();
    let v: BTreeMap<String, u8> =
        serde::Deserialize::deserialize(node.into_deserializer()).unwrap();
    assert_eq!(v["a"], 1);
}

#[test]
fn node_into_foreign_serializer() {
    use kvd_rs::value::{Map, Node, Shape};
    use serde::Serializer;
    // Minimal sink serializer that records what it receives.
    #[derive(Debug, Default)]
    struct Sink {
        out: Vec<String>,
    }
    #[derive(Debug)]
    struct SinkErr(String);
    impl std::fmt::Display for SinkErr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }
    impl std::error::Error for SinkErr {}
    impl serde::ser::Error for SinkErr {
        fn custom<T: std::fmt::Display>(m: T) -> Self {
            SinkErr(m.to_string())
        }
    }
    impl Serializer for &mut Sink {
        type Ok = ();
        type Error = SinkErr;
        type SerializeSeq = serde::ser::Impossible<(), SinkErr>;
        type SerializeTuple = serde::ser::Impossible<(), SinkErr>;
        type SerializeTupleStruct = serde::ser::Impossible<(), SinkErr>;
        type SerializeTupleVariant = serde::ser::Impossible<(), SinkErr>;
        type SerializeMap = serde::ser::Impossible<(), SinkErr>;
        type SerializeStruct = serde::ser::Impossible<(), SinkErr>;
        type SerializeStructVariant = serde::ser::Impossible<(), SinkErr>;
        fn serialize_bool(self, v: bool) -> Result<(), SinkErr> {
            self.out.push(format!("bool:{v}"));
            Ok(())
        }
        fn serialize_i64(self, v: i64) -> Result<(), SinkErr> {
            self.out.push(format!("i64:{v}"));
            Ok(())
        }
        fn serialize_u64(self, v: u64) -> Result<(), SinkErr> {
            self.out.push(format!("u64:{v}"));
            Ok(())
        }
        fn serialize_f64(self, v: f64) -> Result<(), SinkErr> {
            self.out.push(format!("f64:{v}"));
            Ok(())
        }
        fn serialize_str(self, v: &str) -> Result<(), SinkErr> {
            self.out.push(format!("str:{v}"));
            Ok(())
        }
        fn serialize_none(self) -> Result<(), SinkErr> {
            self.out.push("none".into());
            Ok(())
        }
        fn serialize_char(self, v: char) -> Result<(), SinkErr> {
            self.out.push(format!("char:{v}"));
            Ok(())
        }
        fn serialize_bytes(self, v: &[u8]) -> Result<(), SinkErr> {
            self.out.push(format!("bytes:{}", v.len()));
            Ok(())
        }
        fn serialize_unit(self) -> Result<(), SinkErr> {
            self.out.push("unit".into());
            Ok(())
        }
        fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<(), SinkErr> {
            v.serialize(self)
        }
        fn serialize_newtype_struct<T: Serialize + ?Sized>(
            self,
            _n: &'static str,
            v: &T,
        ) -> Result<(), SinkErr> {
            v.serialize(self)
        }
        fn serialize_seq(
            self,
            _l: Option<usize>,
        ) -> Result<serde::ser::Impossible<(), SinkErr>, SinkErr> {
            Err(SinkErr("seq".into()))
        }
        fn serialize_tuple(
            self,
            _l: usize,
        ) -> Result<serde::ser::Impossible<(), SinkErr>, SinkErr> {
            Err(SinkErr("tup".into()))
        }
        fn serialize_tuple_struct(
            self,
            _n: &'static str,
            _l: usize,
        ) -> Result<serde::ser::Impossible<(), SinkErr>, SinkErr> {
            Err(SinkErr("ts".into()))
        }
        fn serialize_tuple_variant(
            self,
            _n: &'static str,
            _i: u32,
            _v: &'static str,
            _l: usize,
        ) -> Result<serde::ser::Impossible<(), SinkErr>, SinkErr> {
            Err(SinkErr("tv".into()))
        }
        fn serialize_map(
            self,
            _l: Option<usize>,
        ) -> Result<serde::ser::Impossible<(), SinkErr>, SinkErr> {
            Err(SinkErr("map".into()))
        }
        fn serialize_struct(
            self,
            _n: &'static str,
            _l: usize,
        ) -> Result<serde::ser::Impossible<(), SinkErr>, SinkErr> {
            Err(SinkErr("struct".into()))
        }
        fn serialize_struct_variant(
            self,
            _n: &'static str,
            _i: u32,
            _v: &'static str,
            _l: usize,
        ) -> Result<serde::ser::Impossible<(), SinkErr>, SinkErr> {
            Err(SinkErr("sv".into()))
        }
        fn serialize_unit_struct(self, _n: &'static str) -> Result<(), SinkErr> {
            self.out.push("unit_struct".into());
            Ok(())
        }
        fn serialize_unit_variant(
            self,
            _n: &'static str,
            _i: u32,
            v: &'static str,
        ) -> Result<(), SinkErr> {
            self.out.push(format!("unit_variant:{v}"));
            Ok(())
        }
        fn serialize_newtype_variant<T: Serialize + ?Sized>(
            self,
            _n: &'static str,
            _i: u32,
            _v: &'static str,
            _x: &T,
        ) -> Result<(), SinkErr> {
            Err(SinkErr("ntv".into()))
        }
        fn serialize_i8(self, v: i8) -> Result<(), SinkErr> {
            self.serialize_i64(v as i64)
        }
        fn serialize_i16(self, v: i16) -> Result<(), SinkErr> {
            self.serialize_i64(v as i64)
        }
        fn serialize_i32(self, v: i32) -> Result<(), SinkErr> {
            self.serialize_i64(v as i64)
        }
        fn serialize_u8(self, v: u8) -> Result<(), SinkErr> {
            self.serialize_u64(v as u64)
        }
        fn serialize_u16(self, v: u16) -> Result<(), SinkErr> {
            self.serialize_u64(v as u64)
        }
        fn serialize_u32(self, v: u32) -> Result<(), SinkErr> {
            self.serialize_u64(v as u64)
        }
        fn serialize_f32(self, v: f32) -> Result<(), SinkErr> {
            self.serialize_f64(v as f64)
        }
    }
    use serde::Serialize;
    let mut sink = Sink::default();
    Node::scalar(Shape::Str, "hi").serialize(&mut sink).unwrap();
    Node::scalar(Shape::Int, "42").serialize(&mut sink).unwrap();
    // Fits u64 but not i64: exercises the u64 fallback.
    Node::scalar(Shape::Int, "17000000000000000000")
        .serialize(&mut sink)
        .unwrap();
    Node::scalar(Shape::Float, "0.5")
        .serialize(&mut sink)
        .unwrap();
    Node::scalar(Shape::Bool, "true")
        .serialize(&mut sink)
        .unwrap();
    Node::scalar(Shape::Null, "null")
        .serialize(&mut sink)
        .unwrap();
    assert_eq!(
        sink.out,
        vec![
            "str:hi",
            "i64:42",
            "u64:17000000000000000000",
            "f64:0.5",
            "bool:true",
            "none"
        ]
    );
    // Out-of-range int and bad float error.
    let mut sink = Sink::default();
    assert!(
        Node::scalar(Shape::Int, "99999999999999999999999")
            .serialize(&mut sink)
            .is_err()
    );
    assert!(
        Node::scalar(Shape::Float, "abc")
            .serialize(&mut sink)
            .is_err()
    );
    // Map/list route through serialize_map/serialize_seq (Impossible errors).
    let mut m = Map::new();
    m.insert("a".into(), Node::scalar(Shape::Int, "1"));
    assert!(Node::map(m).serialize(&mut sink).is_err());
    assert!(
        Node::list(vec![Node::scalar(Shape::Int, "1")])
            .serialize(&mut sink)
            .is_err()
    );
}

#[test]
fn tuple_variant_and_bytes_serialize() {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum E {
        T(u8, String),
        S { x: i32 },
    }
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct W {
        e: E,
    }
    let w = W {
        e: E::T(7, "x".into()),
    };
    let text = to_string(&w).unwrap();
    assert_eq!(from_str::<W>(&text).unwrap(), w);
    let w = W { e: E::S { x: 3 } };
    let text = to_string(&w).unwrap();
    assert_eq!(from_str::<W>(&text).unwrap(), w);
    // f32, char, bytes, unit struct, newtype via NodeSerializer directly.
    use kvd_rs::serde::serialize::NodeSerializer;
    let n = 1.5f32.serialize(NodeSerializer).unwrap();
    assert_eq!(n.as_scalar().unwrap().text, "1.5");
    let n = 'q'.serialize(NodeSerializer).unwrap();
    assert_eq!(n.as_scalar().unwrap().text, "q");
    let n = serde::Serialize::serialize(&b"hi"[..], NodeSerializer).unwrap();
    assert_eq!(n.as_list().unwrap().len(), 2);
    #[derive(Debug, Serialize)]
    struct US;
    let n = US.serialize(NodeSerializer).unwrap();
    assert_eq!(n.as_scalar().unwrap().shape, kvd_rs::value::Shape::Null);
}

#[test]
fn serde_describe_and_shape_errors() {
    use kvd_rs::value::{Node, Shape};
    use serde::de::IntoDeserializer;
    // describe() arms via invalid_type: float shape where bool expected.
    let node = Node::scalar(Shape::Float, "0.5");
    assert!(bool::deserialize(node.into_deserializer()).is_err());
    // null shape where bool expected.
    let node = Node::scalar(Shape::Null, "null");
    assert!(bool::deserialize(node.into_deserializer()).is_err());
    // map shape where bool expected.
    let node = kvd_rs::deserialize::from_str("a: 1\n").unwrap();
    assert!(bool::deserialize(node.into_deserializer()).is_err());
    // list shape where bool expected.
    let node = Node::list(vec![Node::scalar(Shape::Int, "1")]);
    assert!(bool::deserialize(node.into_deserializer()).is_err());
    // visit_by_shape bool arm with bad text (unreachable via parser, but
    // reachable by constructing the node directly).
    let node = Node::scalar(Shape::Bool, "yes");
    assert!(bool::deserialize(node.into_deserializer()).is_err());
    // visit_by_shape float arm with unparseable text.
    let node = Node::scalar(Shape::Float, "abc");
    assert!(f64::deserialize(node.into_deserializer()).is_err());
    // i64 overflow via huge literal.
    let node = Node::scalar(Shape::Int, "99999999999999999999999");
    assert!(i64::deserialize(node.into_deserializer()).is_err());
    // u64 negative.
    let node = Node::scalar(Shape::Int, "-5");
    assert!(u64::deserialize(node.into_deserializer()).is_err());
    // char with multi-char string.
    assert!(char::deserialize(Node::scalar(Shape::Str, "xy").into_deserializer()).is_err());
    // unit variant payload errors: newtype/tuple/struct on unit variant.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum E {
        A(u8),
        T(u8, u8),
        S { x: u8 },
    }
    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct H {
        e: E,
    }
    assert!(from_str::<H>("e: \"A\"\n").is_err());
    assert!(from_str::<H>("e: \"T\"\n").is_err());
    assert!(from_str::<H>("e: \"S\"\n").is_err());
    // StrDe enum path (nested enums unsupported).
    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct HE {
        e: E,
    }
    let _ = HE { e: E::A(1) };
    // MapDe pending panic path is unreachable by construction; skip.
    // NodeVisitor remaining arms via serde_json-less round trip through Node.
    let n: Node =
        serde::Deserialize::deserialize(serde::de::value::U64Deserializer::<SerdeError>::new(7))
            .unwrap();
    assert_eq!(n.as_scalar().unwrap().text, "7");
    let n: Node =
        serde::Deserialize::deserialize(serde::de::value::F64Deserializer::<SerdeError>::new(0.5))
            .unwrap();
    assert_eq!(n.as_scalar().unwrap().text, "0.5");
    let n: Node = serde::Deserialize::deserialize(
        serde::de::value::BoolDeserializer::<SerdeError>::new(false),
    )
    .unwrap();
    assert_eq!(n.as_scalar().unwrap().text, "false");
}
