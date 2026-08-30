# KVD — Key-Value Document format

KVD is an opinionated human-readable config/data format.

## Quickstart

### Typed (recommended): serde

If you know the shape of your document, derive `Serialize`/`Deserialize`
and let serde do the traversal. Enable the `serde` feature:

```toml
[dependencies]
kvd-rs = { version = "1.0.0", features = ["serde"] }
serde = { version = "1.0.0", features = ["derive"] }
```

```rust
#[derive(serde::Serialize, serde::Deserialize)]
struct App { port: u16 }

let app: App = kvd_rs::from_str("app:\n  port: 8080\n")?;   // KVD text -> T
assert_eq!(app.port, 8080);
let text = kvd_rs::to_string(&app)?;                        // canonical form
// also: from_reader / from_file / to_writer / to_file
```

### Dynamic: the `Node` value model

When the shape is unknown, or you need lossless round-tripping, schema
verification, or structural editing, use the `Node` API. `Node` is a sum
type (`Scalar | Map | List`); `get`/`get_path` navigate it and return
errors that name the full path on failure:

```rust
let doc = kvd_rs::deserialize::from_str("app:\n  port: 8080\n")?;  // text -> Node
let port = doc.get_path(&["app", "port"])?;                        // Result<&Node>
assert_eq!(port.as_scalar().unwrap().text, "8080");
let text = kvd_rs::serialize::to_string(&doc)?;                    // canonical form
```

`Node` is the core of the crate: the serde integration, schema verification,
and lossless serialization are all built on top of it.

Convert from YAML on the command line:

```sh
cargo install yaml2kvd
yaml2kvd values.yaml > values.kvd          # optional: --schema schema.yaml
yaml2kvd --reverse values.kvd             # KVD → YAML
```

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

Companion schema (`app.schema.kvd`) — a bare tree whose values are types:

```
app:
  name: str
  port: int
  version: str
  endpoints:
    - path: str
      method: str
  hooks:
    - str
  retries:
    type: int
    optional: true   # absent, int, or null
```

Notes:

- Values are one line (`"..."`) or a `"""..."""` block (also usable as a list
  item), or an indented subtree; `{}` and `[]` are the empty-collection
  literals. All string values are double-quoted — there are no bare words in
  value position.
- The only unquoted tokens with non-string meaning are `true`, `false`,
  `null`, integer and float literals, `{}`, `[]`, and type names matching the
  `type` grammar (`[a-z][a-z0-9_-]*`). Type names are
  meaningful only in schema position; in data documents they parse as strings.
  `null` is valid only where the schema says the type is optional
  (`optional: true`).
  Any other unquoted token is an `unexpected-character` error.
- Keys may be quoted with `"..."` (backslash escapes allowed) or `'...'`
  (literal, no escapes). Quoting lets a key contain characters that are
  otherwise separators or invalid in a bare key — most importantly the dots
  and slashes of Kubernetes label and annotation keys, e.g.
  `"app.kubernetes.io/name": guestbook`. A quoted key is always a literal
  key: even `__schema__` written in quotes is data, not a metakey. Quoted
  keys work wherever a key appears — inline paths
  (`metadata.labels."app.kubernetes.io/name"`) and the `Path` API
  (`Path::parse(r#"metadata.labels."app.kubernetes.io/name""#)`).
- Schemas are optional companions: data files parse standalone with default
  shapes. A schema file is a bare `key: type` tree (no metakeys); type names
  are the four builtins (`int`, `float`, `bool`, `str`), and may be embedded
  in a data document under `__schema__`.

## Crates

This repository is a Cargo workspace:

- [`kvd-rs`](crates/kvd-rs) — core library. The `Node` value model
  (`Scalar | Map | List`) is the foundation: the parser
  (`kvd_rs::deserialize::from_str`), serializer
  (`kvd_rs::serialize::to_string`), schema verification
  (`kvd_rs::schema::verify`), and the serde integration are all built on it.
  For typed documents, the `serde` feature exposes `from_str`/`to_string`
  (and reader/writer/file variants) as the recommended entry point. Zero
  dependencies unless you opt into the `serde` feature.
- [`kvd2yaml`](crates/kvd2yaml) — published binary crate. Reads KVD, writes
  YAML to stdout. `--reverse <input.yaml> [--schema <schema.kvd>]` inverts
  the direction (YAML → KVD).
- [`yaml2kvd`](crates/yaml2kvd) — published binary crate. Reads YAML, writes
  KVD to stdout. `--schema <schema.yaml>` verifies the converted document
  before emitting (the schema is a YAML file converted to a KVD schema
  internally). `--reverse <input.kvd>` inverts the direction (KVD → YAML).
  Both binaries are equivalent; install whichever name you prefer.

### Schema verification

```rust
let doc = kvd_rs::deserialize::from_str(text)?;      // text -> Node
let schema = kvd_rs::deserialize::from_str(schema_text)?;   // bare key: type tree
if let Err(violations) = kvd_rs::schema::verify(&doc, &schema) {
    for v in &violations {
        eprintln!("{v}");                            // path + message per violation
    }
}
// optional types: verify(&doc, &schema) where `retries: { type: int, optional: true }` allows absence/null
```

### Serde support

With the optional `serde` feature, derived types read and write KVD
directly — no manual `Node` traversal:

```toml
[dependencies]
kvd-rs = { version = "1.0.0", features = ["serde"] }
serde = { version = "1.0.0", features = ["derive"] }
```

```rust
#[derive(Serialize, Deserialize)]
struct App {
    name: String,
    port: u16,
    retries: Option<u32>,
}

let app: App = kvd_rs::from_str(text)?;      // KVD text -> T
let text = kvd_rs::to_string(&app)?;         // canonical form
// also: from_reader / from_file / to_writer / to_file
```

Shapes are enforced strictly: an integer target rejects quoted text,
floats always emit with a fraction (`1.0`), and non-finite floats are an
error. Externally-tagged enums round-trip as single-entry maps; unit
variants read bare strings.

## Status

Spec 1.0 — see [docs/README.md](docs/README.md) (index) and
[docs/spec/](docs/spec/) for the sections.

## License

MIT
