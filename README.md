# kvd-rs

Rust implementation of KVD, an opinionated human-readable config/data format.

[![crates.io](https://img.shields.io/crates/v/kvd-rs)](https://crates.io/crates/kvd-rs)
[![docs.rs](https://img.shields.io/docsrs/kvd-rs)](https://docs.rs/kvd-rs)
[![CI](https://github.com/key-value-document/kvd-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/key-value-document/kvd-rs/actions/workflows/ci.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Installation

```sh
cargo add kvd-rs --features serde
```

Or add to `Cargo.toml`:

```toml
[dependencies]
kvd-rs = { version = "1.0.0", features = ["serde"] }
serde = { version = "1.0.0", features = ["derive"] }
```

Without `serde` the crate has a single dependency (`regex`) for `§10` validation and exposes the `Node` API; the crate is `std` (validation requires `std`).

## Usage

### Typed: serde

Derive `Serialize`/`Deserialize` and read/write files directly:

```rust
#[derive(serde::Serialize, serde::Deserialize)]
struct App { port: u16, retries: Option<u32> }

let app: App = kvd_rs::from_file("app.kvd")?;
assert_eq!(app.port, 8080);
kvd_rs::to_file("app.kvd", &app)?;
```

Other entry points:

| Function | Input / Output |
|---|---|
| `from_file(path)` | file -> `T` |
| `from_str(text)` | `&str` -> `T` |
| `from_reader(reader)` | `Read` -> `T` |
| `to_file(path, &value)` | `T` -> file |
| `to_string(&value)` | `T` -> `String` |
| `to_writer(writer, &value)` | `T` -> `Write` |

Shapes are enforced strictly: an integer target rejects quoted text, floats always emit with a fraction (`1.0`), and non-finite floats are an error. Externally-tagged enums round-trip as single-entry maps; unit variants read bare strings.

### Dynamic: Node

When the shape is unknown, or you need lossless round-tripping, schema verification, or structural editing, use the `Node` API (`Scalar | Map | List` with `get`/`get_path` that name the full path on failure):

```rust
let text = std::fs::read_to_string("app.kvd")?;
let doc = kvd_rs::deserialize::from_str(&text)?; // text -> Node
let port = doc.get_path(&["app", "port"])?;      // Result<&Node>
assert_eq!(port.as_scalar().unwrap().text, "8080");
let text = kvd_rs::serialize::to_string(&doc)?;  // canonical form
std::fs::write("app.kvd", text)?;
```

`Node` is the core of the crate: serde, schema verification, and lossless serialization are all built on top of it. For typed documents prefer `kvd_rs::from_file` / `to_file`; the `Node` API is file-agnostic so you read the file yourself and then call `deserialize::from_str`.

## Example

Data (`app.kvd`):

```
# server config
app:
  name: "hello"
  port: 8080
  version: "1.5.2"
  endpoints:
    - path: "/health"
      method: "GET"
  hooks:
    - """
      #!/bin/sh
      echo hi
      """
  retries: null
```

Companion schema (`app.schema.kvd`): a bare tree whose values mirror the data structure:

```
app:
  name: str
  port: int
  version: str
  endpoints:
    type: list
    element:
      path: str
      method: str
  hooks:
    type: list
    element: str
  retries:
    type: int
    optional: true # absent, int or null
```

## Schemas and verification

```rust
let doc_text = std::fs::read_to_string("app.kvd")?;
let schema_text = std::fs::read_to_string("app.schema.kvd")?;
let doc = kvd_rs::deserialize::from_str(&doc_text)?;       // text -> Node
let schema = kvd_rs::deserialize::from_str(&schema_text)?; // bare key: type tree
if let Err(violations) = kvd_rs::schema::verify(&doc, &schema) {
    for v in &violations {
        eprintln!("{v}"); // path + message per violation
    }
}
// optional types: verify(&doc, &schema) where `retries: { type: int, optional: true }` allows absence/null
```

To verify against an embedded schema (`__schema__` at the document root), use `kvd_rs::schema::verify_embedded(&doc)`.

## Errors

Parse errors carry `line:col` and an `ErrorKind` (e.g. `bad-indent`, `unexpected-character`). Verification returns a list of `Violation` values with dotted paths:

```rust
match kvd_rs::deserialize::from_str(text) {
    Ok(doc) => println!("ok: {doc:?}"),
    Err(e) => eprintln!("{} at {}:{}", e.kind(), e.line(), e.col()),
}
```

## Documentation

API docs at [docs.rs/kvd-rs](https://docs.rs/kvd-rs). Locally: `cargo doc --open`.

Spec 1.0 at [key-value-document/kvd-spec](https://github.com/key-value-document/kvd-spec).

## License

MIT
