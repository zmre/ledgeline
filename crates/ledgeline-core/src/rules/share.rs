//! Sharing one account's categorization between two rules files (plan 43 §3.2).
//!
//! An account that is synced *and* sometimes imported by hand needs two rules
//! files, one per CSV shape, but its `if` blocks must not be written twice. So
//! they live in a third file, `<account>.categories.rules`, which both rules
//! files `include` as their **last** line. [`plan_share`] works out that split
//! for an existing manual rules file; [`append_include`] adds the line to a sync
//! file created without one.
//!
//! # Pure text edits, and why they are safe
//!
//! Nothing here reads or writes a file, and nothing re-renders a construct. The
//! manual file's items ([`RulesDoc`]) tile it exactly, so the split is a
//! partition of that tiling: the moved items, verbatim and in order, become the
//! categories file, and every other item stays where it was. One line is then
//! appended. [`SharePlan::moved_spans`] and [`SharePlan::include_offset`] say
//! precisely what happened, so a test can undo it byte for byte.
//!
//! `rules.rs` keeps `include` keep-only for the editor; this module is the one
//! server-side path that writes one, and it only ever writes a sibling name
//! [`check_include_name`] accepts. The server also resolves that name with
//! `Discovery::resolve_new` before writing.
//!
//! # hledger facts this rests on (1.52, verified against the binary)
//!
//! - `include FILE` in a rules file works, and the included `if` blocks apply.
//! - `if %NAME PATTERN` matches only when `NAME` is a column in the **including**
//!   file's `fields` line. An assigned field (`description %name`) does not
//!   count. An unknown name is not an error; the rule silently never matches.
//!   Names are case-insensitive.
//! - `%NAME` inside an assignment value reads the CSV column only (never an
//!   hledger field), and an unknown name reads as empty, silently.
//! - A whole-record `if PATTERN` matches in any CSV shape.
//! - A conditional assignment beats a top-level one wherever either sits, and
//!   among conditionals **the last match wins**.
//!
//! # What moves
//!
//! A conditional block moves when every matcher and every `%NAME` in its values
//! names a **shareable** column — one the sync CSV has under the same name,
//! because its name was borrowed from the manual file (see [`plan_share`]) —
//! and every whole-record pattern is layout-free. Everything else stays and is
//! reported as a [`KeptBlock`].
//!
//! A block that sets the transaction `comment` stays as well
//! ([`KeepReason::SetsComment`]): on a synced row it would replace the
//! template's `comment id:%id` and lose the row's id.
//!
//! The last-match-wins rule adds one reason the contract did not list: the
//! include goes last, so a moved block ends up *after* every block that stays.
//! A block that originally came before a kept block assigning the same field
//! would therefore start beating it. Such a block stays too
//! ([`KeepReason::OrderDependent`]), and so, transitively, does anything before
//! it that the same argument reaches.

use super::generate::FeedColumns;
use super::{
    HledgerField, Item, ItemKind, MatchScope, RulesDoc, Span, field_name_text, hledger_field,
};
use std::collections::BTreeSet;
use thiserror::Error;

/// Why a sharing plan could not be made.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ShareError {
    /// The categories file name is not a plain sibling `*.rules` name.
    #[error("not a usable categories file name")]
    BadFileName,
    /// The manual file has no `fields` line this module can read, so there is
    /// no column name to borrow and no way to tell which matchers would match.
    #[error("the rules file has no fields line to share from")]
    NoFieldsLine,
    /// The manual file already includes this categories file.
    #[error("the rules file already includes {0}")]
    AlreadyShared(String),
    /// The split did not re-read as the same constructs. Never expected; a
    /// guard so a parser disagreement refuses rather than writes.
    #[error("the rules file could not be split safely")]
    Unsafe,
}

/// Why a conditional block stays in the manual file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeepReason {
    /// It matches on, or interpolates, a column the sync CSV doesn't have
    /// under that name (`trntype`, `checknum`, a numbered `%3`). The name is as
    /// written in the block.
    FieldNotInSync(String),
    /// A pattern that depends on the file's layout: a whole-record pattern
    /// anchored with `^`/`$` or containing a `,` or a `"`, or a pattern on the
    /// date column when the manual file's dates aren't ISO (the sync CSV's are).
    LayoutDependentPattern,
    /// An `if` table, or a block `rules.rs` keeps opaque.
    Opaque,
    /// It comes before a block that stays and assigns the same field (named
    /// here). The include goes last, so moving it would make it win where it
    /// used to lose.
    OrderDependent(String),
    /// It assigns the transaction `comment` (not a posting's `comment1`…).
    /// Sync writes each row's id there (`comment id:%id`), and a conditional
    /// assignment beats that top-level one, so on a synced row this block
    /// would erase the id and the row would be imported again on every sync.
    SetsComment,
}

impl std::fmt::Display for KeepReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FieldNotInSync(name) => {
                write!(f, "Uses the {name} column, which bank sync doesn't have.")
            }
            Self::LayoutDependentPattern => f.write_str(
                "Its pattern depends on how this bank's file writes a row or a date, and sync's CSV writes them differently.",
            ),
            Self::Opaque => f.write_str("A table or an advanced rule Ledgeline can't move safely."),
            Self::OrderDependent(field) => write!(
                f,
                "A later rule that stays also sets {field}, and moving this one would make it win."
            ),
            Self::SetsComment => {
                f.write_str("Sets the transaction comment, which sync uses for its id.")
            }
        }
    }
}

/// One conditional block that stays in the manual file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptBlock {
    /// The block's first line (its `if` line), for a list.
    pub header: String,
    /// Why it stays.
    pub reason: KeepReason,
}

/// The result of [`plan_share`]: two new texts and what went where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharePlan {
    /// The manual file without the moved blocks, plus `include <categories>` as
    /// its last line.
    pub manual_new_text: String,
    /// A header comment, then the moved blocks: verbatim bytes, original order.
    pub categories_text: String,
    /// How many blocks moved.
    pub moved: usize,
    /// The conditional blocks that stayed, in file order, with the reason.
    pub kept: Vec<KeptBlock>,
    /// The sync CSV's columns, with names borrowed from the manual file.
    pub columns: FeedColumns,
    /// Where each moved item sat in the original text, in order. Inserting
    /// those bytes back at these offsets into `manual_new_text[..include_offset]`
    /// reproduces the original exactly.
    pub moved_spans: Vec<Span>,
    /// Where the appended include (and the blank line before it) starts in
    /// `manual_new_text`.
    pub include_offset: usize,
}

/// The comment heading every categories file.
const CATEGORIES_HEADER: [&str; 3] = [
    "# ledgeline: categorization rules shared by this account's rules files.",
    "# Each of them ends with an include of this file, so a rule here works for",
    "# manual imports and bank sync alike. The last matching rule wins.",
];

/// Whether `name` is a categories file name this module will write into an
/// `include` line: a plain sibling `*.rules` file name.
///
/// No path separators, no whitespace or control characters (one would start a
/// second rule), no glob characters, no leading `.`, `-` or `~`. The server
/// additionally builds the name itself and resolves it with
/// `Discovery::resolve_new`; this is the second, local copy of the question.
///
/// # Errors
/// [`ShareError::BadFileName`] when the name is not acceptable.
pub fn check_include_name(name: &str) -> Result<(), ShareError> {
    let forbidden = |c: char| {
        c.is_whitespace()
            || c.is_control()
            || matches!(c, '/' | '\\' | ':' | '*' | '?' | '[' | ']' | '{' | '}')
    };
    let ok = name.len() <= 255
        && name.ends_with(".rules")
        && name.len() > ".rules".len()
        && !name.starts_with(['.', '-', '~'])
        && !name.chars().any(forbidden);
    if ok {
        Ok(())
    } else {
        Err(ShareError::BadFileName)
    }
}

/// Add `include <file_name>` as the last line of `rules_text`, unless the file
/// already includes it, in which case the text comes back unchanged.
///
/// A blank line goes before the include when the file doesn't already end with
/// one. That keeps it readable, and it is required after an `if` table, whose
/// extent runs to the first empty line.
///
/// # Errors
/// [`ShareError::BadFileName`] when [`check_include_name`] refuses the name.
pub fn append_include(rules_text: &str, file_name: &str) -> Result<String, ShareError> {
    check_include_name(file_name)?;
    let doc = RulesDoc::parse(rules_text);
    if includes(&doc, file_name) {
        return Ok(rules_text.to_string());
    }
    Ok(with_include(rules_text, file_name, doc.newline().as_str()).0)
}

/// Split `manual_text` into a manual file that includes `categories_file_name`
/// and the categories file itself.
///
/// # Borrowing names
///
/// The sync CSV's columns are named after the manual file's, so a moved
/// `%name` matcher still finds its column:
///
/// - **payee** — the column the manual file feeds into `description`, through a
///   top-level `description %X`, or a column literally named `description`
///   when nothing assigns `description`.
/// - **date**, **amount** — `date %X` / `amount %X`, or a column so named.
/// - **memo** — a column named `memo`.
/// - **description** — `description` when the manual file has a column of that
///   name that isn't the payee source, else `sfdescription`.
/// - **id**, **transacted**, **mcc**, **mcc_name**, **extra**, and any role
///   above with nothing to borrow keep their default names, prefixed `sf`
///   while that collides with a manual column or another sync column.
///
/// A borrowed name is shareable: matchers on it move. A name hledger itself
/// would read as a different field (`description %code`) is not borrowed.
///
/// # Errors
/// - [`ShareError::BadFileName`] — see [`check_include_name`].
/// - [`ShareError::NoFieldsLine`] — nothing to borrow names from.
/// - [`ShareError::AlreadyShared`] — the file already includes that name.
/// - [`ShareError::Unsafe`] — the split did not re-read as the same constructs.
pub fn plan_share(manual_text: &str, categories_file_name: &str) -> Result<SharePlan, ShareError> {
    check_include_name(categories_file_name)?;
    let doc = RulesDoc::parse(manual_text);
    if includes(&doc, categories_file_name) {
        return Err(ShareError::AlreadyShared(categories_file_name.to_string()));
    }
    let manual_fields = doc
        .settings()
        .fields
        .map(|setting| setting.value)
        .ok_or(ShareError::NoFieldsLine)?;
    let borrowed = borrow_names(&doc, &manual_fields);
    let dispositions = decide(&doc, &borrowed);

    let text = doc.text();
    let nl = doc.newline().as_str();
    let moved_items = || {
        doc.items()
            .iter()
            .zip(&dispositions)
            .filter(|(_, d)| matches!(d, Disposition::Move))
            .map(|(item, _)| item)
    };
    let remaining: String = doc
        .items()
        .iter()
        .zip(&dispositions)
        .filter(|(_, d)| !matches!(d, Disposition::Move))
        .map(|(item, _)| &text[item.span.clone()])
        .collect();
    let (manual_new_text, include_offset) = with_include(&remaining, categories_file_name, nl);
    let categories_text = categories_text(moved_items().map(|item| &text[item.span.clone()]), nl);

    let plan = SharePlan {
        manual_new_text,
        categories_text,
        moved: moved_items().count(),
        kept: doc
            .items()
            .iter()
            .zip(&dispositions)
            .filter_map(|(item, d)| match d {
                Disposition::Keep(reason) => Some(KeptBlock {
                    header: first_line(&text[item.body.clone()]),
                    reason: reason.clone(),
                }),
                _ => None,
            })
            .collect(),
        columns: borrowed.columns,
        moved_spans: moved_items().map(|item| item.span.clone()).collect(),
        include_offset,
    };
    verify(&doc, &plan, categories_file_name)?;
    Ok(plan)
}

// ---------------------------------------------------------------------------
// Borrowing names
// ---------------------------------------------------------------------------

/// The sync columns and the manual column names they share.
struct Borrowed {
    columns: FeedColumns,
    /// Lowercased manual column names a moved block may refer to.
    shareable: BTreeSet<String>,
    /// Lowercased borrowed names whose *values* are spelled differently in the
    /// sync CSV: the date column, when the manual file's dates aren't ISO. A
    /// value may still interpolate one; a matcher may not test one.
    layout_only: BTreeSet<String>,
}

/// The only date format a feed CSV is written in. A pattern on a date column
/// is portable between the two files only when both spell dates this way.
const SYNC_DATE_FORMAT: &str = "%Y-%m-%d";

/// See [`plan_share`] for the rules.
fn borrow_names(doc: &RulesDoc, manual_fields: &[String]) -> Borrowed {
    let lower = |name: &str| name.to_ascii_lowercase();
    let manual: Vec<&String> = manual_fields.iter().filter(|n| !n.is_empty()).collect();
    let column = |wanted: &str| {
        manual
            .iter()
            .find(|name| lower(name) == wanted)
            .map(|name| (*name).clone())
    };
    // A top-level `FIELD %X` assignment: `Some(Some(X))` when X is a manual
    // column, `Some(None)` when the field is assigned anything else.
    let source_of = |field: &str| -> Option<Option<String>> {
        let wanted = hledger_field(field)?;
        doc.items()
            .iter()
            .rev()
            .find_map(|item| match &item.kind {
                ItemKind::Assignment(a) if a.field == wanted => {
                    Some(single_ref(&doc.text()[a.value_span.clone()]))
                }
                _ => None,
            })
            .map(|reference| reference.and_then(|name| column(&lower(&name))))
    };
    // A name hledger would read as one of its own fields gets that field
    // assigned by itself, so it is borrowed only for the role it already plays.
    let fits =
        |name: &String, role: &str| hledger_field(&lower(name)).is_none() || lower(name) == role;
    let role = |field: &str| {
        match source_of(field) {
            Some(source) => source,
            None => column(field),
        }
        .filter(|name| fits(name, field))
    };

    let payee = role("description");
    let date = role("date");
    let amount = role("amount");
    let description =
        column("description").filter(|_| payee.as_deref().map(lower) != Some("description".into()));
    let memo = column("memo");

    let mut taken: BTreeSet<String> = manual.iter().map(|name| lower(name)).collect();
    let mut shareable = BTreeSet::new();
    let mut pick = |borrowed: Option<String>, default: &str| match borrowed {
        Some(name) if shareable.insert(lower(&name)) => name,
        _ => {
            let mut name = default.to_string();
            while taken.contains(&lower(&name)) {
                name.insert_str(0, "sf");
            }
            taken.insert(lower(&name));
            name
        }
    };
    let columns = FeedColumns {
        payee: pick(payee, "payee"),
        date: pick(date, "date"),
        amount: pick(amount, "amount"),
        description: pick(description, "sfdescription"),
        memo: pick(memo, "memo"),
        id: pick(None, "id"),
        transacted: pick(None, "transacted"),
        mcc: pick(None, "mcc"),
        mcc_name: pick(None, "mcc_name"),
        extra: pick(None, "extra"),
    };
    let iso = doc
        .settings()
        .date_format
        .is_some_and(|setting| setting.value == SYNC_DATE_FORMAT);
    let layout_only: BTreeSet<String> = Some(lower(&columns.date))
        .filter(|date| !iso && shareable.contains(date))
        .into_iter()
        .collect();
    let shareable = shareable.difference(&layout_only).cloned().collect();
    Borrowed {
        columns,
        shareable,
        layout_only,
    }
}

/// `X` when `value` is exactly one `%X` reference, else `None`.
fn single_ref(value: &str) -> Option<String> {
    let name = value.trim().strip_prefix('%')?;
    (!name.is_empty() && name.chars().all(is_ref_char)).then(|| name.to_string())
}

/// hledger's field-reference characters (`%memo-x`, `%memo_y`, verified).
fn is_ref_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

/// Every `%NAME` reference in an assignment value, as written.
fn value_refs(value: &str) -> Vec<&str> {
    value
        .split('%')
        .skip(1)
        .map(|rest| {
            let end = rest
                .char_indices()
                .find(|&(_, c)| !is_ref_char(c))
                .map_or(rest.len(), |(at, _)| at);
            &rest[..end]
        })
        .filter(|name| !name.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// Deciding what moves
// ---------------------------------------------------------------------------

/// What happens to one item.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Disposition {
    /// A conditional block that moves to the categories file.
    Move,
    /// A conditional block that stays, reported to the user.
    Keep(KeepReason),
    /// Anything else. It stays, and isn't a block worth listing.
    Stay,
}

/// The fields a conditional may assign, for the ordering check.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Assigns {
    /// Unknown, so treated as everything: an opaque block we couldn't read, or
    /// an `include` of some other file.
    Anything,
    /// These fields, by [`field_family`].
    Fields(BTreeSet<String>),
}

impl Assigns {
    /// The first of `fields` this may also assign.
    fn overlap<'a>(&'a self, fields: &'a BTreeSet<String>) -> Option<&'a str> {
        match self {
            Self::Anything => fields.iter().next().map(String::as_str),
            Self::Fields(mine) => mine.intersection(fields).next().map(String::as_str),
        }
    }
}

/// Per item, first on its own merits, then the ordering pass.
fn decide(doc: &RulesDoc, borrowed: &Borrowed) -> Vec<Disposition> {
    let first = doc
        .items()
        .iter()
        .map(|item| own_merits(doc, item, borrowed))
        .collect::<Vec<_>>();
    // Walk backwards so every later block's fate is already known. `later`
    // collects what the blocks that stay after this point may assign.
    let mut later: Vec<Assigns> = Vec::new();
    let mut result: Vec<Disposition> = first
        .into_iter()
        .rev()
        .map(|(disposition, assigns)| {
            let disposition = match (disposition, &assigns) {
                (Disposition::Move, Some(Assigns::Fields(mine))) => later
                    .iter()
                    .find_map(|kept| kept.overlap(mine))
                    .map_or(Disposition::Move, |field| {
                        Disposition::Keep(KeepReason::OrderDependent(field.to_string()))
                    }),
                (other, _) => other,
            };
            if !matches!(disposition, Disposition::Move) {
                later.extend(assigns);
            }
            disposition
        })
        .collect();
    result.reverse();
    result
}

/// One item's disposition before ordering, and what it may assign (`None` for
/// an item that is not a conditional and never affects one).
fn own_merits(doc: &RulesDoc, item: &Item, borrowed: &Borrowed) -> (Disposition, Option<Assigns>) {
    let text = doc.text();
    let body = &text[item.body.clone()];
    match &item.kind {
        ItemKind::Include(_) => (Disposition::Stay, Some(Assigns::Anything)),
        ItemKind::IfBlock(block) => {
            let assigns = block
                .assignments
                .iter()
                .map(|a| field_family(&field_name_text(a.field)))
                .chain(
                    block
                        .control
                        .iter()
                        .map(|c| field_name_text(HledgerField::Control(c.kind))),
                )
                .collect();
            let shareable_name =
                |name: &str| borrowed.shareable.contains(&name.to_ascii_lowercase());
            let layout_only =
                |name: &str| borrowed.layout_only.contains(&name.to_ascii_lowercase());
            let matcher_problem = block
                .groups
                .iter()
                .flat_map(|group| &group.matchers)
                .find_map(|matcher| match &matcher.scope {
                    MatchScope::WholeRecord => matcher
                        .pattern
                        .contains(['^', '$', ',', '"'])
                        .then_some(KeepReason::LayoutDependentPattern),
                    MatchScope::Field(name) if layout_only(name) => {
                        Some(KeepReason::LayoutDependentPattern)
                    }
                    MatchScope::Field(name) => {
                        (!shareable_name(name)).then(|| KeepReason::FieldNotInSync(name.clone()))
                    }
                });
            let value_problem = || {
                block
                    .assignments
                    .iter()
                    .flat_map(|a| value_refs(&text[a.value_span.clone()]))
                    .find(|name| !shareable_name(name) && !layout_only(name))
                    .map(|name| KeepReason::FieldNotInSync(name.to_string()))
            };
            let sets_comment = || {
                block
                    .assignments
                    .iter()
                    .any(|a| a.field == HledgerField::Comment)
                    .then_some(KeepReason::SetsComment)
            };
            // A byte-order mark leads the document and must stay there.
            let bom = || {
                text[item.span.clone()]
                    .contains('\u{feff}')
                    .then_some(KeepReason::Opaque)
            };
            let disposition = matcher_problem
                .or_else(value_problem)
                .or_else(sets_comment)
                .or_else(bom)
                .map_or(Disposition::Move, Disposition::Keep);
            (disposition, Some(Assigns::Fields(assigns)))
        }
        ItemKind::Opaque(_) if is_conditional(body) => (
            Disposition::Keep(KeepReason::Opaque),
            Some(opaque_assigns(body)),
        ),
        _ => (Disposition::Stay, None),
    }
}

/// Whether `body` opens with hledger's `if` keyword (a block or a table).
fn is_conditional(body: &str) -> bool {
    body.strip_prefix("if")
        .is_some_and(|rest| rest.chars().next().is_none_or(|c| !c.is_alphanumeric()))
}

/// What an opaque conditional may assign, read as far as it safely can.
///
/// A table names its fields in its header (`if,account2,comment`); a block's
/// are the first word of each indented line. Anything unrecognised makes the
/// answer [`Assigns::Anything`].
fn opaque_assigns(body: &str) -> Assigns {
    let mut lines = body.lines();
    let header = lines.next().unwrap_or_default();
    let rest = &header[2..];
    let names: Option<BTreeSet<String>> = match rest.chars().next() {
        Some(sep) if !sep.is_whitespace() => rest
            .split(sep)
            .skip(1)
            .map(|name| known_family(name.trim()))
            .collect(),
        _ => lines
            .filter(|line| line.starts_with([' ', '\t']) && !line.trim().is_empty())
            .map(|line| {
                let word = line
                    .trim_start()
                    .split([' ', '\t', ':'])
                    .next()
                    .unwrap_or_default();
                known_family(word)
            })
            .collect(),
    };
    names.map_or(Assigns::Anything, Assigns::Fields)
}

/// [`field_family`] of `name` if hledger reads it as a field.
fn known_family(name: &str) -> Option<String> {
    hledger_field(name).map(|field| field_family(&field_name_text(field)))
}

/// Fields that compete for the same output share a family: every spelling of
/// an amount sets the first posting's amount, so they all count as `amount`.
fn field_family(name: &str) -> String {
    if name.starts_with("amount") {
        "amount".to_string()
    } else {
        name.to_string()
    }
}

// ---------------------------------------------------------------------------
// Building and checking the texts
// ---------------------------------------------------------------------------

/// Whether `doc` already has an `include` of `name`.
fn includes(doc: &RulesDoc, name: &str) -> bool {
    doc.items()
        .iter()
        .any(|item| matches!(&item.kind, ItemKind::Include(inc) if inc.target.trim() == name))
}

/// `text` plus a final `include` line, and the offset where the addition starts.
fn with_include(text: &str, name: &str, nl: &str) -> (String, usize) {
    // How many line breaks the text still needs before the include: none when
    // it is empty or already ends with a blank line, one to leave a blank line
    // after a terminated last line, two when the last line has no terminator.
    let breaks = if text.is_empty() || text == nl || text.ends_with(&nl.repeat(2)) {
        0
    } else if text.ends_with('\n') {
        1
    } else {
        2
    };
    (
        format!("{text}{}include {name}{nl}", nl.repeat(breaks)),
        text.len(),
    )
}

/// The categories file: the header, a blank line, then the moved items, each
/// ending with a line break.
fn categories_text<'a>(moved: impl Iterator<Item = &'a str>, nl: &str) -> String {
    let header: String = CATEGORIES_HEADER
        .iter()
        .map(|line| format!("{line}{nl}"))
        .collect();
    moved.fold(format!("{header}{nl}"), |mut out, item| {
        out.push_str(item);
        if !out.ends_with('\n') {
            out.push_str(nl);
        }
        out
    })
}

/// The first line of `body`, without its terminator.
fn first_line(body: &str) -> String {
    body.lines()
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_string()
}

/// Re-read both new texts and check they say what the split meant: the manual
/// file has every construct that stayed plus the include, and the categories
/// file has exactly the moved blocks, each still an `if` block.
fn verify(doc: &RulesDoc, plan: &SharePlan, name: &str) -> Result<(), ShareError> {
    let bodies = |doc: &RulesDoc| -> Vec<String> {
        doc.items()
            .iter()
            .filter(|item| !matches!(item.kind, ItemKind::Trivia))
            .map(|item| doc.text()[item.body.clone()].trim_end().to_string())
            .collect()
    };
    let moved: BTreeSet<usize> = plan.moved_spans.iter().map(|span| span.start).collect();
    let (moved_bodies, mut kept_bodies): (Vec<String>, Vec<String>) = doc
        .items()
        .iter()
        .filter(|item| !matches!(item.kind, ItemKind::Trivia))
        .map(|item| {
            (
                moved.contains(&item.span.start),
                doc.text()[item.body.clone()].trim_end().to_string(),
            )
        })
        .fold(
            (Vec::new(), Vec::new()),
            |(mut m, mut k), (is_moved, body)| {
                if is_moved {
                    m.push(body)
                } else {
                    k.push(body)
                }
                (m, k)
            },
        );
    kept_bodies.push(format!("include {name}"));

    let manual = RulesDoc::parse(&plan.manual_new_text);
    let categories = RulesDoc::parse(&plan.categories_text);
    let all_blocks = categories
        .items()
        .iter()
        .filter(|item| !matches!(item.kind, ItemKind::Trivia))
        .all(|item| item.if_block().is_some());
    if bodies(&manual) == kept_bodies && bodies(&categories) == moved_bodies && all_blocks {
        Ok(())
    } else {
        Err(ShareError::Unsafe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_refs_read_hledger_s_name_characters() {
        assert_eq!(
            value_refs("a:%memo-x b:%memo_y c:%name."),
            ["memo-x", "memo_y", "name"]
        );
        assert_eq!(value_refs("100% sure %"), Vec::<&str>::new());
    }

    #[test]
    fn a_single_reference_is_only_a_bare_percent_name() {
        assert_eq!(single_ref(" %name "), Some("name".to_string()));
        assert_eq!(single_ref("%name %memo"), None);
        assert_eq!(single_ref("PAYEE"), None);
    }

    #[test]
    fn include_names_are_plain_sibling_rules_files() {
        assert!(check_include_name("chase.categories.rules").is_ok());
        for bad in [
            "",
            ".rules",
            "../x.rules",
            "dir/x.rules",
            "x.rules\ninclude y.rules",
            "x y.rules",
            ".hidden.rules",
            "*.rules",
            "x.csv",
            "~x.rules",
        ] {
            assert_eq!(
                check_include_name(bad),
                Err(ShareError::BadFileName),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_table_s_header_names_what_it_assigns() {
        let expected = Assigns::Fields(["account2".to_string(), "comment".to_string()].into());
        assert_eq!(opaque_assigns("if,account2,comment\nX,a,b\n"), expected);
        assert_eq!(opaque_assigns("if|account2|comment\nX|a|b\n"), expected);
        assert_eq!(opaque_assigns("if,bogus\nX,a\n"), Assigns::Anything);
        assert_eq!(
            opaque_assigns("if ! X\n  amount-out %x\n"),
            Assigns::Fields(["amount".to_string()].into())
        );
    }

    #[test]
    fn appending_puts_a_blank_line_before_the_include() {
        assert_eq!(
            with_include("a\n", "c.rules", "\n").0,
            "a\n\ninclude c.rules\n"
        );
        assert_eq!(
            with_include("a\n\n", "c.rules", "\n").0,
            "a\n\ninclude c.rules\n"
        );
        assert_eq!(
            with_include("a", "c.rules", "\n").0,
            "a\n\ninclude c.rules\n"
        );
        assert_eq!(with_include("", "c.rules", "\n").0, "include c.rules\n");
        assert_eq!(
            with_include("a\r\n", "c.rules", "\r\n").0,
            "a\r\n\r\ninclude c.rules\r\n"
        );
    }
}
