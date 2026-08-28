#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    // Parsing must never panic: invalid input is an Err, not a crash.
    let first = kvd_rs::deserialize::from_str(text);
    // Parsing must be deterministic: identical input yields identical output.
    // This guards against stateful/unordered parsing or hashmap iteration leaks.
    let second = kvd_rs::deserialize::from_str(text);
    assert_eq!(
        first, second,
        "parser is non-deterministic for input {text:?}"
    );
});
