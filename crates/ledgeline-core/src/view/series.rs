//! The journal view's insights: the footer total, the big numbers, and the
//! pie/line charts over the FILTERED transactions (plan 26 B5; a port of the
//! SPA's `web/src/lib/insights/series.ts`).
//!
//! Every function takes the filtered POSITIONS in display order (date desc,
//! `tindex` desc) — the order the TypeScript was handed — because two answers
//! depend on it: a commodity's display style is the FIRST style seen in that
//! order (`styleFor`), and a chart's groups are ranked from it.
//!
//! # Sign conventions
//!
//! Each of revenue and expense displays with the sign that makes its DOMINANT
//! money flow positive (income positive when money came in, expenses positive
//! when it went out), detected per commodity over the WHOLE journal so it does
//! not flip as the filter moves. Dominance is magnitude-weighted; a tie keeps
//! hledger's sign. [`sign_table`] computes every commodity in one pass, and a
//! server memoizes it per snapshot.
//!
//! # Categories
//!
//! An account's category comes from the type the ENGINE resolved for it, with
//! the name heuristic only as the fallback for an untyped account — a chart of
//! accounts rooted at `cogs:` or `gastos:` still charts as expenses.

use super::Classes;
use super::filter::JournalFilter;
use super::wire::{DeclaredStyles, WireDec, WireStyle, default_style, to_number};
use crate::decimal::Dec;
use crate::display::{MAX_DISPLAY_DECIMALS, format_amount};
use crate::model::{AmountStyle, Journal, Transaction};
use crate::reports::accounts::clamp_account;
use crate::reports::{AccountType, Interval, RootCategory, bucket_key, categorize, next_bucket};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};

/// The folded tail's label (`format/palette.ts` `OTHER_LABEL`).
pub const OTHER: &str = "(other)";

/// The category scope selector's display order: expenses lead.
const CATEGORY_ORDER: [RootCategory; 6] = [
    RootCategory::Expense,
    RootCategory::Revenue,
    RootCategory::Asset,
    RootCategory::Liability,
    RootCategory::Equity,
    RootCategory::Other,
];

/// The SPA's `RootCategory` spelling.
#[must_use]
pub fn category_code(category: RootCategory) -> &'static str {
    match category {
        RootCategory::Asset => "asset",
        RootCategory::Liability => "liability",
        RootCategory::Equity => "equity",
        RootCategory::Revenue => "revenue",
        RootCategory::Expense => "expense",
        RootCategory::Other => "other",
    }
}

/// Parse the SPA's `RootCategory` spelling.
#[must_use]
pub fn parse_category(code: &str) -> Option<RootCategory> {
    CATEGORY_ORDER
        .into_iter()
        .find(|category| category_code(*category) == code)
}

fn type_category(kind: AccountType) -> RootCategory {
    match kind {
        AccountType::Asset | AccountType::Cash => RootCategory::Asset,
        AccountType::Liability => RootCategory::Liability,
        AccountType::Equity | AccountType::Conversion => RootCategory::Equity,
        AccountType::Revenue | AccountType::Gain => RootCategory::Revenue,
        AccountType::Expense => RootCategory::Expense,
    }
}

/// Per-call memo of account → category: a few hundred distinct accounts behind
/// hundreds of thousands of postings.
struct Categories<'a> {
    classes: &'a Classes,
    memo: HashMap<&'a str, RootCategory>,
}

impl<'a> Categories<'a> {
    fn new(classes: &'a Classes) -> Self {
        Self {
            classes,
            memo: HashMap::new(),
        }
    }

    /// `categoryOf`: the engine type's category, else the name's.
    fn of(&mut self, account: &'a str) -> RootCategory {
        let classes = self.classes;
        *self.memo.entry(account).or_insert_with(|| {
            classes
                .account_type(account)
                .map_or_else(|| categorize(account), type_category)
        })
    }
}

/// `1` or `-1`: the factor each category's amounts display with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SignConventions {
    /// Factor for revenue postings.
    pub revenue: i8,
    /// Factor for expense postings.
    pub expense: i8,
}

impl Default for SignConventions {
    fn default() -> Self {
        Self {
            revenue: 1,
            expense: 1,
        }
    }
}

/// Every commodity's [`SignConventions`], over the whole journal.
#[derive(Debug, Clone, Default)]
pub struct SignTable {
    by_commodity: HashMap<String, SignConventions>,
}

impl SignTable {
    /// The conventions for `commodity`; a commodity with no revenue or expense
    /// flow keeps hledger's signs.
    #[must_use]
    pub fn get(&self, commodity: &str) -> SignConventions {
        self.by_commodity
            .get(commodity)
            .copied()
            .unwrap_or_default()
    }
}

struct Flow {
    positive: Dec,
    negative: Dec,
}

impl Default for Flow {
    fn default() -> Self {
        Self {
            positive: Dec::zero(),
            negative: Dec::zero(),
        }
    }
}

impl Flow {
    fn add(&mut self, qty: Dec) {
        if qty.mantissa > 0 {
            self.positive = self.positive.add(qty).unwrap_or(self.positive);
        } else if qty.mantissa < 0 {
            let magnitude = qty.abs().unwrap_or(qty);
            self.negative = self.negative.add(magnitude).unwrap_or(self.negative);
        }
    }

    fn factor(&self) -> i8 {
        if self.negative > self.positive { -1 } else { 1 }
    }
}

/// Detect every commodity's sign conventions in one pass over the journal
/// (`signConventions`).
#[must_use]
pub fn sign_table(journal: &Journal, classes: &Classes) -> SignTable {
    let mut categories = Categories::new(classes);
    let mut flows: HashMap<&str, (Flow, Flow)> = HashMap::new();
    for posting in journal.transactions.iter().flat_map(|txn| &txn.postings) {
        let category = categories.of(&posting.account.0);
        if !matches!(category, RootCategory::Revenue | RootCategory::Expense) {
            continue;
        }
        for amount in &posting.amounts {
            let (revenue, expense) = flows.entry(amount.commodity.0.as_str()).or_default();
            match category {
                RootCategory::Revenue => revenue.add(amount.quantity),
                _ => expense.add(amount.quantity),
            }
        }
    }
    SignTable {
        by_commodity: flows
            .into_iter()
            .map(|(commodity, (revenue, expense))| {
                (
                    commodity.to_string(),
                    SignConventions {
                        revenue: revenue.factor(),
                        expense: expense.factor(),
                    },
                )
            })
            .collect(),
    }
}

/// A posting amount with the detected display sign applied (`displayQty`).
fn display_qty(qty: Dec, category: RootCategory, signs: SignConventions) -> Dec {
    let flip = match category {
        RootCategory::Revenue => signs.revenue == -1,
        RootCategory::Expense => signs.expense == -1,
        _ => false,
    };
    if flip { qty.neg().unwrap_or(qty) } else { qty }
}

/// The journal-wide inputs every insight reads: the journal, its account
/// classes, its sign conventions and its declared styles — built once per
/// snapshot by a server, borrowed here.
#[derive(Clone, Copy)]
pub struct JournalView<'a> {
    /// The parsed journal.
    pub journal: &'a Journal,
    /// Every account's engine classification.
    pub classes: &'a Classes,
    /// Every commodity's sign conventions ([`sign_table`]).
    pub signs: &'a SignTable,
    /// The declared commodity styles.
    pub declared: &'a DeclaredStyles,
}

fn txns<'a>(journal: &'a Journal, positions: &'a [u32]) -> impl Iterator<Item = &'a Transaction> {
    positions
        .iter()
        .map(|position| &journal.transactions[*position as usize])
}

fn add(sum: Dec, qty: Dec) -> Dec {
    sum.add(qty).unwrap_or(sum)
}

// ---------------------------------------------------------------------------
// Summary: footer total, big numbers, selector inputs
// ---------------------------------------------------------------------------

/// One amount: the footer's "Visible Journal Total".
#[derive(Debug, Clone, Serialize)]
pub struct SummaryAmount {
    commodity: String,
    qty: WireDec,
    style: WireStyle,
}

/// Income / expenses / net and the chart inputs for one commodity.
#[derive(Debug, Clone, Serialize)]
pub struct CommoditySummary {
    commodity: String,
    /// Display style: the first amount of this commodity in display order.
    style: WireStyle,
    /// Sign-adjusted revenue (positive when money came in).
    income: WireDec,
    /// Sign-adjusted expenses (positive when money was spent).
    expenses: WireDec,
    /// `income − expenses`.
    net: WireDec,
    /// Root categories with a posting in this commodity, expenses first.
    categories: Vec<&'static str>,
}

/// `GET /api/journal/summary`: everything the footer, the big numbers and the
/// chart selectors need for one filter, from one pass.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// Transactions matching the filter.
    total: usize,
    /// The primary commodity's net, or empty when there is none or it is zero.
    visible_net: Vec<SummaryAmount>,
    /// Commodities posted to (under the account selection), most-used first.
    commodities: Vec<String>,
    /// One per entry of `commodities`, same order.
    per_commodity: Vec<CommoditySummary>,
    /// Deepest account (segments) under the account selection; at least 1.
    max_depth: usize,
    /// `commodities[0]`, or null for an empty view.
    primary: Option<String>,
}

struct Tally {
    count: usize,
    income: Dec,
    expenses: Dec,
    /// Bit `i` set when `CATEGORY_ORDER[i]` has a posting in this commodity.
    categories: u8,
}

impl Default for Tally {
    fn default() -> Self {
        Self {
            count: 0,
            income: Dec::zero(),
            expenses: Dec::zero(),
            categories: 0,
        }
    }
}

/// `category`'s bit in [`Tally::categories`].
fn category_bit(category: RootCategory) -> u8 {
    let at = CATEGORY_ORDER
        .iter()
        .position(|candidate| *candidate == category)
        .unwrap_or(CATEGORY_ORDER.len() - 1);
    1 << at
}

/// Compute the [`Summary`] for `positions` (display order) under `filter`.
#[must_use]
pub fn summary(view: JournalView<'_>, positions: &[u32], filter: &JournalFilter) -> Summary {
    let JournalView {
        journal,
        classes,
        signs,
        declared,
    } = view;
    let mut categories = Categories::new(classes);
    let mut styles: HashMap<&str, AmountStyle> = HashMap::new();
    let mut tallies: HashMap<&str, Tally> = HashMap::new();
    let mut max_depth = 1;
    for txn in txns(journal, positions) {
        for posting in &txn.postings {
            let account = posting.account.0.as_str();
            let included = filter.includes_account(account);
            let category = included.then(|| categories.of(account));
            if included {
                max_depth = max_depth.max(account.split(':').count());
            }
            for amount in &posting.amounts {
                let symbol = amount.commodity.0.as_str();
                styles
                    .entry(symbol)
                    .or_insert_with(|| declared.display_style(amount));
                let Some(category) = category else {
                    continue;
                };
                let tally = tallies.entry(symbol).or_default();
                tally.count += 1;
                tally.categories |= category_bit(category);
                let shown = display_qty(amount.quantity, category, signs.get(symbol));
                match category {
                    RootCategory::Revenue => tally.income = add(tally.income, shown),
                    RootCategory::Expense => tally.expenses = add(tally.expenses, shown),
                    _ => {}
                }
            }
        }
    }
    let mut ranked: Vec<(&str, Tally)> = tallies.into_iter().collect();
    ranked
        .sort_by(|(a_name, a), (b_name, b)| b.count.cmp(&a.count).then_with(|| a_name.cmp(b_name)));
    let style_of = |symbol: &str| WireStyle::from(styles.get(symbol).unwrap_or(&default_style()));
    let per_commodity: Vec<CommoditySummary> = ranked
        .iter()
        .map(|(symbol, tally)| {
            let net = tally.income.sub(tally.expenses).unwrap_or(tally.income);
            CommoditySummary {
                commodity: (*symbol).to_string(),
                style: style_of(symbol),
                income: tally.income.into(),
                expenses: tally.expenses.into(),
                net: net.into(),
                categories: CATEGORY_ORDER
                    .into_iter()
                    .filter(|category| tally.categories & category_bit(*category) != 0)
                    .map(category_code)
                    .collect(),
            }
        })
        .collect();
    let visible_net = ranked
        .first()
        .map(|(symbol, tally)| {
            (
                symbol,
                tally.income.sub(tally.expenses).unwrap_or(tally.income),
            )
        })
        .filter(|(_, net)| !net.is_zero())
        .map(|(symbol, net)| SummaryAmount {
            commodity: (*symbol).to_string(),
            qty: net.into(),
            style: style_of(symbol),
        })
        .into_iter()
        .collect();
    Summary {
        total: positions.len(),
        visible_net,
        commodities: ranked
            .iter()
            .map(|(symbol, _)| (*symbol).to_string())
            .collect(),
        per_commodity,
        max_depth,
        primary: ranked.first().map(|(symbol, _)| (*symbol).to_string()),
    }
}

// ---------------------------------------------------------------------------
// Charts
// ---------------------------------------------------------------------------

/// One chart request.
#[derive(Debug, Clone)]
pub struct ChartSpec {
    /// Account depth to clamp to.
    pub depth: usize,
    /// The one commodity charted (never summed across commodities).
    pub commodity: String,
    /// Category scope; `None` charts every category.
    pub category: Option<RootCategory>,
    /// Most groups drawn, the tail folded into [`OTHER`]; at least 1.
    pub max: usize,
}

/// Whether a posting counts toward this chart (`postingIncluded`).
fn chart_includes<'a>(
    account: &'a str,
    filter: &JournalFilter,
    spec: &ChartSpec,
    categories: &mut Categories<'a>,
) -> bool {
    spec.category
        .is_none_or(|scope| categories.of(account) == scope)
        && filter.includes_account(account)
}

/// Clamped accounts with postings in the commodity, by total absolute volume
/// (descending; ties alphabetical) — both modes rank from this, so an account
/// keeps its colour across them (`rankedAccounts`).
#[must_use]
pub fn ranked_accounts(
    view: JournalView<'_>,
    positions: &[u32],
    filter: &JournalFilter,
    spec: &ChartSpec,
) -> Vec<String> {
    let journal = view.journal;
    let mut categories = Categories::new(view.classes);
    // Per ACCOUNT first (no allocation per posting), then folded into the
    // clamped groups: exact decimals make the two orders give the same sums.
    let mut per_account: HashMap<&str, Dec> = HashMap::new();
    for posting in txns(journal, positions).flat_map(|txn| &txn.postings) {
        let account = posting.account.0.as_str();
        if !chart_includes(account, filter, spec, &mut categories) {
            continue;
        }
        for amount in posting
            .amounts
            .iter()
            .filter(|a| a.commodity.0 == spec.commodity)
        {
            let magnitude_of = amount.quantity.abs().unwrap_or(amount.quantity);
            let total = per_account.entry(account).or_insert_with(Dec::zero);
            *total = add(*total, magnitude_of);
        }
    }
    let mut magnitude: HashMap<String, Dec> = HashMap::new();
    for (account, total) in per_account {
        let group = magnitude
            .entry(clamp_account(account, spec.depth))
            .or_insert_with(Dec::zero);
        *group = add(*group, total);
    }
    let mut ranked: Vec<(String, Dec)> = magnitude.into_iter().collect();
    ranked.sort_by(|(a_name, a), (b_name, b)| b.cmp(a).then_with(|| a_name.cmp(b_name)));
    ranked.into_iter().map(|(name, _)| name).collect()
}

/// Account → its group: the top `max − 1` keep their name when the ranking
/// overflows, the rest fold into [`OTHER`] (`foldTail`).
fn fold_tail(ranked: &[String], max: usize) -> HashMap<&str, &str> {
    let keep = if ranked.len() > max {
        max - 1
    } else {
        ranked.len()
    };
    ranked
        .iter()
        .enumerate()
        .map(|(at, account)| {
            (
                account.as_str(),
                if at < keep { account.as_str() } else { OTHER },
            )
        })
        .collect()
}

/// The distinct groups the ranking folds into, in rank order — the chart's
/// slice/series order, and what colours are assigned from (`groupOrder`).
#[must_use]
pub fn group_order(ranked: &[String], max: usize) -> Vec<String> {
    let groups = fold_tail(ranked, max.max(1));
    let mut seen: HashSet<&str> = HashSet::new();
    ranked
        .iter()
        .map(|account| groups.get(account.as_str()).copied().unwrap_or(OTHER))
        .filter(|group| seen.insert(group))
        .map(str::to_string)
        .collect()
}

/// The first display style of `commodity` among `positions` (`styleFor`).
fn style_for(
    journal: &Journal,
    positions: &[u32],
    commodity: &str,
    declared: &DeclaredStyles,
) -> AmountStyle {
    txns(journal, positions)
        .flat_map(|txn| &txn.postings)
        .flat_map(|posting| &posting.amounts)
        .find(|amount| amount.commodity.0 == commodity)
        .map_or_else(default_style, |amount| declared.display_style(amount))
}

/// One pie slice.
#[derive(Debug, Clone, Serialize)]
pub struct PieDatum {
    account: String,
    /// Display-sign-adjusted total (display boundary: a double).
    value: f64,
    /// The exact total, formatted for a tooltip.
    formatted: String,
}

/// Period totals per group, sign-adjusted, zero groups dropped, in rank order
/// (`pieData`).
#[must_use]
pub fn pie_data(
    view: JournalView<'_>,
    positions: &[u32],
    filter: &JournalFilter,
    spec: &ChartSpec,
    ranked: &[String],
) -> Vec<PieDatum> {
    let journal = view.journal;
    let groups = fold_tail(ranked, spec.max.max(1));
    let conventions = view.signs.get(&spec.commodity);
    let mut categories = Categories::new(view.classes);
    // Per ACCOUNT first, then folded into groups (see `ranked_accounts`).
    let mut per_account: HashMap<&str, Dec> = HashMap::new();
    for posting in txns(journal, positions).flat_map(|txn| &txn.postings) {
        let account = posting.account.0.as_str();
        if !chart_includes(account, filter, spec, &mut categories) {
            continue;
        }
        let category = categories.of(account);
        for amount in posting
            .amounts
            .iter()
            .filter(|a| a.commodity.0 == spec.commodity)
        {
            let total = per_account.entry(account).or_insert_with(Dec::zero);
            *total = add(*total, display_qty(amount.quantity, category, conventions));
        }
    }
    let mut totals: HashMap<&str, Dec> = HashMap::new();
    for (account, sum) in per_account {
        let group = groups
            .get(clamp_account(account, spec.depth).as_str())
            .copied()
            .unwrap_or(OTHER);
        let total = totals.entry(group).or_insert_with(Dec::zero);
        *total = add(*total, sum);
    }
    let style = style_for(journal, positions, &spec.commodity, view.declared);
    group_order(ranked, spec.max)
        .into_iter()
        .filter_map(|group| {
            let total = totals.get(group.as_str()).copied()?;
            (!total.is_zero()).then(|| PieDatum {
                value: to_number(total),
                formatted: format_amount(&spec.commodity, total, &style, MAX_DISPLAY_DECIMALS),
                account: group,
            })
        })
        .collect()
}

/// One point of a line series.
#[derive(Debug, Clone, Serialize)]
pub struct LinePoint {
    bucket: String,
    value: f64,
}

/// One group's activity per bucket, every bucket of the range present.
#[derive(Debug, Clone, Serialize)]
pub struct LineSeries {
    account: String,
    points: Vec<LinePoint>,
}

/// The most buckets a line chart spans. Thirty years of daily buckets; a
/// journal with a stray far-off date would otherwise ask for millions.
pub const MAX_LINE_BUCKETS: usize = 11_000;

/// Activity per bucket per group, sign-adjusted, zero-filled across the whole
/// bucket range so lines are continuous (`lineData`).
#[must_use]
pub fn line_data(
    view: JournalView<'_>,
    positions: &[u32],
    filter: &JournalFilter,
    spec: &ChartSpec,
    interval: Interval,
    ranked: &[String],
) -> Vec<LineSeries> {
    let journal = view.journal;
    let groups = fold_tail(ranked, spec.max.max(1));
    let conventions = view.signs.get(&spec.commodity);
    let mut categories = Categories::new(view.classes);
    // Per (ACCOUNT, bucket) first, then folded into groups (see
    // `ranked_accounts`).
    let mut per_account: HashMap<&str, HashMap<String, Dec>> = HashMap::new();
    let mut span: Option<(String, String)> = None;
    for txn in txns(journal, positions) {
        for posting in &txn.postings {
            let account = posting.account.0.as_str();
            if !chart_includes(account, filter, spec, &mut categories) {
                continue;
            }
            let category = categories.of(account);
            for amount in posting
                .amounts
                .iter()
                .filter(|a| a.commodity.0 == spec.commodity)
            {
                let bucket = bucket_key(posting.date.as_deref().unwrap_or(&txn.date), interval);
                span = Some(match span.take() {
                    None => (bucket.clone(), bucket.clone()),
                    Some((lo, hi)) => (lo.min(bucket.clone()), hi.max(bucket.clone())),
                });
                let total = per_account
                    .entry(account)
                    .or_default()
                    .entry(bucket)
                    .or_insert_with(Dec::zero);
                *total = add(*total, display_qty(amount.quantity, category, conventions));
            }
        }
    }
    let mut sums: HashMap<&str, HashMap<String, Dec>> = HashMap::new();
    for (account, buckets) in per_account {
        let group = groups
            .get(clamp_account(account, spec.depth).as_str())
            .copied()
            .unwrap_or(OTHER);
        let into = sums.entry(group).or_default();
        for (bucket, sum) in buckets {
            let total = into.entry(bucket).or_insert_with(Dec::zero);
            *total = add(*total, sum);
        }
    }
    let Some((first, last)) = span else {
        return Vec::new();
    };
    let mut buckets = vec![first];
    while buckets.len() < MAX_LINE_BUCKETS {
        let Some(current) = buckets.last() else { break };
        if *current >= last {
            break;
        }
        match next_bucket(current, interval) {
            Ok(next) if next > *current => buckets.push(next),
            _ => break,
        }
    }
    group_order(ranked, spec.max)
        .into_iter()
        .filter_map(|group| {
            let per_bucket = sums.get(group.as_str())?;
            Some(LineSeries {
                points: buckets
                    .iter()
                    .map(|bucket| LinePoint {
                        bucket: bucket.clone(),
                        value: per_bucket.get(bucket).map_or(0.0, |sum| to_number(*sum)),
                    })
                    .collect(),
                account: group,
            })
        })
        .collect()
}

/// `GET /api/journal/chart`: the group order (colour slots) and the active
/// mode's dataset.
#[derive(Debug, Clone, Serialize)]
pub struct Chart {
    /// The commodity charted.
    commodity: String,
    /// Groups in rank order, folded to the request's `max` — the colour slots.
    groups: Vec<String>,
    /// The pie, when the request asked for one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pie: Option<Vec<PieDatum>>,
    /// The line series, when the request asked for them.
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<Vec<LineSeries>>,
}

/// Which dataset a chart request wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartMode {
    /// Period totals per group.
    Pie,
    /// Activity per bucket per group.
    Line(Interval),
}

/// Build the chart for one request: ranking once, then only the mode asked for.
#[must_use]
pub fn chart(
    view: JournalView<'_>,
    positions: &[u32],
    filter: &JournalFilter,
    spec: &ChartSpec,
    mode: ChartMode,
) -> Chart {
    let ranked = ranked_accounts(view, positions, filter, spec);
    let (pie, line) = match mode {
        ChartMode::Pie => (Some(pie_data(view, positions, filter, spec, &ranked)), None),
        ChartMode::Line(interval) => (
            None,
            Some(line_data(view, positions, filter, spec, interval, &ranked)),
        ),
    };
    Chart {
        commodity: spec.commodity.clone(),
        groups: group_order(&ranked, spec.max),
        pie,
        line,
    }
}

/// Every commodity in `journal` with its sign conventions — the golden's
/// `signs` table, and a debugging aid.
#[must_use]
pub fn sign_conventions_of(table: &SignTable) -> BTreeSet<(String, i8, i8)> {
    table
        .by_commodity
        .iter()
        .map(|(commodity, signs)| (commodity.clone(), signs.revenue, signs.expense))
        .collect()
}
