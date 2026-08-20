//! Income statement / P&L — port of `web/src/lib/reports/incomeStatement.ts`,
//! plus the grouped/valued adaptive-GAAP report of
//! `plans/13-income-statement-redesign.md`.

use super::ReportError;
use super::account_groups::{GroupSource, humanized_segment, nearest_declared};
use super::account_types::{AccountType, AccountTypes};
use super::aggregate::{PostingFilter, account_totals, at_depth, roll_up};
use super::balance_sheet::{Valuation, signed, valued_keeping_unpriced};
use super::mixed_amount::MixedAmount;
use super::periods::{add_days, days_between};
use super::prices::{PriceDb, ValuationMeta, priced_count};
use super::sections::build_section;
use super::types::{ReportMeta, SectionedReport};
use crate::model::{AccountDeclaration, Commodity, Journal, PriceDirective, Transaction};
use std::collections::{BTreeMap, BTreeSet};

/// Revenues + expenses over `[from, to]` (both INCLUSIVE). Presentation matches
/// `hledger is`: revenues are sign-flipped (positive = earned); `grand_total` =
/// revenues(displayed) − expenses = net income.
///
/// `declared` carries the journal's `type:` declarations so a cost booked
/// outside an `expenses:` root still counts as an expense.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn income_statement(
    txns: &[Transaction],
    from: &str,
    to: &str,
    depth: usize,
    declared: &BTreeMap<String, AccountType>,
) -> Result<SectionedReport, ReportError> {
    let direct = account_totals(
        txns,
        &PostingFilter {
            from: Some(from),
            to: Some(to),
            ..PostingFilter::default()
        },
    )?;
    let clamped = at_depth(&roll_up(&direct)?, depth);
    let revenues = build_section(
        "Revenues",
        AccountType::Revenue,
        &direct,
        &clamped,
        declared,
        true,
    )?;
    let expenses = build_section(
        "Expenses",
        AccountType::Expense,
        &direct,
        &clamped,
        declared,
        false,
    )?;
    let grand_total = revenues.total.ma_add(&expenses.total.ma_neg()?)?;
    Ok(SectionedReport {
        as_of: None,
        from: Some(from.to_string()),
        to: Some(to.to_string()),
        sections: vec![revenues, expenses],
        grand_total,
    })
}

// ===========================================================================
// Grouped, valued income statement — plans/13-income-statement-redesign.md
// ===========================================================================

/// The `account` tag that picks an income-statement box.
pub const IS_SECTION_TAG: &str = "issection";

/// One of the seven boxes an income-statement account can land in — the closed
/// vocabulary of the [`IS_SECTION_TAG`] tag.
///
/// Closed, and CODED, for the reason recorded in `account-type-not-name`: a
/// classification that decides MEMBERSHIP must never match English words,
/// because the failure mode is a section that reads zero and a chart of accounts
/// may be in any language. `isgroup:` is free text for the same reason inverted
/// — it only decides which LINE an account prints on inside a box it is already
/// in, so there is no table for it to fail to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IsSectionKind {
    /// Revenue / turnover. Displayed sign-flipped, so it reads positive.
    Revenue,
    /// Cost of revenue (COGS) — what Gross profit is measured against.
    Cogs,
    /// Operating expenses; titled plain "Expenses" in the simple two-box form.
    Opex,
    /// Depreciation & amortization, split out of `opex` so EBITDA can exist.
    Depreciation,
    /// Interest expense.
    Interest,
    /// Income taxes.
    Tax,
    /// Everything non-operating, revenue and expense alike. The one genuinely
    /// MIXED box — a grant and a lawsuit settlement can share it — so it is
    /// presented as a signed net contribution and is allowed to print negative.
    Other,
}

/// A subtotal ruled between two boxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsSubtotalKind {
    /// Revenue − cost of revenue.
    GrossProfit,
    /// Gross profit − operating expenses, ABOVE the D&A box.
    Ebitda,
    /// EBITDA − depreciation & amortization.
    OperatingIncome,
    /// Operating income + other income & expense − interest.
    PretaxIncome,
}

/// Inputs to [`income_statement_grouped`].
#[derive(Debug, Clone)]
pub struct IsOpts<'a> {
    /// Inclusive range start.
    pub from: &'a str,
    /// Inclusive range end.
    pub to: &'a str,
    /// The basis every displayed number is on.
    pub value: Valuation,
    /// Override the valuation target; defaults to `prices.base_commodity()`.
    pub value_in: Option<Commodity>,
    /// Also report the immediately preceding window of equal length.
    pub compare: bool,
}

/// An inclusive date range — the window a set of `prior` figures covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateRange {
    /// Inclusive start.
    pub from: String,
    /// Inclusive end.
    pub to: String,
}

/// One figure per window: the current period's, plus the prior window's when
/// comparing.
///
/// `prior` is `None` for the WHOLE report or for none of it — it mirrors
/// [`IsOpts::compare`] — which is why the merge below can be a plain zip: a line
/// present in only one period is a `MixedAmount::new()` on the other side, never
/// a missing `prior`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Amounts {
    /// The reported window's figure.
    pub current: MixedAmount,
    /// The equal-length preceding window's, when comparing.
    pub prior: Option<MixedAmount>,
}

impl Amounts {
    /// The additive identity, shaped for the same windows as the report.
    #[must_use]
    fn zero(compare: bool) -> Self {
        Self {
            current: MixedAmount::new(),
            prior: compare.then(MixedAmount::new),
        }
    }

    /// Commodity-wise addition, window by window. A window present on only one
    /// side is carried through rather than dropped, so this is total even though
    /// the shapes always agree by construction.
    fn ma_add(&self, other: &Self) -> Result<Self, ReportError> {
        Ok(Self {
            current: self.current.ma_add(&other.current)?,
            prior: match (&self.prior, &other.prior) {
                (Some(mine), Some(theirs)) => Some(mine.ma_add(theirs)?),
                (Some(only), None) | (None, Some(only)) => Some(only.clone()),
                (None, None) => None,
            },
        })
    }

    /// Negated in every window.
    fn ma_neg(&self) -> Result<Self, ReportError> {
        self.signed(true)
    }

    /// Negated in every window when the section is displayed sign-flipped —
    /// [`super::balance_sheet::signed`], applied per window so the two
    /// statements cannot disagree about what a flip is.
    fn signed(&self, flip: bool) -> Result<Self, ReportError> {
        Ok(Self {
            current: signed(&self.current, flip)?,
            prior: self
                .prior
                .as_ref()
                .map(|prior| signed(prior, flip))
                .transpose()?,
        })
    }

    /// True when every window is empty.
    fn is_zero(&self) -> bool {
        self.current.is_zero() && self.prior.as_ref().is_none_or(MixedAmount::is_zero)
    }
}

/// One account's line inside an expanded group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsRow {
    /// Full, colon-delimited account name.
    pub account: String,
    /// Number of `:`-separated segments in `account`.
    pub depth: usize,
    /// The account's own total, displayed with its section's sign.
    pub amounts: Amounts,
}

/// One line of a box: an `isgroup:` name or a humanized path segment, plus the
/// accounts behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsGroup {
    /// Display name — an `isgroup:` tag verbatim, or a humanized segment.
    pub name: String,
    /// Which resolution step named it ([`GroupSource::Tag`] or
    /// [`GroupSource::Segment`]; this statement has no built-in groups).
    pub source: GroupSource,
    /// The group's accounts, sorted by name. There is no depth clamp on this
    /// report, so these are simply its members.
    pub rows: Vec<IsRow>,
    /// Summed over the group's MEMBERS, not over `rows` (RPT-1/RPT-4) — the two
    /// coincide here, and the test that says so is what keeps them coinciding.
    pub total: Amounts,
}

/// A subtotal ruled beneath the box it follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsSubtotal {
    /// Which rung of the ladder.
    pub kind: IsSubtotalKind,
    /// Display label.
    pub label: String,
    /// A running total of every box printed ABOVE it — never of anything below.
    pub total: Amounts,
}

/// One box of the statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsSection {
    /// Which box.
    pub kind: IsSectionKind,
    /// Display title (which, for `opex` alone, depends on
    /// [`IncomeStatementReport::multi_step`]).
    pub title: String,
    /// Lines, sorted by name.
    pub groups: Vec<IsGroup>,
    /// Summed over the section's MEMBERS, not over `groups`.
    pub total: Amounts,
    /// Subtotals printed after this box. They hang off a section rather than
    /// floating in the section list so a subtotal can never outlive the box it
    /// follows — when a box is omitted, its subtotals fall to the previous one.
    pub trailing: Vec<IsSubtotal>,
}

/// A grouped, valued income statement with an adaptive GAAP subtotal ladder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomeStatementReport {
    /// Inclusive range start.
    pub from: String,
    /// Inclusive range end.
    pub to: String,
    /// The window the `prior` figures cover, when comparing.
    pub prior: Option<DateRange>,
    /// The commodity everything is valued into — `Some` only when
    /// [`Valuation::Market`] found a target.
    pub base: Option<Commodity>,
    /// Non-empty sections only, in ladder order.
    pub sections: Vec<IsSection>,
    /// `−sum(every classified account)`, in each window.
    pub net_income: Amounts,
    /// Whether the GAAP ladder materialised — see [`multi_step`].
    pub multi_step: bool,
    /// Commodities the market valuation could not reach (sorted, deduped).
    pub meta: ReportMeta,
}

/// Parse an [`IS_SECTION_TAG`] value, case-insensitively; `None` when it is not
/// one of the seven codes.
///
/// Deliberately WITHOUT the forgiving superset
/// [`super::account_types::parse_account_type_tag`] grew. That one had to accept
/// plurals because hledger rejects the directive outright and ours could not, so
/// the choice was between a superset and a silent misfiling. Here there is no
/// such constraint — this tag is ours alone — so the third option is available
/// and is the right one: [`account_sections_from`] turns anything unrecognised
/// into a [`ReportError::UnknownIsSection`] naming the account, the value and
/// the alternatives.
#[must_use]
pub fn parse_is_section_tag(value: &str) -> Option<IsSectionKind> {
    match value.trim().to_lowercase().as_str() {
        "revenue" => Some(IsSectionKind::Revenue),
        "cogs" => Some(IsSectionKind::Cogs),
        "opex" => Some(IsSectionKind::Opex),
        "depreciation" => Some(IsSectionKind::Depreciation),
        "interest" => Some(IsSectionKind::Interest),
        "tax" => Some(IsSectionKind::Tax),
        "other" => Some(IsSectionKind::Other),
        _ => None,
    }
}

/// Declared `issection:` values per account, read off a parsed journal's
/// `account` directives — the section analogue of
/// [`super::account_types::declared_types`].
///
/// # Errors
/// Returns [`ReportError::UnknownIsSection`] for a value outside the closed
/// vocabulary.
pub fn account_sections(journal: &Journal) -> Result<BTreeMap<String, IsSectionKind>, ReportError> {
    account_sections_from(&journal.accounts)
}

/// [`account_sections`] over a bare slice of declarations.
///
/// An EMPTY value reads as no declaration at all, exactly as
/// [`super::account_groups::declared_groups_from`] treats one: `; issection:`
/// with nothing after it names no section, and the two tag readers answering
/// differently about the same syntax would be its own trap.
///
/// # Errors
/// Returns [`ReportError::UnknownIsSection`] for a non-empty value outside the
/// closed vocabulary.
pub fn account_sections_from(
    accounts: &[AccountDeclaration],
) -> Result<BTreeMap<String, IsSectionKind>, ReportError> {
    accounts
        .iter()
        .filter_map(|decl| {
            decl.tags
                .iter()
                .find(|(key, _)| key == IS_SECTION_TAG)
                .map(|(_, value)| value.trim())
                .filter(|value| !value.is_empty())
                .map(|value| {
                    parse_is_section_tag(value)
                        .map(|kind| (decl.name.0.clone(), kind))
                        .ok_or_else(|| ReportError::UnknownIsSection {
                            account: decl.name.0.clone(),
                            value: value.to_string(),
                        })
                })
        })
        .collect()
}

/// One window's per-account figures on the display basis.
type Totals = BTreeMap<String, MixedAmount>;

/// When a subtotal is worth printing.
enum Guard {
    /// Only when the named box renders at all.
    Section(IsSectionKind),
    /// Only once the ladder has materialised.
    MultiStep,
}

impl Guard {
    fn holds(&self, present: &BTreeSet<IsSectionKind>, multi_step: bool) -> bool {
        match self {
            Self::Section(kind) => present.contains(kind),
            Self::MultiStep => multi_step,
        }
    }
}

/// One rung of the ladder: a box, what it is called, whether it is displayed
/// sign-flipped, and the subtotal ruled beneath it.
struct Rung {
    kind: IsSectionKind,
    /// Title in the simple two-box form.
    simple_title: &'static str,
    /// Title once the ladder has materialised, when it differs.
    multi_step_title: Option<&'static str>,
    /// Displayed as `−sum` rather than `+sum`.
    flip: bool,
    /// `(kind, label, guard)` for the subtotal printed after this box.
    trailing: Option<(IsSubtotalKind, &'static str, Guard)>,
}

impl Rung {
    fn title(&self, multi_step: bool) -> &'static str {
        match self.multi_step_title {
            Some(title) if multi_step => title,
            _ => self.simple_title,
        }
    }
}

/// The statement, in presentation order.
///
/// # Why EBITDA sits above D&A
///
/// It makes every subtotal a running total of everything printed above it, which
/// is the order a real statement uses and the property that stops any line being
/// the sum of things both above and below it. EBITDA is suppressed when there is
/// no D&A box because it would then be numerically identical to Operating
/// income — the duplicate-total complaint this whole report exists to fix.
///
/// # Why `flip` is one field and not two
///
/// A section's contribution to net income is `−sum(members)` for ALL seven,
/// whatever `flip` says: `flip` negates the DISPLAYED figure, and for a flipped
/// section the contribution is `+displayed` while for an unflipped one it is
/// `−displayed`. Both reduce to `−sum`. So `flip` is purely presentational and
/// the ladder arithmetic never consults it — which is what makes `other`, the
/// mixed box, need no special case at all.
const LADDER: [Rung; 7] = [
    Rung {
        kind: IsSectionKind::Revenue,
        simple_title: "Revenue",
        multi_step_title: None,
        flip: true,
        trailing: None,
    },
    Rung {
        kind: IsSectionKind::Cogs,
        simple_title: "Cost of revenue",
        multi_step_title: None,
        flip: false,
        trailing: Some((
            IsSubtotalKind::GrossProfit,
            "Gross profit",
            Guard::Section(IsSectionKind::Cogs),
        )),
    },
    Rung {
        kind: IsSectionKind::Opex,
        // The same section either way: only the label moves, so no account
        // changes box when a journal grows its first `cogs:` tag.
        simple_title: "Expenses",
        multi_step_title: Some("Operating expenses"),
        flip: false,
        trailing: Some((
            IsSubtotalKind::Ebitda,
            "EBITDA",
            Guard::Section(IsSectionKind::Depreciation),
        )),
    },
    Rung {
        kind: IsSectionKind::Depreciation,
        simple_title: "Depreciation & amortization",
        multi_step_title: None,
        flip: false,
        trailing: Some((
            IsSubtotalKind::OperatingIncome,
            "Operating income",
            Guard::MultiStep,
        )),
    },
    Rung {
        kind: IsSectionKind::Other,
        simple_title: "Other income & expense",
        multi_step_title: None,
        flip: true,
        trailing: None,
    },
    Rung {
        kind: IsSectionKind::Interest,
        simple_title: "Interest",
        multi_step_title: None,
        flip: false,
        trailing: Some((
            IsSubtotalKind::PretaxIncome,
            "Income before taxes",
            Guard::Section(IsSectionKind::Tax),
        )),
    },
    Rung {
        kind: IsSectionKind::Tax,
        simple_title: "Income taxes",
        multi_step_title: None,
        flip: false,
        trailing: None,
    },
];

/// Both windows' figures, keyed by account.
struct Windows {
    current: Totals,
    prior: Option<Totals>,
}

impl Windows {
    /// Whether a prior window is being reported.
    fn comparing(&self) -> bool {
        self.prior.is_some()
    }

    /// One account's figure in each window. ABSENT reads as zero, which is
    /// precisely what a line present in only one period must show on the other
    /// side — the union merge, in one line.
    fn amounts(&self, account: &str) -> Amounts {
        Amounts {
            current: self.current.get(account).cloned().unwrap_or_default(),
            prior: self
                .prior
                .as_ref()
                .map(|totals| totals.get(account).cloned().unwrap_or_default()),
        }
    }

    /// Commodity-wise sum over `accounts`, in each window.
    ///
    /// EVERY total in this report comes through here, over a set of MEMBER
    /// account names — never over displayed rows — so a group total, a section
    /// total and net income cannot disagree (RPT-1/RPT-4).
    fn sum(&self, accounts: &BTreeSet<String>) -> Result<Amounts, ReportError> {
        accounts
            .iter()
            .try_fold(Amounts::zero(self.comparing()), |acc, account| {
                acc.ma_add(&self.amounts(account))
            })
    }
}

/// The immediately preceding window of EQUAL length: `prior_to = from − 1 day`,
/// `prior_from = prior_to − (to − from)`.
///
/// No calendar special-casing. A full calendar year already yields the prior
/// calendar year (`2026-01-01..2026-12-31` → `2025-01-01..2025-12-31`), and
/// every other range gets an honest apples-to-apples duration whose dates the
/// caller can put in the column header.
fn prior_window(current: &DateRange) -> DateRange {
    let to = add_days(&current.from, -1);
    let span = days_between(&current.from, &current.to);
    DateRange {
        from: add_days(&to, -span),
        to,
    }
}

/// One window's per-account figures on the display basis, restricted to the
/// accounts this statement shows.
///
/// The window is valued at ITS OWN end, so the prior column agrees with the
/// `hledger is -V` you would have run over that range last year. Valuing both at
/// the report's `to` would make a cleaner change column and a prior column that
/// disagrees with the report it is being compared against; parity wins.
fn window_totals(
    txns: &[Transaction],
    window: &DateRange,
    opts: &IsOpts,
    prices: &PriceDb,
    target: Option<&Commodity>,
    on_statement: &impl Fn(&str) -> bool,
    meta: &mut ValuationMeta,
) -> Result<Totals, ReportError> {
    let direct = account_totals(
        txns,
        &PostingFilter {
            from: Some(&window.from),
            to: Some(&window.to),
            at_cost: opts.value == Valuation::Cost,
            ..PostingFilter::default()
        },
    )?;
    // RPT-2: membership is decided on the DIRECT totals, before anything is
    // valued or summed. Narrowing here also keeps `meta.unpriced` to commodities
    // the reader can actually see on this statement.
    let on_sheet = direct
        .into_iter()
        .filter(|(account, _)| on_statement(account));
    match (opts.value, target) {
        (Valuation::Market, Some(base)) => on_sheet
            .map(|(account, ma)| {
                Ok((
                    account,
                    valued_keeping_unpriced(&ma, base, prices, &window.to, meta)?,
                ))
            })
            .collect(),
        _ => Ok(on_sheet.collect()),
    }
}

/// The segment index the untagged fallback names a group from: the first
/// segment AFTER the prefix every member of the section shares, capped at
/// `min_segments − 1`.
///
/// The cap is what makes the rule total — it guarantees at least one segment
/// remains for the shortest member, so a direct posting to a section root still
/// gets a line rather than an empty name.
///
/// | Section members | shared | index |
/// |---|---|---|
/// | `income:salary`, `income:dividends` | `income` | 1 (Salary, Dividends) |
/// | `expenses:food:groceries`, `expenses:housing:rent` | `expenses` | 1 (Food, Housing) |
/// | `cogs:materials`, `expenses:rent` | *(none)* | 0 (Cogs, Expenses) |
/// | `expenses`, `expenses:food:groceries` | *(capped)* | 0 (Expenses) |
///
/// On a single-rooted chart — which `assets:`/`liabilities:`/`equity:` always
/// are — this is EXACTLY the balance sheet's "second path segment", so the two
/// statements group alike rather than contradicting each other.
fn group_segment_index(members: &BTreeSet<String>) -> usize {
    let mut names = members.iter();
    let Some(first) = names.next() else {
        return 0;
    };
    let first: Vec<&str> = first.split(':').collect();
    let common = names.fold(first.len(), |common, account| {
        account
            .split(':')
            .zip(first.iter().copied())
            .take(common)
            .take_while(|(segment, shared)| segment == shared)
            .count()
    });
    let min_segments = members
        .iter()
        .map(|account| account.split(':').count())
        .min()
        .unwrap_or(1);
    common.min(min_segments.saturating_sub(1))
}

/// The line an account prints on inside its box: `isgroup:` on the account
/// itself, else on its nearest declared ANCESTOR (it inherits, like `type:`),
/// else the humanized segment at `index`.
///
/// There is no type or commodity step here, unlike the balance sheet's: an
/// income-statement account has already been placed in a box by `issection:`,
/// and neither signal would say anything further about which line it belongs on.
fn resolve_group(
    account: &str,
    declared: &BTreeMap<String, String>,
    index: usize,
) -> (String, GroupSource) {
    match nearest_declared(declared, account) {
        Some(name) => (name.clone(), GroupSource::Tag),
        None => (
            humanized_segment(account.split(':').nth(index).unwrap_or_default()),
            GroupSource::Segment,
        ),
    }
}

/// One box's lines, sorted by name (a `BTreeMap` of buckets, so the sort is the
/// collection).
///
/// A group every one of whose members is exactly zero in every window is
/// dropped, matching hledger's own omission of all-zero accounts and the
/// balance sheet's rule: a heading with nothing under it is noise, and since its
/// total is zero by definition, dropping it cannot move the section total. Zero
/// ROWS inside a non-empty group are kept — that is exactly how a line present
/// in only one period shows its zero.
fn build_groups(
    members: &BTreeSet<String>,
    declared: &BTreeMap<String, String>,
    windows: &Windows,
    flip: bool,
) -> Result<Vec<IsGroup>, ReportError> {
    let index = group_segment_index(members);
    let mut buckets: BTreeMap<String, (GroupSource, BTreeSet<String>)> = BTreeMap::new();
    for account in members {
        let (name, source) = resolve_group(account, declared, index);
        buckets
            .entry(name)
            .or_insert_with(|| (source, BTreeSet::new()))
            .1
            .insert(account.clone());
    }
    buckets
        .into_iter()
        .filter(|(_, (_, accounts))| {
            accounts
                .iter()
                .any(|account| !windows.amounts(account).is_zero())
        })
        .map(|(name, (source, accounts))| {
            Ok(IsGroup {
                name,
                source,
                rows: accounts
                    .iter()
                    .map(|account| {
                        Ok(IsRow {
                            depth: account.split(':').count(),
                            amounts: windows.amounts(account).signed(flip)?,
                            account: account.clone(),
                        })
                    })
                    .collect::<Result<_, ReportError>>()?,
                total: windows.sum(&accounts)?.signed(flip)?,
            })
        })
        .collect()
}

/// A grouped, market-valued income statement whose GAAP subtotal ladder appears
/// line by line, only as the journal's tags ask for it.
///
/// # The shape is adaptive, and that is the whole design
///
/// An untagged journal gets two boxes — Revenue and Expenses — and a single Net
/// income figure, with no subtotals at all. That is the entire personal-finance
/// experience and it requires no tags. Each rung of the ladder materialises only
/// when the sections it needs exist, so there is no mode switch, no empty box
/// and no jargon a household journal never asked for. See [`LADDER`].
///
/// # Section resolution — first match wins
///
/// 1. `issection:` on the account itself.
/// 2. `issection:` on the nearest declared ANCESTOR (it inherits, like `type:`).
/// 3. Effective account type is `Revenue` → `revenue`.
/// 4. Effective account type is `Expense` → `opex`.
/// 5. Otherwise the account is not on this statement at all.
///
/// There is deliberately NO inference for `cogs`, `tax`, `interest` or
/// `depreciation`. Every rule that could produce them from an untagged journal
/// would be a name match, which is the `account-type-not-name` failure. A
/// journal rooted at `cogs:` with `type: X` lands in operating expenses and
/// reads correctly; splitting it out is one tag.
///
/// # Why the two periods are merged HERE
///
/// Sections, groups and their names are resolved once over the UNION of both
/// windows' accounts, and each window's figures are then filled in — so a line
/// present in only one period appears with a zero on the other side rather than
/// vanishing, and the same account cannot be named two different things in two
/// columns. Resolving over the current window alone would be worse than untidy:
/// a section with prior activity and none this period would be omitted outright,
/// and the prior column's boxes would no longer sum to the prior net income.
///
/// Doing this join in the client was rejected in the plan for the same reason it
/// is done over a union here — it is exactly the kind of key matching that
/// silently drops rows.
///
/// # Totals
///
/// Every one is summed over MEMBERS (see [`Windows::sum`]), never over displayed
/// rows, so a collapsed group and an expanded one report the same number
/// (RPT-1/RPT-4), and membership is decided before any valuation (RPT-2).
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn income_statement_grouped(
    txns: &[Transaction],
    explicit_prices: &[PriceDirective],
    opts: &IsOpts,
    declared: &BTreeMap<String, AccountType>,
    sections: &BTreeMap<String, IsSectionKind>,
    groups: &BTreeMap<String, String>,
) -> Result<IncomeStatementReport, ReportError> {
    let types = AccountTypes::from_declared(declared.clone());
    let resolve_section = |account: &str| -> Option<IsSectionKind> {
        nearest_declared(sections, account).copied().or_else(|| {
            if types.is_type(account, AccountType::Revenue) {
                Some(IsSectionKind::Revenue)
            } else if types.is_type(account, AccountType::Expense) {
                Some(IsSectionKind::Opex)
            } else {
                None
            }
        })
    };
    let on_statement = |account: &str| resolve_section(account).is_some();

    let prices = PriceDb::build(explicit_prices);
    let target: Option<Commodity> = opts
        .value_in
        .clone()
        .or_else(|| prices.base_commodity().cloned());

    let current = DateRange {
        from: opts.from.to_string(),
        to: opts.to.to_string(),
    };
    let prior = opts.compare.then(|| prior_window(&current));

    let mut meta = ValuationMeta::default();
    let windows = Windows {
        current: window_totals(
            txns,
            &current,
            opts,
            &prices,
            target.as_ref(),
            &on_statement,
            &mut meta,
        )?,
        prior: prior
            .as_ref()
            .map(|window| {
                window_totals(
                    txns,
                    window,
                    opts,
                    &prices,
                    target.as_ref(),
                    &on_statement,
                    &mut meta,
                )
            })
            .transpose()?,
    };

    let mut members: BTreeMap<IsSectionKind, BTreeSet<String>> = BTreeMap::new();
    for account in windows
        .current
        .keys()
        .chain(windows.prior.iter().flat_map(BTreeMap::keys))
    {
        if let Some(kind) = resolve_section(account) {
            members.entry(kind).or_default().insert(account.clone());
        }
    }

    // Pass 1: which boxes exist and what they hold. `multi_step` — and with it
    // every title and the Operating income guard — depends on the answer, so it
    // cannot be decided while emitting.
    let mut boxes: BTreeMap<IsSectionKind, (Vec<IsGroup>, Amounts)> = BTreeMap::new();
    let mut contributions: BTreeMap<IsSectionKind, Amounts> = BTreeMap::new();
    for rung in &LADDER {
        let section_members = members.remove(&rung.kind).unwrap_or_default();
        let sum = windows.sum(&section_members)?;
        contributions.insert(rung.kind, sum.ma_neg()?);
        let lines = build_groups(&section_members, groups, &windows, rung.flip)?;
        if !lines.is_empty() {
            boxes.insert(rung.kind, (lines, sum.signed(rung.flip)?));
        }
    }
    // Taken from the boxes that RENDER, not from raw membership. The plan words
    // it as "any member resolves to a section other than revenue or opex", which
    // is the same thing except for a tagged account whose balance is zero across
    // both windows: that section is omitted entirely, and letting it retitle
    // every box and print an Operating income line beneath a box that is not
    // there would contradict "no empty boxes" for no reader's benefit.
    let present: BTreeSet<IsSectionKind> = boxes.keys().copied().collect();
    let multi_step = present
        .iter()
        .any(|kind| !matches!(kind, IsSectionKind::Revenue | IsSectionKind::Opex));

    // Pass 2: emit in ladder order, accumulating the running total each subtotal
    // is a prefix of.
    let mut rendered: Vec<IsSection> = Vec::with_capacity(present.len());
    let mut running = Amounts::zero(opts.compare);
    for rung in &LADDER {
        running = running.ma_add(&contributions[&rung.kind])?;
        if let Some((lines, total)) = boxes.remove(&rung.kind) {
            rendered.push(IsSection {
                kind: rung.kind,
                title: rung.title(multi_step).to_string(),
                groups: lines,
                total,
                trailing: Vec::new(),
            });
        }
        // A subtotal attaches to the last box PRINTED, which is this rung's when
        // it rendered and the previous surviving one when it did not — so
        // omitting the D&A box moves Operating income up under Expenses rather
        // than leaving it floating. With no box above it at all there is nothing
        // for it to be a subtotal OF, and it is dropped.
        if let Some((kind, label, guard)) = &rung.trailing
            && guard.holds(&present, multi_step)
            && let Some(last) = rendered.last_mut()
        {
            last.trailing.push(IsSubtotal {
                kind: *kind,
                label: (*label).to_string(),
                total: running.clone(),
            });
        }
    }

    meta.unpriced.sort();
    meta.unpriced.dedup();
    Ok(IncomeStatementReport {
        from: current.from,
        to: current.to,
        prior,
        base: match opts.value {
            Valuation::Market => target,
            Valuation::Cost | Valuation::None => None,
        },
        sections: rendered,
        net_income: running,
        multi_step,
        meta: ReportMeta {
            unpriced: meta.unpriced,
        },
    })
}

/// True when valuing this income statement in `target` prices at least one of
/// the commodities it would display (vacuously true for a statement with no
/// rows in either window).
///
/// The HTTP layer's admission test for an explicit `valueIn` on
/// `/api/reports/incomestatement/grouped` —
/// `balance_sheet::prices_any_on_sheet`'s question, asked about THIS
/// statement's own windows: the as-written direct totals of every on-statement
/// account (the same section resolution [`income_statement_grouped`] narrows
/// by), each window measured at ITS OWN end, exactly as [`window_totals`]
/// values it, through the same explicit-`P`-only [`PriceDb`]. When `compare`
/// is on, the prior window counts too: a commodity pricing only prior-period
/// rows still retargets the prior column, so it is not refused.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn prices_any_on_statement(
    txns: &[Transaction],
    explicit_prices: &[PriceDirective],
    window: &DateRange,
    compare: bool,
    declared: &BTreeMap<String, AccountType>,
    sections: &BTreeMap<String, IsSectionKind>,
    target: &Commodity,
) -> Result<bool, ReportError> {
    let types = AccountTypes::from_declared(declared.clone());
    let on_statement = |account: &str| {
        nearest_declared(sections, account).is_some()
            || types.is_type(account, AccountType::Revenue)
            || types.is_type(account, AccountType::Expense)
    };
    let held_in = |window: &DateRange| -> Result<BTreeSet<Commodity>, ReportError> {
        let direct = account_totals(
            txns,
            &PostingFilter {
                from: Some(&window.from),
                to: Some(&window.to),
                ..PostingFilter::default()
            },
        )?;
        Ok(direct
            .iter()
            .filter(|(account, _)| on_statement(account))
            .flat_map(|(_, ma)| ma.iter().map(|(commodity, _)| commodity.clone()))
            .collect())
    };
    let windows: Vec<(BTreeSet<Commodity>, String)> = std::iter::once(window.clone())
        .chain(compare.then(|| prior_window(window)))
        .map(|window| Ok((held_in(&window)?, window.to)))
        .collect::<Result<_, ReportError>>()?;
    if windows.iter().all(|(held, _)| held.is_empty()) {
        return Ok(true);
    }
    let db = PriceDb::build(explicit_prices);
    windows.iter().try_fold(false, |admitted, (held, to)| {
        Ok(admitted || priced_count(held, target, &db, to)? > 0)
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{amount, mixed, price, txn, usd};
    use super::*;
    use crate::reports::mixed_amount::MixedAmount;

    fn sample() -> Vec<Transaction> {
        vec![
            // Before the range:
            txn(
                1,
                "2025-12-31",
                vec![
                    ("income:salary", vec![usd(-500_000)]),
                    ("assets:bank:checking", vec![usd(500_000)]),
                ],
            ),
            txn(
                2,
                "2026-01-15",
                vec![
                    ("income:salary", vec![usd(-400_000)]),
                    ("assets:bank:checking", vec![usd(400_000)]),
                ],
            ),
            txn(
                3,
                "2026-02-20",
                vec![
                    ("expenses:food:groceries", vec![usd(15_000)]),
                    ("liabilities:cc", vec![usd(-15_000)]),
                ],
            ),
            // "revenues" root categorizes as revenue alongside "income":
            txn(
                4,
                "2026-03-05",
                vec![
                    ("revenues:consulting", vec![usd(-20_000)]),
                    ("assets:bank:checking", vec![usd(20_000)]),
                ],
            ),
            // After the range:
            txn(
                5,
                "2026-07-01",
                vec![
                    ("expenses:food", vec![usd(9999)]),
                    ("assets:bank:checking", vec![usd(-9999)]),
                ],
            ),
        ]
    }

    fn usd_ma(cents: i128) -> MixedAmount {
        mixed(&[("$", cents, 2)])
    }

    /// The HTTP layer's admission question, measured over THIS statement's own
    /// rows and windows: an off-statement commodity admits nothing, a priced
    /// route into the current window's rows admits, and with `compare` on the
    /// PRIOR window's rows count too — each at its own end, as
    /// [`window_totals`] values them.
    #[test]
    fn prices_any_on_statement_measures_the_statements_own_rows_and_windows() {
        let c = |symbol: &str| Commodity(symbol.to_string());
        // One EUR expense in the current window, one GBP expense in the prior
        // one, and an asset-only commodity (STK) the statement never shows.
        let txns = vec![
            txn(
                1,
                "2026-03-01",
                vec![
                    ("expenses:travel", vec![amount("EUR", 10_000, 2)]),
                    ("assets:bank", vec![usd(-11_000)]),
                ],
            ),
            txn(
                2,
                "2025-03-01",
                vec![
                    ("expenses:travel", vec![amount("GBP", 5_000, 2)]),
                    ("assets:bank", vec![usd(-6_000)]),
                ],
            ),
            txn(
                3,
                "2026-02-01",
                vec![
                    ("assets:broker:stk", vec![amount("STK", 1, 0)]),
                    ("assets:bank", vec![usd(-100)]),
                ],
            ),
        ];
        let window = DateRange {
            from: "2026-01-01".to_string(),
            to: "2026-12-31".to_string(),
        };
        let admits = |prices: &[PriceDirective], compare: bool, target: &str| {
            prices_any_on_statement(
                &txns,
                prices,
                &window,
                compare,
                &BTreeMap::new(),
                &BTreeMap::new(),
                &c(target),
            )
            .expect("no overflow")
        };

        assert!(admits(&[], false, "EUR"), "EUR is itself on the statement");
        assert!(
            !admits(&[], false, "STK"),
            "STK is held by an asset account only — the statement never values it"
        );
        assert!(
            !admits(&[], false, "$"),
            "with no prices, nothing routes the EUR row to the dollar"
        );
        let eur_price = vec![price("2026-06-30", "EUR", amount("$", 110, 2))];
        assert!(
            admits(&eur_price, false, "$"),
            "P EUR $1.10 prices the current window's one row"
        );
        assert!(
            !admits(&[], false, "GBP"),
            "the prior window's rows do not count without a comparison"
        );
        assert!(
            admits(&[], true, "GBP"),
            "with compare on, a commodity pricing only the PRIOR window's rows \
             still retargets the prior column"
        );
        // Vacuously true when neither window has a row to value.
        let empty_window = DateRange {
            from: "2020-01-01".to_string(),
            to: "2020-12-31".to_string(),
        };
        assert!(
            prices_any_on_statement(
                &txns,
                &[],
                &empty_window,
                true,
                &BTreeMap::new(),
                &BTreeMap::new(),
                &c("USDD"),
            )
            .expect("no overflow"),
            "an empty statement admits vacuously"
        );
    }

    #[test]
    fn sign_flipped_revenues_and_natural_expenses_over_inclusive_range() {
        let report =
            income_statement(&sample(), "2026-01-01", "2026-06-30", 2, &BTreeMap::new()).unwrap();
        assert_eq!(report.from.as_deref(), Some("2026-01-01"));
        assert_eq!(report.to.as_deref(), Some("2026-06-30"));
        assert_eq!(
            report
                .sections
                .iter()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>(),
            ["Revenues", "Expenses"]
        );

        let revenues = &report.sections[0];
        assert_eq!(
            revenues
                .rows
                .iter()
                .map(|r| (r.account.as_str(), r.inclusive.clone()))
                .collect::<Vec<_>>(),
            [
                ("income", usd_ma(400_000)), // displayed positive; Dec txn out of range
                ("income:salary", usd_ma(400_000)),
                ("revenues", usd_ma(20_000)),
                ("revenues:consulting", usd_ma(20_000)),
            ]
        );
        assert_eq!(revenues.total, usd_ma(420_000)); // sums BOTH revenue roots

        let expenses = &report.sections[1];
        assert_eq!(
            expenses
                .rows
                .iter()
                .map(|r| (r.account.as_str(), r.inclusive.clone()))
                .collect::<Vec<_>>(),
            [
                ("expenses", usd_ma(15_000)), // July txn out of range
                ("expenses:food", usd_ma(15_000)),
            ]
        );
        assert_eq!(expenses.total, usd_ma(15_000));

        assert_eq!(report.grand_total, usd_ma(405_000)); // revenues − expenses
    }

    #[test]
    fn range_boundaries_inclusive_on_both_ends() {
        let report =
            income_statement(&sample(), "2025-12-31", "2026-07-01", 1, &BTreeMap::new()).unwrap();
        assert_eq!(report.sections[0].total, usd_ma(920_000)); // 5000 + 4000 + 200
        assert_eq!(report.sections[1].total, usd_ma(24_999)); // 150.00 + 99.99
        assert_eq!(report.grand_total, usd_ma(895_001));
    }
}
