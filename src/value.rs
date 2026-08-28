//! Value model for KVD (spec §5).
//!
//! A document is an ordered map of keys to nodes; a node is a scalar, a
//! map, or a list. Maps preserve insertion order. Scalars keep enough raw
//! fidelity (original token, block mode) for the emitter to normalize
//! spelling without losing information.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::error::{Error, ErrorKind, Result};

/// Shape of a scalar as written (spec §5). `true`/`false` are shape
/// literals, not reserved words; quoting always yields a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Integer literal, e.g. `42`.
    Int,
    /// Floating-point literal, e.g. `0.75` or `1e3`.
    Float,
    /// The bare literals `true`/`false`.
    Bool,
    /// Any quoted or bare string.
    Str,
    /// The bare literal `null`. Valid only under an optional schema type
    /// (`optional: true`); verification rejects it anywhere else (spec §5).
    Null,
}

impl Shape {
    /// The lowercase name of this shape, as used in diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Shape::Int => "int",
            Shape::Float => "float",
            Shape::Bool => "bool",
            Shape::Str => "str",
            Shape::Null => "null",
        }
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A scalar value with raw-token fidelity.
#[derive(Debug, Clone, Eq)]
pub struct Scalar {
    /// Shape as written.
    pub shape: Shape,
    /// Decoded value: unescaped for double-quoted strings, the literal
    /// digits for numbers.
    pub text: String,
    /// The token exactly as written (quotes included for quoted strings).
    /// Provenance only: two scalars are equal when shape and text match,
    /// regardless of how they were spelled in the source.
    pub raw: String,
}

/// Semantic equality: spelling (`raw`) is deliberately ignored, so a value
/// written `"cluster.local"` equals the same value written bare.
impl PartialEq for Scalar {
    fn eq(&self, other: &Self) -> bool {
        self.shape == other.shape && self.text == other.text
    }
}

impl Scalar {
    /// Creates a scalar whose raw token equals its decoded text.
    pub fn new(shape: Shape, text: impl Into<String>) -> Self {
        let text = text.into();
        Scalar {
            shape,
            raw: text.clone(),
            text,
        }
    }

    /// Creates a scalar with a raw token distinct from its decoded text
    /// (e.g. a quoted string whose raw form includes the quotes).
    pub fn with_raw(shape: Shape, text: impl Into<String>, raw: impl Into<String>) -> Self {
        Scalar {
            shape,
            text: text.into(),
            raw: raw.into(),
        }
    }
}

impl From<&str> for Scalar {
    fn from(s: &str) -> Self {
        Scalar::new(Shape::Str, s)
    }
}

impl From<String> for Scalar {
    fn from(s: String) -> Self {
        Scalar::new(Shape::Str, s)
    }
}

impl From<i64> for Scalar {
    fn from(n: i64) -> Self {
        Scalar::new(Shape::Int, n.to_string())
    }
}

impl From<f64> for Scalar {
    fn from(n: f64) -> Self {
        Scalar::new(Shape::Float, n.to_string())
    }
}

impl From<bool> for Scalar {
    fn from(b: bool) -> Self {
        Scalar::new(Shape::Bool, b.to_string())
    }
}

/// An ordered map of keys to nodes (insertion order preserved).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Map {
    entries: Vec<(String, Node)>,
}

impl Map {
    /// Creates an empty map.
    pub fn new() -> Self {
        Map {
            entries: Vec::new(),
        }
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the map has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the node stored under `key`, if any.
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Returns a mutable reference to the node stored under `key`, if any.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Node> {
        self.entries
            .iter_mut()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// True when `key` is present.
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    /// Appends a key/value pair. Duplicate detection is the parser's job
    /// (it needs line:col for the error), so this does not check.
    pub fn insert(&mut self, key: String, value: Node) {
        self.entries.push((key, value));
    }

    /// Removes the entry at `index`, returning its key and node.
    /// Panics if `index >= len()`.
    pub fn remove_at(&mut self, index: usize) -> (String, Node) {
        self.entries.remove(index)
    }

    /// Iterates over `(&str, &Node)` pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Node)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Borrows all entries as `(key, node)` pairs in insertion order.
    pub fn entries(&self) -> &[(String, Node)] {
        &self.entries
    }
}

impl IntoIterator for Map {
    type Item = (String, Node);
    type IntoIter = alloc::vec::IntoIter<(String, Node)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

/// A KVD node: scalar, map, or list.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Node {
    /// A scalar value.
    Scalar(Scalar),
    /// An ordered mapping.
    Map(Map),
    /// An ordered list.
    List(Vec<Node>),
}

impl Node {
    /// Creates a scalar node.
    pub fn scalar(shape: Shape, text: impl Into<String>) -> Self {
        Node::Scalar(Scalar::new(shape, text))
    }

    /// Creates a map node.
    pub fn map(map: Map) -> Self {
        Node::Map(map)
    }

    /// Creates a list node.
    pub fn list(items: Vec<Node>) -> Self {
        Node::List(items)
    }

    /// Borrows this node as a scalar, if it is one.
    pub fn as_scalar(&self) -> Option<&Scalar> {
        match self {
            Node::Scalar(s) => Some(s),
            _ => None,
        }
    }

    /// Borrows this node mutably as a scalar, if it is one.
    pub fn as_scalar_mut(&mut self) -> Option<&mut Scalar> {
        match self {
            Node::Scalar(s) => Some(s),
            _ => None,
        }
    }

    /// Borrows this node as a map, if it is one.
    pub fn as_map(&self) -> Option<&Map> {
        match self {
            Node::Map(m) => Some(m),
            _ => None,
        }
    }

    /// Borrows this node mutably as a map, if it is one.
    pub fn as_map_mut(&mut self) -> Option<&mut Map> {
        match self {
            Node::Map(m) => Some(m),
            _ => None,
        }
    }

    /// Borrows this node as a list, if it is one.
    pub fn as_list(&self) -> Option<&[Node]> {
        match self {
            Node::List(l) => Some(l),
            _ => None,
        }
    }

    /// Borrows this node mutably as a list, if it is one.
    pub fn as_list_mut(&mut self) -> Option<&mut Vec<Node>> {
        match self {
            Node::List(l) => Some(l),
            _ => None,
        }
    }

    /// Navigates this node as a map and returns the child for `key`.
    ///
    /// Errors if this node is not a map (e.g. it is a scalar or a list),
    /// or if `key` is absent. Use [`Node::get_opt`] when a missing key
    /// should yield `None` instead of an error. For errors that name the
    /// full navigation chain (e.g. `a.b.c`), use [`Node::get_path`].
    pub fn get(&self, key: &str) -> Result<&Node> {
        let map = self.as_map().ok_or_else(|| {
            Error::new(
                ErrorKind::NotAMap,
                0,
                0,
                format!(
                    "cannot index `{key}` into a non-map node ({})",
                    non_map_kind(self)
                ),
            )
        })?;
        map.get(key).ok_or_else(|| {
            Error::new(
                ErrorKind::KeyNotFound,
                0,
                0,
                format!("key `{key}` not found"),
            )
        })
    }

    /// Like [`Node::get`], but returns `None` instead of erroring when the
    /// node is not a map or the key is absent.
    pub fn get_opt(&self, key: &str) -> Option<&Node> {
        self.as_map().and_then(|m| m.get(key))
    }

    /// Mutable counterpart of [`Node::get`].
    pub fn get_mut(&mut self, key: &str) -> Result<&mut Node> {
        let map = self.as_map_mut().ok_or_else(|| {
            Error::new(
                ErrorKind::NotAMap,
                0,
                0,
                "node is not a map; cannot index by key",
            )
        })?;
        map.get_mut(key).ok_or_else(|| {
            Error::new(
                ErrorKind::KeyNotFound,
                0,
                0,
                format!("key `{key}` not found"),
            )
        })
    }

    /// Mutable counterpart of [`Node::get_opt`].
    pub fn get_opt_mut(&mut self, key: &str) -> Option<&mut Node> {
        self.as_map_mut().and_then(|m| m.get_mut(key))
    }

    /// Navigates a full key path (e.g. `["a", "b", "c"]`) through nested
    /// maps, returning the deepest node. The error message carries the full
    /// chain, e.g. `a.b.c: key not found` or `d.e.f: node is not a map`.
    pub fn get_path(&self, path: &[&str]) -> Result<&Node> {
        self.get_path_inner(path.iter().copied(), "")
    }

    /// Like [`Node::get_path`], but accepts a dotted path string (`"a.b.c"`),
    /// splitting on `.` per the crate's path convention.
    pub fn get_path_str(&self, path: &str) -> Result<&Node> {
        let keys: Vec<&str> = path.split('.').collect();
        self.get_path(&keys)
    }

    /// Like [`Node::get_path`], but returns `None` instead of erroring when
    /// any segment is missing or a non-map node is indexed.
    pub fn get_path_opt(&self, path: &[&str]) -> Option<&Node> {
        self.get_path(path).ok()
    }

    /// Like [`Node::get_path_str`], but returns `None` on any failure.
    pub fn get_path_str_opt(&self, path: &str) -> Option<&Node> {
        self.get_path_str(path).ok()
    }

    /// Mutable counterpart of [`Node::get_path`].
    pub fn get_path_mut(&mut self, path: &[&str]) -> Result<&mut Node> {
        self.get_path_mut_inner(path.iter().copied(), "")
    }

    /// Mutable counterpart of [`Node::get_path_str`].
    pub fn get_path_str_mut(&mut self, path: &str) -> Result<&mut Node> {
        let keys: Vec<&str> = path.split('.').collect();
        self.get_path_mut(&keys)
    }

    /// Mutable counterpart of [`Node::get_path_opt`].
    pub fn get_path_opt_mut(&mut self, path: &[&str]) -> Option<&mut Node> {
        self.get_path_mut(path).ok()
    }

    /// Mutable counterpart of [`Node::get_path_str_opt`].
    pub fn get_path_str_opt_mut(&mut self, path: &str) -> Option<&mut Node> {
        self.get_path_str_mut(path).ok()
    }

    fn get_path_inner<'a, I>(&self, mut path: I, prefix: &str) -> Result<&Node>
    where
        I: Iterator<Item = &'a str>,
    {
        match path.next() {
            None => Ok(self),
            Some(key) => {
                let here = join_path(prefix, key);
                let map = self.as_map().ok_or_else(|| {
                    let loc = if prefix.is_empty() {
                        "root".to_string()
                    } else {
                        prefix.to_string()
                    };
                    Error::new(
                        ErrorKind::NotAMap,
                        0,
                        0,
                        format!("{loc} is {}; cannot index `{key}`", non_map_kind(self)),
                    )
                })?;
                let child = map.get(key).ok_or_else(|| {
                    Error::new(
                        ErrorKind::KeyNotFound,
                        0,
                        0,
                        format!("{here}: key not found"),
                    )
                })?;
                child.get_path_inner(path, &here)
            }
        }
    }

    fn get_path_mut_inner<'a, I>(&mut self, mut path: I, prefix: &str) -> Result<&mut Node>
    where
        I: Iterator<Item = &'a str>,
    {
        match path.next() {
            None => Ok(self),
            Some(key) => {
                let here = join_path(prefix, key);
                // Check shape with a transient immutable borrow so the error
                // branch can describe `self` without conflicting with the
                // mutable descent below.
                if self.as_map().is_none() {
                    let loc = if prefix.is_empty() {
                        "root".to_string()
                    } else {
                        prefix.to_string()
                    };
                    return Err(Error::new(
                        ErrorKind::NotAMap,
                        0,
                        0,
                        format!("{loc} is {}; cannot index `{key}`", non_map_kind(self)),
                    ));
                }
                let map = self.as_map_mut().unwrap();
                let child = match map.get_mut(key) {
                    Some(child) => child,
                    None => {
                        return Err(Error::new(
                            ErrorKind::KeyNotFound,
                            0,
                            0,
                            format!("{here}: key not found"),
                        ))
                    }
                };
                child.get_path_mut_inner(path, &here)
            }
        }
    }
}

/// Joins a path prefix with one more key using the crate's dotted convention.
fn join_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// Describes a node that was indexed as a map but isn't one, for error text.
fn non_map_kind(node: &Node) -> String {
    match node {
        Node::Scalar(s) => format!("a scalar ({})", s.shape),
        Node::List(_) => "a list".to_string(),
        Node::Map(_) => "a map".to_string(),
    }
}

impl From<Scalar> for Node {
    fn from(s: Scalar) -> Self {
        Node::Scalar(s)
    }
}

impl From<Map> for Node {
    fn from(m: Map) -> Self {
        Node::Map(m)
    }
}

impl From<Vec<Node>> for Node {
    fn from(l: Vec<Node>) -> Self {
        Node::List(l)
    }
}

/// A parsed KVD document: the root is always a mapping (spec §4).
pub type Document = Map;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_from_str() {
        let s = Scalar::from("hello");
        assert_eq!(s.shape, Shape::Str);
        assert_eq!(s.text, "hello");
        assert_eq!(s.raw, "hello");
    }

    #[test]
    fn scalar_from_numbers_and_bool() {
        assert_eq!(Scalar::from(42i64).shape, Shape::Int);
        assert_eq!(Scalar::from(42i64).text, "42");
        assert_eq!(Scalar::from(0.5f64).shape, Shape::Float);
        assert_eq!(Scalar::from(0.5f64).text, "0.5");
        assert_eq!(Scalar::from(true).shape, Shape::Bool);
        assert_eq!(Scalar::from(true).text, "true");
    }

    #[test]
    fn scalar_with_raw_keeps_token() {
        let s = Scalar::with_raw(Shape::Str, "hello", "\"hello\"");
        assert_eq!(s.text, "hello");
        assert_eq!(s.raw, "\"hello\"");
    }

    #[test]
    fn map_preserves_insertion_order() {
        let mut m = Map::new();
        m.insert("b".into(), Node::scalar(Shape::Int, "1"));
        m.insert("a".into(), Node::scalar(Shape::Int, "2"));
        let keys: Vec<&str> = m.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!["b", "a"]);
        assert_eq!(m.len(), 2);
        assert!(!m.is_empty());
        assert!(m.contains_key("a"));
        assert!(!m.contains_key("z"));
    }

    #[test]
    fn map_get_and_get_mut() {
        let mut m = Map::new();
        m.insert("k".into(), Node::scalar(Shape::Int, "1"));
        assert!(m.get("k").is_some());
        assert!(m.get("missing").is_none());
        if let Some(node) = m.get_mut("k") {
            *node = Node::scalar(Shape::Int, "2");
        }
        assert_eq!(m.get("k").unwrap().as_scalar().unwrap().text, "2");
    }

    #[test]
    fn map_into_iter_yields_owned_pairs() {
        let mut m = Map::new();
        m.insert("k".into(), Node::scalar(Shape::Int, "1"));
        let pairs: Vec<(String, Node)> = m.into_iter().collect();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "k");
    }

    #[test]
    fn node_accessors() {
        let s = Node::scalar(Shape::Str, "x");
        assert!(s.as_scalar().is_some());
        assert!(s.as_map().is_none());
        assert!(s.as_list().is_none());

        let m = Node::map(Map::new());
        assert!(m.as_map().is_some());
        assert!(m.as_scalar().is_none());

        let mut l = Node::list(vec![Node::scalar(Shape::Int, "1")]);
        assert_eq!(l.as_list().unwrap().len(), 1);
        assert!(l.as_list_mut().is_some());
        assert!(l.as_scalar_mut().is_none());
    }

    #[test]
    fn node_from_impls() {
        let n: Node = Scalar::from("x").into();
        assert!(n.as_scalar().is_some());
        let n: Node = Map::new().into();
        assert!(n.as_map().is_some());
        let n: Node = vec![Node::scalar(Shape::Int, "1")].into();
        assert!(n.as_list().is_some());
    }

    #[test]
    fn shape_as_str_and_display() {
        assert_eq!(Shape::Int.as_str(), "int");
        assert_eq!(Shape::Float.as_str(), "float");
        assert_eq!(Shape::Bool.as_str(), "bool");
        assert_eq!(Shape::Str.as_str(), "str");
        assert_eq!(Shape::Null.as_str(), "null");
        assert_eq!(format!("{}", Shape::Int), "int");
        assert_eq!(format!("{}", Shape::Null), "null");
    }

    #[test]
    fn scalar_from_string() {
        let s: Scalar = String::from("hello").into();
        assert_eq!(s.shape, Shape::Str);
        assert_eq!(s.text, "hello");
    }

    #[test]
    fn map_entries_borrows_raw_slice() {
        let mut m = Map::new();
        m.insert("k".into(), Node::scalar(Shape::Int, "1"));
        let entries = m.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "k");
    }

    #[test]
    fn node_mut_accessors_and_fallbacks() {
        // as_scalar_mut on a scalar node hits the Some branch.
        let mut s = Node::scalar(Shape::Str, "x");
        assert!(s.as_scalar_mut().is_some());
        assert!(s.as_scalar_mut().unwrap().text == "x");

        // as_map_mut: Some on a map, None on a non-map.
        let mut m = Node::map(Map::new());
        assert!(m.as_map_mut().is_some());
        let mut s2 = Node::scalar(Shape::Str, "x");
        assert!(s2.as_map_mut().is_none());

        // as_list_mut: Some on a list, None on a non-list.
        let mut l = Node::list(vec![Node::scalar(Shape::Int, "1")]);
        assert!(l.as_list_mut().is_some());
        let mut s3 = Node::scalar(Shape::Str, "x");
        assert!(s3.as_list_mut().is_none());
    }

    #[test]
    fn node_get_navigates_and_errors() {
        let mut app = Node::map(Map::new());
        app.as_map_mut()
            .unwrap()
            .insert("port".into(), Node::scalar(Shape::Int, "8080"));
        let mut root = Node::map(Map::new());
        root.as_map_mut().unwrap().insert("app".into(), app);

        // strict get chains through maps
        let port = root.get("app").unwrap().get("port").unwrap();
        assert_eq!(port.as_scalar().unwrap().text, "8080");

        // get on a scalar errors with NotAMap
        let port_node = root.get("app").unwrap().get("port").unwrap();
        assert!(matches!(port_node.get("x"), Err(e) if e.kind == ErrorKind::NotAMap));

        // missing key errors with KeyNotFound
        assert!(matches!(
            root.get("app").unwrap().get("missing"),
            Err(e) if e.kind == ErrorKind::KeyNotFound
        ));

        // get_opt yields None for non-map and missing key
        assert!(port_node.get_opt("x").is_none());
        assert!(root.get("app").unwrap().get_opt("missing").is_none());
    }

    #[test]
    fn node_get_path_reports_chain() {
        let mut app = Node::map(Map::new());
        app.as_map_mut()
            .unwrap()
            .insert("port".into(), Node::scalar(Shape::Int, "8080"));
        let mut root = Node::map(Map::new());
        root.as_map_mut().unwrap().insert("app".into(), app);

        // full chain resolves (slice and dotted forms agree)
        assert_eq!(
            root.get_path(&["app", "port"])
                .unwrap()
                .as_scalar()
                .unwrap()
                .text,
            "8080"
        );
        assert_eq!(
            root.get_path_str("app.port")
                .unwrap()
                .as_scalar()
                .unwrap()
                .text,
            "8080"
        );

        // missing key reports the full chain
        let e = root.get_path(&["app", "missing"]).unwrap_err();
        assert_eq!(e.kind, ErrorKind::KeyNotFound);
        assert!(e.message.starts_with("app.missing:"), "got: {}", e.message);

        // indexing into a scalar reports the chain up to the scalar
        let e = root.get_path(&["app", "port", "x"]).unwrap_err();
        assert_eq!(e.kind, ErrorKind::NotAMap);
        assert!(e.message.contains("app.port"), "got: {}", e.message);
        assert!(e.message.contains("scalar"), "got: {}", e.message);

        // optional variants yield None
        assert!(root.get_path_opt(&["app", "missing"]).is_none());
        assert!(root.get_path_str_opt("app.port.x").is_none());
    }
}
