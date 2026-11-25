//! Frontmatter: finding the YAML block, reading it losslessly into
//! [`serde_json::Value`], and editing it by key (ARCHITECTURE §5.5).
//!
//! # Reading
//!
//! The block is found exactly where MBR finds it ([`locate`] ports
//! `mbr_core::frontmatter::leading_metadata`, YAML style only): a `---`
//! first line, a non-blank second line, and the first later `---` or `...`
//! line. It is parsed with `yaml-rust2`, the crate MBR uses, so a note reads
//! the same in both. The conversion keeps the structure MBR's lossy
//! `SimpleMetadata` flattens (nested maps, lists of maps) and matches MBR's
//! lossless converter value for value: integers are numbers, floats are
//! `f64` numbers (only non-finite ones stay text), dates and `no` are strings
//! (YAML 1.2 core schema), and aliases are resolved. Money never comes from
//! frontmatter (§3.2), so no amount is ever read through a float.
//!
//! # Editing
//!
//! Rust has no formatting-preserving YAML library, so [`edit`] works on the
//! top-level `key: value` lines themselves:
//!
//! - the block is cut into one *entry* per top-level key: its key line plus
//!   the indented (or `- `) lines under it. Column-0 comments and trailing
//!   blank lines belong to no entry and are never touched;
//! - the scanned keys must equal, in order, the keys the YAML parser reads.
//!   Anything the scanner doesn't understand (a flow mapping at the top, a
//!   `? ` complex key, a column-0 comment inside a list) makes the edit refuse
//!   rather than guess;
//! - a set rewrites only that entry's lines; a new key goes after the last
//!   entry; a remove deletes the entry's lines. A single-line value keeps its
//!   trailing `# comment` and its quoting style;
//! - the result is re-parsed: the edited key must read back as the value
//!   asked for, every other key exactly as before, in the same order, and the
//!   body must be byte-identical. Otherwise nothing is returned but an error.
//!
//! Everything outside the edited entry is byte-identical by construction (one
//! splice), which the proptests check too.

use serde_json::{Map, Value};
use std::collections::HashMap;
use std::ops::Range;
use thiserror::Error;
use yaml_rust2::parser::{Event, Parser};
use yaml_rust2::{Yaml, YamlLoader};

/// A note's frontmatter fields. Look up key order in [`Frontmatter::order`].
pub type Fields = Map<String, Value>;

const BOM: &str = "\u{feff}";

/// Where the frontmatter block is in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// From the opening `---` to the end of the closing line, its terminator
    /// excluded.
    pub block: Range<usize>,
    /// The lines between the delimiter lines, terminators included.
    pub content: Range<usize>,
    /// The body's first byte: just past the closing line's terminator.
    pub body_start: usize,
}

/// The frontmatter block at the start of `source`, after an optional BOM.
///
/// pulldown-cmark's (and so MBR's) rules: the first line is exactly `---`
/// followed only by whitespace; the next line is neither blank nor a closing
/// line; the block ends at the first later line that is exactly `---` or
/// `...` followed only by spaces. No closing line, no block.
#[must_use]
pub fn locate(source: &str) -> Option<Block> {
    let offset = if source.starts_with(BOM) {
        BOM.len()
    } else {
        0
    };
    let bytes = &source.as_bytes()[offset..];
    if run_of(bytes, b'-') != 3 {
        return None;
    }
    let content_start = next_line(bytes, 3);
    if !bytes[3..content_start].iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let first = &bytes[content_start..];
    if closing_line(first).is_some() || blank_line(first).is_some() {
        return None;
    }
    let mut line = next_line(bytes, content_start);
    while line < bytes.len() {
        if let Some(closer) = closing_line(&bytes[line..]) {
            let end = line + closer;
            return Some(Block {
                block: offset..offset + end,
                content: offset + content_start..offset + line,
                body_start: offset + end + blank_line(&bytes[end..]).unwrap_or(0),
            });
        }
        line = next_line(bytes, line);
    }
    None
}

fn run_of(bytes: &[u8], byte: u8) -> usize {
    bytes.iter().take_while(|&&b| b == byte).count()
}

fn next_line(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |at| from + at + 1)
}

fn line_end(bytes: &[u8]) -> Option<usize> {
    match bytes {
        [] => Some(0),
        [b'\r', b'\n', ..] => Some(2),
        [b'\n' | b'\r', ..] => Some(1),
        _ => None,
    }
}

fn blank_line(bytes: &[u8]) -> Option<usize> {
    let spaces = bytes
        .iter()
        .take_while(|&&b| matches!(b, b' ' | b'\t' | 0x0b | 0x0c))
        .count();
    line_end(&bytes[spaces..]).map(|eol| spaces + eol)
}

fn closing_line(bytes: &[u8]) -> Option<usize> {
    if run_of(bytes, b'-') != 3 && run_of(bytes, b'.') != 3 {
        return None;
    }
    let end = 3 + run_of(&bytes[3..], b' ');
    line_end(&bytes[end..]).map(|_| end)
}

/// 1-based line number of byte `at`.
#[must_use]
pub fn line_of(source: &str, at: usize) -> usize {
    source.as_bytes()[..at.min(source.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

/// Why frontmatter can't be read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FrontmatterError {
    /// Not YAML.
    #[error("the frontmatter is not valid YAML (line {line}): {message}")]
    Yaml {
        /// 1-based line in the document.
        line: usize,
        /// The parser's message.
        message: String,
    },
    /// YAML, but not a mapping of keys to values.
    #[error("the frontmatter (line {line}) is not a list of `key: value` lines")]
    NotAMapping {
        /// 1-based line in the document.
        line: usize,
    },
}

impl FrontmatterError {
    /// The 1-based line in the document.
    #[must_use]
    pub fn line(&self) -> usize {
        match self {
            Self::Yaml { line, .. } | Self::NotAMapping { line } => *line,
        }
    }
}

/// A document's frontmatter, read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Frontmatter {
    /// Where it is; `None` when the document has none.
    pub block: Option<Block>,
    /// The fields.
    pub fields: Fields,
    /// The top-level keys in document order.
    pub order: Vec<String>,
}

impl Frontmatter {
    /// Read `source`'s frontmatter. A document without a block has no fields.
    ///
    /// # Errors
    /// [`FrontmatterError`] when the block isn't a YAML mapping.
    pub fn parse(source: &str) -> Result<Self, FrontmatterError> {
        let Some(block) = locate(source) else {
            return Ok(Self::default());
        };
        let first_line = line_of(source, block.content.start);
        let (fields, order) = parse_yaml(&source[block.content.clone()], first_line)?;
        Ok(Self {
            block: Some(block),
            fields,
            order,
        })
    }

    /// A field.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }

    /// A field that is text.
    #[must_use]
    pub fn str(&self, key: &str) -> Option<&str> {
        self.fields.get(key).and_then(Value::as_str)
    }

    /// The first byte after the frontmatter (0 without one).
    #[must_use]
    pub fn body_start(&self) -> usize {
        self.block.as_ref().map_or(0, |block| block.body_start)
    }
}

/// Parse a block's YAML into fields plus key order. `first_line` is the
/// content's first line in the document, for error lines.
fn parse_yaml(content: &str, first_line: usize) -> Result<(Fields, Vec<String>), FrontmatterError> {
    if let Some(line) = over_expanded(content) {
        return Err(FrontmatterError::Yaml {
            line: first_line + line.saturating_sub(1),
            message: format!("its aliases expand to more than {MAX_YAML_NODES} values"),
        });
    }
    let docs = YamlLoader::load_from_str(content).map_err(|e| FrontmatterError::Yaml {
        line: first_line + e.marker().line().saturating_sub(1),
        message: e.info().to_string(),
    })?;
    match docs.into_iter().next() {
        None => Ok((Fields::new(), Vec::new())),
        Some(Yaml::Hash(hash)) => {
            let order: Vec<String> = hash.keys().map(key_text).collect();
            let fields = hash
                .iter()
                .map(|(k, v)| (key_text(k), yaml_to_json(v)))
                .collect();
            Ok((fields, order))
        }
        Some(_) => Err(FrontmatterError::NotAMapping { line: first_line }),
    }
}

/// The most values a frontmatter block may expand to, each alias counted
/// at the size of what it names. yaml-rust2 deep-copies an anchored node
/// for every alias, so a few lines of nested aliases ("billion laughs")
/// would otherwise expand to billions of values (SEC-4).
const MAX_YAML_NODES: u64 = 100_000;

/// The 1-based line (within `content`) where its expansion passes
/// [`MAX_YAML_NODES`], or `None` when it stays under. Syntax errors are left
/// for the loader to report.
fn over_expanded(content: &str) -> Option<usize> {
    // No alias, no expansion: one value per event at most.
    if !content.contains('*') {
        return None;
    }
    let mut parser = Parser::new_from_str(content);
    // Expanded sizes of the anchors seen; the open collections' anchors and
    // sizes so far; every value expanded.
    let mut anchors: HashMap<usize, u64> = HashMap::new();
    let mut open: Vec<(usize, u64)> = Vec::new();
    let mut total: u64 = 0;
    // A node is complete: remember its size under its anchor, add it to its
    // collection's.
    let finish = |anchors: &mut HashMap<usize, u64>, open: &mut Vec<(usize, u64)>, anchor, size| {
        if anchor != 0 {
            anchors.insert(anchor, size);
        }
        if let Some((_, parent)) = open.last_mut() {
            *parent = parent.saturating_add(size);
        }
    };
    loop {
        let (event, mark) = parser.next_token().ok()?;
        match event {
            Event::StreamEnd => return None,
            Event::Scalar(_, _, anchor, _) => {
                total = total.saturating_add(1);
                finish(&mut anchors, &mut open, anchor, 1);
            }
            Event::Alias(id) => {
                let size = anchors.get(&id).copied().unwrap_or(1);
                total = total.saturating_add(size);
                finish(&mut anchors, &mut open, 0, size);
            }
            Event::SequenceStart(anchor, _) | Event::MappingStart(anchor, _) => {
                total = total.saturating_add(1);
                open.push((anchor, 1));
            }
            Event::SequenceEnd | Event::MappingEnd => {
                if let Some((anchor, size)) = open.pop() {
                    finish(&mut anchors, &mut open, anchor, size);
                }
            }
            _ => {}
        }
        if total > MAX_YAML_NODES {
            return Some(mark.line());
        }
    }
}

/// A YAML key as text: strings as written, scalars as YAML would print
/// them, anything else empty (as MBR does).
fn key_text(key: &Yaml) -> String {
    match key {
        Yaml::String(s) | Yaml::Real(s) => s.clone(),
        Yaml::Integer(i) => i.to_string(),
        Yaml::Boolean(b) => b.to_string(),
        _ => String::new(),
    }
}

/// A YAML value as JSON, as MBR's lossless converter has it.
#[must_use]
pub fn yaml_to_json(value: &Yaml) -> Value {
    match value {
        Yaml::String(s) => Value::String(s.clone()),
        Yaml::Integer(i) => Value::Number((*i).into()),
        Yaml::Real(raw) => raw
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(|| Value::String(raw.clone()), Value::Number),
        Yaml::Boolean(b) => Value::Bool(*b),
        Yaml::Array(items) => Value::Array(items.iter().map(yaml_to_json).collect()),
        Yaml::Hash(hash) => Value::Object(
            hash.iter()
                .map(|(k, v)| (key_text(k), yaml_to_json(v)))
                .collect(),
        ),
        Yaml::Null | Yaml::BadValue | Yaml::Alias(_) => Value::Null,
    }
}

// ---------------------------------------------------------------------------
// The entry scanner
// ---------------------------------------------------------------------------

/// One line of the block.
struct Line<'a> {
    /// Its bytes in the content, terminator included.
    range: Range<usize>,
    /// Its text, terminator excluded.
    text: &'a str,
}

fn lines_of(content: &str) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < content.len() {
        let end = content[start..]
            .find('\n')
            .map_or(content.len(), |at| start + at + 1);
        let text = content[start..end]
            .strip_suffix('\n')
            .unwrap_or(&content[start..end]);
        let text = text.strip_suffix('\r').unwrap_or(text);
        lines.push(Line {
            range: start..end,
            text,
        });
        start = end;
    }
    lines
}

/// A top-level key's lines.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// The key, decoded.
    key: String,
    /// The key as written (up to the colon).
    raw_key: String,
    /// The value text on the key line (after the colon).
    value_on_line: String,
    /// Line indices in the block: the key line to the last line of its value.
    lines: Range<usize>,
    /// Whether its value holds a `|+` block, which owns trailing blank lines.
    keeps: bool,
}

enum LineKind {
    Blank,
    Comment,
    Continuation,
    Key { colon: usize },
}

/// Why a block's layout can't be edited safely.
type LayoutProblem = (usize, &'static str);

fn classify(text: &str) -> Result<LineKind, &'static str> {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return Ok(LineKind::Blank);
    };
    let second = chars.next();
    let spaced = |c: Option<char>| matches!(c, None | Some(' ' | '\t'));
    match first {
        _ if text.trim().is_empty() => Ok(LineKind::Blank),
        ' ' | '\t' => Ok(LineKind::Continuation),
        '#' => Ok(LineKind::Comment),
        '-' if spaced(second) => Ok(LineKind::Continuation),
        ']' | '}' => Ok(LineKind::Continuation),
        '?' if spaced(second) => Err("a `? ` complex key"),
        '[' | '{' => Err("a flow collection at the top level"),
        '%' | '&' | '*' | '!' | '|' | '>' | '@' | '`' | ',' => {
            Err("a top-level line that isn't `key: value`")
        }
        '"' | '\'' => quoted_key_end(text, first)
            .and_then(|end| colon_after(text, end))
            .map(|colon| LineKind::Key { colon })
            .ok_or("a quoted key that doesn't end in `:`"),
        _ => plain_key_colon(text)
            .map(|colon| LineKind::Key { colon })
            .ok_or("a top-level line that isn't `key: value`"),
    }
}

/// The byte just past a quoted key's closing quote.
fn quoted_key_end(text: &str, quote: char) -> Option<usize> {
    let mut iter = text.char_indices().skip(1).peekable();
    while let Some((at, c)) = iter.next() {
        match (quote, c) {
            ('"', '\\') => {
                iter.next();
            }
            ('\'', '\'') if iter.peek().is_some_and(|(_, n)| *n == '\'') => {
                iter.next();
            }
            (q, c) if q == c => return Some(at + c.len_utf8()),
            _ => {}
        }
    }
    None
}

/// The colon after a quoted key (spaces allowed between), followed by a
/// space, a tab or the end of the line.
fn colon_after(text: &str, from: usize) -> Option<usize> {
    let rest = &text[from..];
    let colon = from + rest.len() - rest.trim_start_matches([' ', '\t']).len();
    (text[colon..].starts_with(':')
        && matches!(text[colon + 1..].chars().next(), None | Some(' ' | '\t')))
    .then_some(colon)
}

/// The colon ending a plain key: the first `:` followed by a space, a tab
/// or the end of the line, with no ` #` comment before it.
fn plain_key_colon(text: &str) -> Option<usize> {
    let colon = text.char_indices().find_map(|(at, c)| {
        (c == ':' && matches!(text[at + 1..].chars().next(), None | Some(' ' | '\t'))).then_some(at)
    })?;
    let key = &text[..colon];
    (!key.contains(" #") && !key.contains("\t#")).then_some(colon)
}

/// Decode a key as written: a plain key is its trimmed text; a quoted one
/// goes through the YAML parser itself.
fn decode_key(raw: &str) -> Option<String> {
    if !raw.starts_with(['"', '\'']) {
        let key = raw.trim_end();
        return Some(key.to_string())
            .filter(|k| !matches!(k.as_str(), "~" | "null" | "Null" | "NULL"));
    }
    let line = format!("{raw}: ~\n");
    if over_expanded(&line).is_some() {
        return None;
    }
    let docs = YamlLoader::load_from_str(&line).ok()?;
    match docs.into_iter().next()? {
        Yaml::Hash(hash) => hash.keys().next().map(key_text),
        _ => None,
    }
}

/// Whether a line ends in a block-scalar indicator that keeps trailing
/// blank lines (`|+`, `>2+`, `- |+`).
fn keeps_trailing_blanks(text: &str) -> bool {
    strip_comment(text)
        .split_whitespace()
        .last()
        .is_some_and(|token| token.starts_with(['|', '>']) && token.contains('+'))
}

/// Cut a block into entries.
fn scan_entries(lines: &[Line<'_>]) -> Result<Vec<Entry>, LayoutProblem> {
    struct Open {
        entry: Entry,
        last_content: usize,
        keep: bool,
    }
    let close = |open: Open, upto: usize| -> Entry {
        let end = if open.keep {
            // A `|+` value keeps its trailing blank lines: they are its own.
            upto.max(open.last_content + 1)
        } else {
            open.last_content + 1
        };
        Entry {
            lines: open.entry.lines.start..end,
            keeps: open.keep,
            ..open.entry
        }
    };
    let mut entries = Vec::new();
    let mut open: Option<Open> = None;
    for (i, line) in lines.iter().enumerate() {
        match classify(line.text).map_err(|why| (i, why))? {
            LineKind::Blank => {}
            LineKind::Comment => {
                if let Some(o) = open.take() {
                    entries.push(close(o, i));
                }
            }
            LineKind::Continuation => match open.as_mut() {
                Some(o) => {
                    o.last_content = i;
                    o.keep |= keeps_trailing_blanks(line.text);
                }
                None => return Err((i, "an indented line before the first key")),
            },
            LineKind::Key { colon } => {
                if let Some(o) = open.take() {
                    entries.push(close(o, i));
                }
                let raw_key = line.text[..colon].to_string();
                let key = decode_key(&raw_key).ok_or((i, "a key that doesn't read"))?;
                open = Some(Open {
                    entry: Entry {
                        key,
                        raw_key,
                        value_on_line: line.text[colon + 1..].to_string(),
                        lines: i..i + 1,
                        keeps: false,
                    },
                    last_content: i,
                    keep: keeps_trailing_blanks(&line.text[colon + 1..]),
                });
            }
        }
    }
    if let Some(o) = open.take() {
        entries.push(close(o, lines.len()));
    }
    Ok(entries)
}

/// The byte where a single-line value's `# comment` starts (the whitespace
/// before the `#`), if it has one.
fn comment_start(value: &str) -> Option<usize> {
    let lead = value.len() - value.trim_start_matches([' ', '\t']).len();
    let body = &value[lead..];
    if body.starts_with('#') {
        return Some(0);
    }
    let scan_from = match body.chars().next() {
        Some(q @ ('"' | '\'')) => lead + quoted_key_end(body, q)?,
        _ => lead,
    };
    let bytes = value.as_bytes();
    (scan_from.max(1)..value.len())
        .find(|&at| bytes[at] == b'#' && matches!(bytes[at - 1], b' ' | b'\t'))
        .map(|at| value[..at].trim_end_matches([' ', '\t']).len())
}

fn strip_comment(value: &str) -> &str {
    comment_start(value).map_or(value, |at| &value[..at])
}

/// A top-level key as the editor sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyLine {
    /// The key.
    pub key: String,
    /// Its 1-based line in the document.
    pub line: usize,
    /// The value text on the key line, comment and surrounding spaces
    /// removed: `|` for a literal block, `"[[Customer X]]"` with its quotes,
    /// `0042` as written.
    pub raw_value: String,
    /// Whether the value continues on following lines.
    pub multiline: bool,
}

/// The top-level keys of `source`'s frontmatter with their lines and raw
/// text, or `None` when there is no block or its layout isn't one the
/// editor understands. Reading never depends on this; problems use it for
/// line numbers and for what the parser normalizes away (`0042`, `>`).
#[must_use]
pub fn key_lines(source: &str) -> Option<Vec<KeyLine>> {
    let block = locate(source)?;
    let content = &source[block.content.clone()];
    let first_line = line_of(source, block.content.start);
    let entries = scan_entries(&lines_of(content)).ok()?;
    Some(
        entries
            .into_iter()
            .map(|entry| KeyLine {
                line: first_line + entry.lines.start,
                raw_value: strip_comment(&entry.value_on_line).trim().to_string(),
                multiline: entry.lines.len() > 1,
                key: entry.key,
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Rendering values
// ---------------------------------------------------------------------------

/// A quoting preference for a string written on the key line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quote {
    /// Plain when that reads back as the same string, else double quotes.
    Auto,
    Double,
    Single,
}

fn needs_escape(c: char) -> bool {
    (c.is_control() && c != '\t') || matches!(c, '\u{2028}' | '\u{2029}' | '\u{feff}')
}

fn double_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if needs_escape(c) => {
                let code = u32::from(c);
                if code < 0x100 {
                    out.push_str(&format!("\\x{code:02X}"));
                } else {
                    out.push_str(&format!("\\u{code:04X}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn single_quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Whether `s` can be written unquoted and read back as the same string.
fn plain_ok(s: &str) -> bool {
    let Some(first) = s.chars().next() else {
        return false;
    };
    let second = s.chars().nth(1);
    let indicator_ok = match first {
        '-' | '?' | ':' => !matches!(second, None | Some(' ' | '\t')),
        ',' | '[' | ']' | '{' | '}' | '#' | '&' | '*' | '!' | '|' | '>' | '\'' | '"' | '%'
        | '@' | '`' => false,
        _ => true,
    };
    indicator_ok
        && s.trim() == s
        && !s.contains(": ")
        && !s.contains(" #")
        && !s.ends_with(':')
        && !s.chars().any(|c| c == '\t' || c == '\n' || needs_escape(c))
        && matches!(Yaml::from_str(s), Yaml::String(_))
        && matches!(
            YamlLoader::load_from_str(&format!("k: {s}\n")).ok().and_then(|d| d.into_iter().next()),
            Some(Yaml::Hash(h)) if h.values().next() == Some(&Yaml::String(s.to_string()))
        )
}

/// A literal block (`|`) for a multi-line string, or `None` when one can't
/// hold it exactly. Lines are indented by `indent`.
fn literal_block(s: &str, indent: usize) -> Option<String> {
    let body = s.trim_end_matches('\n');
    let trailing = s.len() - body.len();
    let first_content = body.lines().find(|line| !line.is_empty())?;
    if first_content.starts_with([' ', '\t'])
        || s.chars()
            .any(|c| c == '\r' || (c != '\n' && needs_escape(c)))
        || body
            .lines()
            .any(|line| !line.is_empty() && line.trim().is_empty())
    {
        return None;
    }
    // Keep chomping (`|+`) is never written: its value would swallow any
    // blank line that follows the entry. Such strings are double-quoted.
    let chomp = match trailing {
        0 => "-",
        1 => "",
        _ => return None,
    };
    let pad = " ".repeat(indent);
    let mut out = format!(" |{chomp}\n");
    for line in body.split('\n') {
        if line.is_empty() {
            out.push('\n');
        } else {
            out.push_str(&pad);
            out.push_str(line);
            out.push('\n');
        }
    }
    Some(out)
}

fn render_key(key: &str) -> String {
    if plain_ok(key) {
        key.to_string()
    } else {
        double_quoted(key)
    }
}

/// Write `value` after a `key:` or `-` indicator: ` text\n` for a scalar, or
/// a newline and nested lines indented by `indent`.
fn write_value(out: &mut String, value: &Value, indent: usize, quote: Quote) {
    let pad = " ".repeat(indent);
    match value {
        Value::Null => out.push('\n'),
        Value::Bool(b) => out.push_str(&format!(" {b}\n")),
        Value::Number(n) => out.push_str(&format!(" {n}\n")),
        Value::String(s) => {
            let block = s.contains('\n').then(|| literal_block(s, indent)).flatten();
            match block {
                Some(block) => out.push_str(&block),
                None => {
                    let text = match quote {
                        _ if s.contains('\n') => double_quoted(s),
                        Quote::Auto if plain_ok(s) => s.clone(),
                        Quote::Single if !s.chars().any(needs_escape) => single_quoted(s),
                        _ => double_quoted(s),
                    };
                    out.push(' ');
                    out.push_str(&text);
                    out.push('\n');
                }
            }
        }
        Value::Array(items) if items.is_empty() => out.push_str(" []\n"),
        Value::Object(map) if map.is_empty() => out.push_str(" {}\n"),
        Value::Array(items) => {
            out.push('\n');
            for item in items {
                out.push_str(&pad);
                out.push('-');
                write_item(out, item, indent + 2);
            }
        }
        Value::Object(map) => {
            out.push('\n');
            for (k, v) in map {
                out.push_str(&pad);
                out.push_str(&render_key(k));
                out.push(':');
                write_value(out, v, indent + 2, Quote::Auto);
            }
        }
    }
}

/// A sequence item: a map's first key goes on the dash line.
fn write_item(out: &mut String, item: &Value, indent: usize) {
    match item {
        Value::Object(map) if !map.is_empty() => {
            let pad = " ".repeat(indent);
            for (i, (k, v)) in map.iter().enumerate() {
                if i == 0 {
                    out.push(' ');
                } else {
                    out.push_str(&pad);
                }
                out.push_str(&render_key(k));
                out.push(':');
                write_value(out, v, indent + 2, Quote::Auto);
            }
        }
        _ => write_value(out, item, indent, Quote::Auto),
    }
}

/// One entry's lines, `\n`-terminated.
fn render_entry(raw_key: &str, value: &Value, quote: Quote, comment: Option<&str>) -> String {
    let mut out = format!("{raw_key}:");
    write_value(&mut out, value, 2, quote);
    match comment {
        Some(comment) if out.matches('\n').count() == 1 => {
            out.pop();
            out.push_str(comment);
            out.push('\n');
            out
        }
        _ => out,
    }
}

// ---------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------

/// One change to a document's frontmatter.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldEdit {
    /// Set a top-level key, adding it after the last key when it's absent.
    Set(String, Value),
    /// Remove a top-level key; nothing happens when it's absent.
    Remove(String),
}

impl FieldEdit {
    fn key(&self) -> &str {
        match self {
            Self::Set(key, _) | Self::Remove(key) => key,
        }
    }
}

/// Why an edit was refused. Nothing is written when one is returned.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EditError {
    /// The frontmatter doesn't read.
    #[error(transparent)]
    Read(#[from] FrontmatterError),
    /// The key can't be a top-level key.
    #[error("`{0}` can't be a frontmatter key")]
    BadKey(String),
    /// The block's layout isn't one the editor can change safely.
    #[error("the frontmatter can't be edited safely at line {line}: {reason}; edit it by hand")]
    Layout {
        /// 1-based line in the document.
        line: usize,
        /// What the editor didn't understand.
        reason: String,
    },
    /// Re-reading the result didn't give what was asked for.
    #[error("editing `{key}` would change more than `{key}`; nothing was written")]
    Verify {
        /// The key edited.
        key: String,
    },
}

/// Set one key.
///
/// # Errors
/// As [`edit`].
pub fn set(source: &str, key: &str, value: Value) -> Result<String, EditError> {
    edit(source, &[FieldEdit::Set(key.to_string(), value)])
}

/// Remove one key.
///
/// # Errors
/// As [`edit`].
pub fn remove(source: &str, key: &str) -> Result<String, EditError> {
    edit(source, &[FieldEdit::Remove(key.to_string())])
}

/// Apply `edits` in order, each verified by re-parsing (see the module docs).
///
/// # Errors
/// [`EditError`]; the source is never partly edited.
pub fn edit(source: &str, edits: &[FieldEdit]) -> Result<String, EditError> {
    edits
        .iter()
        .try_fold(source.to_string(), |text, change| apply(&text, change))
}

fn check_key(key: &str) -> Result<(), EditError> {
    let bad = key.is_empty()
        || key.trim() != key
        || key.contains(['\n', '\r'])
        || key.chars().any(needs_escape);
    if bad {
        Err(EditError::BadKey(key.to_string()))
    } else {
        Ok(())
    }
}

fn apply(source: &str, change: &FieldEdit) -> Result<String, EditError> {
    let key = change.key();
    check_key(key)?;
    let before = Frontmatter::parse(source)?;
    let Some(block) = before.block.clone() else {
        return create_block(source, change);
    };
    let content = &source[block.content.clone()];
    let first_line = line_of(source, block.content.start);
    let crlf = source[..block.content.start].ends_with("\r\n");
    let lines = lines_of(content);
    let entries = scan_entries(&lines).map_err(|(i, reason)| EditError::Layout {
        line: first_line + i,
        reason: reason.to_string(),
    })?;
    let scanned: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
    if scanned != before.order.iter().map(String::as_str).collect::<Vec<_>>() {
        return Err(EditError::Layout {
            line: first_line,
            reason: "its keys aren't all plain top-level `key: value` lines".to_string(),
        });
    }
    let byte_range = |lines_range: &Range<usize>| -> Range<usize> {
        let start = lines[lines_range.start].range.start;
        let end = lines[lines_range.end - 1].range.end;
        block.content.start + start..block.content.start + end
    };
    let existing = entries.iter().find(|e| e.key == key);
    let (range, text) = match (change, existing) {
        (FieldEdit::Remove(_), None) => return Ok(source.to_string()),
        (FieldEdit::Remove(_), Some(entry)) => {
            let at = entries.iter().position(|e| e.key == key).unwrap_or(0);
            let range = byte_range(&entry.lines);
            // Blank lines after the entry would join a `|+` value above it.
            let follows_keep = at > 0 && entries[at - 1].keeps;
            let end = if follows_keep {
                lines[entry.lines.end..]
                    .iter()
                    .take_while(|line| line.text.trim().is_empty())
                    .last()
                    .map_or(range.end, |line| block.content.start + line.range.end)
            } else {
                range.end
            };
            (range.start..end, String::new())
        }
        (FieldEdit::Set(_, value), Some(entry)) => {
            let single = entry.lines.len() == 1;
            let comment = single
                .then(|| comment_start(&entry.value_on_line))
                .flatten()
                .map(|at| &entry.value_on_line[at..]);
            let quote = match entry.value_on_line.trim_start().chars().next() {
                Some('"') if single => Quote::Double,
                Some('\'') if single => Quote::Single,
                _ => Quote::Auto,
            };
            (
                byte_range(&entry.lines),
                render_entry(&entry.raw_key, value, quote, comment),
            )
        }
        (FieldEdit::Set(_, value), None) => {
            let at = entries
                .last()
                .map_or(block.content.end, |last| byte_range(&last.lines).end);
            (
                at..at,
                render_entry(&render_key(key), value, Quote::Auto, None),
            )
        }
    };
    let text = if crlf {
        text.replace('\n', "\r\n")
    } else {
        text
    };
    let remaining = format!(
        "{}{text}{}",
        &source[block.content.start..range.start],
        &source[range.end..block.content.end]
    );
    let out = if remaining.trim().is_empty() {
        // Nothing left: an empty `---`/`---` pair isn't frontmatter (it would
        // read as two thematic breaks), so the block goes.
        source[block.body_start..].to_string()
    } else {
        // A block's first line may not be blank, or it stops being one.
        let kept = remaining.trim_start_matches(['\r', '\n']);
        let kept = if kept.starts_with([' ', '\t']) {
            remaining.as_str()
        } else {
            kept
        };
        format!(
            "{}{kept}{}",
            &source[..block.content.start],
            &source[block.content.end..]
        )
    };
    verify(source, &before, &out, change)?;
    Ok(out)
}

/// A new block at the top of a document that has none.
fn create_block(source: &str, change: &FieldEdit) -> Result<String, EditError> {
    let FieldEdit::Set(key, value) = change else {
        return Ok(source.to_string());
    };
    if source.trim_start_matches(BOM).starts_with("---") {
        return Err(EditError::Layout {
            line: 1,
            reason: "it starts with `---` but has no frontmatter block".to_string(),
        });
    }
    let crlf = source
        .find('\n')
        .is_some_and(|at| source[..at].ends_with('\r'));
    let entry = render_entry(&render_key(key), value, Quote::Auto, None);
    let head = format!("---\n{entry}---\n");
    let head = if crlf {
        head.replace('\n', "\r\n")
    } else {
        head
    };
    let bom = if source.starts_with(BOM) { BOM } else { "" };
    let rest = &source[bom.len()..];
    let sep = match (rest.is_empty(), crlf) {
        (true, _) => "",
        (false, true) => "\r\n",
        (false, false) => "\n",
    };
    let out = format!("{bom}{head}{sep}{rest}");
    verify(source, &Frontmatter::default(), &out, change)?;
    Ok(out)
}

fn verify(
    old_source: &str,
    before: &Frontmatter,
    new_source: &str,
    change: &FieldEdit,
) -> Result<(), EditError> {
    let key = change.key();
    let fail = || EditError::Verify {
        key: key.to_string(),
    };
    let after = Frontmatter::parse(new_source).map_err(|_| fail())?;
    let others_before: Vec<&String> = before.order.iter().filter(|k| *k != key).collect();
    let others_after: Vec<&String> = after.order.iter().filter(|k| *k != key).collect();
    let same_others = others_before == others_after
        && others_before
            .iter()
            .all(|k| before.fields.get(*k) == after.fields.get(*k));
    let edited = match change {
        FieldEdit::Set(_, value) => after.fields.get(key) == Some(value),
        FieldEdit::Remove(_) => !after.fields.contains_key(key),
    };
    let body_same = old_source[before.body_start()..] == new_source[after.body_start()..]
        || (before.block.is_none() && new_source.ends_with(old_source.trim_start_matches(BOM)));
    if same_others && edited && body_same {
        Ok(())
    } else {
        Err(fail())
    }
}

/// A whole frontmatter block (`---` … `---\n`) holding `fields` in order,
/// verified by re-parsing. For creating documents.
///
/// # Errors
/// [`EditError::BadKey`] for a key that can't be written, or
/// [`EditError::Verify`] if a value wouldn't read back.
pub fn render_block(fields: &[(&str, Value)]) -> Result<String, EditError> {
    let mut out = String::from("---\n");
    for (key, value) in fields {
        check_key(key)?;
        out.push_str(&render_entry(&render_key(key), value, Quote::Auto, None));
    }
    out.push_str("---\n");
    let parsed = Frontmatter::parse(&out).map_err(|_| EditError::Verify {
        key: fields
            .first()
            .map_or(String::new(), |(k, _)| (*k).to_string()),
    })?;
    match fields
        .iter()
        .find(|(k, v)| parsed.fields.get(*k) != Some(v))
    {
        Some((key, _)) => Err(EditError::Verify {
            key: (*key).to_string(),
        }),
        None if parsed.order.len() == fields.len() => Ok(out),
        None => Err(EditError::Verify { key: String::new() }),
    }
}

#[cfg(test)]
mod tests;
