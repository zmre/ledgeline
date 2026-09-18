//! The Ledgeline journal data model.
//!
//! These are our own plain, immutable domain types — deliberately serde-free.
//! The [`crate::wire`] layer maps them to hledger-compatible JSON; keeping the
//! model independent means the wire shape can evolve without contaminating the
//! engine's internal representation.

use crate::decimal::{Dec, DecError};
use std::path::PathBuf;
use std::sync::Arc;

/// A commodity symbol, e.g. `$`, `EUR`, `AAPL`.
///
/// `Ord`/`PartialOrd` compare by the inner symbol so a `Commodity` can key a
/// `BTreeMap` (the report engine's `MixedAmount`), giving deterministic,
/// lexically-sorted commodity iteration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Commodity(pub String);

/// A full, colon-delimited account name, e.g. `assets:bank:checking`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountName(pub String);

impl AccountName {
    /// This account and all of its ancestors, most-specific first.
    ///
    /// `a:b:c` yields `["a:b:c", "a:b", "a"]`.
    #[must_use]
    pub fn self_and_ancestors(&self) -> Vec<String> {
        let segments: Vec<&str> = self.0.split(':').collect();
        (1..=segments.len())
            .rev()
            .map(|n| segments[..n].join(":"))
            .collect()
    }
}

/// A transaction's 1-based file-order index (hledger's `tindex`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tindex(pub u32);

/// Clearing status of a transaction or posting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// No status marker.
    Unmarked,
    /// `!` pending.
    Pending,
    /// `*` cleared.
    Cleared,
}

/// Whether a posting is real, an unbalanced virtual (`(a)`), or a balanced
/// virtual (`[a]`) posting. Mirrors hledger's `ptype`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostingType {
    /// A normal, balanced posting.
    Regular,
    /// An unbalanced virtual posting, written `(account)`; excluded from the
    /// transaction balance.
    Virtual,
    /// A balanced virtual posting, written `[account]`; balanced among the
    /// other balanced-virtual postings only.
    BalancedVirtual,
}

/// Which side of the number the commodity symbol is written on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommoditySide {
    /// Symbol on the left, e.g. `$5.00`.
    Left,
    /// Symbol on the right, e.g. `5.00 EUR`.
    Right,
}

/// Digit-group formatting: a separator and the group sizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigitGroups {
    /// Group separator character (e.g. `,` or `.`).
    pub mark: char,
    /// Group sizes; simple thousands grouping is `[3]`.
    pub sizes: Vec<u8>,
}

/// How an amount is rendered: side, spacing, marks, grouping, precision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmountStyle {
    /// Commodity side.
    pub side: CommoditySide,
    /// Whether a space separates the symbol and the number.
    pub spaced: bool,
    /// Decimal mark character, or `None` when the commodity is displayed without
    /// one (hledger's `asdecimalpoint`: `Nothing` for a commodity that only
    /// appears as integers within priced transactions).
    pub decimal_mark: Option<char>,
    /// Digit grouping, if any.
    //
    // `Arc`, not an owned `DigitGroups`: a `commodity` directive's style is
    // cloned into every amount of that commodity (`parse::parse_amount`), so
    // the group vector used to be retained once per AMOUNT while the directive
    // that declared it was charged once against the file size — a 127 KB
    // journal retained 80 MB of group entries. `parse::MAX_DIGIT_GROUPS` still
    // bounds the size of any one `DigitGroups` (the 39 entries an `i128`
    // mantissa can possibly render), but sharing the allocation via `Arc` is
    // what bounds the fan-out. `wire.rs` reads it through `Deref` and needs no
    // edit.
    pub digit_groups: Option<Arc<DigitGroups>>,
    /// Display precision (as-written fractional digit count, or the precision
    /// carried through inference).
    pub precision: u32,
}

/// Whether a cost is per-unit (`@`) or a transaction total (`@@`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostKind {
    /// Per-unit cost (`@`).
    Unit,
    /// Total cost (`@@`).
    Total,
}

/// A cost/price annotation attached to an amount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cost {
    /// Unit vs total.
    pub kind: CostKind,
    /// The price amount itself.
    pub amount: Amount,
}

/// A single-commodity amount with an optional cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amount {
    /// The commodity.
    pub commodity: Commodity,
    /// The exact quantity.
    pub quantity: Dec,
    /// Display style.
    pub style: AmountStyle,
    /// Optional cost annotation.
    pub cost: Option<Box<Cost>>,
}

impl Amount {
    /// What this amount is worth **at cost** — hledger's `-B`/`--cost`: the cost
    /// commodity and the quantity its annotation names, or the amount itself
    /// when it carries none.
    ///
    /// This is the single definition of "at cost" in the engine. The parser
    /// infers elided amounts and verifies transaction balance through it
    /// (`parse::cost_value`), and the grouped balance sheet totals accounts
    /// through it, so that report's check line is *exactly* the residual the
    /// parser would call an imbalance. The two cannot drift into disagreeing
    /// about whether a journal balances.
    ///
    /// A `@@` TOTAL cost is a magnitude and takes the sign of the amount it
    /// annotates (hledger's `amountCost`), so `-3 AAPL @@ $600.00` costs
    /// `$-600.00`.
    ///
    /// # Errors
    /// Returns [`DecError`] on decimal overflow.
    pub fn at_cost(&self) -> Result<(&Commodity, Dec), DecError> {
        let Some(cost) = self.cost.as_deref() else {
            return Ok((&self.commodity, self.quantity));
        };
        let quantity = match cost.kind {
            CostKind::Unit => self.quantity.mul(cost.amount.quantity)?,
            CostKind::Total => {
                let magnitude = cost.amount.quantity.abs()?;
                if self.quantity.mantissa < 0 {
                    magnitude.neg()?
                } else {
                    magnitude
                }
            }
        };
        Ok((&cost.amount.commodity, quantity))
    }
}

/// A 1-based source location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePos {
    /// 1-based line.
    pub line: u32,
    /// 1-based column.
    pub column: u32,
}

/// A `= AMOUNT` balance assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceAssertion {
    /// The asserted amount.
    pub amount: Amount,
    /// Subaccount-inclusive assertion, written with a trailing `*` (`=*`/`==*`):
    /// the asserted balance includes the account's subaccounts.
    pub inclusive: bool,
    /// Total assertion, written `==`/`==*`: asserts the account holds *only* the
    /// asserted commodity, i.e. every other commodity's balance is zero.
    pub total: bool,
    /// Position of the `=` sign.
    pub position: SourcePos,
}

/// A posting within a transaction. After balancing, `amounts` is fully
/// populated (an inferred posting may carry one amount per unbalanced
/// commodity — a mixed amount).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posting {
    /// Posting-level status.
    pub status: Status,
    /// Real / virtual / balanced-virtual (hledger's `ptype`).
    pub ptype: PostingType,
    /// The posting's account.
    pub account: AccountName,
    /// The posting's amounts (a mixed amount; length 1 for explicit postings).
    pub amounts: Vec<Amount>,
    /// Optional balance assertion.
    pub balance_assertion: Option<BalanceAssertion>,
    /// Posting date (hledger's `pdate`), set from a `date:` comment tag and
    /// normalized to ISO `YYYY-MM-DD` (yearless values take the transaction's
    /// year). `None` when the posting has no `date:` tag.
    pub date: Option<String>,
    /// Secondary posting date (`pdate2`), from a `date2:` tag.
    pub date2: Option<String>,
    /// Raw comment text, including a trailing newline, or empty.
    pub comment: String,
    /// The posting's **own** comment tags (not account-inherited ones).
    pub tags: Vec<(String, String)>,
}

/// A journal transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    /// 1-based file order.
    pub index: Tindex,
    /// Primary date, `YYYY-MM-DD`.
    pub date: String,
    /// Secondary date, if any.
    pub date2: Option<String>,
    /// Transaction status.
    pub status: Status,
    /// Optional `(code)`.
    pub code: String,
    /// The full description string (never split on `|`).
    pub description: String,
    /// Raw transaction comment (trailing newline) or empty.
    pub comment: String,
    /// Comment collected immediately before the transaction (empty here).
    pub preceding_comment: String,
    /// Transaction tags parsed from its comment.
    pub tags: Vec<(String, String)>,
    /// The postings, in file order.
    pub postings: Vec<Posting>,
    /// `[first line, line after last posting]`, both at column 1. The lines are
    /// relative to [`source_file`](Self::source_file), NOT to the main journal.
    pub source_span: (SourcePos, SourcePos),
    /// The resolved (absolute, canonicalized when it exists on disk) path of the
    /// file this transaction was parsed from — the same file its `source_span`
    /// lines are relative to. For a transaction in an `include`d file this is the
    /// included file, not the main journal. Purely an internal editing concern:
    /// the wire/report layers key off [`Journal::source_name`] and are unaffected.
    pub source_file: PathBuf,
}

/// An `account NAME  ; tags...` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDeclaration {
    /// The declared account.
    pub name: AccountName,
    /// Tags parsed from the declaration comment.
    pub tags: Vec<(String, String)>,
    /// Raw declaration comment, including a trailing newline, or empty. Mirrors
    /// hledger's `adicomment` (e.g. `"type: C\n"`).
    pub comment: String,
    /// Position of the `account` keyword (column is always 1 for a top-level
    /// directive). Mirrors hledger's `adisourcepos`.
    pub position: SourcePos,
    /// The resolved file this was declared in, like [`Transaction::source_file`].
    pub source_file: PathBuf,
}

/// An `alias OLD = NEW` / `alias /REGEX/ = REPLACEMENT` directive.
///
/// Recorded exactly like [`AccountDeclaration`] — read, modeled and carried, but
/// **not applied**. Ledgeline does not rewrite account names when it reads a
/// journal; hledger does. See [`crate::aliases`] for that decision in full, and
/// for the format-preserving editor over these lines.
///
/// The one thing this type is *for* is the import pipeline: hledger's `alias`
/// directive does not reach a CSV during `hledger import` (verified against
/// 1.52 — the account came through unmapped), but `--alias` does, so the server
/// forwards these on every invocation that reads a statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasDirective {
    /// The left-hand side, whitespace-trimmed. For [`regex`](Self::regex) this is
    /// the pattern **without** its surrounding slashes.
    pub pattern: String,
    /// The right-hand side, whitespace-trimmed and otherwise verbatim to end of
    /// line.
    ///
    /// There is deliberately no comment stripping. `alias a = b ; note` declares
    /// the account literally named `b ; note` — verified against hledger 1.52 —
    /// so treating the `;` as a comment would record a mapping the file does not
    /// contain.
    pub replacement: String,
    /// Whether the left-hand side was written `/REGEX/`.
    pub regex: bool,
    /// The resolved file this was declared in, like [`Transaction::source_file`].
    pub source_file: PathBuf,
    /// Position of the `alias` keyword (column is always 1: a directive is
    /// top-level).
    pub position: SourcePos,
    /// A later `end aliases` **in the same file** closed this alias's scope.
    ///
    /// hledger's aliases are positional and file-scoped: they apply from their
    /// line to the end of their file (flowing into anything `include`d after
    /// them, never back out), and `end aliases` stops them early. An ended alias
    /// is still parsed, listed and editable — it is simply never forwarded to
    /// `--alias`, because `--alias` is global and the user wrote down where this
    /// one stops. See [`Journal::aliases_in_force`].
    pub ended: bool,
}

/// One of hledger's five fixed intervals — the *unit* a period expression
/// counts in, and the same five a report buckets by.
///
/// This type deliberately does NOT describe a whole period expression. It is one
/// word (`weekly`, or the `weeks` in `every 2 weeks`); the multiplier, the
/// anchor and the bounds live in [`PeriodSpec`]. Keeping it a payload-free
/// five-variant enum is what lets it go on meaning "a report interval" to
/// [`crate::reports`] and "a recurrence the editor can write" to
/// [`crate::periodic`], which are the only two questions anything asks of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeriodExpr {
    /// `daily`.
    Daily,
    /// `weekly` (ISO weeks, Mon–Sun).
    Weekly,
    /// `monthly`.
    Monthly,
    /// `quarterly`.
    Quarterly,
    /// `yearly`.
    Yearly,
}

/// Where inside its unit an anchored rule fires.
///
/// Weekdays are ISO numbers — 1 = Monday … 7 = Sunday — matching
/// `reports::periods`' own weekday convention, so an anchor never has to be
/// translated between two numberings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// `every 15th day of month`: the Nth day of each month, clamped to the
    /// month's length (hledger's `every 31st day of month` fires on Feb 28).
    DayOfMonth(u32),
    /// `every 3rd tuesday of month`: the Nth `weekday` of each month.
    NthWeekday {
        /// 1-based ordinal within the month.
        nth: u32,
        /// ISO weekday, 1 = Monday.
        weekday: u32,
    },
    /// `every tuesday` / `every 2nd day of week`: one weekday each week.
    Weekday(u32),
}

/// The shape of a period expression, once its words are understood.
///
/// The variants are the ones hledger's grammar actually produces, not a
/// convenient subset: a journal that hledger opens must open here too, and a
/// form we cannot compute is [`PeriodKind::Unsupported`] — still parsed, still
/// carried, still locked — rather than a refused journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeriodKind {
    /// `daily` … `yearly`, `every month`, `biweekly`, `every 2 weeks`: a fixed
    /// unit with a multiplier (1 unless written otherwise).
    Every {
        /// The unit counted.
        unit: PeriodExpr,
        /// How many units per step; 1 for a bare interval.
        multiplier: u32,
    },
    /// `every 15th day of month`, `every 3rd tuesday of month`, `every tuesday`:
    /// one occurrence per `unit`, on a day the anchor names rather than on the
    /// unit's first day.
    Anchored {
        /// The unit counted — `Monthly` or `Weekly`.
        unit: PeriodExpr,
        /// Which day within it.
        anchor: Anchor,
    },
    /// `every 12/25` — one occurrence a year on a fixed month/day.
    Annual {
        /// 1-12.
        month: u32,
        /// 1-31, clamped to the month's length.
        day: u32,
    },
    /// `~ 2027-03-01`, `~ 2027-03`, `~ from D to D2`: fires exactly once.
    Once,
    /// Syntactically well-formed to hledger (or not), but not a recurrence this
    /// engine can enumerate — `every weekday`, `every weekendday`, anything the
    /// grammar below does not recognise. Kept whole, via [`PeriodSpec::raw`].
    ///
    /// This is the variant that replaces a failed parse. Decision 2 of
    /// `plans/21-periodic-rule-parser.md`: a period expression is one line of one
    /// rule, and refusing to open a 40,000-line ledger over it is not a
    /// proportionate response.
    Unsupported,
}

/// One of hledger's period-expression shapes, as written.
///
/// `raw` is the bytes between `~` and the double space, preserved so the editor
/// can quote a rule it will not rewrite and so a rewrite that does not touch the
/// header is provably byte-identical.
///
/// # The two bounds are not symmetric
///
/// Verified against hledger 1.52 (`bal -M --budget` over scratch journals):
///
/// - `from D` is the **phase anchor**, used raw. `~ monthly from 2026-01-15`
///   fires on the 15th of each month, not on the 1st; `~ monthly from 2026-01-31`
///   fires Jan 31, Feb 28, Mar 31 — i.e. `anchor + i months` with the day clamped
///   per target month, NOT an iterated add (which would stick at Feb 28).
/// - `to D` is an **exclusive filter** on the occurrence date, with no snapping.
///   `~ monthly from 2026 to 2028` yields exactly 24 occurrences, 2026-01 through
///   2027-12.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodSpec {
    /// The period expression exactly as the file writes it, between `~` and the
    /// two-space gap, trimmed.
    pub raw: String,
    /// What the expression says, once understood.
    pub kind: PeriodKind,
    /// The `from` date, normalized to ISO `YYYY-MM-DD` (`from 2027` is
    /// `2027-01-01`). The rule's phase anchor; see the type docs.
    pub start: Option<String>,
    /// The `to` date, normalized to ISO `YYYY-MM-DD`. **EXCLUSIVE**, as hledger
    /// writes it — an occurrence exactly on this date does not fire.
    pub end: Option<String>,
}

impl PeriodSpec {
    /// The fixed interval this rule steps by, or `None` when it is not a **bare**
    /// one.
    ///
    /// "Bare" means all three things at once: one of the five units, no
    /// multiplier, and no `from`/`to`. Anything else — `every 2 weeks`,
    /// `every 15th day of month`, `monthly from 2027` — answers `None`, because
    /// the single word this returns cannot express what it leaves out, and a
    /// caller that wrote the word back into a journal would silently drop the
    /// rest of the rule.
    ///
    /// This is the editor's question and the lock's question. It is deliberately
    /// **not** the goal-generation question: `reports::budget::occurrences` reads
    /// the whole spec, because `~ monthly from 2027 to 2028` does contribute
    /// goals and hledger says which ones.
    #[must_use]
    pub fn interval(&self) -> Option<PeriodExpr> {
        match self.kind {
            PeriodKind::Every {
                unit,
                multiplier: 1,
            } if self.start.is_none() && self.end.is_none() => Some(unit),
            _ => None,
        }
    }

    /// True when the editor can add to / rewrite goals in this rule.
    ///
    /// The same predicate as [`interval`](Self::interval) being `Some`, spelled
    /// separately because it is a different *question* asked by a different
    /// caller: one wants the word, the other wants a yes/no. Should the editor
    /// ever learn to write a bounded header, this is the one place that changes.
    #[must_use]
    pub fn is_simple(&self) -> bool {
        self.interval().is_some()
    }
}

/// A `~ PERIODEXPR  [DESCRIPTION]` periodic transaction rule.
///
/// Its postings are parsed and balanced exactly like a normal transaction's (so
/// an elided balancing posting is inferred). The rule is stored apart from
/// [`Journal::transactions`] and is deliberately never surfaced through the wire
/// `/transactions` view — it supplies budget goals to the budget report, and its
/// position is what lets [`crate::periodic`] edit it in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodicTransaction {
    /// The rule's recurrence, as written and as understood.
    ///
    /// A whole [`PeriodSpec`] rather than a [`PeriodExpr`] beside a bound: two
    /// fields that can disagree about the same fact are a bug waiting for a
    /// maintainer. Callers that only want "which of the five intervals is this"
    /// ask [`PeriodSpec::interval`].
    pub period: PeriodSpec,
    /// The rule description: the text after the period expression (separated by
    /// two-or-more spaces). `--budget=DESCPAT` matches a case-insensitive
    /// substring of it. Empty when the rule has no description.
    pub description: String,
    /// Raw header comment text, including a trailing newline, or empty — the
    /// same shape, and built by the same code, as [`Posting::comment`].
    ///
    /// hledger carries this onto every transaction a `~` rule forecasts, so it is
    /// the one thing a rule's header holds that nothing else can. Postings have
    /// always kept theirs; the rule keeping its own is the removal of an
    /// asymmetry, not a new feature.
    pub comment: String,
    /// Tags parsed from [`comment`](Self::comment), by the same rule as
    /// [`Posting::tags`] — so `; growth: 3%/yr` means the same thing on a rule
    /// header as it does on a posting line.
    pub tags: Vec<(String, String)>,
    /// The rule's postings, after amount inference/balancing.
    pub postings: Vec<Posting>,
    /// `[first line, line after last posting]`, both at column 1, exactly as
    /// [`Transaction::source_span`] is defined — and relative to
    /// [`source_file`](Self::source_file), not to the main journal.
    ///
    /// A rule with no position could be reported but never edited: the budget
    /// editor has to be able to say *which* `~` block in *which* file a goal
    /// came from before it will rewrite a byte of it.
    pub source_span: (SourcePos, SourcePos),
    /// The resolved (absolute, canonicalized when it exists on disk) path of the
    /// file this rule was parsed from. Same meaning, and same purpose, as
    /// [`Transaction::source_file`].
    pub source_file: PathBuf,
}

/// A `P DATE COMMODITY PRICE` market-price directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceDirective {
    /// Price date.
    pub date: String,
    /// The commodity being priced.
    pub commodity: Commodity,
    /// The price amount.
    pub price: Amount,
    /// The resolved file this was declared in, like [`Transaction::source_file`].
    /// For a directive INFERRED from a cost annotation (never written to disk,
    /// never part of [`Journal::prices`]) this is the transaction's own file —
    /// the natural owner of a price it implies.
    pub source_file: PathBuf,
}

/// A fully-parsed, balanced journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journal {
    /// Absolute path recorded in source positions (environment-specific).
    pub source_name: String,
    /// Every file that fed this journal: the main file first, then each
    /// `include`d file in first-read order, each as a resolved (canonicalized
    /// when it exists on disk) absolute path, deduplicated. Unlike
    /// [`Transaction::source_file`], this also covers `include`d files that
    /// contribute only directives (no transactions), so a live-reload watcher
    /// can monitor the complete set of files the journal depends on.
    pub source_files: Vec<PathBuf>,
    /// Transactions in file order.
    pub transactions: Vec<Transaction>,
    /// Periodic (`~`) transaction rules in file order. Kept out of
    /// `transactions` (and thus the wire `/transactions` view); consumed only by
    /// the budget report.
    pub periodic_transactions: Vec<PeriodicTransaction>,
    /// Account declarations in file order.
    pub accounts: Vec<AccountDeclaration>,
    /// `alias` directives in file order. Recorded, never applied — see
    /// [`AliasDirective`].
    pub aliases: Vec<AliasDirective>,
    /// Canonical display style per commodity (from `commodity` directives or
    /// first occurrence).
    pub commodity_styles: Vec<(Commodity, AmountStyle)>,
    /// Tags declared on `commodity` directives, in declaration order. hledger
    /// propagates these to the `ptags` of postings whose amounts use that
    /// commodity (account and posting tags of the same name take precedence).
    pub commodity_tags: Vec<(Commodity, Vec<(String, String)>)>,
    /// Market-price directives.
    pub prices: Vec<PriceDirective>,
    /// The commodity declared by a `D AMOUNT` default-commodity directive (the
    /// last one wins), if any.
    ///
    /// hledger uses it only to give bare-number amounts a commodity, which the
    /// parser already does. It is kept here because it is also the one place a
    /// journal states, in the author's own words, which commodity it is
    /// denominated in — so a report that has to pick a single valuation
    /// commodity can prefer it over guessing from price-directive frequency
    /// (see `holdings::HoldingsScope::value_in`).
    pub default_commodity: Option<Commodity>,
    /// The MAIN file's leading comment: the text of its first non-empty line
    /// when that line is a comment, with the marker and surrounding whitespace
    /// stripped. `None` when the file opens with anything else.
    ///
    /// The parser discards every other comment that is not attached to a
    /// transaction, posting or declaration. This one is retained because it is
    /// the one place a journal states, in the author's own words, WHOSE books
    /// it is — `; Acme Books`, `; Personal ledger 2026`. Every other fact about
    /// a journal is derived from its ledger; this is the file's own label for
    /// itself, and it is what [`crate::title`] prefers over anything guessed
    /// from a path.
    ///
    /// Only the main file contributes. An `include`d file's header describes
    /// that file, not the journal the user opened.
    pub leading_comment: Option<String>,
}

impl Journal {
    /// Look up the declared tags for an exact account name.
    #[must_use]
    pub fn account_tags(&self, account: &str) -> Option<&[(String, String)]> {
        self.accounts
            .iter()
            .find(|decl| decl.name.0 == account)
            .map(|decl| decl.tags.as_slice())
    }

    /// The aliases still in force where a new entry would be appended, in file
    /// order — i.e. every one whose scope was not closed by an `end aliases`.
    ///
    /// This is the set the import pipeline forwards as `--alias`, and the rule
    /// is chosen to match what the user would get by *typing* the transaction
    /// instead of importing it: `hledger import` appends, and an alias in force
    /// at the append point is one that would have applied to it.
    ///
    /// Scope is honoured per file only as far as `end aliases`. Ledgeline does
    /// **not** work out whether an alias declared in one file would reach the
    /// particular file an import writes to, because `--alias` has no per-file
    /// form to express that with — so the set is journal-wide, and the UI shows
    /// it rather than applying it invisibly.
    pub fn aliases_in_force(&self) -> impl Iterator<Item = &AliasDirective> {
        self.aliases.iter().filter(|alias| !alias.ended)
    }
}
