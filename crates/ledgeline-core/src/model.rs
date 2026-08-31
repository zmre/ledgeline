//! The Ledgeline journal data model.
//!
//! These are our own plain, immutable domain types — deliberately serde-free.
//! The [`crate::wire`] layer maps them to hledger-compatible JSON; keeping the
//! model independent means the wire shape can evolve without contaminating the
//! engine's internal representation.

use crate::decimal::{Dec, DecError};
use std::path::PathBuf;

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
    pub digit_groups: Option<DigitGroups>,
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

/// The period of a `~` periodic transaction rule.
///
/// Only hledger's standard fixed intervals are modeled. Richer period
/// expressions (`every 2 weeks`, `every 15th of month`, `from…to…`) are
/// deferred: the parser rejects them with a clear error rather than misreading
/// them.
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

/// A `~ PERIODEXPR  [DESCRIPTION]` periodic transaction rule.
///
/// Its postings are parsed and balanced exactly like a normal transaction's (so
/// an elided balancing posting is inferred). The rule is stored apart from
/// [`Journal::transactions`] and is deliberately never surfaced through the wire
/// `/transactions` view — it supplies budget goals to the budget report, and its
/// position is what lets [`crate::periodic`] edit it in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodicTransaction {
    /// The rule's recurrence period.
    pub period: PeriodExpr,
    /// The rule description: the text after the period expression (separated by
    /// two-or-more spaces). `--budget=DESCPAT` matches a case-insensitive
    /// substring of it. Empty when the rule has no description.
    pub description: String,
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
