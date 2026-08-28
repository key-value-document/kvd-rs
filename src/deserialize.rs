//! Deserializer: KVD text → [`Node`] (spec §4 grammar).
//!
//! Mirrors [`serialize::to_string`](crate::serialize::to_string): where the
//! serializer turns a node tree into text, [`from_str`] turns text back into
//! the node tree. Parsing is line-based recursive descent with one token of
//! lookahead; errors carry a line:col position and a category from
//! [`ErrorKind`] (spec §7).
//!
//! The parser is registry-free and lossless: the `__schema__` metakey is
//! validated but kept as an ordinary entry of the returned root map, so
//! `serialize(from_str(text))` reproduces schema documents verbatim.
//! Schema verification is a separate pass (spec §5).

use crate::error::{Error, ErrorKind, Result};
use crate::grammar::{
    is_builtin_type, is_float, is_int, is_key, is_known_metakey, is_metakey, is_type_name,
};
use crate::value::{Map, Node, Scalar, Shape};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Parses a KVD document into a node tree. The root is always a mapping;
/// an empty (or comments-only) document parses as an empty mapping.
pub fn from_str(text: &str) -> Result<Node> {
    Parser::new(text).parse_document()
}

/// One source line, split into indent and content.
struct Line {
    /// 1-based source line number.
    no: usize,
    /// Number of leading spaces.
    indent: usize,
    /// Content after the indent, verbatim.
    rest: String,
    /// The full original line (blocks need the leading spaces).
    raw: String,
}

impl Line {
    /// The comment-stripped, right-trimmed form of this line's content.
    fn stripped(&self) -> &str {
        strip_comment(&self.rest).trim_end()
    }

    /// True when this line starts a list item (`-`, `- `, `- x`).
    fn is_marker(&self) -> bool {
        let s = self.stripped();
        s == "-" || s.starts_with("- ")
    }
}

struct Parser {
    lines: Vec<Line>,
    /// Index of the next significant (non-blank, non-comment) line.
    pos: usize,
}

impl Parser {
    fn new(text: &str) -> Self {
        let total = text.split('\n').count();
        let mut lines = Vec::new();
        for (i, raw) in text.split('\n').enumerate() {
            // A trailing newline produces one empty final element; drop it.
            if i + 1 == total && raw.is_empty() {
                break;
            }
            let indent = raw.len() - raw.trim_start_matches(' ').len();
            lines.push(Line {
                no: i + 1,
                indent,
                rest: raw[indent..].to_string(),
                raw: raw.to_string(),
            });
        }
        let mut p = Parser { lines, pos: 0 };
        p.skip_insignificant();
        p
    }

    /// Advances `pos` past blank and comment-only lines.
    fn skip_insignificant(&mut self) {
        while self.pos < self.lines.len() && self.lines[self.pos].stripped().is_empty() {
            self.pos += 1;
        }
    }

    /// Borrows the next significant line, if any.
    fn peek(&self) -> Option<&Line> {
        self.lines.get(self.pos)
    }

    fn parse_document(&mut self) -> Result<Node> {
        let map = self.parse_pairs(0, true, 1)?;
        Ok(Node::Map(map))
    }

    /// Parses consecutive `path: value` pairs whose keys start at `indent`.
    fn parse_pairs(&mut self, indent: usize, root: bool, depth: usize) -> Result<Map> {
        let mut map = Map::new();
        loop {
            self.skip_insignificant();
            let l = match self.peek() {
                None => break,
                Some(l) => l,
            };
            if l.indent < indent {
                break;
            }
            if l.indent > indent {
                return Err(error(
                    ErrorKind::BadIndent,
                    l.no,
                    l.indent + 1,
                    format!("expected keys at column {}", indent + 1),
                ));
            }
            if l.is_marker() {
                return Err(error(
                    ErrorKind::BadListMarker,
                    l.no,
                    l.indent + 1,
                    "list item where a mapping pair was expected",
                ));
            }
            let (path, value) = self.parse_pair(indent, root, depth)?;
            insert_path(&mut map, &path, value, last_line_no(&self.lines, self.pos))?;
        }
        Ok(map)
    }

    /// Parses one `path: value` pair starting at the current line.
    ///
    /// Precondition: `self.pos` points at the key line and its indent equals
    /// `keycol`. Consumes the key line; block content and subtrees are read
    /// from the following lines.
    fn parse_pair(
        &mut self,
        keycol: usize,
        root: bool,
        depth: usize,
    ) -> Result<(Vec<String>, Node)> {
        let line = &self.lines[self.pos];
        let line_no = line.no;
        let stripped = line.stripped().to_string();
        check_tab(&line.rest, line_no, keycol + 1)?;
        self.pos += 1;

        let Some((segs, after)) = scan_path(&stripped, line_no, keycol + 1, root)? else {
            return Err(error(
                ErrorKind::UnexpectedCharacter,
                line_no,
                keycol + 1,
                "expected `key:`",
            ));
        };
        if depth + segs.len() - 1 > crate::MAX_DEPTH {
            return Err(error(
                ErrorKind::DepthLimit,
                line_no,
                keycol + 1,
                format!("nesting exceeds the maximum depth of {}", crate::MAX_DEPTH),
            ));
        }

        let value_col = keycol + 1 + stripped[..after].chars().count();
        let value = if after >= stripped.len() {
            // `key:` — the value is an indented subtree on the following lines.
            self.skip_insignificant();
            match self.peek() {
                None => return Err(missing_value(line_no, keycol)),
                Some(l) if l.indent <= keycol => return Err(missing_value(line_no, keycol)),
                Some(l) => {
                    let child = l.indent;
                    if child != keycol + 2 {
                        return Err(error(
                            ErrorKind::BadIndent,
                            l.no,
                            child + 1,
                            format!("subtree must be indented to column {}", keycol + 3),
                        ));
                    }
                    // The subtree hangs off the pair's leaf, `segs.len()`
                    // levels below this pair's own depth slot.
                    if l.is_marker() {
                        Node::List(self.parse_items(child, depth + segs.len())?)
                    } else {
                        Node::Map(self.parse_pairs(child, false, depth + segs.len())?)
                    }
                }
            }
        } else if let Some(v) = stripped[after..].strip_prefix(' ') {
            if v.starts_with(' ') {
                return Err(error(
                    ErrorKind::UnexpectedCharacter,
                    line_no,
                    value_col + 1,
                    "expected exactly one space after ':'",
                ));
            }
            match v {
                "\"\"\"" => Node::Scalar(self.read_triple(keycol)?),
                _ if v.starts_with("\"\"\"") => {
                    return Err(error(
                        ErrorKind::UnexpectedCharacter,
                        line_no,
                        value_col + 1,
                        "triple-quoted string must start on its own line",
                    ));
                }
                _ => self.scalar_from_token(v, line_no, value_col + 1)?,
            }
        } else {
            return Err(error(
                ErrorKind::UnexpectedCharacter,
                line_no,
                value_col,
                "expected exactly one space after ':'",
            ));
        };
        Ok((segs, value))
    }

    /// Parses consecutive list items whose `-` markers start at `indent`.
    fn parse_items(&mut self, indent: usize, depth: usize) -> Result<Vec<Node>> {
        let mut items = Vec::new();
        loop {
            self.skip_insignificant();
            let l = match self.peek() {
                None => break,
                Some(l) => l,
            };
            if l.indent < indent {
                break;
            }
            if l.indent > indent {
                return Err(error(
                    ErrorKind::BadIndent,
                    l.no,
                    l.indent + 1,
                    format!("expected list markers at column {}", indent + 1),
                ));
            }
            let stripped = l.stripped().to_string();
            if !(stripped == "-" || stripped.starts_with("- ")) {
                break; // not an item; the caller decides what comes next
            }
            let line_no = l.no;
            if stripped == "-" || stripped == "- " {
                // Bare marker: the item is an indented subtree below.
                self.pos += 1;
                items.push(self.parse_bare_item(indent, line_no, depth)?);
                continue;
            }
            // `- content`: exactly one space after the dash.
            if self.lines[self.pos].rest[2..].starts_with(' ') {
                return Err(error(
                    ErrorKind::BadListMarker,
                    line_no,
                    indent + 3,
                    "expected exactly one space after '-'",
                ));
            }
            let content = strip_comment(&self.lines[self.pos].rest[2..])
                .trim_end()
                .to_string();
            if content.is_empty() {
                // `- ` with nothing but trailing space behaves like `-`.
                self.pos += 1;
                items.push(self.parse_bare_item(indent, line_no, depth)?);
            } else if content == "-" || content.starts_with("- ") {
                // Compact nested list (`- - x`): rewrite the line so the
                // inner marker stands alone at indent + 2, then recurse.
                let inner = self.lines[self.pos].rest[2..].to_string();
                let slot = &mut self.lines[self.pos];
                slot.indent += 2;
                slot.rest = inner;
                items.push(Node::List(self.parse_items(indent + 2, depth + 1)?));
            } else if content == "\"\"\"" {
                // Triple-quoted string as a list item (spec §5): content
                // lines and closer both sit two columns past the marker.
                self.pos += 1;
                items.push(Node::Scalar(self.read_triple(indent + 2)?));
            } else if matches!(scan_path(&content, line_no, indent + 3, false), Ok(Some(_))) {
                // Inline first pair of a mapping item: rewrite the line so
                // the pair starts at its real column, parse it, then consume
                // continuation pairs aligned to that column.
                let inner = self.lines[self.pos].rest[2..].to_string();
                let slot = &mut self.lines[self.pos];
                slot.indent += 2;
                slot.rest = inner;
                let keycol = indent + 2;
                let (path, value) = self.parse_pair(keycol, false, depth + 1)?;
                let mut map = Map::new();
                insert_path(&mut map, &path, value, line_no)?;
                loop {
                    let cont = match self.peek() {
                        None => break,
                        Some(l2) => l2,
                    };
                    if cont.indent < keycol {
                        break;
                    }
                    if cont.indent > keycol {
                        return Err(error(
                            ErrorKind::MisalignedKey,
                            cont.no,
                            cont.indent + 1,
                            format!("item keys must align to column {}", keycol + 1),
                        ));
                    }
                    if cont.is_marker() {
                        break; // next item
                    }
                    let cont_no = cont.no;
                    let (p2, v2) = self.parse_pair(keycol, false, depth + 1)?;
                    insert_path(&mut map, &p2, v2, cont_no)?;
                }
                items.push(Node::Map(map));
            } else {
                // Plain scalar item.
                self.pos += 1;
                items.push(self.scalar_from_token(&content, line_no, indent + 3)?);
            }
        }
        Ok(items)
    }

    /// Parses the item introduced by a bare `-` marker: either an indented
    /// mapping or an indented nested list on the following lines.
    fn parse_bare_item(&mut self, indent: usize, line_no: usize, depth: usize) -> Result<Node> {
        self.skip_insignificant();
        let Some(l) = self.peek() else {
            return Err(missing_value(line_no, indent));
        };
        if l.indent <= indent {
            return Err(missing_value(line_no, indent));
        }
        let child = l.indent;
        if child != indent + 2 {
            return Err(error(
                ErrorKind::BadIndent,
                l.no,
                child + 1,
                format!("item content must be indented to column {}", indent + 3),
            ));
        }
        if l.is_marker() {
            Ok(Node::List(self.parse_items(child, depth + 1)?))
        } else {
            Ok(Node::Map(self.parse_pairs(child, false, depth + 1)?))
        }
    }

    /// Reads a `"""` block whose key line has been consumed. Content is
    /// every following line indented past `keycol`; blanks are content too.
    /// The common indent of non-blank lines is stripped. The block ends at
    /// a `"""` closer; a missing closer is an error.
    ///
    /// Two closer forms (spec §5):
    ///   - Standalone: `"""` alone on a line at exactly `keycol` → trailing `\n` added.
    ///   - Inline: `"""` appended to the last content line → no trailing `\n`.
    ///
    /// Escapes are processed per content line (same rules as `"..."`).
    fn read_triple(&mut self, keycol: usize) -> Result<Scalar> {
        let mut content: Vec<(String, usize, usize)> = Vec::new();
        let mut trailing_nl = false;
        let mut i = self.pos;
        loop {
            if i >= self.lines.len() {
                return Err(error(
                    ErrorKind::Unterminated,
                    self.lines.last().map(|l| l.no).unwrap_or(0),
                    1,
                    "unterminated triple-quoted string (missing `\"\"\"`)",
                ));
            }
            let l = &self.lines[i];
            // Blank lines are kept as empty content; their indent=0 must be
            // checked before the indent guard below.
            if l.rest.trim().is_empty() {
                content.push((String::new(), l.no, l.indent));
                i += 1;
                continue;
            }
            // Standalone closer: `"""` alone at exactly keycol → trailing \n.
            // Checked before the indent guard so that list-item `"""` blocks,
            // where content and closer share the same column, are handled correctly.
            if l.indent == keycol && l.rest.trim() == "\"\"\"" {
                self.pos = i + 1;
                trailing_nl = true;
                break;
            }
            if l.indent < keycol {
                return Err(error(
                    ErrorKind::Unterminated,
                    l.no,
                    l.indent + 1,
                    "unterminated triple-quoted string (missing `\"\"\"`)",
                ));
            }
            // Inline closer: content line ending with `"""` → no trailing \n.
            if l.rest.trim_end().ends_with("\"\"\"") {
                let raw = &l.raw;
                let closer_pos = raw.rfind("\"\"\"").expect("just confirmed");
                content.push((raw[..closer_pos].to_string(), l.no, l.indent));
                self.pos = i + 1;
                break;
            }
            content.push((l.raw.clone(), l.no, l.indent));
            i += 1;
        }

        let common = content
            .iter()
            .filter(|(raw, _, _)| !raw.is_empty())
            .map(|(raw, _, _)| raw.len() - raw.trim_start_matches(' ').len())
            .min()
            .unwrap_or(0);
        let body: Vec<String> = content
            .iter()
            .map(|(raw, no, indent)| {
                if raw.is_empty() {
                    Ok(String::new())
                } else {
                    let stripped = &raw[common.min(raw.len())..];
                    scan_escapes(stripped, *no, *indent + 1 + common)
                }
            })
            .collect::<Result<Vec<String>>>()?;

        let text = if trailing_nl && !body.is_empty() {
            format!("{}\n", body.join("\n"))
        } else {
            body.join("\n")
        };
        Ok(Scalar {
            shape: Shape::Str,
            text: text.clone(),
            raw: format!("\"{}\"", text),
        })
    }

    /// Classifies a value token into a scalar (or `{}`/`[]` literal).
    fn scalar_from_token(&self, tok: &str, line_no: usize, col: usize) -> Result<Node> {
        if let Some(rest) = tok.strip_prefix('"') {
            let (text, used) = scan_dquote(tok, line_no, col)?;
            if used != tok.len() {
                return Err(unexpected_after(line_no, col + rest.chars().count() + used));
            }
            return Ok(Node::Scalar(Scalar::with_raw(Shape::Str, text, tok)));
        }
        if let Some(rest) = tok.strip_prefix('\'') {
            let (text, used) = scan_squote(tok, line_no, col)?;
            if used != tok.len() {
                return Err(unexpected_after(line_no, col + rest.chars().count() + used));
            }
            return Ok(Node::Scalar(Scalar::with_raw(Shape::Str, text, tok)));
        }
        match tok {
            "{}" => return Ok(Node::map(Map::new())),
            "[]" => return Ok(Node::list(Vec::new())),
            "true" => return Ok(Node::scalar(Shape::Bool, "true")),
            "false" => return Ok(Node::scalar(Shape::Bool, "false")),
            "null" => return Ok(Node::scalar(Shape::Null, "null")),
            _ => {}
        }
        if is_int(tok) {
            return Ok(Node::scalar(Shape::Int, tok));
        }
        if is_float(tok) {
            return Ok(Node::scalar(Shape::Float, tok));
        }
        if is_builtin_type(tok) || is_type_name(tok) {
            // Type names (builtins and unknown) may appear bare in schema
            // position (spec §5). The parser is registry-free and cannot
            // distinguish schema from data context, so unknown names are
            // accepted here and produce an unknown-type violation at verify
            // time, not a parse error.
            return Ok(Node::Scalar(Scalar::new(Shape::Str, tok)));
        }
        Err(error(
            ErrorKind::UnexpectedCharacter,
            line_no,
            col,
            format!("invalid value token `{tok}` (string values must be double-quoted)"),
        ))
    }
}

/// Inserts `value` at `path`, creating intermediate maps. Collisions are
/// hard errors (spec §4 structural rules).
fn insert_path(map: &mut Map, path: &[String], value: Node, line_no: usize) -> Result<()> {
    let mut cur = map;
    for (i, seg) in path.iter().enumerate() {
        if i + 1 == path.len() {
            if cur.contains_key(seg) {
                return Err(error(
                    ErrorKind::DuplicateKey,
                    line_no,
                    1,
                    format!("duplicate key `{}`", path.join(".")),
                ));
            }
            cur.insert(seg.clone(), value);
            return Ok(());
        }
        if !cur.contains_key(seg) {
            cur.insert(seg.clone(), Node::map(Map::new()));
        }
        match cur.get_mut(seg) {
            Some(Node::Map(inner)) => cur = inner,
            Some(_) => {
                return Err(error(
                    ErrorKind::LeafInteriorConflict,
                    line_no,
                    1,
                    format!(
                        "`{}` is already a value but `{}` nests under it",
                        path[..=i].join("."),
                        path.join(".")
                    ),
                ));
            }
            None => unreachable!("just inserted"),
        }
    }
    unreachable!("empty path")
}

/// Scans a dotted path `seg ("." seg)* ":"` at the start of `s`.
///
/// Returns `Ok(None)` when `s` cannot be a pair (no top-level `:`), which
/// lets list-item parsing fall back to scalar. Returns the segments and the
/// offset just past the `:` otherwise. Metakey routing is validated here:
/// known metakeys only at the document root, unknown ones rejected.
fn scan_path(
    s: &str,
    line_no: usize,
    base_col: usize,
    root: bool,
) -> Result<Option<(Vec<String>, usize)>> {
    let mut segs: Vec<String> = Vec::new();
    let mut c = 0usize;
    loop {
        let col = base_col + s[..c].chars().count();
        let seg = match s.as_bytes().get(c) {
            Some(b'"') | Some(b'\'') => {
                // Quoted segment: a literal key that may contain '.', '/',
                // ':', etc. Metakey recognition applies only to bare segments
                // (spec §4), so a quoted "__schema__" is data, not a metakey.
                let (text, used) = scan_quoted_segment(&s[c..], line_no, col)?;
                c += used;
                if text.is_empty() {
                    return Err(error(ErrorKind::BadPath, line_no, col, "empty quoted key"));
                }
                text
            }
            Some(_) => {
                let end = s[c..].find(['.', ':']).map(|i| c + i).unwrap_or(s.len());
                let raw = &s[c..end];
                c = end;
                if raw.is_empty() {
                    return Err(error(
                        ErrorKind::BadPath,
                        line_no,
                        col,
                        "empty path segment",
                    ));
                }
                validate_key_seg(raw, segs.is_empty(), root, line_no, col)?;
                raw.to_string()
            }
            None => {
                if segs.is_empty() {
                    return Ok(None); // no path at all: not a pair
                }
                return Err(error(
                    ErrorKind::UnexpectedCharacter,
                    line_no,
                    col,
                    "expected ':'",
                ));
            }
        };
        segs.push(seg);
        match s.as_bytes().get(c) {
            Some(b'.') => c += 1,
            Some(b':') => return Ok(Some((segs, c + 1))),
            _ => {
                if segs.len() == 1 && !s[..c].contains('.') {
                    return Ok(None); // single bare segment, no ':': scalar candidate
                }
                return Err(error(
                    ErrorKind::UnexpectedCharacter,
                    line_no,
                    col,
                    "expected '.' or ':'",
                ));
            }
        }
    }
}

/// Validates one unquoted path segment against the key/metakey grammar.
fn validate_key_seg(raw: &str, first: bool, root: bool, line_no: usize, col: usize) -> Result<()> {
    if is_metakey(raw) {
        if first && root {
            if !is_known_metakey(raw) {
                return Err(error(
                    ErrorKind::UnknownMetakey,
                    line_no,
                    col,
                    format!("unknown metakey `{raw}`"),
                ));
            }
            return Ok(());
        }
        return Err(error(
            ErrorKind::MetakeyOutsideRoot,
            line_no,
            col,
            format!("metakey `{raw}` is only allowed at the document root"),
        ));
    }
    if !is_key(raw) {
        return Err(error(
            ErrorKind::UnexpectedCharacter,
            line_no,
            col,
            format!("invalid key `{raw}` (keys are alphanumerics plus '-'/'_', quoted otherwise)"),
        ));
    }
    Ok(())
}

/// Scans a double-quoted string starting at `s[0] == '"'`. Returns the
/// decoded text and the number of bytes consumed (including both quotes).
fn scan_dquote(s: &str, line_no: usize, col: usize) -> Result<(String, usize)> {
    // Find the closing quote, skipping escaped `\"`.
    let mut end = None;
    let mut chars = s[1..].char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        match ch {
            '\\' => {
                chars.next();
            }
            '"' => {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    let end = end.ok_or_else(|| {
        error(
            ErrorKind::Unterminated,
            line_no,
            col,
            "unterminated double-quoted string",
        )
    })?;
    let text = scan_escapes(&s[1..1 + end], line_no, col + 1)?;
    Ok((text, end + 2))
}

/// Scans a single-quoted string starting at `s[0] == '\''`. Literal: no
/// escapes, so the string cannot contain `'` (spec §3).
fn scan_squote(s: &str, line_no: usize, col: usize) -> Result<(String, usize)> {
    match s[1..].find('\'') {
        Some(i) => Ok((s[1..1 + i].to_string(), i + 2)),
        None => Err(error(
            ErrorKind::Unterminated,
            line_no,
            col,
            "unterminated single-quoted string",
        )),
    }
}

/// Scans a quoted key/segment starting at `s[0]` (`"` or `'`). Returns the
/// decoded text and the number of bytes consumed (including both quotes).
/// Shared by the document key parser and the `Path` API so a quoted key
/// means the same thing in a document and in a `Path` string (spec §4/§8.5).
pub(crate) fn scan_quoted_segment(s: &str, line_no: usize, col: usize) -> Result<(String, usize)> {
    match s.as_bytes().first() {
        Some(b'"') => scan_dquote(s, line_no, col),
        Some(b'\'') => scan_squote(s, line_no, col),
        _ => Err(error(
            ErrorKind::UnexpectedCharacter,
            line_no,
            col,
            "expected quoted key",
        )),
    }
}

/// Processes KVD escape sequences in `s` (no quote scanning). Used for both
/// `"..."` strings and `"""` block content lines (spec §3 `escape`).
fn scan_escapes(s: &str, line_no: usize, col: usize) -> Result<String> {
    let mut out = String::new();
    let mut chars = s.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some((_, 'n')) => out.push('\n'),
                Some((_, 't')) => out.push('\t'),
                Some((_, '\\')) => out.push('\\'),
                Some((_, '"')) => out.push('"'),
                Some((_, '\'')) => out.push('\''),
                Some((j, 'u')) => {
                    let hex: String = chars.by_ref().take(4).map(|(_, c)| c).collect();
                    if hex.len() != 4 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                        return Err(error(
                            ErrorKind::UnexpectedCharacter,
                            line_no,
                            col + j,
                            "invalid \\u escape (expected 4 hex digits)",
                        ));
                    }
                    let code = u32::from_str_radix(&hex, 16).expect("hex digits");
                    match char::from_u32(code) {
                        Some(c) => out.push(c),
                        None => {
                            return Err(error(
                                ErrorKind::UnexpectedCharacter,
                                line_no,
                                col + j,
                                "invalid \\u escape (surrogate)",
                            ));
                        }
                    }
                }
                Some((j, other)) => {
                    return Err(error(
                        ErrorKind::UnexpectedCharacter,
                        line_no,
                        col + j,
                        format!("invalid escape `\\{other}`"),
                    ));
                }
                None => {
                    return Err(error(
                        ErrorKind::Unterminated,
                        line_no,
                        col,
                        "unterminated escape",
                    ));
                }
            },
            _ => out.push(ch),
        }
    }
    Ok(out)
}

/// Truncates `s` at the first `#` that sits outside a double-quoted string.
fn strip_comment(s: &str) -> &str {
    let mut in_dq = false;
    let mut esc = false;
    for (i, ch) in s.char_indices() {
        if in_dq {
            match ch {
                '\\' => esc = !esc,
                '"' if !esc => in_dq = false,
                _ => esc = false,
            }
        } else {
            match ch {
                '#' => return &s[..i],
                '"' => in_dq = true,
                _ => {}
            }
        }
    }
    s
}

/// Errors when a tab appears outside a double-quoted string (spec §2).
fn check_tab(s: &str, line_no: usize, base_col: usize) -> Result<()> {
    let mut in_dq = false;
    let mut esc = false;
    for (i, ch) in s.char_indices() {
        if in_dq {
            match ch {
                '\\' => esc = !esc,
                '"' if !esc => in_dq = false,
                _ => esc = false,
            }
        } else {
            match ch {
                '\t' => {
                    return Err(error(
                        ErrorKind::Tab,
                        line_no,
                        base_col + s[..i].chars().count(),
                        "tab outside quoted strings",
                    ));
                }
                '"' => in_dq = true,
                _ => {}
            }
        }
    }
    Ok(())
}

fn missing_value(line_no: usize, col: usize) -> Error {
    error(
        ErrorKind::MissingValue,
        line_no,
        col + 1,
        "key has no value and no indented subtree",
    )
}

fn unexpected_after(line_no: usize, col: usize) -> Error {
    error(
        ErrorKind::UnexpectedCharacter,
        line_no,
        col,
        "unexpected content after string",
    )
}

fn error(kind: ErrorKind, line: usize, col: usize, message: impl Into<String>) -> Error {
    Error::new(kind, line, col, message)
}

fn last_line_no(lines: &[Line], pos: usize) -> usize {
    lines.get(pos.saturating_sub(1)).map_or(1, |l| l.no)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind::*;
    use crate::serialize;

    fn doc(text: &str) -> Map {
        match from_str(text).expect("parse") {
            Node::Map(m) => m,
            other => panic!("expected map, got {other:?}"),
        }
    }

    fn err(text: &str) -> Error {
        from_str(text).expect_err("expected error")
    }

    fn scalar_text(text: &str, path: &str) -> String {
        let mut segs = path.split('.').peekable();
        let mut cur = doc(text).get(segs.next().unwrap()).expect("key").clone();
        for seg in segs {
            cur = cur.as_map().expect("map").get(seg).expect("key").clone();
        }
        cur.as_scalar().expect("scalar").text.clone()
    }

    fn shape_of(text: &str, path: &str) -> Shape {
        let mut segs = path.split('.').peekable();
        let mut cur = doc(text).get(segs.next().unwrap()).expect("key").clone();
        for seg in segs {
            cur = cur.as_map().expect("map").get(seg).expect("key").clone();
        }
        cur.as_scalar().expect("scalar").shape
    }

    // ---- scalars ----

    #[test]
    fn scalars_and_shapes() {
        let t = "\
a: 1
b: -5
c: +7
d: 1_234_567
e: 3.14
f: -0.5
g: true
h: false
i: \"hello\"
j: \"+foo\"
k: \"-foo2\"
l: \"quoted: value\"
m: \"single # quoted\"
o: {}
p: []";
        assert_eq!(shape_of(t, "a"), Shape::Int);
        assert_eq!(scalar_text(t, "b"), "-5");
        assert_eq!(shape_of(t, "c"), Shape::Int);
        assert_eq!(scalar_text(t, "d"), "1_234_567");
        assert_eq!(shape_of(t, "e"), Shape::Float);
        assert_eq!(shape_of(t, "f"), Shape::Float);
        assert_eq!(shape_of(t, "g"), Shape::Bool);
        assert_eq!(shape_of(t, "h"), Shape::Bool);
        assert_eq!(shape_of(t, "i"), Shape::Str);
        assert_eq!(scalar_text(t, "j"), "+foo");
        assert_eq!(scalar_text(t, "k"), "-foo2");
        assert_eq!(scalar_text(t, "l"), "quoted: value");
        assert_eq!(scalar_text(t, "m"), "single # quoted");
        assert!(doc(t).get("o").unwrap().as_map().unwrap().is_empty());
        assert!(doc(t).get("p").unwrap().as_list().unwrap().is_empty());
    }

    #[test]
    fn digit_start_non_numbers_are_errors() {
        // Not valid numbers (strict thousands groups) and not bare words:
        // under mandatory double-quoting these are parse errors.
        for v in ["1_2", "12_34_5", "5foo", "2026-08-20"] {
            assert_eq!(err(format!("a: {v}").as_str()).kind, UnexpectedCharacter);
        }
    }

    #[test]
    fn dquote_escapes() {
        let t = r#"a: "x\ny\tz\\q\"r's""#;
        assert_eq!(scalar_text(t, "a"), "x\ny\tz\\q\"r's");
    }

    #[test]
    fn unicode_escape() {
        let t = r#"a: "café""#;
        assert_eq!(scalar_text(t, "a"), "café");
    }

    #[test]
    fn comments_stripped() {
        let t = "\
# leading comment
a: 1 # trailing
# another
b: \"has # inside\" # ok";
        assert_eq!(scalar_text(t, "a"), "1");
        assert_eq!(scalar_text(t, "b"), "has # inside");
    }

    // ---- structure ----

    #[test]
    fn empty_document() {
        assert!(doc("").is_empty());
        assert!(doc("\n# only comments\n\n").is_empty());
    }

    #[test]
    fn dotted_paths_nest() {
        let t = "a.b.c: 1\na.b.d: 2\na.e: 3";
        let m = doc(t);
        assert_eq!(m.len(), 1);
        assert_eq!(scalar_text(t, "a.b.c"), "1");
        assert_eq!(scalar_text(t, "a.b.d"), "2");
        assert_eq!(scalar_text(t, "a.e"), "3");
    }

    #[test]
    fn indented_subtree() {
        let t = "\
outer:
  inner: 1
  deep:
    leaf: \"two\"
other: 3";
        assert_eq!(scalar_text(t, "outer.inner"), "1");
        assert_eq!(scalar_text(t, "outer.deep.leaf"), "two");
        assert_eq!(scalar_text(t, "other"), "3");
    }

    #[test]
    fn digit_start_keys() {
        let t = "8080: \"http\"\n2fa.enabled: true";
        assert_eq!(scalar_text(t, "8080"), "http");
        assert_eq!(shape_of(t, "2fa.enabled"), Shape::Bool);
    }

    #[test]
    fn quoted_keys_parse() {
        // Quoted keys are valid and may contain spaces/dots (spec §4).
        let d = from_str("\"weird key\": 1").unwrap();
        assert_eq!(
            d.as_map()
                .unwrap()
                .get("weird key")
                .unwrap()
                .as_scalar()
                .unwrap()
                .text,
            "1"
        );
        let d2 = from_str("\"a.b\": 2").unwrap();
        assert_eq!(
            d2.as_map()
                .unwrap()
                .get("a.b")
                .unwrap()
                .as_scalar()
                .unwrap()
                .text,
            "2"
        );
    }

    #[test]
    fn metakeys_at_root() {
        let t = "__schema__:\n  db.port: int";
        assert_eq!(scalar_text(t, "__schema__.db.port"), "int");
    }

    // ---- lists ----

    #[test]
    fn list_of_scalars() {
        let t = "l:\n  - \"a\"\n  - 2\n  - true\n  - -3.5";
        let items = doc(t).get("l").unwrap().as_list().unwrap().to_vec();
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].as_scalar().unwrap().text, "a");
        assert_eq!(items[1].as_scalar().unwrap().text, "2");
        assert_eq!(items[3].as_scalar().unwrap().text, "-3.5");
    }

    #[test]
    fn list_empty_literals() {
        let t = "l:\n  - {}\n  - []\n  - \"x\"";
        let items = doc(t).get("l").unwrap().as_list().unwrap().to_vec();
        assert!(items[0].as_map().unwrap().is_empty());
        assert!(items[1].as_list().unwrap().is_empty());
        assert_eq!(items[2].as_scalar().unwrap().text, "x");
    }

    #[test]
    fn nested_list_bare_marker() {
        let t = "l:\n  -\n    - \"a\"\n    - \"b\"\n  -\n    - \"c\"";
        let items = doc(t).get("l").unwrap().as_list().unwrap().to_vec();
        assert_eq!(items.len(), 2);
        let first = items[0].as_list().unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[1].as_scalar().unwrap().text, "b");
    }

    #[test]
    fn nested_list_compact() {
        let t = "l:\n  - - \"a\"\n    - \"b\"\n  - - \"c\"";
        let items = doc(t).get("l").unwrap().as_list().unwrap().to_vec();
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].as_list().unwrap()[1].as_scalar().unwrap().text,
            "b"
        );
        assert_eq!(
            items[1].as_list().unwrap()[0].as_scalar().unwrap().text,
            "c"
        );
    }

    #[test]
    fn list_of_mappings() {
        let t = "\
servers:
  - name: \"one\"
    port: 80
  - name: \"two\"
    port: 443
    tls:
      enabled: true";
        let items = doc(t).get("servers").unwrap().as_list().unwrap().to_vec();
        assert_eq!(items.len(), 2);
        let second = items[1].as_map().unwrap();
        assert_eq!(
            second
                .get("tls")
                .unwrap()
                .as_map()
                .unwrap()
                .get("enabled")
                .unwrap(),
            &Node::scalar(Shape::Bool, "true")
        );
    }

    #[test]
    fn dashed_strings_in_list_items() {
        // A leading `-` is a list marker at line start, but inside a quoted
        // string it is just content (spec §5 `"""`).
        let t = "args:\n  - \"-c\"\n  - \"--flag\"\n  - \"-v2\"";
        let items = doc(t).get("args").unwrap().as_list().unwrap().to_vec();
        assert_eq!(items[0].as_scalar().unwrap().text, "-c");
        assert_eq!(items[1].as_scalar().unwrap().text, "--flag");
        assert_eq!(items[2].as_scalar().unwrap().text, "-v2");
    }

    // ---- triple-quoted strings ----

    #[test]
    fn triple_keep_and_strip() {
        // Standalone closer ("""  on its own line): trailing \n is added.
        // Inline closer (""" appended to last content line): no trailing \n.
        let t = "\
a: \"\"\"
  line one
  line two
\"\"\"
b: \"\"\"
  no newline\"\"\"
";
        assert_eq!(scalar_text(t, "a"), "line one\nline two\n");
        assert_eq!(scalar_text(t, "b"), "no newline");
    }

    #[test]
    fn triple_common_indent_stripped() {
        // Standalone closer adds trailing \n.
        let t = "script: \"\"\"\n  if true;\n    nested;\n\"\"\"";
        assert_eq!(scalar_text(t, "script"), "if true;\n  nested;\n");
    }

    // ---- round-trip ----

    #[test]
    fn round_trip_canonical() {
        let cases = [
            "a: 1\n",
            "a:\n  b: \"hello world\"\n",
            "a:\n  b:\n    c: true\n",
            "l:\n  - 1\n  - {}\n  - []\n",
            "l:\n  -\n    - \"a\"\n    - \"b\"\n",
            "s:\n  - k: \"v\"\n    n: 2\n",
            "a: \"\"\"\n  keep\n  me\n\"\"\"\n",
            "a: \"\"\"\n  line a\n  line b\n\"\"\"\n",
            "o: {}\np: []\n",
            "big: 1_234_567\nneg: -12.5\nplus: +9\nw: \"-foo\"\n",
        ];
        for text in cases {
            let node = from_str(text).expect("parse");
            let out = crate::serialize::to_string(&node).expect("serialize");
            assert_eq!(out, text, "round-trip failed for {text:?}");
        }
    }

    #[test]
    fn reparse_stability() {
        let node = from_str("a:\n  b:\n    - \"x\"\n    - y: 2\n").expect("parse");
        let out = crate::serialize::to_string(&node).expect("serialize");
        assert_eq!(from_str(&out).expect("re-parse"), node);
    }

    // ---- errors ----

    #[test]
    fn err_duplicate_key() {
        let e = err("a: 1\na: 2");
        assert_eq!(e.kind, DuplicateKey);
        assert_eq!(e.line, 2);
    }

    #[test]
    fn err_leaf_interior_conflict() {
        let e = err("a: 1\na.b: 2");
        assert_eq!(e.kind, LeafInteriorConflict);
    }

    #[test]
    fn err_missing_value() {
        let e = err("a: 1\nb:");
        assert_eq!(e.kind, MissingValue);
        assert_eq!(e.line, 2);
    }

    #[test]
    fn err_bad_subtree_indent() {
        let e = err("a:\n   b: 1");
        assert_eq!(e.kind, BadIndent);
    }

    #[test]
    fn err_top_level_indent() {
        let e = err("a: 1\n  b: 2");
        assert_eq!(e.kind, BadIndent);
    }

    #[test]
    fn err_tab() {
        let e = err("a:\tx");
        assert_eq!(e.kind, Tab);
    }

    #[test]
    fn err_unterminated_string() {
        assert_eq!(err("a: \"oops").kind, Unterminated);
        // A triple-quoted string missing its closer is unterminated too.
        assert_eq!(err("a: \"\"\"").kind, Unterminated);
    }

    #[test]
    fn err_invalid_value_token() {
        assert_eq!(err("a: -").kind, UnexpectedCharacter);
        assert_eq!(err("a: --flag").kind, UnexpectedCharacter);
        // A colon must be followed by a word character (spec §3).
        assert_eq!(err("a: x:").kind, UnexpectedCharacter);
        assert_eq!(err("a: x: y").kind, UnexpectedCharacter);
        // `=` is not a word character.
        assert_eq!(err("a: x=y").kind, UnexpectedCharacter);
    }

    #[test]
    fn extended_words_are_errors() {
        // Bare words are no longer valid string values: under mandatory
        // double-quoting these are parse errors (spec §3).
        for v in [
            "/health", "a/b/c", "foo.bar", "1.5.2", "http://x", "10:30", "@daily", "-5foo",
        ] {
            assert_eq!(
                err(format!("k: {v}").as_str()).kind,
                UnexpectedCharacter,
                "`{v}` should be an error without quotes"
            );
        }
    }

    #[test]
    fn builtin_type_names_bare_in_value_position() {
        // The four builtin type names may appear bare; they are meaningful
        // only in schema position (spec §5).
        for v in ["int", "float", "bool", "str"] {
            assert_eq!(scalar_text(&format!("k: {v}"), "k"), v);
        }
    }

    #[test]
    fn null_literal_parses() {
        match doc("k: null\n").get("k") {
            Some(Node::Scalar(s)) => {
                assert_eq!(s.shape, Shape::Null);
                assert_eq!(s.text, "null");
            }
            other => panic!("expected null scalar, got {other:?}"),
        }
        // Keyword-exact: near-misses must be quoted. `Null` starts uppercase
        // (not a type name) and `null.x` contains `.` (also not a type name),
        // so they are parse errors. `nullish` matches the type-name grammar
        // and parses as the string "nullish" (an unknown type name produces a
        // verify-time error, not a parse error, spec §5).
        for w in ["Null", "null.x"] {
            assert_eq!(err(format!("k: {w}").as_str()).kind, UnexpectedCharacter);
        }
        assert_eq!(scalar_text("k: nullish", "k"), "nullish");
    }

    #[test]
    fn triple_as_list_item() {
        // Standalone closer adds \n; inline closer does not.
        let text = "l:\n  - \"\"\"\n    #!/bin/sh\n    echo hi\n    \"\"\"\n  - \"\"\"\n    no newline\"\"\"\n  - \"plain\"\n";
        let parsed = from_str(text).expect("parse");
        let items = parsed
            .as_map()
            .expect("map")
            .get("l")
            .expect("l")
            .as_list()
            .expect("list");
        assert_eq!(items.len(), 3);
        assert_eq!(
            items[0].as_scalar().expect("scalar").text,
            "#!/bin/sh\necho hi\n"
        );
        assert_eq!(items[1].as_scalar().expect("scalar").text, "no newline");
        assert_eq!(items[2].as_scalar().expect("scalar").text, "plain");
        // Serializer: trailing-\n strings use standalone closer; others use inline.
        let expected = "l:\n  - \"\"\"\n    #!/bin/sh\n    echo hi\n    \"\"\"\n  - \"no newline\"\n  - \"plain\"\n";
        assert_eq!(serialize::to_string(&parsed).unwrap(), expected);
    }

    #[test]
    fn err_unknown_metakey() {
        assert_eq!(err("__bogus__: 1").kind, UnknownMetakey);
    }

    #[test]
    fn err_metakey_not_at_root() {
        assert_eq!(err("a:\n  __schema__: x").kind, MetakeyOutsideRoot);
    }

    #[test]
    fn err_list_marker_in_mapping() {
        assert_eq!(err("- x").kind, BadListMarker);
    }

    #[test]
    fn err_double_space_after_colon() {
        assert_eq!(err("a:  x").kind, UnexpectedCharacter);
    }

    #[test]
    fn err_depth_limit() {
        let mut t = String::from("a");
        for _ in 0..120 {
            t.push_str(".b");
        }
        t.push_str(": 1");
        assert_eq!(err(&t).kind, DepthLimit);
    }

    #[test]
    fn err_depth_limit_across_dotted_pairs() {
        // Regression: each pair passed the depth check in isolation, but
        // the second dotted path grafts onto the first's leaf, so the
        // grafted tree exceeded MAX_DEPTH while both checks passed.
        let parent: Vec<String> = (0..60).map(|i| format!("k{i}")).collect();
        let child: Vec<String> = (0..60).map(|i| format!("m{i}")).collect();
        let t = format!("{}:\n  {}: 0\n", parent.join("."), child.join("."));
        assert_eq!(err(&t).kind, DepthLimit);
    }

    #[test]
    fn depth_exactly_max_round_trips() {
        // 100 nested keys (51 dotted segments + 49 block levels) sits
        // exactly at the limit: the second pair's check is 52 + 49 - 1 =
        // MAX_DEPTH (root keys occupy slot 1). It parses, serializes, and
        // re-parses identically.
        let parent: Vec<String> = (0..51).map(|i| format!("k{i}")).collect();
        let child: Vec<String> = (0..49).map(|i| format!("m{i}")).collect();
        let t = format!("{}:\n  {}: 0\n", parent.join("."), child.join("."));
        let doc = from_str(&t).unwrap();
        let out = crate::serialize::to_string(&doc).unwrap();
        assert_eq!(from_str(&out).unwrap(), doc);
    }

    #[test]
    fn quoted_key_with_dots() {
        let doc = from_str("metadata:\n  labels:\n    \"app.kubernetes.io/name\": guestbook\n")
            .expect("parse");
        let labels = doc
            .as_map()
            .unwrap()
            .get("metadata")
            .unwrap()
            .as_map()
            .unwrap()
            .get("labels")
            .unwrap()
            .as_map()
            .unwrap();
        assert_eq!(
            labels
                .get("app.kubernetes.io/name")
                .unwrap()
                .as_scalar()
                .unwrap()
                .text,
            "guestbook"
        );
    }

    #[test]
    fn quoted_key_single_and_double() {
        let doc = from_str("'a.b.c': \"x\"\n\"d/e.f\": \"y\"\n").unwrap();
        let m = doc.as_map().unwrap();
        assert_eq!(m.get("a.b.c").unwrap().as_scalar().unwrap().text, "x");
        assert_eq!(m.get("d/e.f").unwrap().as_scalar().unwrap().text, "y");
    }

    #[test]
    fn quoted_key_round_trips() {
        let text = "k:\n  \"app.kubernetes.io/name\": guestbook\n";
        let doc = from_str(text).unwrap();
        let out = crate::serialize::to_string(&doc).unwrap();
        assert_eq!(from_str(&out).unwrap(), doc);
    }

    #[test]
    fn empty_quoted_key_rejected() {
        assert!(from_str("\"\": x\n").is_err());
    }

    #[test]
    fn quoted_metakey_is_literal() {
        let doc = from_str("\"__schema__\": x\n").unwrap();
        assert!(doc.as_map().unwrap().get("__schema__").is_some());
    }

    #[test]
    fn err_depth_limit_one_past_max() {
        // One segment more than the boundary case above.
        let parent: Vec<String> = (0..60).map(|i| format!("k{i}")).collect();
        let child: Vec<String> = (0..42).map(|i| format!("m{i}")).collect();
        let t = format!("{}:\n  {}: 0\n", parent.join("."), child.join("."));
        assert_eq!(err(&t).kind, DepthLimit);
    }
}
