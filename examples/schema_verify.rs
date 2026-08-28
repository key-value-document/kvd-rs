//! Schema verification demo (spec §5).
//!
//! Run with: cargo run -p kvd-rs --example schema_verify

use kvd_rs::schema::{verify_from_str, VerifyError};

fn main() {
    // 1. Standalone schema document (bare tree, builtin types only).
    let data = "server:\n  host: localhost\n  ports: 5432\n";
    let schema = "server:\n  host: str\n  ports: int\n";
    match verify_from_str(data, schema) {
        Ok(()) => println!("external: OK"),
        Err(VerifyError::Violations(vs)) => {
            println!("external: {} violation(s)", vs.len());
            for v in &vs {
                println!("  {v}");
            }
        }
        Err(e) => println!("external: {e}"),
    }

    // 2. A violation: `ports` is a string, not an int.
    let bad = "server:\n  host: localhost\n  ports: five-thousand\n";
    match verify_from_str(bad, schema) {
        Ok(()) => println!("bad: OK (unexpected)"),
        Err(VerifyError::Violations(vs)) => {
            println!("bad: {} violation(s)", vs.len());
            for v in &vs {
                println!("  {v}");
            }
        }
        Err(e) => println!("bad: {e}"),
    }
}
