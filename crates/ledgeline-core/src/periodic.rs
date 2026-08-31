//! `~` periodic transaction rules: the span document model that lets a budget
//! goal be rewritten without touching another byte of the journal.
//!
//! # Why this module is not called `budget`
//!
//! A `~ PERIODEXPR  DESCRIPTION` block is hledger's *one* recurring-entry
//! construct. `hledger balance --budget` reads those blocks as goals, and
//! `hledger --forecast` reads the very same blocks as future transactions. The
//! budget editor is the first consumer; the planning/forecast work in `TODO.md`
//! is the second, and it will want this document rather than a second one that
//! has to agree with it. So the model here is "a periodic rule", and "a budget
//! goal" is a reading of one.
//!
//! # The discipline
//!
//! The same one [`crate::aliases`] states, and for a file of the same value:
//!
//! > **An edit rewrites bytes only inside the spans it names, and every other
//! > byte of the file comes out the `&str` slice it went in as.**
//!
//! [`PeriodicDoc::apply`] copies the original text verbatim between the spans it
//! splices, and the only spans it will splice are one posting line's **amount**
//! extent, one posting line's **whole line** (a delete), one block's **whole
//! extent** (deleting its last goal), or an insertion point between lines. It
//! never rewrites an account name, never a `~` header, never the whitespace
//! between an account and its amount — so a column-aligned block stays aligned
//! with no alignment code anywhere.
//!
//! [`PeriodicDoc::verify`] then refuses rather than trusting that: it re-renders
//! the plan, requires the bytes to match, re-scans the result, and requires every
//! *unedited* line to come back byte-identical and every edited one to read back
//! as exactly the amount that was asked for.
//!
//! What is deliberately **not** here is whether the result is still a journal,
//! and whether the goal now *means* the requested number. This module is a
//! text-shape model — it does not parse amounts, which is the parser's job and
//! not a job worth doing twice. The caller re-parses the whole journal with the
//! edited text in memory and compares the resulting
//! [`PeriodicTransaction`]s, exactly as `alias_api` does. That check is stronger
//! than anything this module could do alone, and it belongs where the journal is
//! known.
//!
//! # Keeping a rule balanced
//!
//! hledger balances a periodic rule's real postings like a transaction's. Which
//! shape a block is in decides what an edit has to do, and the three shapes are
//! told apart structurally — no account-name heuristics, no type declarations:
//!
//! - [`BlockBalance::Free`] — every posting is an unbalanced-virtual `(account)`
//!   one. This is the idiom every hledger budget example uses, and nothing
//!   constrains the amounts. An edit rewrites one number and stops.
//! - [`BlockBalance::Inferred`] — exactly one real posting has no written
//!   amount. hledger derives it from the others, so an edit *still* rewrites one
//!   number and stops: the leg re-infers itself. This is the case where doing
//!   less is doing it right.
//! - [`BlockBalance::Explicit`] — every real amount is written down, so changing
//!   one without changing another leaves the rule unbalanced. The counter-leg is
//!   rewritten too, and [`counterparty`] says which leg that is.
//!
//! ## Which leg is the counter-leg
//!
//! **The unique real posting, other than the one being changed, whose amount is
//! signed opposite to the change.** If there is not exactly one, this module
//! refuses ([`PeriodicError::AmbiguousCounterparty`]) instead of picking.
//!
//! That rule is worth stating in the concrete, because the refusals are as
//! deliberate as the successes. Given
//!
//! ```text
//! ~ monthly  budget
//!     expenses:food      $400
//!     expenses:rent     $1500
//!     assets:checking  $-1900
//! ```
//!
//! raising the food goal to `$450` finds exactly one opposite-signed leg
//! (`assets:checking`) and takes it to `$-1950`. Editing `assets:checking`
//! *itself* finds two opposite-signed legs, which is genuinely ambiguous — there
//! is no fact in the file that says whether food or rent should absorb the
//! difference — so it is refused with a sentence rather than resolved by
//! coin-flip. A user who wants that edits their journal.
//!
//! The arithmetic is `counter -= delta`, where `delta` is the change in the sum
//! of every other real posting. That is exact for a set amount, a delete and an
//! append alike, which is why all three share one code path.
//!
//! # What is presented read-only
//!
//! Following `aliases.rs`'s "when in doubt, opaque": a line or a block this
//! module cannot promise to rewrite safely is listed with a [`GoalLock`] or a
//! [`BlockLock`] naming what stopped it. It is still shown, still reported, still
//! feeds the budget report. It simply cannot be edited here.

use crate::decimal::{Dec, DecError};
use crate::model::{Amount, PeriodExpr, PeriodicTransaction, PostingType};
use crate::rules::Newline;
use std::ops::Range;
use thiserror::Error;

/// A byte range into [`PeriodicDoc::text`]. Byte offsets, like `aliases::Span`,
/// and for the same reason: every one lands on a boundary the scan found in the
/// text, so slicing can never split a code point.
pub type Span = Range<usize>;

/// Longest account name this module will write, in bytes. The same cap
/// `aliases.rs` puts on a replacement, which is also an account name.
pub const MAX_ACCOUNT_BYTES: usize = 512;

/// Longest rule description this module will write, in bytes.
pub const MAX_DESCRIPTION_BYTES: usize = 256;

/// Longest rendered amount this module will write, in bytes. A number plus a
/// commodity symbol; anything near this is not a budget figure.
pub const MAX_AMOUNT_BYTES: usize = 128;

/// The indentation a newly written posting line gets when its block has no
/// existing line to copy from. Four spaces, which is what hledger's own
/// documentation and every fixture in this repo use.
const DEFAULT_INDENT: &str = "    ";

// ---------------------------------------------------------------------------
// Locks
// ---------------------------------------------------------------------------

/// Why one posting line inside a `~` block is presented read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalLock {
    /// The line has no written amount: it is the leg hledger infers. There is no
    /// amount extent to splice, so setting one would mean inventing separator
    /// bytes and changing the block's shape from inferred to explicit — a bigger
    /// edit than the user asked for. Changing any *other* line in the block is
    /// what moves this one.
    Inferred,
    /// The amount carries an `@`/`@@` cost annotation. Rewriting the quantity
    /// without deciding what happens to the price would change what the line
    /// means.
    Cost,
    /// The amount carries a `=` balance assertion. A goal is not a place for one,
    /// and rewriting around it is not this module's business.
    Assertion,
    /// The amount text holds more than one amount (a `+`-joined mixed amount).
    Multiple,
    /// A control character on the line.
    Control,
    /// Longer than this module will rewrite.
    TooLong,
}

impl GoalLock {
    /// A sentence completing "this budget line cannot be edited here because …".
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Inferred => {
                "it has no written amount — hledger works it out from the other lines in the \
                 rule, so it changes when they do"
            }
            Self::Cost => {
                "its amount carries an `@` cost annotation, and rewriting the quantity without \
                 deciding what happens to the price would change what the line means"
            }
            Self::Assertion => "its amount carries a `=` balance assertion",
            Self::Multiple => "its amount is more than one amount added together",
            Self::Control => "it contains a control character",
            Self::TooLong => "it is longer than Ledgeline will rewrite",
        }
    }
}

/// Why a whole `~` block is presented read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockLock {
    /// The period expression is not one of hledger's five fixed intervals.
    /// Ledgeline does not model richer ones (see [`PeriodExpr`]), so it will not
    /// write into a block whose recurrence it cannot state.
    Period,
    /// The block holds a balanced-virtual `[account]` posting. Those balance
    /// among themselves, as a second group alongside the real one, and a rule
    /// with two balance groups has two counter-legs to keep straight. Rare
    /// enough in a budget file to be worth refusing rather than modelling.
    BalancedVirtual,
    /// The block's real postings are not all in one commodity, so there is no
    /// single arithmetic that keeps them balanced.
    MultiCommodity,
    /// More than one real posting has no written amount, or exactly one real
    /// posting carries an amount by itself. Neither is a rule hledger accepts,
    /// so the file is not in a state to edit from.
    Unbalanceable,
}

impl BlockLock {
    /// A sentence completing "this budget rule cannot be edited here because …".
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Period => {
                "its period is not one of daily, weekly, monthly, quarterly or yearly, and \
                 Ledgeline will not rewrite a rule whose recurrence it cannot state"
            }
            Self::BalancedVirtual => {
                "it uses balanced-virtual `[account]` postings, which balance as a second group \
                 alongside the real ones"
            }
            Self::MultiCommodity => "its postings are not all in one commodity",
            Self::Unbalanceable => {
                "its postings do not balance in a way hledger accepts, so there is no consistent \
                 edit to make"
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The document model
// ---------------------------------------------------------------------------

/// How a `~` block's real postings stay balanced, and therefore what an edit to
/// one of its amounts has to do. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockBalance {
    /// No real postings at all — every line is an unbalanced-virtual
    /// `(account)` goal. Amounts move freely.
    Free,
    /// Exactly one real posting has no written amount; hledger infers it, so an
    /// edit to any other line needs no counter-edit.
    Inferred,
    /// Every real amount is written, so an edit must rewrite a counter-leg.
    Explicit,
}

/// One posting line inside a `~` block, located in a file's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalLine {
    /// 0-based position among **this file's** posting lines, across every block
    /// — the handle an edit names. Like `aliases::AliasLine::index` it is a
    /// scan-time ordinal and is not durable across saves; the caller's revision
    /// check is what makes that safe.
    pub index: usize,
    /// Which block this line belongs to, by [`PeriodicBlock::index`].
    pub block: usize,
    /// 0-based position among its own block's posting lines. This is what lines
    /// up with [`PeriodicTransaction::postings`].
    pub at: usize,
    /// 1-based file line, numbered LF-only exactly as [`str::lines`] does.
    pub line: u32,
    /// The line's content, without its terminator.
    pub span: Span,
    /// The line's content **with** its terminator — what a delete removes.
    pub full: Span,
    /// The written amount's extent, trimmed. Empty (and never spliced) when the
    /// line has no amount.
    pub amount_span: Span,
    /// The account as written, **without** any `(…)`/`[…]` wrapper.
    pub account: String,
    /// Real, unbalanced-virtual `(a)`, or balanced-virtual `[a]`.
    pub ptype: PostingType,
    /// The amount as written, or `None` when the line has none.
    pub amount: Option<String>,
    /// `Some` when the line is read-only.
    pub lock: Option<GoalLock>,
}

impl GoalLine {
    /// Whether this line's amount may be rewritten here. A block-level lock is a
    /// separate question — ask [`PeriodicDoc::line_lock`] for both at once.
    #[must_use]
    pub fn editable(&self) -> bool {
        self.lock.is_none()
    }
}

/// One `~ PERIODEXPR  [DESCRIPTION]` block, located in a file's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodicBlock {
    /// 0-based position among this file's `~` blocks. This is the ordinal that
    /// lines up with the `Journal::periodic_transactions` entries whose
    /// `source_file` is this file.
    pub index: usize,
    /// 1-based file line of the `~` header.
    pub line: u32,
    /// The whole block — header line, every body line, terminators and all.
    /// What deleting the block's last goal removes.
    pub full: Span,
    /// The period expression as written, e.g. `monthly`.
    pub period_text: String,
    /// The period, when it is one Ledgeline models. `None` is
    /// [`BlockLock::Period`].
    pub period: Option<PeriodExpr>,
    /// The description as written, trimmed. Empty when the rule has none.
    pub description: String,
    /// Indices into [`PeriodicDoc::lines`], in file order.
    pub lines: Vec<usize>,
    /// How this block's real postings stay balanced.
    pub balance: BlockBalance,
    /// `Some` when the whole block is read-only.
    pub lock: Option<BlockLock>,
}

/// One journal file's `~` blocks, over its original text.
///
/// Immutable. An edit is a [`PeriodicPlan`] rendered by [`PeriodicDoc::apply`],
/// which returns a new `String` and never mutates `self`, so a refused edit
/// cannot leave half a document behind.
#[derive(Debug, Clone)]
pub struct PeriodicDoc {
    text: String,
    newline: Newline,
    blocks: Vec<PeriodicBlock>,
    lines: Vec<GoalLine>,
}

/// One change to one file's periodic rules.
///
/// Note what is absent, as in `aliases::AliasEdit`: no variant carries a
/// rendered line. Every byte [`PeriodicDoc::apply`] writes is either a byte read
/// from the file moments earlier or this module's own rendering of a validated
/// account/amount pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeriodicEdit {
    /// Rewrite one posting line's amount, leaving its account, its separator
    /// whitespace, its comment and its terminator exactly as they are.
    SetAmount {
        /// Which line, by [`GoalLine::index`].
        index: usize,
        /// The amount to write.
        amount: Amount,
    },
    /// Remove one posting line, terminator and all — or the whole block, when it
    /// is the block's only posting. See [`PeriodicDoc::splices`].
    Delete {
        /// Which line, by [`GoalLine::index`].
        index: usize,
    },
    /// Add a posting line at the end of an existing block's body.
    AppendLine {
        /// Which block, by [`PeriodicBlock::index`].
        block: usize,
        /// The account, without any wrapper.
        account: String,
        /// Real, or the `(account)` form every budget example uses.
        ptype: PostingType,
        /// The amount to write.
        amount: Amount,
    },
    /// Add a whole new block at EOF, holding one posting line.
    AppendBlock {
        /// The rule's recurrence.
        period: PeriodExpr,
        /// The rule description, which `--budget=DESCPAT` matches on. May be
        /// empty.
        description: String,
        /// The account, without any wrapper.
        account: String,
        /// Real, or the `(account)` form every budget example uses.
        ptype: PostingType,
        /// The amount to write.
        amount: Amount,
    },
}

impl PeriodicEdit {
    /// The existing posting line this edit names, or `None` for an append.
    #[must_use]
    pub fn index(&self) -> Option<usize> {
        match self {
            Self::SetAmount { index, .. } | Self::Delete { index } => Some(*index),
            Self::AppendLine { .. } | Self::AppendBlock { .. } => None,
        }
    }

    /// The amount this edit writes, if it writes one.
    fn amount(&self) -> Option<&Amount> {
        match self {
            Self::SetAmount { amount, .. }
            | Self::AppendLine { amount, .. }
            | Self::AppendBlock { amount, .. } => Some(amount),
            Self::Delete { .. } => None,
        }
    }

    /// The account this edit writes, if it writes one.
    fn account(&self) -> Option<&str> {
        match self {
            Self::AppendLine { account, .. } | Self::AppendBlock { account, .. } => Some(account),
            Self::SetAmount { .. } | Self::Delete { .. } => None,
        }
    }
}

/// A complete set of changes to one file's periodic rules.
///
/// Built by [`plan`] from ONE user gesture, which is the only way a
/// counter-leg edit gets into it. A hand-assembled plan is accepted — `apply`
/// validates whatever it is given — but it is then the assembler's job to keep
/// the rule balanced, and the caller's whole-journal re-parse that catches it if
/// they did not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeriodicPlan {
    /// The changes, in any order. At most one per existing line.
    pub edits: Vec<PeriodicEdit>,
}

impl PeriodicPlan {
    /// The edit naming `index`, if any.
    fn edit_for(&self, index: usize) -> Option<&PeriodicEdit> {
        self.edits.iter().find(|edit| edit.index() == Some(index))
    }
}

/// Errors from planning or checking a periodic-rule rewrite.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PeriodicError {
    /// An edit named a posting line this file does not have — almost always a
    /// stale index from a client that planned against an older scan.
    #[error("this file has no budget line number {0}")]
    UnknownLine(usize),
    /// An edit named a `~` block this file does not have.
    #[error("this file has no budget rule number {0}")]
    UnknownBlock(usize),
    /// Two edits named the same line.
    #[error("budget line number {0} was named by more than one change")]
    DuplicateLine(usize),
    /// An edit asked to rewrite a line this module presents read-only.
    #[error("budget line number {index} cannot be edited here because {why}")]
    LockedLine {
        /// Which line.
        index: usize,
        /// The lock's own sentence.
        why: &'static str,
    },
    /// An edit asked to rewrite a block this module presents read-only.
    #[error("budget rule number {index} cannot be edited here because {why}")]
    LockedBlock {
        /// Which block.
        index: usize,
        /// The lock's own sentence.
        why: &'static str,
    },
    /// The block needs a counter-leg rewritten and there is not exactly one
    /// candidate. See the module docs for why this is a refusal and not a guess.
    #[error(
        "this change would leave budget rule number {index} unbalanced, and its postings do not \
         say which one should absorb the difference; edit this rule in your journal instead"
    )]
    AmbiguousCounterparty {
        /// Which block.
        index: usize,
    },
    /// An append named an account the block already budgets. hledger would add
    /// the two postings together, so a second one is not "another goal" — it is
    /// an unreadable way of writing the first. Changing what is already there is
    /// a [`GoalRequest::Set`].
    #[error(
        "budget rule number {index} already has a goal for {account}; change that goal rather \
         than adding a second one to the same rule"
    )]
    DuplicateGoal {
        /// Which block.
        index: usize,
        /// The account it already budgets.
        account: String,
    },
    /// A client-supplied value is not something this module will write into a
    /// journal.
    #[error("{0}")]
    Invalid(String),
    /// Exact-decimal arithmetic on the counter-leg overflowed.
    #[error("the counter-posting's new amount is out of range: {0}")]
    Overflow(String),
    /// The rewritten text could not be proved to be the requested edit and
    /// nothing else, so the caller must write nothing.
    #[error("the rewritten journal failed its round-trip check; nothing was written")]
    RoundTripMismatch,
}

impl From<DecError> for PeriodicError {
    fn from(error: DecError) -> Self {
        Self::Overflow(error.to_string())
    }
}

impl PeriodicDoc {
    /// Scan one journal file's `~` blocks. **Infallible** — a file always opens,
    /// and a construct this module will not rewrite becomes a locked block or a
    /// locked line rather than a refusal, exactly as `AliasDoc::parse` never
    /// fails.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let (blocks, lines) = scan(text);
        Self {
            text: text.to_string(),
            newline: Newline::detect(text),
            blocks,
            lines,
        }
    }

    /// The original text, byte for byte.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The `~` blocks, in file order. Index equals [`PeriodicBlock::index`], and
    /// equals the ordinal of the matching `Journal::periodic_transactions` entry
    /// for this file.
    #[must_use]
    pub fn blocks(&self) -> &[PeriodicBlock] {
        &self.blocks
    }

    /// Every posting line, in file order. Index equals [`GoalLine::index`].
    #[must_use]
    pub fn lines(&self) -> &[GoalLine] {
        &self.lines
    }

    /// The block a new goal of this recurrence and name should JOIN: the first,
    /// in file order, whose interval and description both match and that this
    /// module is willing to rewrite. `None` means no rule states it yet, and the
    /// caller should open one.
    ///
    /// **Both halves have to match.** The description is not decoration —
    /// `hledger balance --budget=DESCPAT` filters on it — so folding a goal into
    /// a rule of the right period but another name would quietly move it into,
    /// or out of, somebody's filtered report.
    ///
    /// The comparison is on the trimmed, whitespace-collapsed description,
    /// because the spacing of a header is not part of what it says: a rule
    /// written `~ monthly   monthly budget` states the same thing as
    /// `~ monthly  monthly budget`, and whoever widened that gap was aligning a
    /// column, not naming a second rule. Only the COMPARISON is normalised. The
    /// header is never rewritten to match it — an append touches no byte of the
    /// block it joins except the one line it adds.
    ///
    /// A locked block is skipped rather than joined, and the caller opens a new
    /// one instead. A [`BlockLock`] is this module saying it will not rewrite
    /// that construct, and appending a line to it is rewriting it.
    #[must_use]
    pub fn joinable_block(&self, period: PeriodExpr, description: &str) -> Option<usize> {
        let wanted = collapse(description);
        self.blocks
            .iter()
            .find(|block| {
                block.lock.is_none()
                    && block.period == Some(period)
                    && collapse(&block.description) == wanted
            })
            .map(|block| block.index)
    }

    /// The block a line belongs to.
    fn block_of(&self, line: &GoalLine) -> &PeriodicBlock {
        // A line's `block` is assigned by `scan` from the block it was found in,
        // so this index is always in range.
        &self.blocks[line.block]
    }

    /// Why this line cannot be edited, asking BOTH questions: its own lock and
    /// its block's. A caller showing an edit affordance wants this, not
    /// [`GoalLine::editable`] alone.
    #[must_use]
    pub fn line_lock(&self, index: usize) -> Option<&'static str> {
        let line = self.lines.get(index)?;
        self.block_of(line)
            .lock
            .map(BlockLock::message)
            .or_else(|| line.lock.map(GoalLock::message))
    }

    /// Render the file under `plan`. Pure; no I/O; `self` is untouched.
    ///
    /// # Errors
    /// Validation runs to completion before a byte is rendered, so a rejected
    /// plan never produces partial text.
    pub fn apply(&self, plan: &PeriodicPlan) -> Result<String, PeriodicError> {
        let splices = self.splices(plan)?;
        // Every byte outside a splice is copied from `self.text` verbatim. That
        // is the isolation guarantee, and it holds by construction rather than
        // by inspection.
        let mut out = String::with_capacity(self.text.len());
        let mut cursor = 0;
        for (span, replacement) in &splices {
            // A splice that starts behind the cursor would mean two edits
            // overlap, which `check_plan` rules out — but slicing on it would
            // panic, so the invariant is asserted rather than assumed.
            if span.start < cursor {
                return Err(PeriodicError::RoundTripMismatch);
            }
            out.push_str(&self.text[cursor..span.start]);
            out.push_str(replacement);
            cursor = span.end;
        }
        out.push_str(&self.text[cursor..]);
        Ok(out)
    }

    /// Prove `new_text` is exactly the edit `plan` asked for, and nothing else.
    ///
    /// Byte preservation alone is not enough — the same argument `aliases::verify`
    /// makes — so this re-scans and checks structure:
    ///
    /// 1. re-render the plan and require `new_text` to match byte for byte;
    /// 2. require the re-scan to hold exactly the blocks and lines the plan
    ///    implies;
    /// 3. require every **unedited** surviving line to come back byte-identical
    ///    to its original span content;
    /// 4. require every edited line to read back with the same account it had and
    ///    the exact amount text that was rendered for it;
    /// 5. require every appended line to read back as exactly the
    ///    account/amount that was requested.
    ///
    /// What is deliberately *not* here: whether the file still parses as part of
    /// its journal, and whether the goal now means the number the user typed.
    /// Both are the caller's, and both are stronger for being done against a
    /// real parse — see the module docs.
    ///
    /// # Errors
    /// [`PeriodicError::RoundTripMismatch`] if any step fails, plus whatever
    /// [`PeriodicDoc::apply`] reports for an invalid plan.
    pub fn verify(&self, plan: &PeriodicPlan, new_text: &str) -> Result<(), PeriodicError> {
        if self.apply(plan)? != new_text {
            return Err(PeriodicError::RoundTripMismatch);
        }
        let reparsed = Self::parse(new_text);

        // Which original lines the plan removes — including every line of a
        // block that a delete takes down with it.
        let removed = self.removed_lines(plan);
        let survivors: Vec<&GoalLine> = self
            .lines
            .iter()
            .filter(|line| !removed.contains(&line.index))
            .collect();
        let appended: Vec<&PeriodicEdit> = plan
            .edits
            .iter()
            .filter(|edit| {
                matches!(
                    edit,
                    PeriodicEdit::AppendLine { .. } | PeriodicEdit::AppendBlock { .. }
                )
            })
            .collect();
        if reparsed.lines.len() != survivors.len() + appended.len() {
            return Err(PeriodicError::RoundTripMismatch);
        }

        // An appended LINE lands inside an existing block, so it is not
        // necessarily last in file order; an appended BLOCK is always at EOF.
        // Rather than reason about interleaving, each surviving original is
        // matched by its span content or by its requested value, and the
        // leftovers must be exactly the appends. That is order-insensitive and
        // therefore cannot be fooled by a splice landing in the wrong place —
        // the byte-identity check below is what pins position.
        let mut after = reparsed.lines.iter();
        for before in &survivors {
            let Some(line) = after.next() else {
                return Err(PeriodicError::RoundTripMismatch);
            };
            // An append can sit between two survivors; skip forward over any
            // line that is not the survivor we are looking for.
            let mut line = line;
            while line.account != before.account {
                let Some(next) = after.next() else {
                    return Err(PeriodicError::RoundTripMismatch);
                };
                line = next;
            }
            match plan.edit_for(before.index) {
                Some(PeriodicEdit::SetAmount { amount, .. }) => {
                    if line.amount.as_deref() != Some(render_amount(amount).as_str())
                        || line.account != before.account
                        || line.ptype != before.ptype
                    {
                        return Err(PeriodicError::RoundTripMismatch);
                    }
                }
                // Untouched: byte-identical, which is the isolation property
                // stated as a check rather than as a hope.
                _ => {
                    if new_text[line.span.clone()] != self.text[before.span.clone()] {
                        return Err(PeriodicError::RoundTripMismatch);
                    }
                }
            }
        }

        // Every appended value must be present in the result. Checked by
        // membership over the lines the survivors did not claim, because an
        // append's position is a splice-point property and is already pinned by
        // the byte-for-byte equality in step 1.
        let claimed: Vec<&str> = survivors.iter().map(|line| line.account.as_str()).collect();
        let mut spare: Vec<&GoalLine> = Vec::new();
        for line in &reparsed.lines {
            let seen = claimed.iter().filter(|a| **a == line.account).count();
            let taken = spare.iter().filter(|l| l.account == line.account).count();
            if seen == 0 || taken >= seen {
                spare.push(line);
            }
        }
        for edit in &appended {
            let (account, amount, ptype) = match edit {
                PeriodicEdit::AppendLine {
                    account,
                    amount,
                    ptype,
                    ..
                }
                | PeriodicEdit::AppendBlock {
                    account,
                    amount,
                    ptype,
                    ..
                } => (account, amount, ptype),
                _ => return Err(PeriodicError::RoundTripMismatch),
            };
            let wanted = render_amount(amount);
            let at = spare
                .iter()
                .position(|line| {
                    line.account == *account
                        && line.ptype == *ptype
                        && line.amount.as_deref() == Some(wanted.as_str())
                })
                .ok_or(PeriodicError::RoundTripMismatch)?;
            spare.remove(at);
        }

        // Blocks: the count must be exactly what the plan implies, so a splice
        // that dissolved a `~` header is caught even when every line survives.
        let dropped = self.dropped_blocks(plan).len();
        let added = plan
            .edits
            .iter()
            .filter(|edit| matches!(edit, PeriodicEdit::AppendBlock { .. }))
            .count();
        if reparsed.blocks.len() + dropped != self.blocks.len() + added {
            return Err(PeriodicError::RoundTripMismatch);
        }
        Ok(())
    }

    /// The blocks a plan removes entirely: those whose every posting line is
    /// deleted. Deleting a rule's last goal deletes the rule — a bare `~` header
    /// with no postings is not a construct worth leaving behind, and it is not
    /// one hledger accepts.
    fn dropped_blocks(&self, plan: &PeriodicPlan) -> Vec<usize> {
        let deleted: Vec<usize> = plan
            .edits
            .iter()
            .filter_map(|edit| match edit {
                PeriodicEdit::Delete { index } => Some(*index),
                _ => None,
            })
            .collect();
        self.blocks
            .iter()
            .filter(|block| {
                !block.lines.is_empty() && block.lines.iter().all(|at| deleted.contains(at))
            })
            .map(|block| block.index)
            .collect()
    }

    /// Every original line index a plan removes, including the ones removed as
    /// collateral when their whole block goes.
    fn removed_lines(&self, plan: &PeriodicPlan) -> Vec<usize> {
        let dropped = self.dropped_blocks(plan);
        self.lines
            .iter()
            .filter(|line| {
                dropped.contains(&line.block)
                    || matches!(plan.edit_for(line.index), Some(PeriodicEdit::Delete { .. }))
            })
            .map(|line| line.index)
            .collect()
    }

    /// The ordered, non-overlapping `(span, text)` list one plan splices.
    fn splices(&self, plan: &PeriodicPlan) -> Result<Vec<(Span, String)>, PeriodicError> {
        self.check_plan(plan)?;
        let dropped = self.dropped_blocks(plan);
        let mut splices: Vec<(Span, String)> = Vec::new();
        for edit in &plan.edits {
            match edit {
                PeriodicEdit::SetAmount { index, amount } => {
                    let line = self.line(*index)?;
                    // The amount extent ONLY: the account, the whitespace that
                    // aligns the column, and any trailing comment are re-emitted
                    // verbatim, which is what keeps a tidy block tidy.
                    splices.push((line.amount_span.clone(), render_amount(amount)));
                }
                PeriodicEdit::Delete { index } => {
                    let line = self.line(*index)?;
                    if dropped.contains(&line.block) {
                        // The block goes as a unit. Several deletes can name the
                        // same block, so this is de-duplicated below rather than
                        // pushed once per line.
                        let block = self.block_of(line);
                        let span = block.full.clone();
                        if !splices.iter().any(|(existing, _)| *existing == span) {
                            splices.push((span, String::new()));
                        }
                    } else {
                        splices.push((line.full.clone(), String::new()));
                    }
                }
                PeriodicEdit::AppendLine {
                    block,
                    account,
                    ptype,
                    amount,
                } => {
                    let block = self.block(*block)?;
                    let at = block.full.end;
                    splices.push((at..at, self.appended_line(block, account, *ptype, amount)));
                }
                PeriodicEdit::AppendBlock {
                    period,
                    description,
                    account,
                    ptype,
                    amount,
                } => {
                    let at = self.text.len();
                    splices.push((
                        at..at,
                        self.appended_block(*period, description, account, *ptype, amount),
                    ));
                }
            }
        }
        splices.sort_by_key(|(span, _)| (span.start, span.end));
        Ok(splices)
    }

    /// The text an [`PeriodicEdit::AppendLine`] inserts at the end of `block`.
    ///
    /// The indent is copied from the block's last posting line, and the amount is
    /// padded so that it *ends* in the same column as that line's amount — which
    /// is how hledger's own examples, and every fixture in this repo, lay a block
    /// out. Alignment is imitated rather than computed: a block written with two
    /// spaces stays written with two spaces, and a right-aligned one gains a
    /// right-aligned line, without this module having an opinion about which is
    /// right.
    ///
    /// Two spaces is the floor, always. It is the minimum hledger accepts between
    /// an account and an amount, so an account too long to align gets a line that
    /// is merely correct rather than one that is pretty and wrong.
    fn appended_line(
        &self,
        block: &PeriodicBlock,
        account: &str,
        ptype: PostingType,
        amount: &Amount,
    ) -> String {
        let newline = self.newline.as_str();
        let last = block.lines.last().map(|at| &self.lines[*at]);
        let indent = last
            .map(|line| {
                let content = &self.text[line.span.clone()];
                &content[..content.len() - content.trim_start().len()]
            })
            .filter(|indent| !indent.is_empty())
            .unwrap_or(DEFAULT_INDENT);
        let written = write_account(account, ptype);
        let rendered = render_amount(amount);
        // The column the previous line's amount ENDS at, measured in chars from
        // the start of that line.
        let column = last.and_then(|line| {
            (!line.amount_span.is_empty()).then(|| {
                self.text[line.span.start..line.amount_span.end]
                    .chars()
                    .count()
            })
        });
        let filled = indent.chars().count() + written.chars().count() + rendered.chars().count();
        let gap = column
            .and_then(|column| column.checked_sub(filled))
            .unwrap_or(0)
            .max(2);
        let at = block.full.end;
        let lead = if at == 0 || self.text[..at].ends_with('\n') {
            ""
        } else {
            newline
        };
        format!(
            "{lead}{indent}{written}{}{rendered}{newline}",
            " ".repeat(gap)
        )
    }

    /// The text an [`PeriodicEdit::AppendBlock`] inserts at EOF.
    ///
    /// EOF, and nowhere else, for the reason `AliasDoc::insertion_point` gives:
    /// it is the one position provably unable to change the meaning of anything
    /// already in the file. A `~` block's extent runs to the first line that is
    /// not indented, so a block placed anywhere else would have to reason about
    /// what it interrupts.
    ///
    /// A blank line precedes it — unlike an appended alias, and for a reason a
    /// budget file makes plain: a `~` block is a multi-line construct, and
    /// putting one directly against whatever came before it produces a file
    /// nobody would have written by hand.
    fn appended_block(
        &self,
        period: PeriodExpr,
        description: &str,
        account: &str,
        ptype: PostingType,
        amount: &Amount,
    ) -> String {
        let newline = self.newline.as_str();
        let lead = if self.text.is_empty() {
            String::new()
        } else if self.text.ends_with('\n') {
            newline.to_string()
        } else {
            format!("{newline}{newline}")
        };
        // hledger separates the period expression from the description by
        // two-or-more spaces; with no description there is nothing to separate.
        let header = if description.is_empty() {
            format!("~ {}", period_word(period))
        } else {
            format!("~ {}  {description}", period_word(period))
        };
        format!(
            "{lead}{header}{newline}{DEFAULT_INDENT}{}  {}{newline}",
            write_account(account, ptype),
            render_amount(amount)
        )
    }

    /// Reject a plan before a byte of it is rendered.
    fn check_plan(&self, plan: &PeriodicPlan) -> Result<(), PeriodicError> {
        let mut named: Vec<usize> = Vec::new();
        for edit in &plan.edits {
            if let Some(index) = edit.index() {
                let line = self.line(index)?;
                if named.contains(&index) {
                    return Err(PeriodicError::DuplicateLine(index));
                }
                named.push(index);
                let block = self.block_of(line);
                if let Some(lock) = block.lock {
                    return Err(PeriodicError::LockedBlock {
                        index: block.index,
                        why: lock.message(),
                    });
                }
                // A locked LINE can still be deleted — removing a whole line
                // cannot inject anything, the same narrowing `aliases.rs` makes
                // for a locked alias.
                if let (Some(lock), PeriodicEdit::SetAmount { .. }) = (line.lock, edit) {
                    return Err(PeriodicError::LockedLine {
                        index,
                        why: lock.message(),
                    });
                }
            }
            if let PeriodicEdit::AppendLine { block, account, .. } = edit {
                let block = self.block(*block)?;
                if let Some(lock) = block.lock {
                    return Err(PeriodicError::LockedBlock {
                        index: block.index,
                        why: lock.message(),
                    });
                }
                // A rule states a given account's goal once. Refused here, up
                // front and with a sentence, rather than left to `verify` — which
                // cannot match a second line of the same account against the
                // first and so reports a round-trip failure, i.e. blames the
                // engine for what the caller asked for.
                if let Some(at) = block
                    .lines
                    .iter()
                    .find(|at| self.lines[**at].account == *account)
                {
                    return Err(PeriodicError::DuplicateGoal {
                        index: block.index,
                        account: self.lines[*at].account.clone(),
                    });
                }
            }
            if let PeriodicEdit::AppendBlock { description, .. } = edit {
                check_description(description)?;
            }
            if let Some(account) = edit.account() {
                check_account(account)?;
            }
            if let Some(amount) = edit.amount() {
                check_amount(amount)?;
            }
        }
        Ok(())
    }

    /// The posting line with this index.
    fn line(&self, index: usize) -> Result<&GoalLine, PeriodicError> {
        self.lines
            .get(index)
            .ok_or(PeriodicError::UnknownLine(index))
    }

    /// The block with this index.
    fn block(&self, index: usize) -> Result<&PeriodicBlock, PeriodicError> {
        self.blocks
            .get(index)
            .ok_or(PeriodicError::UnknownBlock(index))
    }
}

/// A rule description as it is COMPARED: trimmed, and every run of whitespace a
/// single space. Never a form anything is written in — see
/// [`PeriodicDoc::joinable_block`].
fn collapse(description: &str) -> String {
    description
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Planning: one gesture becomes a balanced plan
// ---------------------------------------------------------------------------

/// What the user asked for, before it is worked out into a [`PeriodicPlan`].
///
/// Deliberately smaller than [`PeriodicEdit`]: a request names a quantity, and
/// [`plan`] decides which lines have to move — including a counter-leg the user
/// never mentioned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalRequest {
    /// Set one existing goal to `quantity`, in the commodity it is already
    /// written in.
    Set {
        /// Which line, by [`GoalLine::index`].
        index: usize,
        /// The new quantity, in the line's own commodity.
        quantity: Dec,
    },
    /// Remove one existing goal.
    Remove {
        /// Which line, by [`GoalLine::index`].
        index: usize,
    },
    /// Add a goal to an existing block.
    Add {
        /// Which block, by [`PeriodicBlock::index`].
        block: usize,
        /// The account, without any wrapper.
        account: String,
        /// The amount, fully specified (commodity and display style included).
        amount: Amount,
    },
    /// Add a goal under a rule of this recurrence and name: joining the rule
    /// that already states it, or opening one at EOF when none does. Which of
    /// the two is [`PeriodicDoc::joinable_block`]'s answer, not the caller's.
    AddBlock {
        /// The rule's recurrence.
        period: PeriodExpr,
        /// The rule description. May be empty.
        description: String,
        /// The account, without any wrapper.
        account: String,
        /// The amount, fully specified.
        amount: Amount,
    },
}

/// Turn one [`GoalRequest`] into a plan that leaves every rule balanced.
///
/// `rules` is the file's own parsed `~` rules, in file order — the ones whose
/// `source_file` is this document's file. It supplies the exact quantities the
/// counter-leg arithmetic needs; the document itself models text shape only, and
/// deliberately does not re-implement the number parser (see the module docs).
///
/// `rules[i]` must correspond to `doc.blocks()[i]`, and `rules[i].postings[k]` to
/// that block's `k`-th posting line. Both hold by construction: the scan here and
/// the parser skip the same `comment` blocks and consume the same body lines.
/// The correspondence is checked rather than assumed.
///
/// # Errors
/// [`PeriodicError::UnknownLine`] / [`PeriodicError::UnknownBlock`] for a stale
/// handle, [`PeriodicError::LockedLine`] / [`PeriodicError::LockedBlock`] for a
/// read-only construct, [`PeriodicError::AmbiguousCounterparty`] when the rule
/// does not say which leg absorbs the change, and
/// [`PeriodicError::Invalid`] for a value this module will not write.
pub fn plan(
    doc: &PeriodicDoc,
    rules: &[PeriodicTransaction],
    request: &GoalRequest,
) -> Result<PeriodicPlan, PeriodicError> {
    let (block_index, primary, delta) = match request {
        GoalRequest::Set { index, quantity } => {
            let line = doc.line(*index)?;
            let rule = aligned_rule(doc, rules, line.block)?;
            let posting = rule
                .postings
                .get(line.at)
                .ok_or(PeriodicError::UnknownLine(*index))?;
            let current = single_amount(posting).ok_or(PeriodicError::LockedLine {
                index: *index,
                why: GoalLock::Multiple.message(),
            })?;
            let amount = Amount {
                quantity: *quantity,
                ..current.clone()
            };
            let delta = quantity.sub(current.quantity)?;
            (
                line.block,
                PeriodicEdit::SetAmount {
                    index: *index,
                    amount,
                },
                counted(posting.ptype, delta)?,
            )
        }
        GoalRequest::Remove { index } => {
            let line = doc.line(*index)?;
            let rule = aligned_rule(doc, rules, line.block)?;
            let posting = rule
                .postings
                .get(line.at)
                .ok_or(PeriodicError::UnknownLine(*index))?;
            // Removing a goal removes its contribution, so the counter-leg moves
            // by the negative of what was there.
            let removed =
                single_amount(posting).map_or(Ok(Dec::new(0, 0)), |a| a.quantity.neg())?;
            (
                line.block,
                PeriodicEdit::Delete { index: *index },
                counted(posting.ptype, removed)?,
            )
        }
        GoalRequest::Add {
            block,
            account,
            amount,
        } => {
            let ptype = goal_ptype(doc.block(*block)?);
            (
                *block,
                PeriodicEdit::AppendLine {
                    block: *block,
                    account: account.clone(),
                    ptype,
                    amount: amount.clone(),
                },
                counted(ptype, amount.quantity)?,
            )
        }
        GoalRequest::AddBlock {
            period,
            description,
            account,
            amount,
        } => {
            // One rule per interval and name: a goal joins the rule that already
            // states its recurrence under its description, and a block is opened
            // only when none does. That is decision 1 of
            // `plans/15-budget-editor.md` and what `docs/budget.md` promises; a
            // block per goal is legible to nobody and is not what
            // `--budget=DESCPAT` is filtering over.
            //
            // Joining IS `Add`: the same append, the same imitated alignment,
            // and the same counter-leg arithmetic when the rule it lands in
            // balances explicitly. So it is planned as one, rather than as a
            // second path that would have to go on agreeing with the first.
            if let Some(block) = doc.joinable_block(*period, description) {
                return plan(
                    doc,
                    rules,
                    &GoalRequest::Add {
                        block,
                        account: account.clone(),
                        amount: amount.clone(),
                    },
                );
            }
            // A brand-new block holds one unbalanced-virtual goal, which is the
            // idiom every hledger budget example uses and the only shape that
            // needs no counter-leg at all.
            return Ok(PeriodicPlan {
                edits: vec![PeriodicEdit::AppendBlock {
                    period: *period,
                    description: description.clone(),
                    account: account.clone(),
                    ptype: PostingType::Virtual,
                    amount: amount.clone(),
                }],
            });
        }
    };

    let block = doc.block(block_index)?;
    // `Free` (no real postings) and `Inferred` (hledger derives the leg) both
    // need nothing further: the first has no constraint, and the second
    // re-satisfies its constraint on its own.
    if delta.is_zero() || block.balance != BlockBalance::Explicit {
        return Ok(PeriodicPlan {
            edits: vec![primary],
        });
    }

    let rule = aligned_rule(doc, rules, block_index)?;
    let edited = primary.index().and_then(|index| doc.lines.get(index));
    let counter = counterparty(doc, block, rule, edited.map(|line| line.at), delta)?;
    let counter_line = &doc.lines[block.lines[counter]];
    let posting = &rule.postings[counter_line.at];
    let current = single_amount(posting)
        .ok_or(PeriodicError::AmbiguousCounterparty { index: block_index })?;
    let amount = Amount {
        quantity: current.quantity.sub(delta)?,
        ..current.clone()
    };
    Ok(PeriodicPlan {
        edits: vec![
            primary,
            PeriodicEdit::SetAmount {
                index: counter_line.index,
                amount,
            },
        ],
    })
}

/// The real posting that absorbs `delta`, by position within `block.lines`.
///
/// **The unique real posting, other than `edited`, whose current amount is signed
/// opposite to `delta`.** Not exactly one is
/// [`PeriodicError::AmbiguousCounterparty`] — see the module docs for why that is
/// a refusal rather than a choice.
fn counterparty(
    doc: &PeriodicDoc,
    block: &PeriodicBlock,
    rule: &PeriodicTransaction,
    edited: Option<usize>,
    delta: Dec,
) -> Result<usize, PeriodicError> {
    let wanted_negative = delta.mantissa > 0;
    let candidates: Vec<usize> = block
        .lines
        .iter()
        .enumerate()
        .filter(|(_, index)| {
            let line = &doc.lines[**index];
            if line.ptype != PostingType::Regular || Some(line.at) == edited {
                return false;
            }
            let Some(amount) = rule.postings.get(line.at).and_then(single_amount) else {
                return false;
            };
            // A zero leg has no sign to be opposite to, so it is never chosen:
            // "the leg that funds this" is a claim, and a zero amount does not
            // support it.
            amount.quantity.mantissa != 0 && (amount.quantity.mantissa < 0) == wanted_negative
        })
        .map(|(at, _)| at)
        .collect();
    match candidates.as_slice() {
        [only] => Ok(*only),
        _ => Err(PeriodicError::AmbiguousCounterparty { index: block.index }),
    }
}

/// A change of `delta` on a posting of `ptype`, as it is felt by the real
/// balance group: an unbalanced-virtual posting contributes nothing to it, so
/// changing one moves no counter-leg at all.
fn counted(ptype: PostingType, delta: Dec) -> Result<Dec, PeriodicError> {
    Ok(match ptype {
        PostingType::Regular => delta,
        PostingType::Virtual | PostingType::BalancedVirtual => Dec::new(0, 0),
    })
}

/// The posting form a new goal in `block` should be written in: whatever the
/// block's existing goals use, defaulting to the `(account)` idiom.
fn goal_ptype(block: &PeriodicBlock) -> PostingType {
    match block.balance {
        BlockBalance::Free => PostingType::Virtual,
        // A block with a real balance group gets a real posting, so the goal
        // participates in the same arithmetic as the rest of the rule.
        BlockBalance::Inferred | BlockBalance::Explicit => PostingType::Regular,
    }
}

/// The parsed rule for `block`, with the correspondence checked rather than
/// assumed. A mismatch means the scan and the parse disagree about this file,
/// which is a reason to write nothing.
fn aligned_rule<'a>(
    doc: &PeriodicDoc,
    rules: &'a [PeriodicTransaction],
    block: usize,
) -> Result<&'a PeriodicTransaction, PeriodicError> {
    let rule = rules.get(block).ok_or(PeriodicError::UnknownBlock(block))?;
    let block = doc.block(block)?;
    if rule.postings.len() != block.lines.len() {
        return Err(PeriodicError::RoundTripMismatch);
    }
    Ok(rule)
}

/// A posting's amount when it has exactly one, which is every posting this
/// module will do arithmetic on.
fn single_amount(posting: &crate::model::Posting) -> Option<&Amount> {
    match posting.amounts.as_slice() {
        [only] => Some(only),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Rendering and validation
// ---------------------------------------------------------------------------

/// An account as a posting line writes it, with the wrapper its type calls for.
fn write_account(account: &str, ptype: PostingType) -> String {
    match ptype {
        PostingType::Regular => account.to_string(),
        PostingType::Virtual => format!("({account})"),
        PostingType::BalancedVirtual => format!("[{account}]"),
    }
}

/// The word hledger writes a [`PeriodExpr`] as — the inverse of
/// `parse::parse_period_expr`, spelled out rather than derived from `Debug` so a
/// rename cannot silently change what lands in a journal.
#[must_use]
pub const fn period_word(period: PeriodExpr) -> &'static str {
    match period {
        PeriodExpr::Daily => "daily",
        PeriodExpr::Weekly => "weekly",
        PeriodExpr::Monthly => "monthly",
        PeriodExpr::Quarterly => "quarterly",
        PeriodExpr::Yearly => "yearly",
    }
}

/// A period expression as written, when it is one Ledgeline models. Must agree
/// with `parse::parse_period_expr`, which is what pins a block's ordinal to its
/// parsed rule's.
fn period_of(text: &str) -> Option<PeriodExpr> {
    match text {
        "daily" => Some(PeriodExpr::Daily),
        "weekly" => Some(PeriodExpr::Weekly),
        "monthly" => Some(PeriodExpr::Monthly),
        "quarterly" => Some(PeriodExpr::Quarterly),
        "yearly" => Some(PeriodExpr::Yearly),
        _ => None,
    }
}

/// One amount as a journal line writes it. The transaction editor's renderer, so
/// the two cannot drift apart about a decimal mark.
fn render_amount(amount: &Amount) -> String {
    crate::edit::render_amount(amount)
}

/// Any ASCII control character. Same test, and same reasoning, as
/// `aliases::has_control`.
fn has_control(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_control())
}

/// Reject an account name this module will not write into a posting line.
///
/// Every rule is one that would make the written line read back as something
/// other than what was asked for — the failure `verify` would catch afterwards,
/// reported up front with a sentence instead.
fn check_account(account: &str) -> Result<(), PeriodicError> {
    let invalid = |why: &str| Err(PeriodicError::Invalid(format!("an account name {why}")));
    if account.is_empty() {
        return invalid("may not be empty");
    }
    if account.len() > MAX_ACCOUNT_BYTES {
        return invalid(&format!(
            "is {} bytes; the limit is {MAX_ACCOUNT_BYTES}",
            account.len()
        ));
    }
    if has_control(account) {
        return invalid("may not contain a control character");
    }
    if account.trim() != account {
        return invalid(
            "may not begin or end with whitespace: hledger trims it, so the value would be \
             written and then read back without it",
        );
    }
    if account.contains(';') || account.contains('#') {
        return invalid("may not contain `;` or `#`, which hledger reads as starting a comment");
    }
    if account.contains("  ") || account.contains('\t') {
        return invalid(
            "may not contain two consecutive spaces or a tab: hledger splits a posting line at \
             the first one, so the rest of the name would be read as an amount",
        );
    }
    if account.contains(['(', ')', '[', ']']) {
        return invalid(
            "may not contain brackets: those mark a posting as virtual, which is a property of \
             the line rather than part of the name",
        );
    }
    Ok(())
}

/// Reject a rule description this module will not write.
fn check_description(description: &str) -> Result<(), PeriodicError> {
    let invalid = |why: &str| Err(PeriodicError::Invalid(format!("a budget rule name {why}")));
    if description.len() > MAX_DESCRIPTION_BYTES {
        return invalid(&format!(
            "is {} bytes; the limit is {MAX_DESCRIPTION_BYTES}",
            description.len()
        ));
    }
    if has_control(description) {
        return invalid("may not contain a control character");
    }
    if description.trim() != description {
        return invalid("may not begin or end with whitespace");
    }
    if description.contains(';') || description.contains('#') {
        return invalid("may not contain `;` or `#`, which hledger reads as starting a comment");
    }
    Ok(())
}

/// Reject an amount this module will not write.
fn check_amount(amount: &Amount) -> Result<(), PeriodicError> {
    let invalid = |why: &str| Err(PeriodicError::Invalid(format!("a budget amount {why}")));
    if amount.cost.is_some() {
        return invalid("may not carry an `@` cost annotation");
    }
    if has_control(&amount.commodity.0) {
        return invalid("may not name a commodity containing a control character");
    }
    if amount.commodity.0.contains([';', '#']) {
        return invalid(
            "may not name a commodity containing `;` or `#`, which hledger reads as starting a \
             comment",
        );
    }
    let rendered = render_amount(amount);
    if rendered.len() > MAX_AMOUNT_BYTES {
        return invalid(&format!(
            "renders to {} bytes; the limit is {MAX_AMOUNT_BYTES}",
            rendered.len()
        ));
    }
    if rendered.contains("  ") || rendered.contains('\t') {
        return invalid(
            "may not render with two consecutive spaces or a tab, which hledger reads as the end \
             of the amount",
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

/// Every `~` block in `text`, with the spans of its parts.
///
/// A `~` rule is top-level, so a block opens only when `~` is at column 1 —
/// exactly the dispatch `parse.rs` performs — and its body is the run of
/// indented, non-blank lines that follows, which is exactly the extent
/// `parse_periodic_transaction` consumes.
///
/// The two must agree on the *set* and the *order*, not merely the shape,
/// because a [`PeriodicBlock::index`] is an ordinal that the caller joins against
/// `Journal::periodic_transactions`. `comment`/`end comment` is the one construct
/// that would break that: hledger swallows everything between them, so a `~` line
/// inside a commented-out block is not a rule at all, and counting it here would
/// shift the index of every rule below it — an edit landing on the wrong rule. So
/// the block is skipped, with the parser's exact (deliberately hazardous)
/// unterminated-block behaviour: it runs to EOF.
fn scan(text: &str) -> (Vec<PeriodicBlock>, Vec<GoalLine>) {
    let mut blocks: Vec<PeriodicBlock> = Vec::new();
    let mut lines: Vec<GoalLine> = Vec::new();
    let mut start = 0;
    let mut number = 0u32;
    let mut commented = false;
    let mut open: Option<PeriodicBlock> = None;

    while start < text.len() {
        let (content_len, full_len) = line_extents(&text[start..]);
        number = number.saturating_add(1);
        let content = &text[start..start + content_len];
        let indented = content.starts_with([' ', '\t']);
        let blank = content.trim().is_empty();

        if commented {
            commented = content.trim() != "end comment";
            start += full_len;
            continue;
        }
        if content.split_whitespace().next() == Some("comment") && !indented {
            // An open block ends at the directive, exactly as it ends at any
            // other column-1 line.
            close(&mut blocks, &mut open);
            commented = true;
            start += full_len;
            continue;
        }

        if let Some(block) = open.as_mut() {
            if indented && !blank {
                block.full.end = start + full_len;
                // A comment-only line belongs to the block (and, to the parser,
                // to the preceding posting) but is not a posting of its own.
                if !content.trim_start().starts_with(';') {
                    let index = lines.len();
                    let at = block.lines.len();
                    let line = goal_line(
                        text,
                        start,
                        content_len,
                        full_len,
                        index,
                        block.index,
                        at,
                        number,
                    );
                    block.lines.push(index);
                    lines.push(line);
                }
                start += full_len;
                continue;
            }
            close(&mut blocks, &mut open);
        }

        if !indented && content.starts_with('~') {
            open = Some(header(
                text,
                start,
                content_len,
                full_len,
                blocks.len(),
                number,
            ));
        }
        start += full_len;
    }
    close(&mut blocks, &mut open);

    // Balance and lock verdicts need the whole block, so they are settled once
    // every line is in hand rather than incrementally.
    for block in &mut blocks {
        let (balance, lock) = classify(block, &lines);
        block.balance = balance;
        block.lock = block.lock.or(lock);
    }
    (blocks, lines)
}

/// `(content length without terminator, full length with it)` for the line at
/// the start of `rest`.
///
/// A `\r\n` file keeps its `\r` out of the line's content and inside its
/// terminator, so a delete removes both and a span never ends mid-terminator.
fn line_extents(rest: &str) -> (usize, usize) {
    match rest.find('\n') {
        Some(at) => (at - usize::from(rest[..at].ends_with('\r')), at + 1),
        None => (rest.len(), rest.len()),
    }
}

/// Finish the open block, if there is one.
fn close(blocks: &mut Vec<PeriodicBlock>, open: &mut Option<PeriodicBlock>) {
    if let Some(block) = open.take() {
        blocks.push(block);
    }
}

/// One `~ PERIODEXPR  [DESCRIPTION]` header.
fn header(
    text: &str,
    base: usize,
    content_len: usize,
    full_len: usize,
    index: usize,
    number: u32,
) -> PeriodicBlock {
    let content = &text[base..base + content_len];
    let after = content.strip_prefix('~').unwrap_or("");
    let main = after.split(';').next().unwrap_or("");
    // hledger requires a two-space gap between the period expression and the
    // description; `split_account_amount` splits on exactly that, which is the
    // same call `parse_periodic_transaction` makes.
    let (period_text, description) = crate::parse::split_account_amount(main.trim());
    let period_text = period_text.trim().to_string();
    let period = period_of(&period_text);
    PeriodicBlock {
        index,
        line: number,
        full: base..base + full_len,
        period,
        period_text,
        description: description.trim().to_string(),
        lines: Vec::new(),
        // Settled by `classify` once the body is known.
        balance: BlockBalance::Free,
        lock: (period.is_none()).then_some(BlockLock::Period),
    }
}

/// One posting line inside a block.
#[allow(clippy::too_many_arguments)]
fn goal_line(
    text: &str,
    base: usize,
    content_len: usize,
    full_len: usize,
    index: usize,
    block: usize,
    at: usize,
    number: u32,
) -> GoalLine {
    let content = &text[base..base + content_len];
    let indent = content.len() - content.trim_start().len();
    let after_indent = &content[indent..];
    // Everything before a `;`. On a posting line hledger DOES read `;` as a
    // comment (unlike on an `alias` line), so this is the parser's own reading.
    let main = after_indent.split(';').next().unwrap_or("");
    let (account_text, amount_text) = crate::parse::split_account_amount(main);

    let lead = amount_text.len() - amount_text.trim_start().len();
    let written = amount_text.trim();
    let amount_start = base + indent + account_text.len() + lead;
    let amount_span = amount_start..amount_start + written.len();

    let (ptype, account) = read_account(account_text.trim());
    let lock = goal_lock(content, written);

    GoalLine {
        index,
        block,
        at,
        line: number,
        span: base..base + content_len,
        full: base..base + full_len,
        amount_span,
        account,
        ptype,
        amount: (!written.is_empty()).then(|| written.to_string()),
        lock,
    }
}

/// An account as written → its posting type and its bare name.
fn read_account(written: &str) -> (PostingType, String) {
    let wrapped = |open: char, close: char| {
        written
            .strip_prefix(open)
            .and_then(|rest| rest.strip_suffix(close))
            .map(|inner| inner.trim().to_string())
    };
    if let Some(inner) = wrapped('(', ')') {
        (PostingType::Virtual, inner)
    } else if let Some(inner) = wrapped('[', ']') {
        (PostingType::BalancedVirtual, inner)
    } else {
        (PostingType::Regular, written.to_string())
    }
}

/// Why this posting line is read-only, if it is.
fn goal_lock(content: &str, amount: &str) -> Option<GoalLock> {
    if has_control(content) {
        return Some(GoalLock::Control);
    }
    if content.len() > MAX_ACCOUNT_BYTES + MAX_AMOUNT_BYTES + MAX_DESCRIPTION_BYTES {
        return Some(GoalLock::TooLong);
    }
    if amount.is_empty() {
        return Some(GoalLock::Inferred);
    }
    if amount.contains('@') {
        return Some(GoalLock::Cost);
    }
    if amount.contains('=') {
        return Some(GoalLock::Assertion);
    }
    // hledger joins the parts of a mixed amount with `+`. A leading sign is not
    // one, so only an interior `+` counts.
    if amount[1..].contains('+') {
        return Some(GoalLock::Multiple);
    }
    None
}

/// A block's balance shape and whole-block lock, from its posting lines.
fn classify(block: &PeriodicBlock, lines: &[GoalLine]) -> (BlockBalance, Option<BlockLock>) {
    let own: Vec<&GoalLine> = block.lines.iter().map(|at| &lines[*at]).collect();
    if own
        .iter()
        .any(|line| line.ptype == PostingType::BalancedVirtual)
    {
        return (BlockBalance::Free, Some(BlockLock::BalancedVirtual));
    }
    let real: Vec<&&GoalLine> = own
        .iter()
        .filter(|line| line.ptype == PostingType::Regular)
        .collect();
    if real.is_empty() {
        return (BlockBalance::Free, None);
    }
    // The commodity is compared as the written symbol rather than as a parsed
    // one: this module does not parse amounts, and two lines whose symbols
    // differ textually are not a pair to do arithmetic across.
    let symbols: Vec<String> = real
        .iter()
        .filter_map(|line| line.amount.as_deref().map(commodity_of))
        .collect();
    if symbols.windows(2).any(|pair| pair[0] != pair[1]) {
        return (BlockBalance::Free, Some(BlockLock::MultiCommodity));
    }
    match real.iter().filter(|line| line.amount.is_none()).count() {
        0 if real.len() >= 2 => (BlockBalance::Explicit, None),
        // One real posting carrying its own amount does not balance, and hledger
        // will not accept the file at all.
        0 => (BlockBalance::Free, Some(BlockLock::Unbalanceable)),
        1 => (BlockBalance::Inferred, None),
        _ => (BlockBalance::Free, Some(BlockLock::Unbalanceable)),
    }
}

/// The commodity symbol in an amount as written: everything that is not part of
/// the number. Textual and deliberately crude — it is used only to ask whether
/// two lines are in the *same* commodity.
fn commodity_of(amount: &str) -> String {
    amount
        .chars()
        .filter(|c| !c.is_ascii_digit() && !matches!(c, '.' | ',' | '-' | '+' | ' ' | '\u{a0}'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AmountStyle, Commodity, CommoditySide};
    use crate::parse::parse_journal;

    /// `$n` with the style every fixture in this repo writes.
    fn usd(quantity: Dec) -> Amount {
        Amount {
            commodity: Commodity("$".into()),
            quantity,
            style: AmountStyle {
                side: CommoditySide::Left,
                spaced: false,
                decimal_mark: Some('.'),
                digit_groups: None,
                precision: 0,
            },
            cost: None,
        }
    }

    /// The whole round trip a caller performs: scan, plan, apply, verify.
    fn edited(text: &str, request: &GoalRequest) -> Result<String, PeriodicError> {
        let journal = parse_journal(text, "t.journal").unwrap();
        let doc = PeriodicDoc::parse(text);
        let plan = plan(&doc, &journal.periodic_transactions, request)?;
        let new_text = doc.apply(&plan)?;
        doc.verify(&plan, &new_text)?;
        // The caller's own check, done here too: the result must still parse.
        parse_journal(&new_text, "t.journal").unwrap();
        Ok(new_text)
    }

    const VIRTUAL: &str = concat!(
        "; a budget\n",
        "\n",
        "~ monthly  household budget\n",
        "    (expenses:food)      $400\n",
        "    (expenses:bus)        $20\n",
        "\n",
        "2026-01-05 grocery\n",
        "    expenses:food   $352\n",
        "    assets:checking\n",
    );

    #[test]
    fn scans_blocks_and_lines_in_parser_order() {
        let doc = PeriodicDoc::parse(VIRTUAL);
        assert_eq!(doc.blocks().len(), 1);
        let block = &doc.blocks()[0];
        assert_eq!(block.line, 3);
        assert_eq!(block.period, Some(PeriodExpr::Monthly));
        assert_eq!(block.description, "household budget");
        assert_eq!(block.balance, BlockBalance::Free);
        assert_eq!(block.lock, None);

        assert_eq!(doc.lines().len(), 2);
        assert_eq!(doc.lines()[0].account, "expenses:food");
        assert_eq!(doc.lines()[0].ptype, PostingType::Virtual);
        assert_eq!(doc.lines()[0].amount.as_deref(), Some("$400"));
        assert_eq!(doc.lines()[1].account, "expenses:bus");

        // The transaction below the rule is NOT a goal line: a block ends at the
        // first line that is not indented.
        assert!(
            doc.lines()
                .iter()
                .all(|line| line.account != "assets:checking")
        );
    }

    /// The ordinals this module hands out must be the parser's, or an edit lands
    /// on a different rule than the one the user was looking at.
    #[test]
    fn block_ordinals_match_the_parsers() {
        let text = concat!(
            "comment\n",
            "~ monthly  commented out\n",
            "    (expenses:ghost)  $1\n",
            "end comment\n",
            "\n",
            "~ monthly  real one\n",
            "    (expenses:food)  $400\n",
            "\n",
            "~ yearly  another\n",
            "    (expenses:tax)   $500\n",
        );
        let journal = parse_journal(text, "t.journal").unwrap();
        let doc = PeriodicDoc::parse(text);
        assert_eq!(doc.blocks().len(), journal.periodic_transactions.len());
        assert_eq!(doc.blocks()[0].description, "real one");
        assert_eq!(journal.periodic_transactions[0].description, "real one");
        assert_eq!(doc.blocks()[1].description, "another");
        assert_eq!(journal.periodic_transactions[1].description, "another");
    }

    /// The headline case: one number changes, the alignment does not.
    #[test]
    fn set_amount_rewrites_only_the_number() {
        let out = edited(
            VIRTUAL,
            &GoalRequest::Set {
                index: 0,
                quantity: Dec::new(450, 0),
            },
        )
        .unwrap();
        assert!(out.contains("    (expenses:food)      $450\n"), "{out}");
        // Everything else is byte-identical, including the second goal's column.
        assert!(out.contains("    (expenses:bus)        $20\n"));
        assert!(out.contains("2026-01-05 grocery\n"));
        assert_eq!(out.lines().count(), VIRTUAL.lines().count());
    }

    /// An `Inferred` block is the case where doing less is doing it right: the
    /// balancing leg has no written amount, so nothing is written to it.
    #[test]
    fn an_inferred_leg_is_left_alone() {
        let text = concat!(
            "~ monthly  budget\n",
            "    expenses:food   $400\n",
            "    assets:checking\n",
        );
        let doc = PeriodicDoc::parse(text);
        assert_eq!(doc.blocks()[0].balance, BlockBalance::Inferred);
        assert_eq!(doc.lines()[1].lock, Some(GoalLock::Inferred));

        let out = edited(
            text,
            &GoalRequest::Set {
                index: 0,
                quantity: Dec::new(450, 0),
            },
        )
        .unwrap();
        assert_eq!(
            out,
            "~ monthly  budget\n    expenses:food   $450\n    assets:checking\n"
        );
    }

    /// An `Explicit` block's counter-leg is rewritten, and by the exact delta.
    #[test]
    fn an_explicit_counter_leg_is_rewritten() {
        let text = concat!(
            "~ monthly  budget\n",
            "    expenses:food      $400\n",
            "    expenses:rent     $1500\n",
            "    assets:checking  $-1900\n",
        );
        let doc = PeriodicDoc::parse(text);
        assert_eq!(doc.blocks()[0].balance, BlockBalance::Explicit);

        let out = edited(
            text,
            &GoalRequest::Set {
                index: 0,
                quantity: Dec::new(450, 0),
            },
        )
        .unwrap();
        assert!(out.contains("    expenses:food      $450\n"), "{out}");
        assert!(out.contains("    expenses:rent     $1500\n"), "{out}");
        assert!(out.contains("    assets:checking  $-1950\n"), "{out}");
    }

    /// Editing the funding leg of a three-way rule is genuinely ambiguous — no
    /// fact in the file says whether food or rent absorbs it — so it is refused.
    #[test]
    fn an_ambiguous_counter_leg_is_refused_not_guessed() {
        let text = concat!(
            "~ monthly  budget\n",
            "    expenses:food      $400\n",
            "    expenses:rent     $1500\n",
            "    assets:checking  $-1900\n",
        );
        let error = edited(
            text,
            &GoalRequest::Set {
                index: 2,
                quantity: Dec::new(-2000, 0),
            },
        )
        .unwrap_err();
        assert_eq!(error, PeriodicError::AmbiguousCounterparty { index: 0 });
    }

    /// A two-posting explicit rule is never ambiguous: the other leg is the only
    /// candidate, whichever one is edited.
    #[test]
    fn a_two_posting_explicit_rule_resolves_either_way() {
        let text = concat!(
            "~ monthly  budget\n",
            "    expenses:food     $400\n",
            "    assets:checking  $-400\n",
        );
        let out = edited(
            text,
            &GoalRequest::Set {
                index: 1,
                quantity: Dec::new(-500, 0),
            },
        )
        .unwrap();
        assert!(out.contains("    expenses:food     $500\n"), "{out}");
        assert!(out.contains("    assets:checking  $-500\n"), "{out}");
    }

    /// A new goal copies the block's indent and right-aligns onto its amount
    /// column, which is how the block was already laid out.
    #[test]
    fn an_appended_goal_imitates_the_blocks_alignment() {
        let out = edited(
            VIRTUAL,
            &GoalRequest::Add {
                block: 0,
                account: "expenses:shopping".into(),
                amount: usd(Dec::new(250, 0)),
            },
        )
        .unwrap();
        assert!(
            out.contains("    (expenses:bus)        $20\n    (expenses:shopping)  $250\n"),
            "{out}"
        );
    }

    /// An account too long to align still gets a line hledger reads correctly:
    /// two spaces is the floor, and a pretty-but-wrong line is not on offer.
    #[test]
    fn an_account_too_long_to_align_falls_back_to_two_spaces() {
        let out = edited(
            VIRTUAL,
            &GoalRequest::Add {
                block: 0,
                account: "expenses:household:cleaning:supplies".into(),
                amount: usd(Dec::new(75, 0)),
            },
        )
        .unwrap();
        assert!(
            out.contains("    (expenses:household:cleaning:supplies)  $75\n"),
            "{out}"
        );
    }

    /// Appending into an explicit block moves the counter-leg by the new amount.
    #[test]
    fn an_appended_goal_funds_itself_from_the_counter_leg() {
        let text = concat!(
            "~ monthly  budget\n",
            "    expenses:food     $400\n",
            "    assets:checking  $-400\n",
        );
        let out = edited(
            text,
            &GoalRequest::Add {
                block: 0,
                account: "expenses:bus".into(),
                amount: usd(Dec::new(20, 0)),
            },
        )
        .unwrap();
        assert!(out.contains("    assets:checking  $-420\n"), "{out}");
        assert!(out.contains("    expenses:bus"), "{out}");
    }

    /// A brand-new block goes at EOF, in the `(account)` idiom, after a blank
    /// line.
    #[test]
    fn a_new_block_is_appended_at_eof() {
        let out = edited(
            VIRTUAL,
            &GoalRequest::AddBlock {
                period: PeriodExpr::Yearly,
                description: "annual budget".into(),
                account: "income:interest".into(),
                amount: usd(Dec::new(-1200, 0)),
            },
        )
        .unwrap();
        assert!(out.starts_with(VIRTUAL), "the original text is untouched");
        assert!(
            out.ends_with("\n~ yearly  annual budget\n    (income:interest)  $-1200\n"),
            "{out}"
        );
    }

    /// The rule this goal belongs to already exists, so the goal joins it: one
    /// block per interval and name, which is decision 1 of
    /// `plans/15-budget-editor.md` and what `docs/budget.md` promises.
    #[test]
    fn a_new_goal_joins_the_rule_that_already_states_its_period_and_name() {
        let out = edited(
            VIRTUAL,
            &GoalRequest::AddBlock {
                period: PeriodExpr::Monthly,
                description: "household budget".into(),
                account: "expenses:shopping".into(),
                amount: usd(Dec::new(250, 0)),
            },
        )
        .unwrap();
        // Inside the block, at the end of its postings, on its own column — the
        // same line `GoalRequest::Add` would have written.
        assert!(
            out.contains("    (expenses:bus)        $20\n    (expenses:shopping)  $250\n"),
            "{out}"
        );
        // And no second header: this is the whole bug.
        assert_eq!(out.matches("~ monthly").count(), 1, "{out}");
        // The blank line between the rule and the ledger below it is still one
        // blank line: an insertion INSIDE a block must not disturb its edges.
        assert!(out.contains("$250\n\n2026-01-05 grocery\n"), "{out}");
    }

    /// Spacing is not part of a rule's identity — but the header is not touched
    /// to prove it. The description is compared trimmed and whitespace-collapsed,
    /// which is what makes a hand-widened separator or a doubled space inside the
    /// name the SAME rule rather than a reason to open a second one.
    #[test]
    fn header_spacing_does_not_make_a_second_rule_and_is_not_rewritten() {
        let text = concat!(
            "~ monthly   monthly    budget\n",
            "    (expenses:food)  $400\n",
        );
        let out = edited(
            text,
            &GoalRequest::AddBlock {
                period: PeriodExpr::Monthly,
                description: "monthly budget".into(),
                account: "expenses:bus".into(),
                amount: usd(Dec::new(20, 0)),
            },
        )
        .unwrap();
        assert_eq!(
            out,
            "~ monthly   monthly    budget\n    (expenses:food)  $400\n    (expenses:bus)    $20\n",
            "the header must come back byte-identical"
        );
    }

    /// Two rules of the same period and name: the first in file order, always.
    /// `plans/15-budget-editor.md` — "a journal that already has two monthly
    /// blocks keeps both; we append to the first and leave the second alone".
    #[test]
    fn the_first_matching_rule_in_file_order_is_the_one_joined() {
        let text = concat!(
            "~ monthly  monthly budget\n",
            "    (expenses:food)  $400\n",
            "\n",
            "~ monthly  monthly budget\n",
            "    (expenses:bus)    $20\n",
        );
        let out = edited(
            text,
            &GoalRequest::AddBlock {
                period: PeriodExpr::Monthly,
                description: "monthly budget".into(),
                account: "expenses:coffee".into(),
                amount: usd(Dec::new(15, 0)),
            },
        )
        .unwrap();
        assert_eq!(
            out,
            concat!(
                "~ monthly  monthly budget\n",
                "    (expenses:food)  $400\n",
                "    (expenses:coffee)  $15\n",
                "\n",
                "~ monthly  monthly budget\n",
                "    (expenses:bus)    $20\n",
            ),
            "the second rule must be left alone"
        );
    }

    /// A rule this module presents read-only is NOT joined: appending to it is
    /// rewriting it, and the whole point of a [`BlockLock`] is that we said we
    /// would not. A new rule is opened instead.
    #[test]
    fn a_locked_rule_is_not_joined_and_a_new_one_is_opened() {
        let text = concat!(
            "~ monthly  monthly budget\n",
            "    [expenses:food]     $400\n",
            "    [assets:checking]  $-400\n",
        );
        let doc = PeriodicDoc::parse(text);
        assert_eq!(doc.blocks()[0].lock, Some(BlockLock::BalancedVirtual));

        let out = edited(
            text,
            &GoalRequest::AddBlock {
                period: PeriodExpr::Monthly,
                description: "monthly budget".into(),
                account: "expenses:bus".into(),
                amount: usd(Dec::new(20, 0)),
            },
        )
        .unwrap();
        assert!(
            out.starts_with(text),
            "the locked rule was rewritten: {out}"
        );
        assert!(
            out.ends_with("\n~ monthly  monthly budget\n    (expenses:bus)  $20\n"),
            "{out}"
        );
    }

    /// Another name, or another period, is another rule. Both halves have to
    /// match, because `--budget=DESCPAT` filters on the description: folding a
    /// goal into a differently-named rule would quietly change which report it
    /// shows up in.
    #[test]
    fn a_rule_with_another_name_or_period_is_not_joined() {
        for request in [
            GoalRequest::AddBlock {
                period: PeriodExpr::Monthly,
                description: "monthly budget".into(),
                account: "expenses:coffee".into(),
                amount: usd(Dec::new(15, 0)),
            },
            GoalRequest::AddBlock {
                period: PeriodExpr::Yearly,
                description: "household budget".into(),
                account: "expenses:coffee".into(),
                amount: usd(Dec::new(15, 0)),
            },
        ] {
            // VIRTUAL holds `~ monthly  household budget` and nothing else.
            let out = edited(VIRTUAL, &request).unwrap();
            assert!(out.starts_with(VIRTUAL), "{out}");
            assert!(out.ends_with("    (expenses:coffee)  $15\n"), "{out}");
        }
    }

    /// A rule states an account's goal once. Asking for a second is refused with
    /// a sentence naming the goal to change instead — on the joining path and on
    /// a direct `Add` alike, because they are the same append.
    ///
    /// Before this was checked, the refusal came from `verify` as a round-trip
    /// failure: it cannot tell a second line of the same account from the first,
    /// so it blamed the engine for what the caller had asked for.
    #[test]
    fn a_second_goal_for_the_same_account_is_refused() {
        for request in [
            GoalRequest::Add {
                block: 0,
                account: "expenses:food".into(),
                amount: usd(Dec::new(50, 0)),
            },
            GoalRequest::AddBlock {
                period: PeriodExpr::Monthly,
                description: "household budget".into(),
                account: "expenses:food".into(),
                amount: usd(Dec::new(50, 0)),
            },
        ] {
            assert_eq!(
                edited(VIRTUAL, &request).unwrap_err(),
                PeriodicError::DuplicateGoal {
                    index: 0,
                    account: "expenses:food".into(),
                },
                "{request:?}"
            );
        }
    }

    /// Removing a rule's only goal removes the rule: a bare `~` header is not a
    /// construct to leave behind, and hledger does not accept one.
    #[test]
    fn deleting_the_last_goal_deletes_the_block() {
        let text = concat!(
            "~ monthly  food\n",
            "    (expenses:food)  $400\n",
            "\n",
            "~ yearly  tax\n",
            "    (expenses:tax)   $500\n",
        );
        let out = edited(text, &GoalRequest::Remove { index: 0 }).unwrap();
        assert_eq!(out, "\n~ yearly  tax\n    (expenses:tax)   $500\n");
    }

    /// Removing one goal of several removes exactly its line.
    #[test]
    fn deleting_one_goal_removes_only_its_line() {
        let out = edited(VIRTUAL, &GoalRequest::Remove { index: 1 }).unwrap();
        assert!(!out.contains("expenses:bus"), "{out}");
        assert!(out.contains("    (expenses:food)      $400\n"), "{out}");
        assert!(out.contains("2026-01-05 grocery\n"), "{out}");
    }

    /// A cost annotation, an assertion and a richer period are all shown and all
    /// read-only — the "when in doubt, opaque" rule, stated as tests.
    #[test]
    fn shapes_this_module_will_not_rewrite_are_locked() {
        let doc = PeriodicDoc::parse(concat!(
            "~ monthly  costed\n",
            "    (assets:brokerage)  10 AAPL @ $150\n",
            "~ monthly  asserted\n",
            "    (assets:cash)  $10 = $500\n",
            "~ every 2 weeks  richer\n",
            "    (expenses:food)  $50\n",
        ));
        assert_eq!(doc.lines()[0].lock, Some(GoalLock::Cost));
        assert_eq!(doc.lines()[1].lock, Some(GoalLock::Assertion));
        assert_eq!(doc.blocks()[2].lock, Some(BlockLock::Period));
        // `line_lock` asks both questions, so a line inside a locked block is
        // read-only even though the line itself is fine.
        assert!(doc.line_lock(2).is_some());
    }

    #[test]
    fn a_locked_line_refuses_a_set_but_allows_a_delete() {
        let text = "~ monthly  costed\n    (assets:brokerage)  10 AAPL @ $150\n";
        let doc = PeriodicDoc::parse(text);
        let set = PeriodicPlan {
            edits: vec![PeriodicEdit::SetAmount {
                index: 0,
                amount: usd(Dec::new(1, 0)),
            }],
        };
        assert!(matches!(
            doc.apply(&set),
            Err(PeriodicError::LockedLine { index: 0, .. })
        ));
        let delete = PeriodicPlan {
            edits: vec![PeriodicEdit::Delete { index: 0 }],
        };
        assert_eq!(doc.apply(&delete).unwrap(), "");
    }

    /// Values that would be written and then read back as something else are
    /// refused up front rather than caught by `verify`.
    #[test]
    fn values_that_would_not_read_back_are_refused() {
        let doc = PeriodicDoc::parse(VIRTUAL);
        for account in [
            "",
            "expenses:a  b", // two spaces: hledger would read `b` as an amount
            "expenses:a\tb",
            "expenses:a ; note",
            "(expenses:a)",
            " expenses:a",
        ] {
            let out = doc.apply(&PeriodicPlan {
                edits: vec![PeriodicEdit::AppendLine {
                    block: 0,
                    account: account.to_string(),
                    ptype: PostingType::Virtual,
                    amount: usd(Dec::new(1, 0)),
                }],
            });
            assert!(
                matches!(out, Err(PeriodicError::Invalid(_))),
                "{account:?} should be refused"
            );
        }
    }

    /// A `\r\n` file keeps its terminators, and a new line gets one to match.
    #[test]
    fn crlf_survives_an_append() {
        let text = "~ monthly  budget\r\n    (expenses:food)  $400\r\n";
        let doc = PeriodicDoc::parse(text);
        let plan = PeriodicPlan {
            edits: vec![PeriodicEdit::AppendLine {
                block: 0,
                account: "expenses:bus".into(),
                ptype: PostingType::Virtual,
                amount: usd(Dec::new(20, 0)),
            }],
        };
        let out = doc.apply(&plan).unwrap();
        doc.verify(&plan, &out).unwrap();
        assert_eq!(
            out,
            "~ monthly  budget\r\n    (expenses:food)  $400\r\n    (expenses:bus)    $20\r\n"
        );
    }

    /// A file whose last line has no terminator gets one, so a new block is not
    /// glued onto it.
    #[test]
    fn a_missing_final_newline_is_supplied() {
        let doc = PeriodicDoc::parse("2026-01-01 x\n    (a)  $1");
        let plan = PeriodicPlan {
            edits: vec![PeriodicEdit::AppendBlock {
                period: PeriodExpr::Monthly,
                description: String::new(),
                account: "expenses:food".into(),
                ptype: PostingType::Virtual,
                amount: usd(Dec::new(400, 0)),
            }],
        };
        let out = doc.apply(&plan).unwrap();
        doc.verify(&plan, &out).unwrap();
        assert_eq!(
            out,
            "2026-01-01 x\n    (a)  $1\n\n~ monthly\n    (expenses:food)  $400\n"
        );
    }

    /// `verify` is a check, not a formality: hand it text that is not what the
    /// plan renders and it must refuse.
    #[test]
    fn verify_refuses_text_the_plan_did_not_produce() {
        let doc = PeriodicDoc::parse(VIRTUAL);
        let plan = PeriodicPlan {
            edits: vec![PeriodicEdit::SetAmount {
                index: 0,
                amount: usd(Dec::new(450, 0)),
            }],
        };
        let tampered = doc.apply(&plan).unwrap().replace("$20", "$25");
        assert_eq!(
            doc.verify(&plan, &tampered),
            Err(PeriodicError::RoundTripMismatch)
        );
    }

    /// A stale handle is a clear refusal rather than an edit to the wrong line.
    #[test]
    fn a_stale_handle_is_refused() {
        let doc = PeriodicDoc::parse(VIRTUAL);
        assert_eq!(
            doc.apply(&PeriodicPlan {
                edits: vec![PeriodicEdit::Delete { index: 9 }]
            }),
            Err(PeriodicError::UnknownLine(9))
        );
        assert_eq!(
            doc.apply(&PeriodicPlan {
                edits: vec![PeriodicEdit::AppendLine {
                    block: 9,
                    account: "expenses:x".into(),
                    ptype: PostingType::Virtual,
                    amount: usd(Dec::new(1, 0)),
                }]
            }),
            Err(PeriodicError::UnknownBlock(9))
        );
    }
}
