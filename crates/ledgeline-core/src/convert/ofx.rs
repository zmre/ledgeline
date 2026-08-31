//! OFX 1.x (SGML), OFX 2.x (XML) and QFX statements, normalised to one [`Tabular`].
//!
//! Hand-rolled on purpose — see `plans/11-enhanced-import.md` § Preprocessor
//! decisions. The crate named `ofx` is the *visual effects* plugin API; `ofx-rs`
//! silently drops entity references **and the whitespace around them**, turning
//! `AT &amp;amp; T` into `ATT`, and hard-errors on the raw `&` that real bank
//! memos are full of. In an app where `NAME` is what categorisation rules match
//! on, silent mangling is the worst possible failure, so we own this.
//!
//! ## The whole parser, in one rule
//!
//! Leaf tags are **unclosed**, aggregate tags are **closed**, and OFX never has
//! mixed content. So after an open tag: if the next non-whitespace byte is `<`
//! it is an aggregate, otherwise the value runs to the next `<`. That single
//! lookahead is the entire grammar, and it reads OFX 1.x and OFX 2.x alike.
//!
//! ## Header syntax and body syntax are independent
//!
//! Some banks ship an OFX 2.x XML header wrapping an SGML unclosed-tag body.
//! The header is used **only** to pick the character decoder; the declared
//! version never selects a parser. One tolerant body scanner reads everything.
//!
//! ## What is deliberately not done
//!
//! - Dates are **not** normalised to UTC. `20260105000000.000[-4:EDT]` is the
//!   5th on the statement it came from, and shifting it by a zone whose *name*
//!   is frequently wrong lands transactions on the wrong day.
//! - `TRNAMT` is **not** parsed to a float or re-rendered. `2500.0` occurs, and
//!   what the bank wrote is what the CSV says. The only place a number is
//!   parsed is the opening/closing balance check, which reports rather than
//!   rewrites.
//! - Statement type is **not** routed on message set: Citi delivers a credit
//!   card as `BANKMSGSRSV1/STMTRS` with `ACCTTYPE=CREDITLINE`, so both `STMTRS`
//!   and `CCSTMTRS` are read wherever they appear. `INVSTMTRS` is refused by
//!   name rather than mis-parsed.

use super::encoding::{self, Decoded, Guess};
use super::{ConvertError, ConvertNote, MAX_INPUT_BYTES, SourceFormat, StatementMeta, Tabular};
use crate::decimal::{Dec, MAX_RENDER_PLACES};
use encoding_rs::{Encoding, UTF_8, WINDOWS_1252};
use std::borrow::Cow;

/// The columns every OFX conversion emits, in order.
///
/// `name` and `memo` are both present because `NAME` truncates at exactly 32
/// characters, which is why banks put the real payee in `MEMO`. Collapsing them
/// would throw away the field rules most often need.
const COLUMNS: [&str; 7] = [
    "date", "amount", "name", "memo", "trntype", "fitid", "checknum",
];

/// Column index of `amount` within [`COLUMNS`], used by the balance check.
const AMOUNT_COLUMN: usize = 1;

/// Nesting cap for the scanner. Real OFX nests about eight deep; the cap exists
/// so a hostile upload cannot build a tree deep enough to overflow the stack in
/// the recursive lookups (or in `Drop`). Past it, an open tag becomes an empty
/// leaf and its contents become siblings.
const MAX_DEPTH: usize = 64;

/// How much of the file the header scan reads. Both header dialects live in the
/// first few hundred bytes.
const HEADER_SCAN_BYTES: usize = 1024;

/// How much of the file [`looks_like_ofx`] inspects.
const SNIFF_BYTES: usize = 4096;

/// Longest entity reference we will consider, so `&` followed by a distant `;`
/// is not scanned as one. `#x10FFFF` is the longest legitimate form.
const MAX_ENTITY_LEN: usize = 12;

/// True when `bytes` look like an OFX, QFX or QBO document.
///
/// Dispatches on **content**, never on the file extension: a `.qfx` is routinely
/// plain OFX, and a `.ofx` is routinely OFX 2.x XML. Both header dialects
/// announce themselves with `OFXHEADER`, and the body always has an `<OFX>`
/// element, so either is enough.
#[must_use]
pub fn looks_like_ofx(bytes: &[u8]) -> bool {
    let prefix = &bytes[..bytes.len().min(SNIFF_BYTES)];
    let text = decode(prefix).text.to_ascii_uppercase();
    text.contains("OFXHEADER") || text.contains("<OFX>") || text.contains("<OFX ")
}

/// Parse an OFX/QFX statement into a [`Tabular`], one row per `STMTTRN`.
///
/// # Errors
/// - [`ConvertError::Empty`] when the input holds nothing but whitespace.
/// - [`ConvertError::TooLarge`] past [`MAX_INPUT_BYTES`], so a caller that
///   bypasses the HTTP layer still cannot blow the cap.
/// - [`ConvertError::InvestmentStatement`] when the file holds an `INVSTMTRS`
///   and no bank or credit-card statement — refused by name, not mis-parsed.
/// - [`ConvertError::Malformed`] when no statement can be found. The detail is
///   a fixed sanitised phrase: it never quotes a path, an offset, or user data.
pub fn parse(bytes: &[u8]) -> Result<Tabular, ConvertError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ConvertError::TooLarge {
            limit: MAX_INPUT_BYTES,
        });
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Err(ConvertError::Empty);
    }

    let decoded = decode(bytes);
    let root = scan(&decoded.text);

    let statement = match find_aggregate(&root, &["STMTRS", "CCSTMTRS"]) {
        Some(statement) => statement,
        // Only refuse when there is nothing else to offer: a brokerage file
        // pairing an investment statement with a cash sub-account still has a
        // bank statement worth importing, and the preview shows what came back.
        None if find_aggregate(&root, &["INVSTMTRS"]).is_some() => {
            return Err(ConvertError::InvestmentStatement);
        }
        None => {
            return Err(ConvertError::Malformed {
                format: SourceFormat::Ofx,
                detail: if looks_like_ofx(bytes) {
                    "no bank or credit card statement was found".to_string()
                } else {
                    "the file does not contain an OFX document".to_string()
                },
            });
        }
    };

    let rows: Vec<Vec<String>> = collect_aggregates(statement, "STMTTRN")
        .into_iter()
        .map(row)
        .collect();

    // Counted over the whole document rather than from the chosen statement, so
    // a `CCSTMTRS` sitting beside a `STMTRS` counts too — "one of each" is what a
    // card-plus-checking download looks like.
    let statements = statement_count(&root);
    let notes = decoded
        .guessed
        .map(|label| ConvertNote::EncodingGuessed { label })
        .into_iter()
        .chain((statements > 1).then_some(ConvertNote::StatementChosen { of: statements }))
        .chain(balance_note(statement, &rows))
        .collect();

    Ok(Tabular {
        header: Some(COLUMNS.iter().map(|c| (*c).to_string()).collect()),
        rows,
        truncated: false,
        statement: statement_meta(statement),
        notes,
    })
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Decode `bytes` to text, using the header — and *only* the header — to choose
/// the decoder. The declared version never selects a parser.
///
/// The pipeline is [`super::encoding::decode`]'s, shared with the delimited
/// lane so the two cannot drift: BOM first (mandatory — `chardetng` cannot
/// detect UTF-16 at all and answers `windows-1252` for the BOM'd UTF-16LE that
/// Excel's "Unicode Text" export writes), then what [`declared_encoding`] found,
/// then UTF-8 validity.
///
/// The residual is [`Guess::Assume`] rather than a detector, because OFX
/// declares its encoding in the header: reaching that case already means the
/// file omitted a field the spec requires, and what is left is a two-way choice
/// between UTF-8 and Windows-1252 that the validity test has just settled.
fn decode(bytes: &[u8]) -> Decoded {
    let header = String::from_utf8_lossy(&bytes[..bytes.len().min(HEADER_SCAN_BYTES)]);
    encoding::decode(
        bytes,
        declared_encoding(&header),
        &Guess::Assume(WINDOWS_1252),
    )
}

/// The encoding the header declares, if it declares one it can be trusted on.
///
/// Handles both dialects because the header dialect is independent of the body
/// dialect: an XML declaration may sit on top of an SGML body. Only the
/// *grammar* is OFX's; every label it extracts is resolved by
/// [`super::encoding::for_label`], which is where the Windows-1252 family is
/// collapsed — so `CHARSET:1252` means Windows-1252 and *not* ISO-8859-1, which
/// matters because the two differ precisely across 0x80-0x9F where smart quotes
/// and em-dashes live, and a bank memo is full of both.
///
/// A label no decoder recognises becomes Windows-1252 rather than nothing: an
/// OFX that names an encoding we cannot read is still overwhelmingly cp1252 in
/// practice, and refusing the file over its header would be a worse answer than
/// reading it.
fn declared_encoding(header: &str) -> Option<&'static Encoding> {
    let upper = header.to_ascii_uppercase();
    // OFX 2.x: `<?xml version="1.0" encoding="LABEL"?>`.
    if let Some(label) = xml_attribute(&upper, "ENCODING") {
        return Some(encoding::for_label(&label).unwrap_or(WINDOWS_1252));
    }
    // OFX 1.x: `ENCODING:` and `CHARSET:` header lines. `ENCODING:USASCII` is
    // decoded as cp1252 because files declaring it routinely carry high bytes
    // anyway. `CHARSET:NONE` declares nothing, so it must not consume the
    // `ENCODING` line's answer.
    match (
        header_line(&upper, "ENCODING").as_deref(),
        header_line(&upper, "CHARSET").as_deref(),
    ) {
        (Some("UTF-8" | "UTF8"), _) => Some(UTF_8),
        (_, Some(charset)) if charset != "NONE" => {
            Some(encoding::for_label(charset).unwrap_or(WINDOWS_1252))
        }
        (Some("USASCII" | "US-ASCII"), _) => Some(WINDOWS_1252),
        // `ENCODING:UNICODE` meant different things to different vendors, so it
        // declares nothing useful: fall through to validity-decides.
        _ => None,
    }
}

/// Value of an OFX 1.x `NAME:VALUE` header line.
fn header_line(header: &str, name: &str) -> Option<String> {
    header.lines().find_map(|line| {
        line.trim()
            .strip_prefix(name)?
            .strip_prefix(':')
            .map(|value| value.trim().to_string())
    })
}

/// Value of a quoted XML attribute, e.g. `encoding="UTF-8"`.
fn xml_attribute(header: &str, name: &str) -> Option<String> {
    let after = header.split_once(&format!("{name}="))?.1.trim_start();
    let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    after[quote.len_utf8()..]
        .split_once(quote)
        .map(|(value, _)| value.trim().to_string())
}

// ---------------------------------------------------------------------------
// The scanner
// ---------------------------------------------------------------------------

/// One node of the document tree. Leaves carry decoded text; aggregates carry
/// children. OFX has no mixed content, so nothing carries both.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Leaf { tag: String, value: String },
    Aggregate(Aggregate),
}

/// A closed tag and everything inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Aggregate {
    tag: String,
    children: Vec<Node>,
}

impl Aggregate {
    fn new(tag: String) -> Self {
        Self {
            tag,
            children: Vec::new(),
        }
    }
}

/// Scan `text` into a document tree rooted at an unnamed aggregate.
///
/// Total: every byte sequence produces a tree. Anything it cannot make sense of
/// is dropped rather than reported, because the meaningful failure ("there is
/// no statement in here") is decided by [`parse`] against the tree, and a byte
/// offset into user data is exactly what our errors may not carry.
fn scan(text: &str) -> Aggregate {
    // A scanner is inherently stateful. The mutation is confined to this
    // function; nothing it builds is observable until it returns.
    let mut stack: Vec<Aggregate> = vec![Aggregate::new(String::new())];
    let mut cursor = 0usize;

    while let Some(offset) = text[cursor..].find('<') {
        let open = cursor + offset;
        let rest = &text[open + 1..];

        // `<?xml?>`, `<!DOCTYPE>` and `<!-- -->` carry no statement data.
        if let Some(skipped) = skip_markup(rest) {
            cursor = open + 1 + skipped;
            continue;
        }
        // A `<` that opens nothing is a character, not a tag. Values are handled
        // below by `find_tag`; this is the same rule for the text BETWEEN
        // elements, so a stray one there cannot invent an aggregate either.
        if !opens_a_tag(rest) {
            cursor = open + 1;
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let inner = &rest[..gt];
        let after = open + 1 + gt + 1;
        cursor = after;

        if let Some(name) = inner.strip_prefix('/') {
            close(&mut stack, &tag_name(name));
            continue;
        }
        let self_closing = inner.ends_with('/');
        let tag = tag_name(inner.trim_end_matches('/'));
        if tag.is_empty() {
            continue;
        }
        if self_closing {
            push_leaf(&mut stack, tag, String::new());
            continue;
        }

        // The one rule: a TAG next means aggregate, anything else means value.
        // "a tag" rather than "a `<`" because a value may legally begin with a
        // raw one — `<MEMO>< 5 DOLLARS` is a memo, not an aggregate.
        let tail = &text[after..];
        let ahead = tail.trim_start();
        if ahead.strip_prefix('<').is_some_and(opens_a_tag) || ahead.is_empty() {
            if stack.len() <= MAX_DEPTH {
                stack.push(Aggregate::new(tag));
            } else {
                push_leaf(&mut stack, tag, String::new());
            }
            continue;
        }
        let end = find_tag(tail).unwrap_or(tail.len());
        push_leaf(
            &mut stack,
            tag,
            decode_entities(tail[..end].trim()).into_owned(),
        );
        cursor = after + end;
    }
    unwind(stack)
}

/// Whether `rest` — the text immediately after a `<` — opens a well-formed tag.
///
/// Banks write a raw `<` into a payee or a memo (`A < B REPAIRS`) rather than
/// the `&lt;` the spec asks for, and the consequence of reading one as a tag is
/// not a mangled string: the stray tag swallows the enclosing `</STMTTRN>`, the
/// next close demotes `STMTTRN` to an empty leaf, and the WHOLE TRANSACTION
/// disappears from a file that still parses and reports no note.
///
/// Two things separate a tag from text, and both are needed:
///
/// - **The name touches the `<`.** `< B REPAIRS` has a space first. A tag's name
///   never does, which is also why attributes are no obstacle — `<OFX
///   VERSION="200">` starts with a letter and its spaces come later.
/// - **It closes before anything else opens.** `<B REPAIRS</STMTTRN>` reaches
///   the `<` of the close tag before it reaches any `>`, so it is text. Without
///   this, a raw `<` followed immediately by a capital letter would still be
///   read as a tag.
///
/// What remains ambiguous is a raw `<` that is followed by a letter AND closed
/// by a `>` before the next tag — `A <B> C`. That is genuinely indistinguishable
/// from markup without knowing the OFX vocabulary, and guessing the other way
/// would break real tags, so it is left as a tag.
fn opens_a_tag(rest: &str) -> bool {
    let body = rest.strip_prefix('/').unwrap_or(rest);
    if !body.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    body.find(['<', '>'])
        .is_some_and(|at| body.as_bytes()[at] == b'>')
}

/// The offset of the next `<` that actually opens a tag, skipping any that are
/// literal text in a value.
fn find_tag(text: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(offset) = text[from..].find('<') {
        let at = from + offset;
        if opens_a_tag(&text[at + 1..]) {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// Bytes consumed by a comment, declaration or processing instruction starting
/// just after a `<`, or `None` when this is an ordinary tag.
fn skip_markup(rest: &str) -> Option<usize> {
    if let Some(body) = rest.strip_prefix("!--") {
        return Some(body.find("-->").map_or(rest.len(), |end| 3 + end + 3));
    }
    if rest.starts_with('!') {
        return Some(rest.find('>').map_or(rest.len(), |end| end + 1));
    }
    if rest.starts_with('?') {
        return Some(rest.find("?>").map_or(rest.len(), |end| end + 2));
    }
    None
}

/// Normalise a tag: first whitespace-delimited token, upper-cased.
///
/// `.` is a legal tag character — QFX's marker is `INTU.BID` — and so are `_`,
/// `-` and `:`, so nothing is stripped. Upper-casing costs nothing and makes a
/// lower-cased XML body read the same as the SGML one.
fn tag_name(raw: &str) -> String {
    raw.split_whitespace()
        .next()
        .map(str::to_ascii_uppercase)
        .unwrap_or_default()
}

/// Append a leaf to the innermost open aggregate.
fn push_leaf(stack: &mut [Aggregate], tag: String, value: String) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(Node::Leaf { tag, value });
    }
}

/// Close `tag`, unwinding anything left open above it.
///
/// A close tag that matches nothing on the stack is ignored — that is every
/// leaf's close tag in an XML body, since leaves are never pushed.
///
/// Anything popped *above* the match was never an aggregate: an empty leaf and
/// an aggregate open identically (`<MEMO>` followed by `<`), and only the
/// arrival of an ancestor's close tag tells them apart. Retroactively demoting
/// it to an empty leaf and re-parenting its children is what keeps a
/// transaction with an empty `<MEMO>` from swallowing its own `<TRNAMT>`.
fn close(stack: &mut Vec<Aggregate>, tag: &str) {
    let Some(index) = stack
        .iter()
        .rposition(|open| open.tag == tag)
        .filter(|found| *found > 0)
    else {
        return;
    };
    while stack.len() > index {
        let Some(node) = stack.pop() else { return };
        let implicit = stack.len() > index;
        let Some(parent) = stack.last_mut() else {
            return;
        };
        if implicit {
            parent.children.push(Node::Leaf {
                tag: node.tag,
                value: String::new(),
            });
            parent.children.extend(node.children);
        } else {
            parent.children.push(Node::Aggregate(node));
        }
    }
}

/// Close everything still open at end of input.
///
/// Unlike [`close`], these stay aggregates. Nothing proved they ended — the
/// file just stopped — and an interrupted download that ends mid-`STMTRS` still
/// has every transaction it managed to write. Demoting them the way `close`
/// does would flatten the statement out of existence to save a `<MEMO>` that
/// was empty at EOF anyway.
fn unwind(mut stack: Vec<Aggregate>) -> Aggregate {
    while stack.len() > 1 {
        let Some(node) = stack.pop() else { break };
        if let Some(parent) = stack.last_mut() {
            parent.children.push(Node::Aggregate(node));
        }
    }
    stack.pop().unwrap_or_else(|| Aggregate::new(String::new()))
}

// ---------------------------------------------------------------------------
// Entity references
// ---------------------------------------------------------------------------

/// Expand entity references, in **one pass**, preserving everything else byte
/// for byte.
///
/// The three properties that matter, all of them regressions in `ofx-rs`:
///
/// - surrounding whitespace survives, so `AT &amp;amp; T` is `AT & T`, never `ATT`;
/// - a raw unescaped `&` — routine in real memos — passes through literally
///   instead of erroring;
/// - one pass only, so a double-escaped `&amp;amp;quot;` yields the literal
///   text `&quot;` rather than a quote character, which is what the bank that
///   double-escaped it actually meant.
fn decode_entities(text: &str) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        let resolved = after
            .find(';')
            .filter(|end| *end <= MAX_ENTITY_LEN)
            .and_then(|end| entity(&after[..end]).map(|c| (c, end)));
        match resolved {
            Some((decoded, end)) => {
                out.push(decoded);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Resolve one entity name (the text between `&` and `;`).
fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        other => other.strip_prefix('#').and_then(numeric_entity),
    }
}

/// Resolve a numeric character reference body: `233` or `xE9`.
fn numeric_entity(digits: &str) -> Option<char> {
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) if hex.chars().all(|c| c.is_ascii_hexdigit()) && !hex.is_empty() => {
            u32::from_str_radix(hex, 16).ok()?
        }
        Some(_) => return None,
        None if digits.chars().all(|c| c.is_ascii_digit()) && !digits.is_empty() => {
            digits.parse::<u32>().ok()?
        }
        None => return None,
    };
    char::from_u32(code)
}

// ---------------------------------------------------------------------------
// Tree lookups
// ---------------------------------------------------------------------------

/// First aggregate with one of `tags`, anywhere below `root`, in document
/// order. Recursion is bounded by [`MAX_DEPTH`].
fn find_aggregate<'a>(root: &'a Aggregate, tags: &[&str]) -> Option<&'a Aggregate> {
    root.children.iter().find_map(|child| match child {
        Node::Aggregate(node) if tags.contains(&node.tag.as_str()) => Some(node),
        Node::Aggregate(node) => find_aggregate(node, tags),
        Node::Leaf { .. } => None,
    })
}

/// First **direct child** aggregate with one of `tags`.
///
/// Used wherever the spec says direct child (`BANKACCTFROM`, `LEDGERBAL`,
/// `BALLIST`), so a transfer's `BANKACCTTO` or a nested balance cannot be read
/// as the statement's own.
fn child_aggregate<'a>(parent: &'a Aggregate, tags: &[&str]) -> Option<&'a Aggregate> {
    parent.children.iter().find_map(|child| match child {
        Node::Aggregate(node) if tags.contains(&node.tag.as_str()) => Some(node),
        _ => None,
    })
}

/// Value of the first direct-child leaf named `tag`.
fn child_leaf<'a>(parent: &'a Aggregate, tag: &str) -> Option<&'a str> {
    parent.children.iter().find_map(|child| match child {
        Node::Leaf { tag: name, value } if name == tag => Some(value.as_str()),
        _ => None,
    })
}

/// Every aggregate named `tag` below `parent`, in document order, without
/// descending into one already matched.
fn collect_aggregates<'a>(parent: &'a Aggregate, tag: &str) -> Vec<&'a Aggregate> {
    parent
        .children
        .iter()
        .flat_map(|child| match child {
            Node::Aggregate(node) if node.tag == tag => vec![node],
            Node::Aggregate(node) => collect_aggregates(node, tag),
            Node::Leaf { .. } => Vec::new(),
        })
        .collect()
}

/// How many statements the document holds.
///
/// A "download all my accounts" file carries one `STMTRS` per account, and only
/// the first is read — the transactions below it belong to THAT account, and a
/// rules file names one `account1` for the whole import, so merging them would
/// post one account's transactions to another. Counting them is what turns the
/// rest from silently discarded into reported.
fn statement_count(root: &Aggregate) -> usize {
    ["STMTRS", "CCSTMTRS"]
        .iter()
        .map(|tag| collect_aggregates(root, tag).len())
        .sum()
}

// ---------------------------------------------------------------------------
// Projection to rows and statement metadata
// ---------------------------------------------------------------------------

/// One `STMTTRN` as a [`COLUMNS`]-shaped row.
fn row(txn: &Aggregate) -> Vec<String> {
    // `DTPOSTED` is the posting date and is what a ledger wants. A handful of
    // FIs omit it and send only `DTUSER` (the initiation date) or `DTAVAIL`, so
    // those are read rather than emitting a dateless row.
    let date = ["DTPOSTED", "DTUSER", "DTAVAIL"]
        .iter()
        .find_map(|tag| child_leaf(txn, tag))
        .map(|raw| iso_date(raw).unwrap_or_else(|| raw.trim().to_string()))
        .unwrap_or_default();

    // `NAME` is the plain leaf; some FIs send a structured `<PAYEE>` aggregate
    // whose own `NAME` holds the payee, and dropping to it costs one lookup.
    let name = child_leaf(txn, "NAME")
        .or_else(|| child_aggregate(txn, &["PAYEE"]).and_then(|payee| child_leaf(payee, "NAME")))
        .unwrap_or_default();

    [
        date.as_str(),
        child_leaf(txn, "TRNAMT").unwrap_or_default(),
        name,
        child_leaf(txn, "MEMO").unwrap_or_default(),
        child_leaf(txn, "TRNTYPE").unwrap_or_default(),
        child_leaf(txn, "FITID").unwrap_or_default(),
        child_leaf(txn, "CHECKNUM").unwrap_or_default(),
    ]
    .iter()
    .map(|cell| (*cell).to_string())
    .collect()
}

/// What the statement said about itself, or `None` when it volunteered nothing.
fn statement_meta(statement: &Aggregate) -> Option<StatementMeta> {
    let ledger = child_aggregate(statement, &["LEDGERBAL"]);
    let meta = StatementMeta {
        account_hint: child_aggregate(statement, &["BANKACCTFROM", "CCACCTFROM"])
            .and_then(|account| child_leaf(account, "ACCTID"))
            .map(mask_account)
            .filter(|hint| !hint.is_empty()),
        currency: child_leaf(statement, "CURDEF")
            .filter(|code| !code.is_empty())
            .map(str::to_string),
        ledger_balance: ledger
            .and_then(|balance| child_leaf(balance, "BALAMT"))
            .filter(|amount| !amount.is_empty())
            .map(str::to_string),
        balance_as_of: ledger
            .and_then(|balance| child_leaf(balance, "DTASOF"))
            .and_then(iso_date),
    };
    (meta != StatementMeta::default()).then_some(meta)
}

/// Keep only the last four characters of an account id. Everything downstream
/// sees this and never the full number.
fn mask_account(id: &str) -> String {
    let tail: Vec<char> = id.trim().chars().rev().take(4).collect();
    tail.into_iter().rev().collect()
}

/// An OFX datetime as a `YYYY-MM-DD` calendar date, or `None` if it is not one.
///
/// Accepts 8, 10, 12 or 14 digits, optional fractional seconds, and an optional
/// `[±H:TZ]` suffix whose offset may be fractional (`[+5.5:IST]`) and whose zone
/// *name* is frequently wrong. All of it after the eighth digit is ignored on
/// purpose: the date is kept in the FI's own local calendar, because shifting
/// `20120720000000.000[-4:EDT]` to UTC moves it to the 19th.
fn iso_date(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let digits = &trimmed[..trimmed
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(trimmed.len())];
    if digits.len() < 8 {
        return None;
    }
    let (year, month, day) = (&digits[..4], &digits[4..6], &digits[6..8]);
    let in_range =
        |text: &str, high: u32| text.parse::<u32>().is_ok_and(|v| (1..=high).contains(&v));
    (in_range(month, 12) && in_range(day, 31)).then(|| format!("{year}-{month}-{day}"))
}

// ---------------------------------------------------------------------------
// Arithmetic validation
// ---------------------------------------------------------------------------

/// Check `opening + Σ(amounts) == closing` when the statement gave us both ends.
///
/// A `LEDGERBAL` on its own cannot be verified — there is nothing to add it to —
/// so it is recorded in [`StatementMeta`] and left alone. When an opening
/// balance is also present the sum is checked, and a failure is **noted, not
/// raised**: the rows are still the best available reading of the file, and the
/// user is the one who can say whether the statement or the parse is wrong.
/// This is the check that would have caught the `ofx-rs` entity bug class on
/// sight.
fn balance_note(statement: &Aggregate, rows: &[Vec<String>]) -> Option<ConvertNote> {
    let closing_text = child_aggregate(statement, &["LEDGERBAL"])
        .and_then(|balance| child_leaf(balance, "BALAMT"))?;
    let opening = opening_balance(statement).and_then(amount)?;
    let closing = amount(closing_text)?;

    // Any unparseable amount abandons the check rather than reporting a
    // mismatch it cannot stand behind.
    let computed = rows.iter().try_fold(opening, |total, row| {
        total.add(amount(row.get(AMOUNT_COLUMN)?)?).ok()
    })?;

    (computed != closing).then(|| ConvertNote::BalanceMismatch {
        expected: closing_text.to_string(),
        computed: render(computed),
    })
}

/// The statement's opening balance, if a `BALLIST` entry declares one.
///
/// OFX has no dedicated opening-balance element; FIs that supply one put it in
/// `BALLIST` under a name of their own choosing, so the name is matched
/// loosely and its `VALUE` read.
fn opening_balance(statement: &Aggregate) -> Option<&str> {
    let list = child_aggregate(statement, &["BALLIST"])?;
    collect_aggregates(list, "BAL")
        .into_iter()
        .find(|entry| {
            ["NAME", "DESC"]
                .iter()
                .filter_map(|tag| child_leaf(entry, tag))
                .any(|label| {
                    let upper = label.to_ascii_uppercase();
                    ["OPEN", "BEGIN", "PRIOR", "PREVIOUS", "START"]
                        .iter()
                        .any(|hint| upper.contains(hint))
                })
        })
        .and_then(|entry| child_leaf(entry, "VALUE"))
}

/// Parse a decimal amount exactly. Never `f64`, and never written back — the
/// cell keeps whatever the bank wrote.
///
/// The decimal mark is the **last** `.` or `,` in the text, which reads
/// `1,234.56` and the European `1.234,56` alike; OFX says `.` but European FIs
/// ship `,` regardless.
fn amount(text: &str) -> Option<Dec> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mark = trimmed
        .rfind(['.', ','])
        .and_then(|at| trimmed[at..].chars().next());
    Dec::parse_with_mark(trimmed, mark).ok()
}

/// Render a [`Dec`] for a diagnostic. Presentation only — nothing compares
/// against this text.
///
/// `places` is clamped to [`MAX_RENDER_PLACES`], the same clamp `edit.rs` and
/// `assertions.rs` apply for the same reason: the padding below is
/// `"0".repeat(places)`, and `places` here comes from a `Dec` the *statement*
/// supplied. A `BALAMT` of `1e-2147483648` is thirteen bytes and asks for a
/// 2.1 GB string, so an unclamped scale makes a mailed `.ofx` a memory bomb.
/// Clamping cannot alter a real balance: an OFX amount is written to the cent,
/// and [`Dec::parse_with_mark`] caps what it stores at ten places anyway.
fn render(value: Dec) -> String {
    let sign = if value.mantissa < 0 { "-" } else { "" };
    let digits = value.mantissa.unsigned_abs().to_string();
    let places = value.places.min(MAX_RENDER_PLACES) as usize;
    if places == 0 {
        return format!("{sign}{digits}");
    }
    // One integer digit is guaranteed before the split, so the index is always
    // in range and always on a char boundary (every byte here is ASCII).
    let padded = match (places + 1).checked_sub(digits.len()) {
        Some(zeros) if zeros > 0 => "0".repeat(zeros) + &digits,
        _ => digits,
    };
    let split = padded.len() - places;
    format!("{sign}{}.{}", &padded[..split], &padded[split..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_and_aggregate_are_told_apart_by_one_lookahead() {
        let tree = scan("<A>\n<B>value\n<C>\n<D>inner\n</C>\n</A>");
        let a = find_aggregate(&tree, &["A"]).expect("A is an aggregate");
        assert_eq!(child_leaf(a, "B"), Some("value"));
        let c = child_aggregate(a, &["C"]).expect("C is an aggregate");
        assert_eq!(child_leaf(c, "D"), Some("inner"));
    }

    #[test]
    fn an_empty_leaf_does_not_swallow_its_siblings() {
        // `<MEMO>` with no value is indistinguishable from an aggregate until
        // `</STMTTRN>` arrives. Without the retroactive demotion in `close`,
        // TRNAMT would end up nested inside MEMO and the row would lose it.
        let tree = scan("<STMTTRN>\n<MEMO>\n<TRNAMT>-20.00\n</STMTTRN>");
        let txn = find_aggregate(&tree, &["STMTTRN"]).expect("transaction parses");
        assert_eq!(child_leaf(txn, "MEMO"), Some(""));
        assert_eq!(child_leaf(txn, "TRNAMT"), Some("-20.00"));
    }

    #[test]
    fn a_close_tag_for_a_leaf_is_ignored() {
        let tree = scan("<A><B>value</B><C>other</C></A>");
        let a = find_aggregate(&tree, &["A"]).expect("A is an aggregate");
        assert_eq!(child_leaf(a, "B"), Some("value"));
        assert_eq!(child_leaf(a, "C"), Some("other"));
    }

    #[test]
    fn nesting_is_capped() {
        let deep = "<A>".repeat(MAX_DEPTH * 4);
        let tree = scan(&deep);
        // Nothing panicked, and the tree cannot be deeper than the cap.
        assert!(depth(&tree) <= MAX_DEPTH + 1);
    }

    fn depth(node: &Aggregate) -> usize {
        1 + node
            .children
            .iter()
            .map(|child| match child {
                Node::Aggregate(inner) => depth(inner),
                Node::Leaf { .. } => 0,
            })
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn dates_keep_the_local_calendar_day() {
        assert_eq!(iso_date("20260105"), Some("2026-01-05".to_string()));
        assert_eq!(iso_date("2026010512"), Some("2026-01-05".to_string()));
        assert_eq!(iso_date("202601051230"), Some("2026-01-05".to_string()));
        assert_eq!(
            iso_date("20260105000000.000[-4:EDT]"),
            Some("2026-01-05".to_string())
        );
        assert_eq!(
            iso_date("20260105083000.000[+5.5:IST]"),
            Some("2026-01-05".to_string())
        );
        assert_eq!(iso_date("2026010"), None);
        assert_eq!(iso_date("20261305"), None);
        assert_eq!(iso_date("not a date"), None);
    }

    #[test]
    fn amounts_read_either_decimal_mark() {
        assert_eq!(amount("-42.17"), Some(Dec::new(-4217, 2)));
        assert_eq!(amount("2500.0"), Some(Dec::new(25000, 1)));
        assert_eq!(amount("1,234.56"), Some(Dec::new(123_456, 2)));
        assert_eq!(amount("1.234,56"), Some(Dec::new(123_456, 2)));
        assert_eq!(amount(""), None);
    }

    #[test]
    fn rendering_round_trips_the_scale() {
        assert_eq!(render(Dec::new(225_783, 2)), "2257.83");
        assert_eq!(render(Dec::new(-4217, 2)), "-42.17");
        assert_eq!(render(Dec::new(5, 3)), "0.005");
        assert_eq!(render(Dec::new(42, 0)), "42");
    }

    #[test]
    fn rendering_clamps_a_hostile_scale_instead_of_allocating_for_it() {
        // The padding below is `"0".repeat(places)`, and before the clamp
        // `places` came straight off the statement: `1e-20000000` in a `BALAMT`
        // is twelve bytes and produced a 20 MB diagnostic, `1e-2147483648` a
        // 2.1 GB one.
        //
        // Stated against `Dec::new` rather than against a fixture on purpose.
        // `Dec::parse_with_mark` now caps what it stores at ten places, so the
        // OFX lane can no longer *deliver* such a scale — but this function is
        // the last thing between a `Dec` and an allocation, and it has to be
        // total on its own terms rather than on a caller's promise.
        let rendered = render(Dec::new(1, 1_000_000));
        assert!(
            rendered.len() <= MAX_RENDER_PLACES as usize + 2,
            "a one-digit mantissa rendered {} bytes",
            rendered.len()
        );
        // Every scale a bank actually writes is far below the clamp, so no real
        // balance changes shape.
        assert_eq!(render(Dec::new(225_783, 2)), "2257.83");
    }

    #[test]
    fn account_ids_are_masked_to_four() {
        assert_eq!(mask_account("000123456789"), "6789");
        assert_eq!(mask_account("  4111111111111111 "), "1111");
        assert_eq!(mask_account("12"), "12");
        assert_eq!(mask_account(""), "");
    }
}
