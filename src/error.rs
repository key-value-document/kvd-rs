//! Error model for KVD (spec §7).
//!
//! Every error carries a line:col position and one of the categories
//! enumerated in the spec. The grammar is deterministic enough that an
//! invalid document has exactly one explanation.

use alloc::string::String;
use core::fmt;

/// Error categories from spec §7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Indentation is not a multiple of 2 spaces.
    BadIndent,
    /// A tab appears outside quoted strings and `"""` blocks.
    Tab,
    /// A `-` marker is not followed by exactly one space.
    BadListMarker,
    /// A list-item key does not align with the first key of its item.
    MisalignedKey,
    /// A key has no value and no indented subtree.
    MissingValue,
    /// The same key/path appears twice.
    DuplicateKey,
    /// A path is both a leaf and an interior node.
    LeafInteriorConflict,
    /// A dotted path is malformed (empty segment, leading/trailing dot).
    BadPath,
    /// A quoted string or `"""` block is not terminated.
    Unterminated,
    /// A `__...__` key is not a defined metakey.
    UnknownMetakey,
    /// A metakey appears outside the document root.
    MetakeyOutsideRoot,
    /// Nesting exceeds the depth limit.
    DepthLimit,
    /// A character is not allowed in this position.
    UnexpectedCharacter,
    /// A navigation operation was attempted on a node that is not a map.
    NotAMap,
    /// A key was not found in a map during navigation.
    KeyNotFound,
}

impl ErrorKind {
    /// The stable machine-readable name of this category.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::BadIndent => "bad-indent",
            ErrorKind::Tab => "tab",
            ErrorKind::BadListMarker => "bad-list-marker",
            ErrorKind::MisalignedKey => "misaligned-key",
            ErrorKind::MissingValue => "missing-value",
            ErrorKind::DuplicateKey => "duplicate-key",
            ErrorKind::LeafInteriorConflict => "leaf-interior-conflict",
            ErrorKind::BadPath => "bad-path",
            ErrorKind::Unterminated => "unterminated",
            ErrorKind::UnknownMetakey => "unknown-metakey",
            ErrorKind::MetakeyOutsideRoot => "metakey-outside-root",
            ErrorKind::DepthLimit => "depth-limit",
            ErrorKind::UnexpectedCharacter => "unexpected-character",
            ErrorKind::NotAMap => "not-a-map",
            ErrorKind::KeyNotFound => "key-not-found",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A KVD parse/validation error with position and category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// Error category.
    pub kind: ErrorKind,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub col: usize,
    /// Human-readable explanation.
    pub message: String,
}

impl Error {
    /// Creates a new error with position and message.
    pub fn new(kind: ErrorKind, line: usize, col: usize, message: impl Into<String>) -> Self {
        Error {
            kind,
            line,
            col,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: {}: {}",
            self.line,
            self.col,
            self.kind.as_str(),
            self.message
        )
    }
}

impl core::error::Error for Error {}

/// Convenience alias for KVD results.
pub type Result<T> = core::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kind_names() {
        assert_eq!(ErrorKind::BadIndent.as_str(), "bad-indent");
        assert_eq!(
            ErrorKind::UnexpectedCharacter.as_str(),
            "unexpected-character"
        );
        assert_eq!(ErrorKind::DuplicateKey.to_string(), "duplicate-key");
    }

    #[test]
    fn error_display_includes_position_and_kind() {
        let e = Error::new(ErrorKind::BadIndent, 3, 5, "expected 2 spaces");
        assert_eq!(e.to_string(), "3:5: bad-indent: expected 2 spaces");
    }

    #[test]
    fn error_is_std_error() {
        let e = Error::new(ErrorKind::Tab, 1, 1, "tabs are illegal");
        let _: &dyn core::error::Error = &e;
    }
}
