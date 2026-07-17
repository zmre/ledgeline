//! The Ledgeline journal data model.
//!
//! These are our own plain, immutable domain types — deliberately serde-free.
//! The [`crate::wire`] layer maps them to hledger-compatible JSON; keeping the
//! model independent means the wire shape can evolve without contaminating the
//! engine's internal representation.

use crate::decimal::Dec;

/// A commodity symbol, e.g. `$`, `EUR`, `AAPL`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    /// Decimal mark character.
    pub decimal_mark: char,
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
    /// Subaccount-inclusive assertion (`=*`). Always `false` here.
    pub inclusive: bool,
    /// Total assertion (`==`). Always `false` here.
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
    /// The posting's account.
    pub account: AccountName,
    /// The posting's amounts (a mixed amount; length 1 for explicit postings).
    pub amounts: Vec<Amount>,
    /// Optional balance assertion.
    pub balance_assertion: Option<BalanceAssertion>,
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
    /// `[first line, line after last posting]`, both at column 1.
    pub source_span: (SourcePos, SourcePos),
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

/// A `P DATE COMMODITY PRICE` market-price directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceDirective {
    /// Price date.
    pub date: String,
    /// The commodity being priced.
    pub commodity: Commodity,
    /// The price amount.
    pub price: Amount,
}

/// A fully-parsed, balanced journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journal {
    /// Absolute path recorded in source positions (environment-specific).
    pub source_name: String,
    /// Transactions in file order.
    pub transactions: Vec<Transaction>,
    /// Account declarations in file order.
    pub accounts: Vec<AccountDeclaration>,
    /// Canonical display style per commodity (from `commodity` directives or
    /// first occurrence).
    pub commodity_styles: Vec<(Commodity, AmountStyle)>,
    /// Market-price directives.
    pub prices: Vec<PriceDirective>,
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
}
