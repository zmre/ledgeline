//! Statement of cash flows, indirect method — `plans/27-cash-flow-statement.md`.
//!
//! Net income, then the change in every non-cash balance-sheet account sorted
//! into operating, investing and financing, then the change in cash with the
//! cash balances on either side of it.
//!
//! # Why it always reconciles
//!
//! Every transaction sums to zero. Split the accounts into cash, P&L and
//! everything else on the balance sheet; then, in hledger's signs, over any
//! window,
//!
//! ```text
//! Δcash = −Δ(P&L) − Σ Δ(other) = net income + Σ (−Δ other)
//! ```
//!
//! So each non-cash balance-sheet line shows **−Δ** of its accounts, and the
//! sections add up to the change in cash exactly, commodity by commodity.
//! Three things are needed to keep that identity true, and each is deliberate:
//!
//! - **Non-cash amounts are taken at cost** ([`Amount::at_cost`]). Buying 10
//!   AAPL for $1,500 moves $1,500 of cash, so the brokerage account's change
//!   has to read as $1,500 too, not as 10 AAPL. The statement is therefore
//!   never market-valued.
//! - **Cash stays in the currency actually held,** because a euro account's
//!   balance is euros. Where a cash posting carries a cost (`500 EUR @
//!   $1.142`), its as-held and at-cost figures differ, and the difference is
//!   the **effect of exchange rates** line, as on a GAAP statement. Changing
//!   $571 into 500 EUR shows there as 500 EUR and −$571.
//! - **Unbalanced virtual postings are skipped** (`(account)` with
//!   parentheses). They do not sum to zero with the rest of their
//!   transaction by definition, so they cannot be part of an identity that
//!   rests on transactions balancing.
//! - **Closing entries are skipped** ([`is_closing_entry`]). Leaving out a
//!   transaction that touches no cash removes zero from both sides, so this
//!   changes the presentation (net income is not counted a second time in
//!   equity) without touching the reconciliation.
//!
//! The one thing that can still split the identity is a **posting date** that
//! puts one leg of a transaction in a different period from another. Each
//! period is then off by that leg, though the whole range still agrees.
//! [`CashFlowStatement::reconciles`] reports it rather than hiding it.

use super::ReportError;
use super::account_groups::{BsTerm, declared_bs_terms, declared_groups_from, nearest_declared};
use super::account_types::{AccountType, AccountTypes};
use super::income_statement::Columns;
use super::mixed_amount::MixedAmount;
use super::periods::{BucketSpan, Interval, range_buckets};
use super::prices::PriceDb;
use crate::holdings::{ValuationRole, declared_valuation_roles};
use crate::model::{AccountDeclaration, Commodity, Journal, PostingType, Transaction};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The `account` tag that picks a cash-flow section.
pub const CF_SECTION_TAG: &str = "cfsection";

/// The `account` tag that names the line an account prints on.
pub const CF_GROUP_TAG: &str = "cfgroup";

/// The closed vocabulary of [`CF_SECTION_TAG`].
///
/// Closed and coded for the reason `IsSectionKind` gives: a classification
/// that decides membership must never match English words. There is no
/// "exclude" value on purpose. Leaving a balance-sheet account out would break
/// the reconciliation to cash without saying so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CfSectionKind {
    /// Cash flow from operating activities. Net income heads it.
    Operating,
    /// Cash flow from investing activities.
    Investing,
    /// Cash flow from financing activities.
    Financing,
    /// Counts as cash: in the cash balances, not an adjustment.
    Cash,
}

impl CfSectionKind {
    /// The tag value, as written in a journal.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Operating => "operating",
            Self::Investing => "investing",
            Self::Financing => "financing",
            Self::Cash => "cash",
        }
    }
}

/// Parse a `cfsection:` value; `None` for anything outside the vocabulary.
#[must_use]
pub fn parse_cf_section_tag(value: &str) -> Option<CfSectionKind> {
    match value.trim().to_lowercase().as_str() {
        "operating" => Some(CfSectionKind::Operating),
        "investing" => Some(CfSectionKind::Investing),
        "financing" => Some(CfSectionKind::Financing),
        "cash" => Some(CfSectionKind::Cash),
        _ => None,
    }
}

/// Declared `cfsection:` values per account. Empty or unrecognised values read
/// as no declaration, exactly as `issection:` does; the typo is reported by
/// [`journal_to_tag_diagnostics`](crate::wire::journal_to_tag_diagnostics).
#[must_use]
pub fn declared_cf_sections(accounts: &[AccountDeclaration]) -> BTreeMap<String, CfSectionKind> {
    accounts
        .iter()
        .filter_map(|decl| {
            decl.tags
                .iter()
                .find(|(key, _)| key == CF_SECTION_TAG)
                .and_then(|(_, value)| parse_cf_section_tag(value))
                .map(|kind| (decl.name.0.clone(), kind))
        })
        .collect()
}

/// Everything about a journal's accounts the statement reads, gathered once.
#[derive(Debug, Clone, Default)]
pub struct CfTags {
    /// Declared `type:` per account.
    pub types: BTreeMap<String, AccountType>,
    /// Declared `cfsection:` per account.
    pub sections: BTreeMap<String, CfSectionKind>,
    /// Declared `cfgroup:` per account.
    pub groups: BTreeMap<String, String>,
    /// Declared `bsterm:` per account.
    pub terms: BTreeMap<String, BsTerm>,
    /// Declared `valuation:` per account.
    pub valuation: BTreeMap<String, ValuationRole>,
    /// The journal's base commodity, as the balance sheet picks it. An asset
    /// that holds anything else is an investment, non-current by default.
    pub base: Option<Commodity>,
}

impl CfTags {
    /// Read every tag the statement uses off `accounts`.
    #[must_use]
    pub fn from_declarations(accounts: &[AccountDeclaration]) -> Self {
        Self {
            types: super::account_types::declared_types(&super::account_types::account_decls_from(
                accounts,
            )),
            sections: declared_cf_sections(accounts),
            groups: declared_groups_from(accounts, CF_GROUP_TAG),
            terms: declared_bs_terms(accounts),
            valuation: declared_valuation_roles(accounts),
            base: None,
        }
    }

    /// [`CfTags::from_declarations`], plus the base commodity the balance
    /// sheet would use, from the journal's price directives.
    #[must_use]
    pub fn from_journal(journal: &Journal) -> Self {
        Self {
            base: PriceDb::build(&journal.prices).base_commodity().cloned(),
            ..Self::from_declarations(&journal.accounts)
        }
    }
}

/// What part an account plays in the statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfRole {
    /// Revenue or expense: part of net income, never its own line.
    NetIncome,
    /// A cash account: in the cash balances.
    Cash,
    /// A non-cash balance-sheet account: a line in this section.
    Line(CfSectionKind),
}

/// The account role resolver, first match wins (the plan's resolution table):
///
/// 1. Revenue/expense (gain included) is net income, whatever it is tagged.
/// 2. `cfsection:` on the account or its nearest declared ancestor.
/// 3. Effective type Cash is cash.
/// 4. A `valuation:` adjustment role (accumulated depreciation, a
///    mark-to-market account) is operating: a non-cash item added back.
/// 5. Asset or liability with `bsterm: noncurrent` is investing (assets) or
///    financing (liabilities). With no `bsterm:`, an asset in `investments`
///    (it has held a commodity other than the base one) counts as
///    non-current, exactly as the balance sheet's Investments group does.
/// 6. Any other asset or liability is operating.
/// 7. Equity (conversion included) is financing.
/// 8. No type at all is operating.
pub fn cf_role_resolver<'a>(
    tags: &'a CfTags,
    types: &'a AccountTypes,
    investments: &'a BTreeSet<String>,
) -> impl Fn(&str) -> CfRole + 'a {
    move |account: &str| {
        if types.is_type(account, AccountType::Revenue)
            || types.is_type(account, AccountType::Expense)
        {
            return CfRole::NetIncome;
        }
        if let Some(kind) = nearest_declared(&tags.sections, account) {
            return match kind {
                CfSectionKind::Cash => CfRole::Cash,
                other => CfRole::Line(*other),
            };
        }
        if types.is_cash(account) {
            return CfRole::Cash;
        }
        if nearest_declared(&tags.valuation, account) == Some(&ValuationRole::Adjustment) {
            return CfRole::Line(CfSectionKind::Operating);
        }
        let term = nearest_declared(&tags.terms, account);
        let noncurrent = term == Some(&BsTerm::NonCurrent);
        if types.is_type(account, AccountType::Asset) {
            let noncurrent = noncurrent || (term.is_none() && investments.contains(account));
            return CfRole::Line(if noncurrent {
                CfSectionKind::Investing
            } else {
                CfSectionKind::Operating
            });
        }
        if types.is_type(account, AccountType::Liability) {
            return CfRole::Line(if noncurrent {
                CfSectionKind::Financing
            } else {
                CfSectionKind::Operating
            });
        }
        if types.is_type(account, AccountType::Equity) {
            return CfRole::Line(CfSectionKind::Financing);
        }
        CfRole::Line(CfSectionKind::Operating)
    }
}

/// Whether `txn` is a closing entry: every posting is to a revenue/expense
/// account or an equity account, with at least one of each.
///
/// Structural, never by name, so `equity:retained-earnings` needs no tag. An
/// opening-balance entry always touches an asset or a liability, so it can
/// never be mistaken for one. Safe to skip by construction: such a
/// transaction touches no cash.
pub fn is_closing_entry(txn: &Transaction, types: &AccountTypes) -> bool {
    let mut pnl = false;
    let mut equity = false;
    for posting in &txn.postings {
        let account = posting.account.0.as_str();
        if types.is_type(account, AccountType::Revenue)
            || types.is_type(account, AccountType::Expense)
        {
            pnl = true;
        } else if types.is_type(account, AccountType::Equity) {
            equity = true;
        } else {
            return false;
        }
    }
    pnl && equity
}

/// Inputs to [`cash_flow_statement`].
#[derive(Debug, Clone)]
pub struct CfOpts<'a> {
    /// Inclusive range start.
    pub from: &'a str,
    /// Inclusive range end.
    pub to: &'a str,
    /// One column per bucket, or `None` for a single Total column.
    pub interval: Option<Interval>,
    /// Account lines without a `cfgroup:` roll up to this many segments.
    pub depth: usize,
}

/// Where a line's name came from. Ordered so a group line sorts before an
/// account line of the same name; they stay separate lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CfLineSource {
    /// A `cfgroup:` tag on the account or a declared ancestor.
    Group,
    /// The account itself, rolled up to the report's depth.
    Account,
}

/// One account inside a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfRow {
    /// Full account name.
    pub account: String,
    /// Its contribution (−Δ) per column, and their Total.
    pub amounts: Columns,
}

/// One line of an operating/investing/financing section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfLine {
    /// A `cfgroup:` name verbatim, or the account path at the report's depth.
    pub name: String,
    /// Which of the two it is.
    pub source: CfLineSource,
    /// The accounts behind the line, by name.
    pub rows: Vec<CfRow>,
    /// Summed over `rows`.
    pub total: Columns,
}

/// One of the three activity sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfSection {
    /// Operating, investing or financing.
    pub kind: CfSectionKind,
    /// Display title.
    pub title: String,
    /// Lines by name. Operating's do not include net income, which is
    /// [`CashFlowStatement::net_income`].
    pub lines: Vec<CfLine>,
    /// Net cash from the activity. Operating's includes net income.
    pub total: Columns,
}

/// One cash account inside a cash line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfCashRow {
    /// Full account name.
    pub account: String,
    /// Balance at the start of each column. The Total is the first column's.
    pub beginning: Columns,
    /// Balance at the end of each column. The Total is the last column's.
    pub ending: Columns,
}

/// One line of the cash balances: a `cfgroup:` or an account at depth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfCashLine {
    /// A `cfgroup:` name verbatim, or the account path at the report's depth.
    pub name: String,
    /// Which of the two it is.
    pub source: CfLineSource,
    /// The accounts behind the line, by name.
    pub rows: Vec<CfCashRow>,
    /// Summed over `rows`.
    pub beginning: Columns,
    /// Summed over `rows`.
    pub ending: Columns,
}

/// A statement of cash flows over `[from, to]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashFlowStatement {
    /// Inclusive range start.
    pub from: String,
    /// Inclusive range end.
    pub to: String,
    /// One per column, oldest first.
    pub buckets: Vec<BucketSpan>,
    /// Revenue minus expenses, at cost, closing entries left out.
    pub net_income: Columns,
    /// Operating always; investing and financing when they have a line.
    pub sections: Vec<CfSection>,
    /// Effect of exchange rates on cash: as-held minus at-cost, for cash
    /// postings that carry a cost. Empty in a single-currency journal.
    pub exchange: Columns,
    /// Net income plus every section's lines, plus `exchange`.
    pub net_change: Columns,
    /// The cash balances, by line.
    pub cash: Vec<CfCashLine>,
    /// Cash at the start of each column (Total: the first's).
    pub beginning_cash: Columns,
    /// Cash at the end of each column (Total: the last's).
    pub ending_cash: Columns,
    /// `net_change` equals `ending_cash − beginning_cash` in every column. See
    /// the module docs for the one way it can be false.
    pub reconciles: bool,
}

/// The statement of cash flows.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or bad bucket math.
pub fn cash_flow_statement(
    txns: &[Transaction],
    opts: &CfOpts,
    tags: &CfTags,
) -> Result<CashFlowStatement, ReportError> {
    let buckets = range_buckets(opts.from, opts.to, opts.interval)?;
    let columns = buckets.len();
    let types = AccountTypes::from_declared(tags.types.clone());
    let investments = investments(txns, opts.to, tags.base.as_ref());
    let resolve = cf_role_resolver(tags, &types, &investments);
    let mut roles: HashMap<String, CfRole> = HashMap::new();

    let mut net_income = vec![MixedAmount::new(); columns];
    // Contribution (−Δ) per non-cash account per column, and its section.
    let mut lines: BTreeMap<String, (CfSectionKind, Vec<MixedAmount>)> = BTreeMap::new();
    // Cash: balance before the range, then the change per column.
    let mut opening: BTreeMap<String, MixedAmount> = BTreeMap::new();
    let mut cash_delta: BTreeMap<String, Vec<MixedAmount>> = BTreeMap::new();
    // As-held minus at-cost, for cash postings that carry a cost.
    let mut exchange = vec![MixedAmount::new(); columns];

    for txn in txns.iter().filter(|txn| !is_closing_entry(txn, &types)) {
        for posting in &txn.postings {
            if posting.ptype == PostingType::Virtual {
                continue;
            }
            let date = posting.date.as_deref().unwrap_or(&txn.date);
            if date > opts.to {
                continue;
            }
            let account = posting.account.0.as_str();
            let role = *roles
                .entry(account.to_string())
                .or_insert_with(|| resolve(account));
            let column = bucket_of(&buckets, date);
            for amount in &posting.amounts {
                let (commodity, quantity) = amount.at_cost()?;
                match (role, column) {
                    (CfRole::Cash, None) => opening
                        .entry(account.to_string())
                        .or_default()
                        .accumulate(&amount.commodity, amount.quantity)?,
                    (CfRole::Cash, Some(column)) => {
                        cash_delta
                            .entry(account.to_string())
                            .or_insert_with(|| vec![MixedAmount::new(); columns])[column]
                            .accumulate(&amount.commodity, amount.quantity)?;
                        if amount.cost.is_some() {
                            exchange[column].accumulate(&amount.commodity, amount.quantity)?;
                            exchange[column].accumulate(commodity, quantity.neg()?)?;
                        }
                    }
                    (CfRole::NetIncome, Some(column)) => {
                        net_income[column].accumulate(commodity, quantity.neg()?)?;
                    }
                    (CfRole::Line(kind), Some(column)) => lines
                        .entry(account.to_string())
                        .or_insert_with(|| (kind, vec![MixedAmount::new(); columns]))
                        .1[column]
                        .accumulate(commodity, quantity.neg()?)?,
                    (CfRole::NetIncome | CfRole::Line(_), None) => {}
                }
            }
        }
    }

    let line_key = |account: &str| match nearest_declared(&tags.groups, account) {
        Some(group) => (group.clone(), CfLineSource::Group),
        None => (clamp(account, opts.depth), CfLineSource::Account),
    };

    let net_income = flow(net_income)?;
    let mut sections = Vec::new();
    for (kind, title) in [
        (CfSectionKind::Operating, "Operating activities"),
        (CfSectionKind::Investing, "Investing activities"),
        (CfSectionKind::Financing, "Financing activities"),
    ] {
        let mut by_line: BTreeMap<(String, CfLineSource), Vec<CfRow>> = BTreeMap::new();
        for (account, (_, values)) in lines.iter().filter(|(_, (k, _))| *k == kind) {
            if values.iter().all(MixedAmount::is_zero) {
                continue;
            }
            let (name, source) = line_key(account);
            by_line.entry((name, source)).or_default().push(CfRow {
                account: account.clone(),
                amounts: flow(values.clone())?,
            });
        }
        let lines: Vec<CfLine> = by_line
            .into_iter()
            .map(|((name, source), rows)| {
                let total = sum_columns(rows.iter().map(|row| &row.amounts), columns)?;
                Ok(CfLine {
                    name,
                    source,
                    rows,
                    total,
                })
            })
            .collect::<Result<_, ReportError>>()?;
        if kind != CfSectionKind::Operating && lines.is_empty() {
            continue;
        }
        let lines_total = sum_columns(lines.iter().map(|line| &line.total), columns)?;
        let total = if kind == CfSectionKind::Operating {
            add_columns(&lines_total, &net_income)?
        } else {
            lines_total
        };
        sections.push(CfSection {
            kind,
            title: title.to_string(),
            lines,
            total,
        });
    }
    let exchange = flow(exchange)?;
    let net_change = add_columns(
        &sum_columns(sections.iter().map(|section| &section.total), columns)?,
        &exchange,
    )?;

    // Every cash account with a balance or a movement anywhere in the range.
    let cash_accounts: Vec<&String> = {
        let mut names: Vec<&String> = opening.keys().chain(cash_delta.keys()).collect();
        names.sort();
        names.dedup();
        names
    };
    let mut by_cash_line: BTreeMap<(String, CfLineSource), Vec<CfCashRow>> = BTreeMap::new();
    for account in cash_accounts {
        let mut balance = opening.get(account).cloned().unwrap_or_default();
        let mut beginning = Vec::with_capacity(columns);
        let mut ending = Vec::with_capacity(columns);
        for column in 0..columns {
            beginning.push(tidy(balance.clone()));
            if let Some(delta) = cash_delta.get(account) {
                balance = balance.ma_add(&delta[column])?;
            }
            ending.push(tidy(balance.clone()));
        }
        if beginning.iter().chain(&ending).all(MixedAmount::is_zero) {
            continue;
        }
        let (name, source) = line_key(account);
        by_cash_line
            .entry((name, source))
            .or_default()
            .push(CfCashRow {
                account: account.clone(),
                beginning: point(beginning, Edge::First),
                ending: point(ending, Edge::Last),
            });
    }
    let cash: Vec<CfCashLine> = by_cash_line
        .into_iter()
        .map(|((name, source), rows)| {
            Ok(CfCashLine {
                beginning: sum_points(rows.iter().map(|row| &row.beginning), columns, Edge::First)?,
                ending: sum_points(rows.iter().map(|row| &row.ending), columns, Edge::Last)?,
                name,
                source,
                rows,
            })
        })
        .collect::<Result<_, ReportError>>()?;
    let beginning_cash = sum_points(
        cash.iter().map(|line| &line.beginning),
        columns,
        Edge::First,
    )?;
    let ending_cash = sum_points(cash.iter().map(|line| &line.ending), columns, Edge::Last)?;

    let mut reconciles = true;
    for column in 0..columns {
        let moved = ending_cash.values[column].ma_add(&beginning_cash.values[column].ma_neg()?)?;
        let gap = moved.ma_add(&net_change.values[column].ma_neg()?)?;
        reconciles &= gap.is_zero();
    }

    Ok(CashFlowStatement {
        from: opts.from.to_string(),
        to: opts.to.to_string(),
        buckets,
        net_income,
        sections,
        exchange,
        net_change,
        cash,
        beginning_cash,
        ending_cash,
        reconciles,
    })
}

/// Every account that has held a commodity other than `base` on or before
/// `to`: the balance sheet's Investments signal, read off postings rather
/// than one closing balance so a position sold during the range still counts.
/// Empty when there is no base commodity, as on the balance sheet.
fn investments(txns: &[Transaction], to: &str, base: Option<&Commodity>) -> BTreeSet<String> {
    let Some(base) = base else {
        return BTreeSet::new();
    };
    txns.iter()
        .flat_map(|txn| txn.postings.iter().map(move |posting| (txn, posting)))
        .filter(|(txn, posting)| posting.date.as_deref().unwrap_or(&txn.date) <= to)
        .filter(|(_, posting)| {
            posting
                .amounts
                .iter()
                .any(|amount| &amount.commodity != base)
        })
        .map(|(_, posting)| posting.account.0.clone())
        .collect()
}

/// The column `date` falls in, or `None` before the range (callers have
/// already dropped dates after it). Buckets are contiguous and ascending.
fn bucket_of(buckets: &[BucketSpan], date: &str) -> Option<usize> {
    let index = buckets.partition_point(|bucket| bucket.to.as_str() < date);
    let bucket = buckets.get(index)?;
    (date >= bucket.from.as_str()).then_some(index)
}

/// `account` cut to its first `depth` segments (at least one).
fn clamp(account: &str, depth: usize) -> String {
    account
        .split(':')
        .take(depth.max(1))
        .collect::<Vec<_>>()
        .join(":")
}

/// `ma` with zero commodities dropped.
fn tidy(mut ma: MixedAmount) -> MixedAmount {
    ma.drop_zeros();
    ma
}

/// Flow figures: the Total column is the sum of the columns.
fn flow(values: Vec<MixedAmount>) -> Result<Columns, ReportError> {
    let values: Vec<MixedAmount> = values.into_iter().map(tidy).collect();
    let total = values
        .iter()
        .try_fold(MixedAmount::new(), |acc, ma| acc.ma_add(ma))?;
    Ok(Columns {
        values,
        total: tidy(total),
    })
}

/// Which column a balance's Total takes.
#[derive(Debug, Clone, Copy)]
enum Edge {
    First,
    Last,
}

/// Balance figures: the Total column is the first (or last) column's, never a
/// sum. Adding up month-end balances means nothing.
fn point(values: Vec<MixedAmount>, edge: Edge) -> Columns {
    let total = match edge {
        Edge::First => values.first(),
        Edge::Last => values.last(),
    }
    .cloned()
    .unwrap_or_default();
    Columns { values, total }
}

/// Column-wise sum of flow figures.
fn sum_columns<'a>(
    all: impl Iterator<Item = &'a Columns>,
    columns: usize,
) -> Result<Columns, ReportError> {
    let mut values = vec![MixedAmount::new(); columns];
    for each in all {
        for (into, from) in values.iter_mut().zip(&each.values) {
            into.ma_add_assign(from)?;
        }
    }
    flow(values)
}

/// Column-wise sum of balance figures.
fn sum_points<'a>(
    all: impl Iterator<Item = &'a Columns>,
    columns: usize,
    edge: Edge,
) -> Result<Columns, ReportError> {
    let mut values = vec![MixedAmount::new(); columns];
    for each in all {
        for (into, from) in values.iter_mut().zip(&each.values) {
            into.ma_add_assign(from)?;
        }
    }
    Ok(point(values.into_iter().map(tidy).collect(), edge))
}

/// Two flow figures added column by column.
fn add_columns(left: &Columns, right: &Columns) -> Result<Columns, ReportError> {
    sum_columns([left, right].into_iter(), left.values.len())
}
