//! `bytes`: the schema-level binary type (spec §5, §9).

use kvd_rs::base64::{BytesError, decode, decoded_len, encode};
use kvd_rs::schema::{VerifyError, verify_from_str};

fn schema(leaf: &str) -> String {
    format!("cert:\n{leaf}")
}

fn check(data: &str, leaf: &str) -> Result<(), VerifyError> {
    verify_from_str(&format!("cert: {data}\n"), &schema(leaf))
}

#[test]
fn accepts_valid_base64() {
    assert_eq!(check("\"AQIDBA==\"", "  type: bytes"), Ok(()));
    assert_eq!(check("\"\"", "  type: bytes"), Ok(()));
    assert_eq!(check("\"abcd\"", "  type: bytes"), Ok(()));
}

#[test]
fn rejects_non_base64_text() {
    for (data, want) in [
        ("\"AQIDB\"", BytesError::Length),         // 5 chars
        ("\"AQID#A==\"", BytesError::Alphabet),    // '#' is not in the alphabet
        ("\"=AAA\"", BytesError::Padding),         // padding is not leading
        ("\"AA=A\"", BytesError::Padding),         // padding is not internal
        ("\"A===\"", BytesError::Padding),         // three pad characters
        ("\"AR==\"", BytesError::PadBits),         // non-zero pad bits
        ("\"AQ IDBA==\"", BytesError::Whitespace), // space is content
    ] {
        match check(data, "  type: bytes") {
            Err(VerifyError::Violations(v)) => {
                assert!(
                    v[0].message.starts_with("invalid bytes"),
                    "{data}: expected an invalid-bytes violation, got {:?}",
                    v[0].message
                );
            }
            other => panic!("{data}: expected a violation, got {other:?}"),
        }
        // The decoder reports the specific rule.
        let text = data.trim_matches('"');
        assert_eq!(decoded_len(text).unwrap_err(), want, "text {text:?}");
    }
}

#[test]
fn pad_bits_are_stricter_than_lenient_decoders() {
    // `AR==` and `AQ==` both decode to 0x01 in most base64 libraries; KVD
    // requires the canonical encoding (RFC 4648 §3.5).
    assert_eq!(decoded_len("AQ==").unwrap(), 1);
    assert_eq!(decoded_len("AR==").unwrap_err(), BytesError::PadBits);
}

#[test]
fn line_breaks_are_layout_and_spaces_are_not() {
    // A wrapped value decodes to the same octets as the unwrapped one.
    assert_eq!(decoded_len("AQID\nBA==").unwrap(), 4);
    assert_eq!(decoded_len("AQID\r\nBA==").unwrap(), 4);
    assert_eq!(
        decoded_len("AQID\n  BA==").unwrap_err(),
        BytesError::Whitespace,
        "a space used for indentation is still content"
    );
}

#[test]
fn multiline_block_decodes() {
    // A `"""` block: the standalone closer's trailing newline is dropped.
    let doc = "cert: \"\"\"\n  AQIDBA==\n\"\"\"\n";
    assert_eq!(verify_from_str(doc, &schema("  type: bytes")), Ok(()));
}

#[test]
fn length_constraints_count_decoded_octets() {
    // "AQIDBA==" is 8 base64 characters but 4 octets.
    assert_eq!(
        check(
            "\"AQIDBA==\"",
            "  type: bytes\n  validation:\n    max_len: 4"
        ),
        Ok(())
    );
    assert_eq!(decoded_len("AQIDBA==").unwrap(), 4);
    // One octet too many.
    match check(
        "\"AQIDBA==\"",
        "  type: bytes\n  validation:\n    max_len: 3",
    ) {
        Err(VerifyError::Violations(v)) => {
            assert!(
                v[0].message.contains("exceeds max_len 3"),
                "{:?}",
                v[0].message
            );
        }
        other => panic!("expected a constraint violation, got {other:?}"),
    }
}

#[test]
fn pattern_and_numeric_bounds_are_malformed_on_bytes() {
    match verify_from_str(
        "cert: \"AQIDBA==\"\n",
        "cert:\n  type: bytes\n  validation:\n    pattern: \"^[A-Za-z]+$\"\n",
    ) {
        Err(VerifyError::SchemaMalformed(v)) => {
            assert!(
                v[0].message.contains("unknown constraint `pattern`"),
                "{:?}",
                v[0].message
            );
        }
        other => panic!("expected a malformed schema, got {other:?}"),
    }
    match verify_from_str(
        "cert: \"AQIDBA==\"\n",
        "cert:\n  type: bytes\n  validation:\n    min: 1\n",
    ) {
        Err(VerifyError::SchemaMalformed(v)) => {
            assert!(
                v[0].message.contains("unknown constraint `min`"),
                "{:?}",
                v[0].message
            );
        }
        other => panic!("expected a malformed schema, got {other:?}"),
    }
}

#[test]
fn only_a_string_is_bytes() {
    // A bare type name is the string "bytes" (spec §5), not a bytes value.
    match check("bytes", "  type: bytes") {
        Err(VerifyError::Violations(v)) => {
            assert!(v[0].message.contains("invalid bytes"), "{:?}", v[0].message);
        }
        other => panic!("expected a violation, got {other:?}"),
    }
    // A number is a number.
    assert!(check("12345", "  type: bytes").is_err());
}

#[test]
fn null_and_absence_follow_the_optional_rule() {
    assert_eq!(check("null", "  type: bytes\n  optional: true"), Ok(()));
    assert!(
        check("null", "  type: bytes").is_err(),
        "null needs optional: true"
    );
}

#[test]
fn roundtrips_through_encode_and_decode() {
    for octets in [
        vec![],
        vec![0u8],
        vec![0, 1, 2, 3],
        vec![0xff, 0x00, 0x7f],
        (0..=255u8).collect::<Vec<u8>>(),
    ] {
        let text = encode(&octets);
        assert_eq!(decoded_len(&text).unwrap(), octets.len());
        assert_eq!(decode(&text).unwrap(), octets);
        // Canonical output is a single unpadded-look line with no whitespace.
        assert!(!text.contains(['\n', '\r', ' ']));
    }
}

#[test]
fn encode_decode_agree_on_known_vectors() {
    // RFC 4648 §10 test vectors.
    assert_eq!(encode(b""), "");
    assert_eq!(encode(b"f"), "Zg==");
    assert_eq!(encode(b"fo"), "Zm8=");
    assert_eq!(encode(b"foo"), "Zm9v");
    assert_eq!(encode(b"foob"), "Zm9vYg==");
    assert_eq!(encode(b"fooba"), "Zm9vYmE=");
    assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    for v in [
        "", "Zg==", "Zm8=", "Zm9v", "Zm9vYg==", "Zm9vYmE=", "Zm9vYmFy",
    ] {
        assert!(decoded_len(v).is_ok(), "vector {v:?} should decode");
    }
}
