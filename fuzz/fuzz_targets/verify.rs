#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    // Schema verification must never panic. Both the document and the schema may
    // fail to parse, which is a valid (Err-returning) input, not a crash. This
    // exercises the verifier, the schema parser, and type-coercion paths.
    let _ = kvd_rs::schema::verify_from_str(text, text);
});
