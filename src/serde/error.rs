//! Error type for serde conversions.

use std::fmt;

/// Error produced by serde conversions.
///
/// Parse errors lose their position because serde's trait objects carry none;
/// parse the DOM first with [`crate::deserialize::from_str`] when positions
/// matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerdeError {
    message: String,
}

impl SerdeError {
    /// Creates an error with the given message.
    pub fn new(message: impl Into<String>) -> Self {
        SerdeError {
            message: message.into(),
        }
    }
}

impl fmt::Display for SerdeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SerdeError {}

impl ::serde::de::Error for SerdeError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        SerdeError::new(message.to_string())
    }
}

impl ::serde::ser::Error for SerdeError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        SerdeError::new(message.to_string())
    }
}

impl From<crate::serialize::SerializeError> for SerdeError {
    fn from(e: crate::serialize::SerializeError) -> Self {
        SerdeError::new(e.to_string())
    }
}

impl From<crate::error::Error> for SerdeError {
    fn from(e: crate::error::Error) -> Self {
        SerdeError::new(e.to_string())
    }
}

impl From<std::io::Error> for SerdeError {
    fn from(e: std::io::Error) -> Self {
        SerdeError::new(e.to_string())
    }
}

/// Canonical spelling of a float per the grammar: `1.0`, not `1`.
pub(crate) fn float_text(v: f64) -> Result<String, SerdeError> {
    if !v.is_finite() {
        return Err(SerdeError::new(
            "non-finite floats are not representable in KVD",
        ));
    }
    let mut s = format!("{v}");
    if !s.contains(['.', 'e', 'E']) {
        s.push_str(".0");
    }
    Ok(s)
}
