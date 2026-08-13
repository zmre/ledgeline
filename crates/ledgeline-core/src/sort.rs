//! A **format-preserving date sort** of one journal file (WP-11).
//!
//! `hledger import` always *appends* to the end of its target file, so a
//! statement containing back-dated rows leaves the journal out of date order and
//! `hledger check ordereddates` starts failing. This module puts it back in
//! order without touching a byte it was not asked to move.
//!
//! # Why not `hledger print`
//!
//! `hledger print` sorts by date, and it is the obvious tool. It is also
//! **lossy**, verified against hledger 1.52: it flattens `include` directives
//! into one file and drops every `account`, `commodity`, `P` and standalone
//! comment. Round-tripping a real journal through it broke `hledger check
//! --strict`. A re-sort that silently deletes the user's account declarations is
//! not a re-sort, so the sort has to be ours.
//!
//! # The invariant that is the whole point
//!
//! **Item spans PARTITION the text**, exactly as [`crate::rules`] does for a
//! rules file: `items[0].span.start == 0`, every item's `span.end` is the next
//! item's `span.start`, and the last item's `span.end` is `text.len()`. The
//! document *is* the concatenation of its items' source text, so a reorder is a
//! permutation of the parts of a partition and cannot lose, duplicate or mangle
//! a byte. CRLF, tabs, column alignment, trailing whitespace and a missing final
//! newline survive with no special cases, because nothing is ever re-rendered.
//!
//! # What moves, and what never does
//!
//! Only **whole transaction items** are reordered, and a transaction's item
//! carries its **leading comment run** — the contiguous column-1 comment lines
//! directly above it, with no blank line between — so the comment explaining a
//! transaction travels with it rather than being stranded on the next one.
//!
//! Everything else keeps the index it had. Directives, standalone comment runs
//! and anything unclassifiable stay exactly where they are, because moving them
//! changes meaning: an `account` declaration is a declaration, but a `commodity`
//! or `decimal-mark` directive changes how the *amounts below it* are read, and
//! reordering `P` price directives reorders the price history.
//!
//! # Blank runs stay put — a deliberate divergence from `rules.rs`
//!
//! [`crate::rules`] makes an item's **trailing blank run** part of the item, and
//! it has to: a conditional table's extent is terminated by a blank line, so a
//! table that shed one and was then moved would swallow its new neighbour.
//!
//! No journal construct is like that. A transaction ends at the first line that
//! is unindented *or* blank, a `comment` block ends at `end comment`, and a
//! directive ends at the first unindented line — a blank line is never the only
//! thing holding two journal constructs apart. So blank runs are items of their
//! own here, and like every other non-transaction item they never move.
//!
//! That is not a stylistic preference, it is what keeps the diff honest. An
//! import appends its rows to the end of the file, so the back-dated row is
//! typically the one with no blank line after it. Were the blank run to travel,
//! sorting that row into the middle would drag the file's spacing along behind
//! it and leave a stray blank line at the end — every byte preserved, and a
//! diff full of lines the user did not ask to change.
//!
//! # Barriers: which transactions may swap with which
//!
//! Leaving directives in place is necessary but not sufficient. Moving a
//! transaction *across* a `Y 2026` changes its date; across an `apply account`
//! it changes its accounts; across a `commodity` it changes how its amounts
//! parse; across an `include` it crosses a file that may contain any of those.
//!
//! So a **barrier** item splits the document, and transactions are sorted only
//! within the run between two barriers. The rule is stated as an allow-list and
//! not a deny-list, deliberately: everything at column 1 that this module does
//! not positively recognize as position-independent is a barrier, so a directive
//! nobody here has heard of fails safe. [`PASSABLE`] is that list, and each entry
//! carries the reason it earned a place on it.
//!
//! The cost of a barrier is only reach, never correctness: the transactions on
//! each side still sort among themselves.
//!
//! # Sorting is stable
//!
//! Same-day transactions keep their relative order. This is load-bearing rather
//! than tidy: `hledger import` deduplicates against a `.latest` state file
//! holding the newest imported date, so shuffling same-day rows changes which
//! rows a later import considers new.
//!
//! # `apply` proves itself or refuses
//!
//! [`apply`] holds to the same obligation [`crate::rules::RulesDoc::verify`]
//! does. It re-parses its own output and requires that
//!
//! 1. every item reappears at the offset the arrangement implies, with the same
//!    extent and the same shape — so an item that merged into its new neighbour
//!    is caught rather than written;
//! 2. every transaction's body is byte-identical to the one it came from (modulo
//!    the single line terminator a formerly-last item is given when it stops
//!    being last);
//! 3. dates are non-decreasing within every segment of the result.
//!
//! Anything less is refused with a [`SortError`] and nothing is written. A sort
//! that cannot prove itself must not be applied — this rewrites real books.
//!
//! # Deliberate divergences and limits
//!
//! - **A yearless date refuses the whole file.** `Y 2026` plus `01/15` is legal
//!   hledger, but a yearless date's sort key depends on which `Y` is in scope,
//!   and getting that subtly wrong reorders someone's ledger. [`plan`] returns
//!   [`SortError::UnreadableDate`] and the caller offers no sort, which is the
//!   honest answer.
//! - **A `comment` … `end comment` block is one item** and never moves. An
//!   unterminated one swallows the rest of the file exactly as hledger's does,
//!   so nothing after it is mistaken for a transaction.
//! - **A missing final newline is preserved** — unless the item that lacked it
//!   stops being last, in which case it is given one, because otherwise it would
//!   be glued onto its new successor.

use crate::rules::Newline;
use itertools::Itertools;
use std::collections::HashMap;
use std::ops::Range;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Errors produced while planning or applying a sort.
///
/// Carries no path and no journal text — only line numbers — so a caller can
/// surface any of them verbatim without disclosing file contents.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SortError {
    /// A transaction header carries a date this module will not sort on: one
    /// with no year (which depends on whichever `Y` directive is in scope), or
    /// one whose components are not three numbers.
    ///
    /// The whole file is refused rather than the one transaction pinned:
    /// silently leaving a row out of a sort the user just confirmed is worse
    /// than declining to offer the sort.
    #[error(
        "the transaction on line {line} has a date this sort cannot read; a date without a year depends on a Y directive, so nothing was reordered"
    )]
    UnreadableDate {
        /// 1-based line of the transaction header.
        line: u32,
    },
    /// [`apply`] was given a [`SortPlan`] that is not the sort of the text it
    /// was handed — the file changed after the plan was shown to the user.
    /// Nothing is written; re-run [`plan`] and confirm the new diff.
    #[error(
        "this sort was planned against different text; the file changed since the diff was shown. Nothing was written."
    )]
    StalePlan,
    /// The rearrangement would not read back as the same journal: at this line a
    /// construct and its new neighbour re-parse as **one** construct rather than
    /// two. Nothing is written.
    #[error(
        "the entry at line {line} would not be read back as an entry of its own after the sort; nothing was written"
    )]
    WouldChangeMeaning {
        /// 1-based line, in the *output*, of the construct that did not survive.
        line: u32,
    },
    /// The sorted text could not be proven to be the requested sort of the
    /// input, so nothing was written. Reaching this means a bug in this module,
    /// not a bad caller.
    #[error("the sorted journal failed its round-trip check; nothing was written")]
    RoundTripMismatch,
}

/// One transaction that would move, described for the diff a user confirms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    /// The transaction's primary date, normalized to `YYYY-MM-DD`.
    pub date: String,
    /// The transaction's description, sanitized for display. Lossy on purpose —
    /// see [`sanitize`]; the file's own bytes are never taken from here.
    pub description: String,
    /// 1-based line the transaction's header sits on **before** the sort.
    pub from_line: u32,
    /// 1-based line the transaction's header would sit on **after** the sort.
    pub to_line: u32,
}

/// The sort of one journal file: which transactions move, and to where.
///
/// A plan is a *description*, not a script. [`apply`] recomputes the sort from
/// the text it is given and refuses ([`SortError::StalePlan`]) if it does not
/// match this — so the bytes written are always the sort the user was shown, of
/// the file as it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortPlan {
    /// The transactions that change position, in output order.
    pub moves: Vec<Move>,
    /// The file is already in date order: applying this plan is the identity.
    pub unchanged: bool,
}

/// Work out how `text` sorts. Pure; no I/O.
///
/// # Errors
/// [`SortError::UnreadableDate`] if any transaction's date cannot be read
/// without knowing which `Y` directive is in scope.
pub fn plan(text: &str) -> Result<SortPlan, SortError> {
    let doc = Document::scan(text)?;
    let arrangement = doc.arrangement();
    let rendered = doc.render(&arrangement);
    let moves = moves(&rendered);
    Ok(SortPlan {
        unchanged: moves.is_empty(),
        moves,
    })
}

/// Render `text` sorted, or refuse.
///
/// The result is proven before it is returned: see the module docs on the
/// round-trip obligation. An unchanged plan returns `text` byte for byte.
///
/// # Errors
/// - [`SortError::UnreadableDate`] as [`plan`];
/// - [`SortError::StalePlan`] if `plan` is not the sort of this `text`;
/// - [`SortError::WouldChangeMeaning`] if an item would fuse with its new
///   neighbour;
/// - [`SortError::RoundTripMismatch`] if the output could not be proven to be
///   this sort of the input.
pub fn apply(text: &str, plan: &SortPlan) -> Result<String, SortError> {
    let doc = Document::scan(text)?;
    let arrangement = doc.arrangement();
    let rendered = doc.render(&arrangement);
    let moves = moves(&rendered);
    if moves != plan.moves || plan.unchanged != moves.is_empty() {
        return Err(SortError::StalePlan);
    }

    let sorted: String = rendered.iter().map(|slot| slot.text.as_str()).collect();
    if moves.is_empty() {
        // An already-sorted file is the exact identity — proven here rather
        // than assumed, so "no rewrite" is a fact about the bytes.
        return (sorted == text)
            .then(|| text.to_string())
            .ok_or(SortError::RoundTripMismatch);
    }
    verify(&rendered, &sorted, doc.newline)?;
    Ok(sorted)
}

// ---------------------------------------------------------------------------
// The document model
// ---------------------------------------------------------------------------

/// A byte range into the journal text. Every span here starts and ends on a line
/// boundary, which is necessarily a `char` boundary, so slicing can never split
/// a code point.
type Span = Range<usize>;

/// A transaction's sort key: `(year, month, day)`, compared componentwise.
///
/// Not the ISO string: `2026-1-5` and `2026-01-05` are the same day to hledger
/// and would not be to a lexical comparison.
type DateKey = (i32, u32, u32);

/// The column-1 keywords a transaction may be reordered **across**.
///
/// An allow-list, not a deny-list: anything absent is a barrier, so a directive
/// this module has never heard of fails safe. Each entry is here because its
/// meaning does not depend on where it sits relative to a transaction:
///
/// - `account`, `payee`, `tag` — declarations, gathered for the whole journal.
///   Verified against hledger 1.52: a transaction that uses an account declared
///   *below* it passes `check --strict`.
/// - `P` — a market price, keyed by its own date; the price history is a set,
///   not a sequence.
///
/// Everything else is a barrier, and the interesting ones are why the list is
/// short: `Y` sets the default year, `D` the default commodity, `decimal-mark`
/// and `commodity` how amounts parse, `apply account` and `alias` how account
/// names resolve, and `include` can contain any of those.
const PASSABLE: &[&str] = &["account", "payee", "tag", "P"];

/// The keyword opening a `comment` … `end comment` block.
const COMMENT_BLOCK: &str = "comment";

/// The line that closes one.
const END_COMMENT: &str = "end comment";

/// The longest [`Move::description`] this module emits, in `char`s.
const DESCRIPTION_MAX_CHARS: usize = 120;

/// What an item is — and, for the only kind that moves, what it sorts on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// A run of blank and/or comment lines with no body to attach to.
    Trivia,
    /// A transaction: the only kind of item that is ever reordered.
    Transaction(Txn),
    /// A construct that stays where it is but which transactions may be sorted
    /// across. See [`PASSABLE`].
    Passable,
    /// A construct that stays where it is and which no transaction may cross.
    Barrier,
}

/// A transaction header, read just far enough to sort and to describe.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Txn {
    key: DateKey,
    date: String,
    description: String,
}

/// An [`Kind`] with its payload forgotten — what the round-trip check compares.
///
/// Comparing the full `Kind` would be wrong rather than merely strict: a
/// correctly moved transaction is a different transaction from the one that used
/// to sit at that offset. The *shape* is what must not change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Trivia,
    Transaction,
    Passable,
    Barrier,
}

impl Kind {
    fn shape(&self) -> Shape {
        match self {
            Self::Trivia => Shape::Trivia,
            Self::Transaction(_) => Shape::Transaction,
            Self::Passable => Shape::Passable,
            Self::Barrier => Shape::Barrier,
        }
    }
}

/// One paragraph of a journal: the unit that can be reordered.
///
/// `span.start..body.start` is the leading comment run, so a move carries a
/// transaction's annotation with it. `span.end == body.end`: a trailing blank
/// run belongs to no construct here — see the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    /// Leading comment run + body. These tile the text.
    span: Span,
    /// The construct itself, inside `span`.
    body: Span,
    /// 1-based line of `body.start`, numbered LF-only exactly as [`str::lines`]
    /// does, matching how `parse.rs` numbers journal lines.
    line: u32,
    kind: Kind,
}

impl Item {
    fn transaction(&self) -> Option<&Txn> {
        match &self.kind {
            Kind::Transaction(txn) => Some(txn),
            _ => None,
        }
    }

    fn is_barrier(&self) -> bool {
        matches!(self.kind, Kind::Barrier)
    }
}

/// A scanned journal file: the text plus the item spans that tile it.
struct Document<'a> {
    text: &'a str,
    newline: Newline,
    items: Vec<Item>,
}

impl<'a> Document<'a> {
    /// Scan `text` into items.
    ///
    /// Unlike [`crate::rules::RulesDoc::parse`] this is *fallible*, and for one
    /// reason only: a transaction whose date cannot be read has no sort key, and
    /// a sort with a guessed key is worse than no sort.
    fn scan(text: &'a str) -> Result<Self, SortError> {
        let lines = LineIndex::new(text);
        let items = lines.paragraphs()?;
        debug_assert!(
            tiles(&items, text.len()),
            "journal item spans must partition the text"
        );
        Ok(Self {
            text,
            newline: detect_newline(text),
            items,
        })
    }

    /// The index ranges transactions may be sorted within: the runs of items
    /// between barriers. Barrier items belong to no range and so never move.
    fn segments(&self) -> Vec<Range<usize>> {
        let (mut segments, open) = self.items.iter().enumerate().fold(
            (Vec::new(), 0usize),
            |(mut segments, open), (at, item)| {
                if item.is_barrier() {
                    if open < at {
                        segments.push(open..at);
                    }
                    (segments, at + 1)
                } else {
                    (segments, open)
                }
            },
        );
        if open < self.items.len() {
            segments.push(open..self.items.len());
        }
        segments
    }

    /// The output order, as one `(source index, item)` pair per output slot.
    ///
    /// Non-transaction items keep the index they had; within each segment the
    /// transaction items are redistributed over the transaction *slots* of that
    /// segment in date order. Only the occupants of transaction slots ever
    /// change, so no item can pass a non-transaction item and the shape of every
    /// slot's neighbourhood is preserved.
    ///
    /// The in-place shuffle is the one piece of mutation here, and it is the
    /// point: sorting a sub-selection of a slice has no allocation-free
    /// functional spelling, and expressing it as one is what keeps "everything
    /// else keeps its index" true by construction rather than by care.
    fn arrangement(&self) -> Vec<(usize, &Item)> {
        let mut slots: Vec<(usize, &Item)> = self.items.iter().enumerate().collect();
        for segment in self.segments() {
            let Some(window) = slots.get_mut(segment) else {
                continue;
            };
            let sorted = window
                .iter()
                .filter(|(_, item)| item.transaction().is_some())
                .copied()
                // `Itertools::sorted_by_key` is `Vec::sort_by_key`, which is
                // STABLE — same-day transactions keep their relative order,
                // which `.latest`-based import dedup depends on.
                .sorted_by_key(|(_, item)| item.transaction().map(|txn| txn.key))
                .collect_vec();
            window
                .iter_mut()
                .filter(|(_, item)| item.transaction().is_some())
                .zip(sorted)
                .for_each(|(slot, moved)| *slot = moved);
        }
        slots
    }

    /// Emit each output slot's bytes, plus everything the round-trip check needs
    /// to locate and identify the construct inside them.
    fn render<'s>(&'s self, arrangement: &[(usize, &'s Item)]) -> Vec<Rendered<'s>> {
        let last = arrangement.len().saturating_sub(1);
        arrangement
            .iter()
            .enumerate()
            .map(|(at, &(source, item))| {
                let base = item.span.start;
                let slot = Rendered {
                    text: self.text[item.span.clone()].to_string(),
                    body: item.body.start - base..item.body.end - base,
                    source_body: &self.text[item.body.clone()],
                    source_line: item.line,
                    moved: source != at,
                    shape: item.kind.shape(),
                    txn: item.transaction().cloned(),
                };
                // Only the document's final slot may end without a terminator.
                // An item that lacked one and stops being last would otherwise
                // be glued onto its successor — a reorder that loses no byte and
                // changes the meaning of two constructs.
                if at == last {
                    slot
                } else {
                    slot.terminated(self.newline)
                }
            })
            .collect()
    }
}

/// One slot's contribution to the sorted document.
struct Rendered<'a> {
    /// The bytes this slot emits.
    text: String,
    /// Where the construct sits inside `text`.
    body: Span,
    /// The construct's bytes as they were **before** the sort.
    source_body: &'a str,
    /// 1-based line the construct sat on before the sort.
    source_line: u32,
    /// This slot holds a different item than the one that used to be here.
    moved: bool,
    /// What the construct is, for the shape check.
    shape: Shape,
    /// The transaction payload, when this slot holds one.
    txn: Option<Txn>,
}

impl Rendered<'_> {
    /// Supply the line terminator this slot's last line lacks.
    ///
    /// The body always grows with it: every item ends where its body ends
    /// ([`tiles`] requires it), because a trailing blank run belongs to no
    /// construct here, so a slot missing a terminator is missing it *from the
    /// construct*.
    fn terminated(self, newline: Newline) -> Self {
        if self.text.ends_with('\n') {
            return self;
        }
        let text = format!("{}{}", self.text, newline.as_str());
        Self {
            body: self.body.start..text.len(),
            text,
            ..self
        }
    }
}

// ---------------------------------------------------------------------------
// Moves and verification
// ---------------------------------------------------------------------------

/// The transactions whose position changed, in output order, with the line each
/// one came from and the line it lands on.
///
/// A transaction "moves" when the slot it occupies is not the slot it came from.
/// A transaction whose neighbours are unchanged but whose byte offset shifted —
/// because a longer transaction sorted above it — has not moved, and reporting
/// it would fill the user's confirmation diff with rows that are not the point.
fn moves(rendered: &[Rendered<'_>]) -> Vec<Move> {
    rendered
        .iter()
        .scan(0u32, |lines_before, slot| {
            let at = *lines_before + newlines(slot.text.get(..slot.body.start).unwrap_or("")) + 1;
            *lines_before += newlines(&slot.text);
            Some((slot, at))
        })
        .filter(|(slot, _)| slot.moved)
        .filter_map(|(slot, to_line)| {
            slot.txn.as_ref().map(|txn| Move {
                date: txn.date.clone(),
                description: txn.description.clone(),
                from_line: slot.source_line,
                to_line,
            })
        })
        .collect()
}

/// Prove `sorted` is the arrangement `rendered` describes and nothing else.
///
/// Three obligations, and none of them is "the bytes concatenate" — byte
/// preservation alone does not preserve meaning. See the module docs.
fn verify(rendered: &[Rendered<'_>], sorted: &str, newline: Newline) -> Result<(), SortError> {
    let reparsed = Document::scan(sorted).map_err(|_| SortError::RoundTripMismatch)?;
    if !tiles(&reparsed.items, sorted.len()) {
        return Err(SortError::RoundTripMismatch);
    }

    // (1) Every construct reappears where the arrangement implies, with the same
    // extent and the same shape. `Trivia` is exempt by design: a comment run
    // legitimately re-associates with whatever construct it now sits above,
    // which is leading-run assembly rather than damage.
    let seen: HashMap<usize, (usize, Shape)> = reparsed
        .items
        .iter()
        .filter(|item| item.kind.shape() != Shape::Trivia)
        .map(|item| (item.body.start, (item.body.end, item.kind.shape())))
        .collect();

    rendered
        .iter()
        .scan(0usize, |offset, slot| {
            let base = *offset;
            *offset += slot.text.len();
            Some((base, slot))
        })
        .filter(|(_, slot)| slot.shape != Shape::Trivia)
        .try_for_each(|(base, slot)| {
            let start = base + slot.body.start;
            let end = base + slot.body.end;
            match seen.get(&start) {
                Some(&(body_end, shape)) if body_end == end && shape == slot.shape => Ok(()),
                _ => Err(SortError::WouldChangeMeaning {
                    line: newlines(sorted.get(..start).unwrap_or("")) + 1,
                }),
            }
        })?;

    // (2) Every transaction's body is byte-identical to the one it came from.
    // The one permitted difference is the single terminator a formerly-last item
    // is given when it stops being last.
    let intact = rendered
        .iter()
        .filter(|slot| slot.txn.is_some())
        .all(|slot| {
            slot.text
                .get(slot.body.clone())
                .is_some_and(|after| same_body(slot.source_body, after, newline))
        });
    if !intact {
        return Err(SortError::RoundTripMismatch);
    }

    // (3) The result really is sorted: dates are non-decreasing within every
    // segment of the re-parse. Asked of the re-parse, not of the arrangement, so
    // it is a fact about the bytes rather than about our own bookkeeping.
    let ordered = reparsed.segments().into_iter().all(|segment| {
        reparsed
            .items
            .get(segment)
            .into_iter()
            .flatten()
            .filter_map(Item::transaction)
            .map(|txn| txn.key)
            .tuple_windows()
            .all(|(before, after)| before <= after)
    });
    if ordered {
        Ok(())
    } else {
        Err(SortError::RoundTripMismatch)
    }
}

/// Is `after` the same construct as `before`, allowing only the one line
/// terminator a formerly-last item is given when it stops being last?
fn same_body(before: &str, after: &str, newline: Newline) -> bool {
    before == after
        || (!before.ends_with('\n') && after.strip_suffix(newline.as_str()) == Some(before))
}

/// Do these spans partition `[0, len)`, with every body inside its own span and
/// every span ending where its body does?
///
/// The invariant the module rests on; asserted on every scan in debug builds and
/// re-checked by [`verify`] on the sorted result. The `span.end == body.end` half
/// is what makes [`Rendered::terminated`] unconditional: an item has a leading
/// comment run but never a trailing one.
fn tiles(items: &[Item], len: usize) -> bool {
    let contiguous = items
        .windows(2)
        .all(|pair| pair[0].span.end == pair[1].span.start);
    let nested = items.iter().all(|item| {
        item.span.start <= item.body.start
            && item.body.start <= item.body.end
            && item.body.end == item.span.end
    });
    let bounded = match (items.first(), items.last()) {
        (Some(first), Some(last)) => first.span.start == 0 && last.span.end == len,
        // No items is only correct for no text.
        _ => len == 0,
    };
    contiguous && nested && bounded
}

// ---------------------------------------------------------------------------
// Line scanning
// ---------------------------------------------------------------------------

/// `CrLf` if the **first** line terminator in `text` is `\r\n`, else `Lf`.
///
/// Only the first is consulted, matching `rules.rs`: a mixed file has no single
/// right answer, and picking the majority would make the choice depend on
/// content a later edit can change.
fn detect_newline(text: &str) -> Newline {
    match text.find('\n') {
        Some(index) if text.get(..index).is_some_and(|head| head.ends_with('\r')) => Newline::CrLf,
        _ => Newline::Lf,
    }
}

/// How many `\n` bytes `text` holds — the line-number arithmetic, LF-only, so it
/// agrees with [`str::lines`] and with `parse.rs`.
fn newlines(text: &str) -> u32 {
    u32::try_from(text.bytes().filter(|&byte| byte == b'\n').count()).unwrap_or(u32::MAX)
}

/// A 1-based line number from a 0-based line index, saturating: a file with more
/// than `u32::MAX` lines cannot be sorted in memory anyway, and a clipped number
/// beats a panic.
fn line_number(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

/// A body extent plus what the construct is.
struct Body {
    /// Exclusive end line index.
    end: usize,
    kind: Kind,
}

/// `text` split into lines, each span including its terminator, with the
/// line-shape predicates the extent rules are written in terms of.
struct LineIndex<'a> {
    text: &'a str,
    spans: Vec<Span>,
}

impl<'a> LineIndex<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            spans: text
                .split_inclusive('\n')
                .scan(0usize, |start, line| {
                    let span = *start..*start + line.len();
                    *start = span.end;
                    Some(span)
                })
                .collect(),
        }
    }

    fn len(&self) -> usize {
        self.spans.len()
    }

    /// The line's text with its terminator removed, on [`str::lines`] rules: a
    /// trailing `\n` goes, and a `\r` goes only because it preceded that `\n`. A
    /// lone `\r` is content, exactly as `parse.rs` sees it.
    fn content(&self, index: usize) -> &'a str {
        self.spans.get(index).map_or("", |span| {
            let raw = self.text.get(span.clone()).unwrap_or("");
            raw.strip_suffix('\n')
                .map_or(raw, |line| line.strip_suffix('\r').unwrap_or(line))
        })
    }

    /// Byte offset where line `index` starts; `text.len()` one past the end, so
    /// spans close cleanly at EOF.
    fn offset(&self, index: usize) -> usize {
        self.spans
            .get(index)
            .map_or(self.text.len(), |span| span.start)
    }

    /// `^[ \t]*$`. This is what ends a transaction body, matching `parse.rs`'s
    /// `line.trim().is_empty()` and hledger's own rule.
    fn is_blank(&self, index: usize) -> bool {
        self.content(index)
            .bytes()
            .all(|byte| byte == b' ' || byte == b'\t')
    }

    /// `^[ \t]*[;#*]`. hledger allows a comment line to be indented, and
    /// `parse.rs` checks for one before it checks for a stray indent.
    fn is_comment(&self, index: usize) -> bool {
        self.content(index)
            .trim_start_matches([' ', '\t'])
            .starts_with([';', '#', '*'])
    }

    fn is_trivia(&self, index: usize) -> bool {
        self.is_blank(index) || self.is_comment(index)
    }

    /// `^[ \t]` — a line belonging to the construct above it.
    fn is_indented(&self, index: usize) -> bool {
        self.content(index).starts_with([' ', '\t'])
    }

    /// A comment line that may be absorbed as the **leading run** of the body
    /// below it.
    ///
    /// Column 1 only, which is stricter than [`LineIndex::is_comment`] and
    /// deliberately so: an indented comment line is how hledger writes an
    /// `account` subdirective and how it continues a posting's comment. Letting
    /// one travel as a lead comment would let a move drop it into a body it does
    /// not belong to.
    fn is_lead_comment(&self, index: usize) -> bool {
        self.is_comment(index) && !self.is_indented(index)
    }

    /// Where the construct starting at `start` ends, and what it is.
    fn body_at(&self, start: usize) -> Result<Body, SortError> {
        // A stray indented line at top level: hledger rejects it and so does
        // `parse.rs`, so this module will not promise to understand it either.
        if self.is_indented(start) {
            return Ok(Body {
                end: self.indented_run_end(start),
                kind: Kind::Barrier,
            });
        }

        let content = self.content(start);
        let keyword = content.split_whitespace().next().unwrap_or("");

        // A `comment` block runs to `end comment`, or — exactly as hledger 1.52
        // does — swallows the rest of the file when it has none. Handled first so
        // nothing inside one is ever mistaken for a transaction.
        if keyword == COMMENT_BLOCK {
            let end = (start + 1..self.len())
                .find(|&i| self.content(i).trim() == END_COMMENT)
                .map_or(self.len(), |i| i + 1);
            return Ok(Body {
                end,
                kind: Kind::Passable,
            });
        }

        let end = self.indented_run_end(start);
        let kind = if content.starts_with(|c: char| c.is_ascii_digit()) {
            Kind::Transaction(read_transaction(content, line_number(start))?)
        } else if PASSABLE.contains(&keyword) {
            Kind::Passable
        } else {
            Kind::Barrier
        };
        Ok(Body { end, kind })
    }

    /// The first line at or after `start + 1` that does **not** continue the
    /// construct at `start`: hledger ends a transaction (and an `account`'s
    /// subdirectives) at the first line that is unindented or blank.
    fn indented_run_end(&self, start: usize) -> usize {
        (start + 1..self.len())
            .find(|&i| !self.is_indented(i) || self.is_blank(i))
            .unwrap_or(self.len())
    }

    /// Assemble the whole file into paragraphs. The single place the tiling
    /// invariant is established: `cursor` only ever moves forward, and every item
    /// spans exactly `[offset(cursor_before), offset(cursor_after))`.
    fn paragraphs(&self) -> Result<Vec<Item>, SortError> {
        let mut items: Vec<Item> = Vec::new();
        let mut cursor = 0usize;

        while cursor < self.len() {
            let trivia_end = (cursor..self.len())
                .find(|&i| !self.is_trivia(i))
                .unwrap_or(self.len());

            // A trivia run with no body below it is its own item.
            if trivia_end == self.len() {
                items.push(self.trivia(cursor, trivia_end));
                break;
            }

            // The leading run is the contiguous column-1 comment run directly
            // above the body: walking back stops at the first blank or indented
            // line, so a blank line is never absorbed upward and a file-header
            // comment separated by one stays an item of its own.
            let lead_start = (cursor..trivia_end)
                .rev()
                .take_while(|&i| self.is_lead_comment(i))
                .last()
                .unwrap_or(trivia_end);
            if lead_start > cursor {
                items.push(self.trivia(cursor, lead_start));
            }

            let body = self.body_at(trivia_end)?;
            items.push(Item {
                span: self.offset(lead_start)..self.offset(body.end),
                body: self.offset(trivia_end)..self.offset(body.end),
                line: line_number(trivia_end),
                kind: body.kind,
            });
            cursor = body.end;
        }

        Ok(items)
    }

    /// A [`Kind::Trivia`] item over `[start, end)`. Its body equals its span:
    /// there is no construct to point at.
    fn trivia(&self, start: usize, end: usize) -> Item {
        let span = self.offset(start)..self.offset(end);
        Item {
            body: span.clone(),
            span,
            line: line_number(start),
            kind: Kind::Trivia,
        }
    }
}

// ---------------------------------------------------------------------------
// Transaction headers
// ---------------------------------------------------------------------------

/// Read a transaction header far enough to sort it and to describe it.
fn read_transaction(content: &str, line: u32) -> Result<Txn, SortError> {
    let (token, rest) = content.split_once([' ', '\t']).unwrap_or((content, ""));
    // `DATE=DATE2` — hledger sorts on the primary date, so the secondary one is
    // read past and never consulted.
    let primary = token.split('=').next().unwrap_or(token);
    let key = date_key(primary).ok_or(SortError::UnreadableDate { line })?;
    Ok(Txn {
        key,
        date: format!("{:04}-{:02}-{:02}", key.0, key.1, key.2),
        description: sanitize(description(rest)),
    })
}

/// `YYYY-MM-DD` / `YYYY/M/D` / `YYYY.MM.DD` as `(year, month, day)`, or `None`.
///
/// A **yearless** date (`01/15`) is `None` on purpose: its year comes from
/// whichever `Y` directive is in scope, so sorting on it would be sorting on a
/// value this module has not established. Calendar validity beyond the component
/// ranges is `parse.rs`'s job — this only needs a total order over days that
/// exist.
fn date_key(token: &str) -> Option<DateKey> {
    let (year, month, day) = token.split(['-', '/', '.']).collect_tuple()?;
    let valid = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    (valid(year) && valid(month) && valid(day)).then_some(())?;
    let month = month.parse::<u32>().ok().filter(|m| (1..=12).contains(m))?;
    let day = day.parse::<u32>().ok().filter(|d| (1..=31).contains(d))?;
    Some((year.parse::<i32>().ok()?, month, day))
}

/// The description part of a transaction header: everything after the date, less
/// an optional status flag, an optional `(code)`, and any trailing `; comment`.
fn description(rest: &str) -> &str {
    let rest = rest.trim_start();
    let rest = rest
        .strip_prefix(['*', '!'])
        .map_or(rest, |after| after.trim_start());
    let rest = rest
        .strip_prefix('(')
        .and_then(|after| after.split_once(')'))
        .map_or(rest, |(_code, after)| after);
    rest.split(';').next().unwrap_or("").trim()
}

/// Make `text` safe and short enough to drop straight into a GUI.
///
/// Control characters are dropped, whitespace runs collapse to one space, and
/// the result is truncated with an ellipsis. Every one of those is lossy, which
/// is exactly why a [`Move`] is a description of an edit and never a source of
/// bytes for one — [`apply`] reads the file, not the plan.
///
/// The truncation counts `char`s, not bytes, so it can never split a code point.
fn sanitize(text: &str) -> String {
    let collapsed = text
        .split_whitespace()
        .map(|word| word.chars().filter(|c| !c.is_control()).collect::<String>())
        .filter(|word| !word.is_empty())
        .join(" ");
    if collapsed.chars().count() <= DESCRIPTION_MAX_CHARS {
        return collapsed;
    }
    collapsed
        .chars()
        .take(DESCRIPTION_MAX_CHARS.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

// ---------------------------------------------------------------------------
// Unit tests — the shapes a fixture cannot express
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<Shape> {
        Document::scan(text)
            .expect("scans")
            .items
            .iter()
            .map(|item| item.kind.shape())
            .collect()
    }

    #[test]
    fn spans_tile_every_shape_of_input() {
        for text in [
            "",
            "\n",
            "   \n\t\n",
            "; just a comment",
            "2026-01-01 a\n    x  $1\n    y\n",
            "2026-01-01 a\n    x  $1\n    y",
            "account a\n    ; type: A\n\nP 2026-01-01 A $1\n",
            "comment\nnot a transaction\n2026-01-01 nope\n",
        ] {
            let doc = Document::scan(text).expect("scans");
            assert!(tiles(&doc.items, text.len()), "must tile: {text:?}");
        }
    }

    #[test]
    fn a_comment_block_is_one_item_and_hides_its_contents() {
        let text = "comment\n2026-01-01 not a transaction\n    x  $1\nend comment\n";
        assert_eq!(kinds(text), vec![Shape::Passable]);
    }

    #[test]
    fn an_unterminated_comment_block_swallows_the_rest_of_the_file() {
        let text = "comment\n2026-01-01 not a transaction\n";
        assert_eq!(kinds(text), vec![Shape::Passable]);
    }

    #[test]
    fn declarations_are_passable_and_everything_else_is_a_barrier() {
        for (line, shape) in [
            ("account assets:bank", Shape::Passable),
            ("payee Someone", Shape::Passable),
            ("tag colour", Shape::Passable),
            ("P 2026-01-01 AAPL $1.00", Shape::Passable),
            ("commodity $1,000.00", Shape::Barrier),
            ("decimal-mark .", Shape::Barrier),
            ("D $1,000.00", Shape::Barrier),
            ("Y 2026", Shape::Barrier),
            ("include other.journal", Shape::Barrier),
            ("apply account assets", Shape::Barrier),
            ("alias a = b", Shape::Barrier),
            ("something-new-in-hledger 3", Shape::Barrier),
            ("    stray indented line", Shape::Barrier),
        ] {
            assert_eq!(kinds(&format!("{line}\n")), vec![shape], "for {line:?}");
        }
    }

    #[test]
    fn an_account_subdirective_belongs_to_its_directive() {
        // Widening: the indented line is swallowed into the directive's body, so
        // no transaction can ever be placed between the two.
        let text = "account assets:bank\n    ; type: A\n";
        let doc = Document::scan(text).expect("scans");
        assert_eq!(doc.items.len(), 1);
        assert_eq!(
            doc.items.first().map(|item| item.body.clone()),
            Some(0..text.len())
        );
    }

    #[test]
    fn a_yearless_date_refuses_the_file() {
        let text = "Y 2026\n\n01/15 Coffee\n    a  $1\n    b\n";
        assert_eq!(plan(text), Err(SortError::UnreadableDate { line: 3 }));
    }

    #[test]
    fn date_separators_and_unpadded_components_all_read() {
        for token in [
            "2026-01-05",
            "2026/1/5",
            "2026.01.05",
            "2026-01-05=2026-02-01",
        ] {
            let txn = read_transaction(&format!("{token} Payee"), 1).expect("reads");
            assert_eq!(txn.key, (2026, 1, 5), "for {token:?}");
            assert_eq!(txn.date, "2026-01-05");
        }
    }

    #[test]
    fn a_nonsense_date_is_refused_rather_than_guessed() {
        for token in [
            "2026-13-01",
            "2026-01-32",
            "2026-01",
            "20260105",
            "2026-ab-01",
        ] {
            assert!(
                read_transaction(&format!("{token} Payee"), 7).is_err(),
                "must refuse {token:?}"
            );
        }
    }

    #[test]
    fn a_description_sheds_status_code_and_comment() {
        assert_eq!(
            description("* (ref-9) Coffee shop  ; tag: x"),
            "Coffee shop"
        );
        assert_eq!(description("! Coffee shop"), "Coffee shop");
        assert_eq!(description("(ref) Coffee shop"), "Coffee shop");
        assert_eq!(description("Coffee shop"), "Coffee shop");
        assert_eq!(description(""), "");
    }

    #[test]
    fn a_description_is_sanitized_for_display() {
        assert_eq!(sanitize("a\u{7}b   c\td"), "ab c d");
        let long = "x".repeat(DESCRIPTION_MAX_CHARS * 2);
        assert_eq!(sanitize(&long).chars().count(), DESCRIPTION_MAX_CHARS);
    }

    #[test]
    fn newline_convention_follows_the_first_terminator() {
        assert_eq!(detect_newline("a\r\nb\n"), Newline::CrLf);
        assert_eq!(detect_newline("a\nb\r\n"), Newline::Lf);
        assert_eq!(detect_newline("a"), Newline::Lf);
    }

    #[test]
    fn a_barrier_confines_the_sort_to_its_own_side() {
        let text = concat!(
            "2026-02-01 later\n    a  $1\n    b\n\n",
            "commodity $1,000.00\n\n",
            "2026-01-01 earlier\n    a  $1\n    b\n",
        );
        // Each side is already sorted on its own, so the barrier means there is
        // nothing to do — the earlier transaction may not cross the directive.
        assert_eq!(plan(text).map(|p| p.unchanged), Ok(true));
        assert_eq!(
            apply(text, &plan(text).expect("plans")).as_deref(),
            Ok(text)
        );
    }

    #[test]
    fn a_stale_plan_is_refused() {
        let text = "2026-02-01 b\n    a  $1\n    b\n\n2026-01-01 a\n    a  $1\n    b\n";
        let stale = SortPlan {
            moves: Vec::new(),
            unchanged: true,
        };
        assert_eq!(apply(text, &stale), Err(SortError::StalePlan));
    }
}
