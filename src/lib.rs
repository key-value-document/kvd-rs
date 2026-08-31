//! KVD — Key-Value Document format (see `docs/README.md`).
//!
//! This crate provides the value model ([`value`]), error model
//! ([`error`]), grammar predicates ([`grammar`]), parser
//! ([`deserialize`]), serializer ([`serialize`]), schema verification
//! ([`schema`]), and programmatic document editing ([`ops`]) for the KVD
//! format. YAML conversion and the `yaml2kvd` binary live in the
//! `kvd-yaml` crate.
//!
//! With the optional `serde` feature, any `T: Deserialize`/`T: Serialize`
//! works directly via the `serde` module's `from_str`/`to_string`
//! functions.
//!
//! ```
//! # #[cfg(feature = "serde")]
//! # mod serde_demo {
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize)]
//! struct App {
//!     name: String,
//!     port: u16,
//!     retries: Option<u32>,
//! }
//!
//! # pub fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let app: App = kvd_rs::from_str("name: \"hello\"\nport: 8080\nretries: null\n")?;
//! assert_eq!(app.port, 8080);
//! assert_eq!(app.retries, None);
//!
//! let canonical = kvd_rs::to_string(&app)?;
//! assert_eq!(canonical, "name: \"hello\"\nport: 8_080\nretries: null\n");
//!
//! // Reader/writer/file variants exist too:
//! let app2: App = kvd_rs::from_reader(std::io::Cursor::new(canonical.as_bytes()))?;
//! assert_eq!(app2.name, "hello");
//! # Ok(())
//! # }
//! # }
//! ```

// The `serde` feature pulls in `std`-only APIs (`std::io`/`std::fs`, file
// round-tripping, and `std::error::Error` for `SerdeError`), so the crate
// must use `std` whenever that feature is enabled.
#![cfg_attr(not(any(test, feature = "serde")), no_std)]
#![warn(missing_docs)]

extern crate alloc;

pub mod deserialize;
pub mod error;
pub mod grammar;
pub mod ops;
pub mod schema;
#[cfg(feature = "serde")]
pub mod serde;
pub mod serialize;
pub mod value;

/// Maximum nesting depth accepted by the parser and enforced by the
/// serializer: indentation levels and dotted path segments counted
/// together (spec §2).
pub const MAX_DEPTH: usize = 100;

#[cfg(feature = "serde")]
pub use serde::{from_file, from_reader, from_str, to_file, to_string, to_writer};
