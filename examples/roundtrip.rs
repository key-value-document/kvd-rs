//! Round-trip check: `cargo run -p kvd-rs --example roundtrip -- FILE`
//! Parses FILE, re-serializes, re-parses, and compares node trees
//! (serialization is canonical: comments and blank lines are not preserved).

use std::fs;

fn main() {
    let path = std::env::args().nth(1).expect("usage: roundtrip FILE");
    let text = fs::read_to_string(&path).expect("read");
    let node = match kvd_rs::deserialize::from_str(&text) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("parse failed: {e}");
            std::process::exit(1);
        }
    };
    let out = kvd_rs::serialize::to_string(&node).expect("serialize");
    let reparsed = match kvd_rs::deserialize::from_str(&out) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("re-parse failed: {e}");
            eprintln!("{out}");
            std::process::exit(1);
        }
    };
    if reparsed == node {
        println!(
            "round-trip OK ({} bytes in, {} bytes canonical)",
            text.len(),
            out.len()
        );
        if out == text {
            println!("byte-identical");
        } else {
            for (i, (a, b)) in text.lines().zip(out.lines()).enumerate() {
                if a != b {
                    println!("first diff at line {}:\n  in : {a}\n  out: {b}", i + 1);
                    break;
                }
            }
        }
    } else {
        println!("NODE MISMATCH after re-parse");
        std::process::exit(1);
    }
}
