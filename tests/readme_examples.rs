//! The README's examples, compiled and run so they cannot drift.
//!
//! README code blocks are not doctests, so nothing else keeps them honest.

#[test]
fn binary_data_example() {
    // README: "Binary data: `bytes` (1.2)"
    let ok = kvd_rs::schema::verify_from_str("cert: \"AQIDBA==\"\n", "cert: bytes\n");
    assert_eq!(ok, Ok(()));

    let octets = kvd_rs::base64::decode("AQIDBA==").expect("valid base64");
    assert_eq!(octets, vec![1, 2, 3, 4]);
    let text = kvd_rs::base64::encode(&octets);
    assert_eq!(text, "AQIDBA==");
}

#[test]
fn installation_snippet_uses_the_current_version() {
    // The README must not advertise a version older than the crate.
    let readme = include_str!("../README.md");
    let manifest = include_str!("../Cargo.toml");
    let version = manifest
        .lines()
        .find_map(|l| l.strip_prefix("version = \""))
        .and_then(|l| l.split('"').next())
        .expect("crate version");
    assert!(
        readme.contains(&format!("version = \"{version}\"")),
        "README should document kvd-rs {version}"
    );
}
