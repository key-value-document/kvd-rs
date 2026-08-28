//! serde support (feature `serde`): bridge between serde's traits and KVD
//! nodes.
//!
//! - [`error`] — [`SerdeError`](error::SerdeError) type shared by both sides.
//! - [`deserialize`] — `T: Deserialize` from a [`Node`](crate::value::Node)
//!   tree, plus `Node: Deserialize`.
//! - [`serialize`] — `T: Serialize` → [`Node`](crate::value::Node) tree,
//!   plus `Node: Serialize`.

pub mod deserialize;
pub mod error;
pub mod serialize;

use error::SerdeError;

/// Deserializes a value of type `T` from KVD text.
///
/// ```no_run
/// # use serde::Deserialize;
/// # #[derive(Deserialize)]
/// # struct App { port: u16 }
/// # #[derive(Deserialize)]
/// # struct Cfg { app: App }
/// # fn main() -> Result<(), kvd_rs::serde::error::SerdeError> {
/// let cfg: Cfg = kvd_rs::from_str("app:\n  port: 8080\n")?;
/// assert_eq!(cfg.app.port, 8080);
/// # Ok(())
/// # }
/// ```
///
/// Shapes are strict: `port` must be an int literal, not a quoted string.
pub fn from_str<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, SerdeError> {
    let node = crate::deserialize::from_str(text)?;
    T::deserialize(deserialize::NodeDe::new(&node))
}

/// Serializes a value of type `T` to canonical KVD text.
///
/// Map keys must be strings; `HashMap` iteration order is unstable, so
/// prefer structs or ordered maps for deterministic output.
pub fn to_string<T: serde::Serialize>(value: &T) -> Result<String, SerdeError> {
    let node = value.serialize(serialize::NodeSerializer)?;
    Ok(crate::serialize::to_string(&node)?)
}

/// Deserializes a value of type `T` from any reader containing KVD text.
pub fn from_reader<R: std::io::Read, T: serde::de::DeserializeOwned>(
    mut reader: R,
) -> Result<T, SerdeError> {
    let mut text = String::new();
    reader.read_to_string(&mut text)?;
    from_str(&text)
}

/// Deserializes a value of type `T` from a UTF-8 KVD file.
pub fn from_file<T: serde::de::DeserializeOwned, P: AsRef<std::path::Path>>(
    path: P,
) -> Result<T, SerdeError> {
    let text = std::fs::read_to_string(path)?;
    from_str(&text)
}

/// Serializes a value of type `T` to canonical KVD text on any writer.
pub fn to_writer<W: std::io::Write, T: serde::Serialize>(
    mut writer: W,
    value: &T,
) -> Result<(), SerdeError> {
    let text = to_string(value)?;
    writer.write_all(text.as_bytes())?;
    Ok(())
}

/// Serializes a value of type `T` to a canonical KVD file. Creates or
/// truncates the file.
pub fn to_file<T: serde::Serialize, P: AsRef<std::path::Path>>(
    path: P,
    value: &T,
) -> Result<(), SerdeError> {
    let text = to_string(value)?;
    std::fs::write(path, text)?;
    Ok(())
}
