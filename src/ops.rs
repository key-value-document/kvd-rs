//! Document operations on parsed KVD node trees (spec §8.5).
//!
//! This module provides a small, allocation-light surface for programmatic
//! editing of a parsed document: resolve a [`Path`], then [`get`],
//! [`get_mut`], [`set`], or [`remove`] a node. All operations are built on
//! the [`Node`]/[`Map`] value model from [`value`](crate::value).
//!
//! Paths use a compact, JSON-Pointer-like syntax:
//!
//! - Map keys are dotted segments: `a.b.c`
//! - A segment may be quoted with `"..."` or `'...'` to include characters
//!   that are otherwise separators or invalid in a bare key — e.g.
//!   `metadata.labels."app.kubernetes.io/name"` (spec §8.5). A quoted
//!   segment is always a literal key.
//! - List items use bracket indices: `a[0]`, `a.b[2]`
//! - They combine freely: `a.b[0].c`, `a[0][1]`
//!
//! Indices are zero-based and non-negative; an out-of-range index is an
//! error, never silent truncation or append.
//!
//! Document merging is intentionally out of scope for this module (see
//! spec §8.5).

#[cfg(not(any(test, feature = "serde")))]
#[allow(unused_imports)]
// format! resolves to this under no_std; clippy misfires on macro imports
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::str::FromStr;

use crate::grammar;
use crate::value::{Map, Node};

/// One step of a [`Path`]: either descend into a map by key, or index a
/// list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Descend into a map entry by key.
    Key(String),
    /// Index into a list (zero-based, non-negative).
    Index(usize),
}

impl Segment {
    /// Returns the key for a [`Segment::Key`], or `None`.
    pub fn as_key(&self) -> Option<&str> {
        match self {
            Segment::Key(k) => Some(k),
            _ => None,
        }
    }

    /// Returns the index for a [`Segment::Index`], or `None`.
    pub fn as_index(&self) -> Option<usize> {
        match self {
            Segment::Index(i) => Some(*i),
            _ => None,
        }
    }
}

/// A resolved navigation path through a KVD node tree.
///
/// Build one with [`Path::parse`] (or `str::parse`) and pass it to the
/// operations. A path is a sequence of [`Segment`]s; an empty path refers
/// to the document root itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Path {
    segments: Vec<Segment>,
}

impl Path {
    /// Parses a path string (spec §8.5).
    ///
    /// Syntax: `key ("." key)*` with optional `[index]` suffixes on any
    /// key (`a.b[0].c`). Keys must satisfy the KVD key grammar; indices
    /// are non-negative integers. Returns [`OpError::BadPath`] on any
    /// malformed input (empty segment, leading/trailing dot, invalid key,
    /// unterminated bracket, or non-numeric/negative index).
    pub fn parse(s: &str) -> Result<Path, OpError> {
        if s.is_empty() {
            return Err(OpError::BadPath("empty path".into()));
        }
        let bytes = s.as_bytes();
        let mut segments = Vec::new();
        let mut i = 0usize;
        while i < bytes.len() {
            let start = i;
            if bytes[i] == b'"' || bytes[i] == b'\'' {
                // Quoted key segment (spec §8.5): may contain '.', '[', etc.
                // Decoded with the same rules as document keys so a quoted key
                // means the same thing in a `Path` string and in a document.
                let (text, used) = crate::deserialize::scan_quoted_segment(&s[i..], 0, i + 1)
                    .map_err(|e| OpError::BadPath(format!("in path `{s}`: {e}")))?;
                if text.is_empty() {
                    return Err(OpError::BadPath(format!("empty quoted key in path `{s}`")));
                }
                i += used;
                segments.push(Segment::Key(text));
            } else {
                while i < bytes.len() && bytes[i] != b'.' && bytes[i] != b'[' {
                    i += 1;
                }
                let key = &s[start..i];
                if key.is_empty() {
                    return Err(OpError::BadPath(format!("empty path segment in `{s}`")));
                }
                if !grammar::is_key(key) {
                    return Err(OpError::BadPath(format!(
                        "invalid key `{key}` in path `{s}`"
                    )));
                }
                segments.push(Segment::Key(key.to_string()));
            }
            // Optional `[index]` suffixes (e.g. `a[0]`, `a[0][1]`).
            while i < bytes.len() && bytes[i] == b'[' {
                let close = match s[i..].find(']') {
                    Some(c) => c,
                    None => {
                        return Err(OpError::BadPath(format!("unterminated `[` in path `{s}`")))
                    }
                };
                let num = &s[i + 1..i + close];
                if num.is_empty() {
                    return Err(OpError::BadPath(format!("empty list index in path `{s}`")));
                }
                let idx: usize = match num.parse() {
                    Ok(n) => n,
                    Err(_) => {
                        return Err(OpError::BadPath(format!(
                            "invalid list index `{num}` in path `{s}`"
                        )))
                    }
                };
                segments.push(Segment::Index(idx));
                i += close + 1;
            }
            if i < bytes.len() {
                if bytes[i] == b'.' {
                    i += 1;
                    if i >= bytes.len() {
                        return Err(OpError::BadPath(format!("trailing `.` in path `{s}`")));
                    }
                } else {
                    let ch = s[i..].chars().next().unwrap();
                    return Err(OpError::BadPath(format!("unexpected `{ch}` in path `{s}`")));
                }
            }
        }
        Ok(Path { segments })
    }

    /// The segments of this path, in order.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// True when the path is empty (refers to the document root).
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

impl FromStr for Path {
    type Err = OpError;
    fn from_str(s: &str) -> Result<Path, OpError> {
        Path::parse(s)
    }
}

/// Errors raised by the document operations (spec §8.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpError {
    /// The path string could not be parsed.
    BadPath(String),
    /// A map key named in the path does not exist.
    MissingKey(String),
    /// A list index is out of range for its list.
    IndexOutOfBounds(usize),
    /// A node expected to be a map was some other shape.
    NotAMap,
    /// A node expected to be a list was some other shape.
    NotAList,
    /// A value had an unexpected shape (reserved for future use).
    TypeMismatch,
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpError::BadPath(m) => write!(f, "bad path: {m}"),
            OpError::MissingKey(k) => write!(f, "missing key `{k}`"),
            OpError::IndexOutOfBounds(i) => write!(f, "index out of bounds: {i}"),
            OpError::NotAMap => write!(f, "expected a map"),
            OpError::NotAList => write!(f, "expected a list"),
            OpError::TypeMismatch => write!(f, "type mismatch"),
        }
    }
}

impl core::error::Error for OpError {}

/// Resolves `path` to an immutable node, or `None` if any segment is
/// missing or mistyped.
pub fn get<'a>(doc: &'a Node, path: &Path) -> Option<&'a Node> {
    let mut cur = doc;
    for seg in &path.segments {
        cur = match seg {
            Segment::Key(k) => cur.as_map()?.get(k)?,
            Segment::Index(i) => cur.as_list()?.get(*i)?,
        };
    }
    Some(cur)
}

/// Resolves `path` to a mutable node, or `None` if any segment is missing
/// or mistyped.
pub fn get_mut<'a>(doc: &'a mut Node, path: &Path) -> Option<&'a mut Node> {
    let mut cur = doc;
    for seg in &path.segments {
        cur = match seg {
            Segment::Key(k) => cur.as_map_mut()?.get_mut(k)?,
            Segment::Index(i) => cur.as_list_mut()?.get_mut(*i)?,
        };
    }
    Some(cur)
}

/// Sets `value` at `path`, creating intermediate maps as needed (like the
/// parser's dotted-key expansion). List indices must already exist; an
/// out-of-range index is [`OpError::IndexOutOfBounds`]. Replaces any node
/// already at the target.
pub fn set(doc: &mut Node, path: &Path, value: Node) -> Result<(), OpError> {
    if path.segments.is_empty() {
        return Err(OpError::BadPath("cannot set the document root".into()));
    }
    do_set(doc, &path.segments, value)
}

fn do_set(node: &mut Node, segs: &[Segment], value: Node) -> Result<(), OpError> {
    match &segs[0] {
        Segment::Key(k) => {
            let map = node.as_map_mut().ok_or(OpError::NotAMap)?;
            if segs.len() == 1 {
                if let Some(existing) = map.get_mut(k) {
                    *existing = value;
                } else {
                    map.insert(k.clone(), value);
                }
                return Ok(());
            }
            if !map.contains_key(k) {
                map.insert(k.clone(), Node::map(Map::new()));
            }
            let child = map.get_mut(k).unwrap();
            do_set(child, &segs[1..], value)
        }
        Segment::Index(i) => {
            let list = node.as_list_mut().ok_or(OpError::NotAList)?;
            if *i >= list.len() {
                return Err(OpError::IndexOutOfBounds(*i));
            }
            if segs.len() == 1 {
                list[*i] = value;
                return Ok(());
            }
            do_set(&mut list[*i], &segs[1..], value)
        }
    }
}

/// Removes the node at `path`, returning it. The document is left with an
/// empty `{}` where the removed node's ancestors became empty (ancestors
/// are *not* pruned). Errors if the path does not resolve
/// ([`OpError::MissingKey`] / [`OpError::IndexOutOfBounds`]) or a node is
/// the wrong shape ([`OpError::NotAMap`] / [`OpError::NotAList`]).
pub fn remove(doc: &mut Node, path: &Path) -> Result<Option<Node>, OpError> {
    if path.segments.is_empty() {
        return Err(OpError::BadPath("cannot remove the document root".into()));
    }
    do_remove(doc, &path.segments, false)
}

/// Like [`remove`], but prunes now-empty ancestor *maps*: a map that
/// becomes empty after the removal is itself removed, recursively up the
/// chain. List elements are never pruned by emptiness.
pub fn remove_recursive(doc: &mut Node, path: &Path) -> Result<Option<Node>, OpError> {
    if path.segments.is_empty() {
        return Err(OpError::BadPath("cannot remove the document root".into()));
    }
    do_remove(doc, &path.segments, true)
}

fn do_remove(node: &mut Node, segs: &[Segment], prune: bool) -> Result<Option<Node>, OpError> {
    match &segs[0] {
        Segment::Key(k) => {
            let map = node.as_map_mut().ok_or(OpError::NotAMap)?;
            if segs.len() == 1 {
                let idx = map
                    .entries()
                    .iter()
                    .position(|(key, _)| key == k)
                    .ok_or_else(|| OpError::MissingKey(k.clone()))?;
                return Ok(Some(map.remove_at(idx).1));
            }
            let removed;
            {
                let child = map
                    .get_mut(k)
                    .ok_or_else(|| OpError::MissingKey(k.clone()))?;
                removed = do_remove(child, &segs[1..], prune)?;
            }
            if prune {
                if let Some(Node::Map(inner)) = map.get(k) {
                    if inner.is_empty() {
                        let idx = map.entries().iter().position(|(key, _)| key == k).unwrap();
                        map.remove_at(idx);
                    }
                }
            }
            Ok(removed)
        }
        Segment::Index(i) => {
            let list = node.as_list_mut().ok_or(OpError::NotAList)?;
            if *i >= list.len() {
                return Err(OpError::IndexOutOfBounds(*i));
            }
            if segs.len() == 1 {
                return Ok(Some(list.remove(*i)));
            }
            do_remove(&mut list[*i], &segs[1..], prune)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Shape;

    fn doc() -> Node {
        // a:
        //   b: "x"
        // list:
        //   - "p"
        //   - "q"
        //   - "r"
        let mut inner = Map::new();
        inner.insert("b".into(), Node::scalar(Shape::Str, "x"));
        let mut root = Map::new();
        root.insert("a".into(), Node::map(inner));
        root.insert(
            "list".into(),
            Node::list(vec![
                Node::scalar(Shape::Str, "p"),
                Node::scalar(Shape::Str, "q"),
                Node::scalar(Shape::Str, "r"),
            ]),
        );
        Node::map(root)
    }

    // --- Path parsing ---

    #[test]
    fn parse_simple_keys() {
        let p = Path::parse("a.b.c").unwrap();
        assert_eq!(
            p.segments(),
            &[
                Segment::Key("a".into()),
                Segment::Key("b".into()),
                Segment::Key("c".into())
            ]
        );
    }

    #[test]
    fn parse_list_indices() {
        let p = Path::parse("a[0].b[2]").unwrap();
        assert_eq!(
            p.segments(),
            &[
                Segment::Key("a".into()),
                Segment::Index(0),
                Segment::Key("b".into()),
                Segment::Index(2),
            ]
        );
    }

    #[test]
    fn parse_nested_list_index() {
        let p = Path::parse("a[0][1]").unwrap();
        assert_eq!(
            p.segments(),
            &[
                Segment::Key("a".into()),
                Segment::Index(0),
                Segment::Index(1)
            ]
        );
    }

    #[test]
    fn parse_empty_path() {
        assert!(Path::parse("").is_err());
        let p = Path::parse("").unwrap_err();
        assert!(matches!(p, OpError::BadPath(_)));
    }

    #[test]
    fn parse_rejects_bad_syntax() {
        assert!(Path::parse(".a").is_err()); // leading dot
        assert!(Path::parse("a.").is_err()); // trailing dot
        assert!(Path::parse("a..b").is_err()); // empty segment
        assert!(Path::parse("a[0").is_err()); // unterminated bracket
        assert!(Path::parse("a[]").is_err()); // empty index
        assert!(Path::parse("a[-1]").is_err()); // negative index
        assert!(Path::parse("a[1.5]").is_err()); // non-integer index
        assert!(Path::parse("a[x]").is_err()); // non-numeric index
        assert!(Path::parse("a.b.c]").is_err()); // stray bracket
        assert!(Path::parse("a.b.c[").is_err()); // stray bracket
    }

    #[test]
    fn parse_accepts_valid_keys_with_dash_underscore() {
        // b-c and b_c are valid keys per the grammar; dots are separators.
        assert!(Path::parse("a.b-c").is_ok());
        assert!(Path::parse("a.b_c").is_ok());
        assert!(Path::parse("8080.port").is_ok());
    }

    #[test]
    fn parse_rejects_key_with_dot() {
        // A literal dot inside a segment is impossible: dots separate
        // segments, so `a.b.c` is three keys, never one key containing a dot.
        let p = Path::parse("a.b.c").unwrap();
        assert_eq!(p.segments().len(), 3);
    }

    #[test]
    fn from_str_works() {
        let p: Path = "a.b[0]".parse().unwrap();
        assert_eq!(p.segments()[2], Segment::Index(0));
    }

    // --- get ---

    #[test]
    fn get_existing_scalar() {
        let d = doc();
        let p = Path::parse("a.b").unwrap();
        assert_eq!(get(&d, &p).unwrap().as_scalar().unwrap().text, "x");
    }

    #[test]
    fn get_list_element() {
        let d = doc();
        let p = Path::parse("list[1]").unwrap();
        assert_eq!(get(&d, &p).unwrap().as_scalar().unwrap().text, "q");
    }

    #[test]
    fn get_missing_returns_none() {
        let d = doc();
        assert!(get(&d, &Path::parse("a.missing").unwrap()).is_none());
        assert!(get(&d, &Path::parse("list[9]").unwrap()).is_none());
        assert!(get(&d, &Path::parse("a.b.c").unwrap()).is_none());
    }

    // --- get_mut ---

    #[test]
    fn get_mut_modifies_value() {
        let mut d = doc();
        let p = Path::parse("a.b").unwrap();
        *get_mut(&mut d, &p).unwrap() = Node::scalar(Shape::Str, "changed");
        assert_eq!(get(&d, &p).unwrap().as_scalar().unwrap().text, "changed");
    }

    // --- set ---

    #[test]
    fn set_new_key() {
        let mut d = doc();
        set(
            &mut d,
            &Path::parse("a.c").unwrap(),
            Node::scalar(Shape::Int, "42"),
        )
        .unwrap();
        assert_eq!(
            get(&d, &Path::parse("a.c").unwrap())
                .unwrap()
                .as_scalar()
                .unwrap()
                .text,
            "42"
        );
    }

    #[test]
    fn set_replaces_existing() {
        let mut d = doc();
        set(
            &mut d,
            &Path::parse("a.b").unwrap(),
            Node::scalar(Shape::Int, "99"),
        )
        .unwrap();
        assert_eq!(
            get(&d, &Path::parse("a.b").unwrap())
                .unwrap()
                .as_scalar()
                .unwrap()
                .text,
            "99"
        );
    }

    #[test]
    fn set_creates_intermediate_maps() {
        let mut d = doc();
        set(
            &mut d,
            &Path::parse("x.y.z").unwrap(),
            Node::scalar(Shape::Str, "deep"),
        )
        .unwrap();
        assert_eq!(
            get(&d, &Path::parse("x.y.z").unwrap())
                .unwrap()
                .as_scalar()
                .unwrap()
                .text,
            "deep"
        );
    }

    #[test]
    fn set_list_element() {
        let mut d = doc();
        set(
            &mut d,
            &Path::parse("list[0]").unwrap(),
            Node::scalar(Shape::Str, "P"),
        )
        .unwrap();
        assert_eq!(
            get(&d, &Path::parse("list[0]").unwrap())
                .unwrap()
                .as_scalar()
                .unwrap()
                .text,
            "P"
        );
    }

    #[test]
    fn set_rejects_out_of_range_index() {
        let mut d = doc();
        let err = set(
            &mut d,
            &Path::parse("list[5]").unwrap(),
            Node::scalar(Shape::Str, "x"),
        )
        .unwrap_err();
        assert!(matches!(err, OpError::IndexOutOfBounds(5)));
    }

    #[test]
    fn set_rejects_wrong_shape() {
        let mut d = doc();
        // a.b is a scalar; cannot descend into it with a.b.c
        let err = set(
            &mut d,
            &Path::parse("a.b.c").unwrap(),
            Node::scalar(Shape::Str, "x"),
        )
        .unwrap_err();
        assert_eq!(err, OpError::NotAMap);
    }

    #[test]
    fn set_rejects_root() {
        let mut d = doc();
        assert!(set(&mut d, &Path::default(), Node::map(Map::new())).is_err());
    }

    // --- remove ---

    #[test]
    fn remove_leaf_returns_node() {
        let mut d = doc();
        let removed = remove(&mut d, &Path::parse("a.b").unwrap()).unwrap();
        assert_eq!(removed.unwrap().as_scalar().unwrap().text, "x");
        assert!(get(&d, &Path::parse("a.b").unwrap()).is_none());
        // ancestor `a` is left as an empty map, not pruned
        assert!(get(&d, &Path::parse("a").unwrap())
            .unwrap()
            .as_map()
            .is_some());
        assert!(get(&d, &Path::parse("a").unwrap())
            .unwrap()
            .as_map()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn remove_list_element() {
        let mut d = doc();
        let removed = remove(&mut d, &Path::parse("list[1]").unwrap()).unwrap();
        assert_eq!(removed.unwrap().as_scalar().unwrap().text, "q");
        assert_eq!(
            get(&d, &Path::parse("list").unwrap())
                .unwrap()
                .as_list()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            get(&d, &Path::parse("list[1]").unwrap())
                .unwrap()
                .as_scalar()
                .unwrap()
                .text,
            "r"
        );
    }

    #[test]
    fn remove_missing_key_errors() {
        let mut d = doc();
        let err = remove(&mut d, &Path::parse("a.missing").unwrap()).unwrap_err();
        assert!(matches!(err, OpError::MissingKey(_)));
    }

    #[test]
    fn remove_out_of_range_errors() {
        let mut d = doc();
        let err = remove(&mut d, &Path::parse("list[9]").unwrap()).unwrap_err();
        assert!(matches!(err, OpError::IndexOutOfBounds(9)));
    }

    // --- remove_recursive ---

    #[test]
    fn remove_recursive_prunes_empty_ancestors() {
        // Build: parent: { child: { leaf: "v" } }
        let mut child = Map::new();
        child.insert("leaf".into(), Node::scalar(Shape::Str, "v"));
        let mut parent = Map::new();
        parent.insert("child".into(), Node::map(child));
        let mut root = Map::new();
        root.insert("parent".into(), Node::map(parent));
        let mut d = Node::map(root);

        remove_recursive(&mut d, &Path::parse("parent.child.leaf").unwrap()).unwrap();
        // both child and parent are now gone
        assert!(get(&d, &Path::parse("parent").unwrap()).is_none());
    }

    #[test]
    fn remove_recursive_stops_at_nonempty_ancestor() {
        let mut child = Map::new();
        child.insert("leaf".into(), Node::scalar(Shape::Str, "v"));
        child.insert("keep".into(), Node::scalar(Shape::Str, "k"));
        let mut parent = Map::new();
        parent.insert("child".into(), Node::map(child));
        let mut root = Map::new();
        root.insert("parent".into(), Node::map(parent));
        let mut d = Node::map(root);

        remove_recursive(&mut d, &Path::parse("parent.child.leaf").unwrap()).unwrap();
        // child still has `keep`, so parent survives
        assert!(get(&d, &Path::parse("parent.child.keep").unwrap()).is_some());
        assert!(get(&d, &Path::parse("parent.child.leaf").unwrap()).is_none());
    }

    #[test]
    fn path_parse_quoted_segments() {
        let p = Path::parse(r#"metadata.labels."app.kubernetes.io/name""#).unwrap();
        let segs: Vec<&str> = p.segments().iter().map(|s| s.as_key().unwrap()).collect();
        assert_eq!(segs, vec!["metadata", "labels", "app.kubernetes.io/name"]);
    }

    #[test]
    fn path_get_quoted_label() {
        let doc = crate::deserialize::from_str(
            "metadata:\n  labels:\n    \"app.kubernetes.io/name\": guestbook\n",
        )
        .unwrap();
        let p = Path::parse(r#"metadata.labels."app.kubernetes.io/name""#).unwrap();
        let v = get(&doc, &p).unwrap();
        assert_eq!(v.as_scalar().unwrap().text, "guestbook");
    }

    #[test]
    fn path_parse_single_quoted() {
        let p = Path::parse("a.'b.c'.d").unwrap();
        let segs: Vec<&str> = p.segments().iter().map(|s| s.as_key().unwrap()).collect();
        assert_eq!(segs, vec!["a", "b.c", "d"]);
    }

    #[test]
    fn path_empty_quoted_key_rejected() {
        assert!(Path::parse("a.\"\".b").is_err());
    }

    #[test]
    fn remove_recursive_does_not_prune_list_elements() {
        // list: [ { leaf: "v" } ]
        let mut inner = Map::new();
        inner.insert("leaf".into(), Node::scalar(Shape::Str, "v"));
        let mut root = Map::new();
        root.insert("list".into(), Node::list(vec![Node::map(inner)]));
        let mut d = Node::map(root);

        remove_recursive(&mut d, &Path::parse("list[0].leaf").unwrap()).unwrap();
        // the list element (now an empty map) is kept, not pruned
        let list = get(&d, &Path::parse("list").unwrap())
            .unwrap()
            .as_list()
            .unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].as_map().unwrap().is_empty());
    }
}
