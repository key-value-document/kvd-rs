#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(doc) = kvd_rs::deserialize::from_str(text) else {
        return;
    };
    // The canonical serializer must be idempotent: serializing a parsed document,
    // re-parsing it, and serializing again must produce byte-identical output.
    // A break here means the serializer is lossy or the format is not stable.
    let s1 = kvd_rs::serialize::to_string(&doc).expect("canonical serialize");
    let doc2 = kvd_rs::deserialize::from_str(&s1).expect("re-parse canonical form");
    let s2 = kvd_rs::serialize::to_string(&doc2).expect("re-serialize canonical form");
    assert_eq!(s1, s2, "canonical form is not idempotent for input {text:?}");
    assert_eq!(doc, doc2, "re-parsed document differs from original for {text:?}");
});
