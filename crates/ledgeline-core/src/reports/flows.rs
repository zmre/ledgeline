//! Money-flow graphs for the income statement: the two Sankey diagrams that sit
//! above the Revenue box and above the first cost box.
//!
//! **Money in** puts the revenue lines on the left and the accounts the money
//! landed in on the right. **Money out** puts the accounts that funded the
//! spending on the left and the cost lines on the right. Each graph is a
//! DECOMPOSITION of one side of the statement below it: `total` is what its links
//! carry and `section_total` is the figure they decompose, so a reader can see at
//! a glance whether the picture is the whole story.
//!
//! # How a posting is attributed
//!
//! A statement line is one side of a transaction; the other side is where the
//! money came from or went to. With two postings that is unambiguous. With more,
//! this allocates each statement posting across the postings on the OPPOSITE side
//! of the ledger, in proportion to their size. A paycheck
//!
//! ```journal
//! 2026-01-27 * Acme Corp | January salary
//!     income:salary             $-5,660.00
//!     expenses:taxes:federal     $1,150.00
//!     expenses:taxes:state         $310.00
//!     assets:bank:checking       $4,200.00
//! ```
//!
//! therefore draws Salary to Taxes: Federal at `$1,150.00`, to Taxes: State at
//! `$310.00` and to Bank: Checking at `$4,200.00` in **Money in**, and Salary to
//! Taxes at `$1,460.00` in **Money out**. The withheld tax was funded by gross
//! pay and not by the cash account, and both graphs say so.
//!
//! Two properties fall out of allocating the STATEMENT side rather than the
//! account side, and both are the reason it is done that way round:
//!
//!   1. **Each statement posting is split exactly.** The shares are integer
//!      mantissa arithmetic with the last one taking the remainder, so no rounding
//!      escapes; the account side carries whatever proportion follows and is not
//!      independently exact.
//!   2. **Market valuation cannot unbalance it.** A transaction balances at COST,
//!      not at market, so a valued transaction's debits and credits may differ.
//!      Allocating a known statement amount across proportions is indifferent to
//!      that; pairing debit totals against credit totals would not be.
//!
//! # Why this is always market-valued into one commodity
//!
//! A Sankey is geometry, and a link's width is one number. There is no
//! `value=cost|none` here for that reason: the widths would have no defined
//! meaning across a multi-commodity journal. `valueIn` is honored, and defaults
//! exactly as the statement's does, so the diagram and the table agree on their
//! basis. A commodity no price reaches is left out of the widths and named in
//! [`ReportMeta::unpriced`], the same as on the statement.
//!
//! # What is NOT drawn
//!
//! Links whose window net is zero or negative. A category refunded more than it
//! was charged has no width to draw, and a Sankey cannot render a negative one.
//! Whatever that removes shows up as the gap between `total` and `section_total`,
//! which is the only place it could be seen.
//!
//! `other` is in neither graph. It is the one box the statement lets print
//! negative because a grant and a lawsuit settlement can share it, so it has no
//! single direction to flow in.

use super::ReportError;
use super::account_groups::humanized_path;
use super::account_types::{AccountType, AccountTypes};
use super::aggregate::{PostingFilter, account_totals};
use super::income_statement::{
    DateRange, IsSectionKind, prior_window, section_groups, section_resolver,
};
use super::mixed_amount::MixedAmount;
use super::prices::{PriceDb, ValuationMeta, value_at};
use super::types::ReportMeta;
use crate::decimal::{Dec, DecError};
use crate::model::{Commodity, PriceDirective, Transaction};
use std::collections::{BTreeMap, BTreeSet};

/// Which end of a link a node sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowSide {
    /// The left column: revenue lines in **Money in**, funding accounts in
    /// **Money out**.
    Source,
    /// The right column: receiving accounts in **Money in**, cost lines in
    /// **Money out**.
    Target,
}

/// One end of a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowNode {
    /// Unique within its graph. Accounts and statement lines are namespaced
    /// apart (`a:`/`g:`) because a group may be named after the account it holds.
    pub key: String,
    /// What to print on it.
    pub label: String,
    /// Which column it belongs to.
    pub side: FlowSide,
    /// The account this node stands for, or `None` for a statement line. Carries
    /// the drill-down target, and is what lets one account keep one colour across
    /// both graphs.
    pub account: Option<String>,
    /// Summed over this node's drawn links.
    pub total: Dec,
}

/// One link, keyed by [`FlowNode::key`] at each end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowLink {
    /// The left node's key.
    pub source: String,
    /// The right node's key.
    pub target: String,
    /// Always positive: a non-positive net is not drawn (see the module doc).
    pub value: Dec,
}

/// One diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowGraph {
    /// Both columns, ordered by `total` descending.
    pub nodes: Vec<FlowNode>,
    /// Ordered by `value` descending.
    pub links: Vec<FlowLink>,
    /// Summed over `links`.
    pub total: Dec,
    /// The statement figure `links` decompose: the base-commodity total of the
    /// boxes this graph reads, displayed with the statement's sign. Equal to
    /// `total` on any journal whose every statement posting has a counterparty
    /// and nets positive over the window.
    pub section_total: Dec,
}

impl FlowGraph {
    /// The empty graph for a report with no valuation target.
    fn empty() -> Self {
        Self {
            nodes: Vec::new(),
            links: Vec::new(),
            total: Dec::zero(),
            section_total: Dec::zero(),
        }
    }
}

/// Both diagrams over one window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowReport {
    /// Inclusive range start.
    pub from: String,
    /// Inclusive range end.
    pub to: String,
    /// The commodity every width is in. `None` when the journal prices nothing,
    /// in which case both graphs are empty.
    pub base: Option<Commodity>,
    /// Revenue lines to the accounts that received them.
    pub inflows: FlowGraph,
    /// The accounts that funded the cost boxes, to those boxes' lines.
    pub outflows: FlowGraph,
    /// Commodities the valuation could not reach (sorted, deduped).
    pub meta: ReportMeta,
}

/// Inputs to [`income_statement_flows`].
#[derive(Debug, Clone)]
pub struct FlowOpts<'a> {
    /// Inclusive range start.
    pub from: &'a str,
    /// Inclusive range end.
    pub to: &'a str,
    /// Override the valuation target; defaults to `prices.base_commodity()`.
    pub value_in: Option<Commodity>,
}

/// Which way one graph reads, and which boxes it decomposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// Revenue on the left.
    In,
    /// Costs on the right.
    Out,
}

impl Direction {
    /// Whether this graph decomposes `kind`.
    fn claims(self, kind: IsSectionKind) -> bool {
        match self {
            Self::In => kind == IsSectionKind::Revenue,
            Self::Out => matches!(
                kind,
                IsSectionKind::Cogs
                    | IsSectionKind::Opex
                    | IsSectionKind::Depreciation
                    | IsSectionKind::Interest
                    | IsSectionKind::Tax
            ),
        }
    }

    /// Whether the boxes this graph reads are displayed sign-flipped. Revenue is
    /// negative internally, so its widths come from `-net`.
    fn flip(self) -> bool {
        self == Self::In
    }

    /// Which column the statement lines occupy.
    fn statement_side(self) -> FlowSide {
        match self {
            Self::In => FlowSide::Source,
            Self::Out => FlowSide::Target,
        }
    }
}

/// `d`'s mantissa restated at `places` fractional digits.
fn mantissa_at(d: Dec, places: u32) -> Result<i128, DecError> {
    let factor = 10i128
        .checked_pow(places.saturating_sub(d.places))
        .ok_or(DecError::Overflow)?;
    d.mantissa.checked_mul(factor).ok_or(DecError::Overflow)
}

/// Split `total` across `weights` in proportion to them.
///
/// The shares sum to `total` EXACTLY: the first `n-1` are truncated integer
/// mantissa quotients and the last takes what is left, so nothing is lost to
/// rounding and no per-link epsilon can accumulate over a year of transactions.
/// `total` may be negative (an expense refunded, revenue returned), in which case
/// every share is.
///
/// # Scale
///
/// The shares carry `total`'s own scale, or a weight's if one is finer. `total`
/// is deliberately NOT normalized first: `$-50.00` normalizes to scale 0, and
/// splitting it three ways at scale 0 gives whole dollars for money written in
/// cents. The weights ARE normalized, because their scale only affects the
/// proportion and valuation products arrive padded with trailing zeros, which
/// would otherwise push the intermediate product toward the `i128` ceiling for
/// nothing.
///
/// `weights` must be non-empty magnitudes summing to something non-zero, which is
/// what [`Edges::absorb`] selects them to be.
fn allocate(total: Dec, weights: &[Dec]) -> Result<Vec<Dec>, DecError> {
    let weights: Vec<Dec> = weights.iter().map(|w| w.normalized()).collect();
    let places = weights
        .iter()
        .map(|w| w.places)
        .chain([total.places])
        .max()
        .unwrap_or(0);

    let target = mantissa_at(total, places)?;
    let scaled: Vec<i128> = weights
        .iter()
        .map(|w| mantissa_at(*w, places))
        .collect::<Result<_, _>>()?;
    let sum = scaled.iter().try_fold(0i128, |acc, w| {
        acc.checked_add(*w).ok_or(DecError::Overflow)
    })?;
    if sum == 0 {
        return Err(DecError::Overflow);
    }

    let mut shares = Vec::with_capacity(scaled.len());
    let mut used: i128 = 0;
    for (i, weight) in scaled.iter().enumerate() {
        let share = if i + 1 == scaled.len() {
            target.checked_sub(used).ok_or(DecError::Overflow)?
        } else {
            target
                .checked_mul(*weight)
                .ok_or(DecError::Overflow)?
                .checked_div(sum)
                .ok_or(DecError::Overflow)?
        };
        used = used.checked_add(share).ok_or(DecError::Overflow)?;
        shares.push(Dec::new(share, places));
    }
    Ok(shares)
}

/// One graph's links while they are still keyed by account at both ends.
///
/// The statement end cannot be resolved to a LINE until the whole window has been
/// read, because the untagged group name depends on the prefix its whole box
/// shares (`income_statement::section_groups`). So this accumulates by account and
/// [`Edges::build`] renames afterwards.
struct Edges {
    direction: Direction,
    /// `(statement account, counterparty account)` to signed base-commodity value.
    totals: BTreeMap<(String, String), Dec>,
}

impl Edges {
    fn new(direction: Direction) -> Self {
        Self {
            direction,
            totals: BTreeMap::new(),
        }
    }

    /// Attribute one transaction's statement postings across their counterparties.
    ///
    /// `nets` is the transaction's per-account signed base-commodity net, and
    /// `claims` says which of those accounts this graph decomposes. Accounts on
    /// the statement side are excluded from the counterparty set: a node that was
    /// both a source and a target would be a cycle, which a Sankey layout has no
    /// reading for. What that excludes is left undrawn and surfaces as the gap
    /// between `total` and `section_total`.
    fn absorb(
        &mut self,
        nets: &[(&str, Dec)],
        claims: &impl Fn(&str) -> bool,
    ) -> Result<(), ReportError> {
        let mut statement: Vec<(&str, Dec)> = Vec::new();
        let mut debits: Vec<(&str, Dec)> = Vec::new();
        let mut credits: Vec<(&str, Dec)> = Vec::new();
        for &(account, value) in nets {
            if value.is_zero() {
                continue;
            }
            if claims(account) {
                statement.push((account, value));
            } else if value.mantissa > 0 {
                debits.push((account, value));
            } else {
                credits.push((account, value.neg()?));
            }
        }

        for (account, value) in statement {
            // A credit was funded by the debits and a debit was funded by the
            // credits, so which list is the counterparty set is this posting's own
            // sign. Reading it per posting rather than per graph is what makes a
            // refund net against the charge it reverses instead of drawing a
            // second link the wrong way.
            let counterparties = if value.mantissa < 0 {
                &debits
            } else {
                &credits
            };
            if counterparties.is_empty() {
                continue;
            }
            let displayed = if self.direction.flip() {
                value.neg()?
            } else {
                value
            };
            let weights: Vec<Dec> = counterparties.iter().map(|&(_, w)| w).collect();
            for (&(other, _), share) in counterparties.iter().zip(allocate(displayed, &weights)?) {
                let slot = self
                    .totals
                    .entry((account.to_string(), other.to_string()))
                    .or_insert_with(Dec::zero);
                *slot = slot.add(share)?;
            }
        }
        Ok(())
    }

    /// Rename the statement end to its box's LINE, drop what cannot be drawn, and
    /// emit the nodes the surviving links need.
    fn build(
        self,
        members: &BTreeMap<IsSectionKind, BTreeSet<String>>,
        groups: &BTreeMap<String, String>,
        section_total: Dec,
    ) -> Result<FlowGraph, ReportError> {
        let lines: BTreeMap<String, String> = members
            .iter()
            .filter(|(kind, _)| self.direction.claims(**kind))
            .flat_map(|(_, accounts)| section_groups(accounts, groups))
            .map(|(account, (name, _))| (account, name))
            .collect();

        // An account with edges but no line is one whose window total nets to
        // exactly zero: the statement omits it, so `section_groups` never saw it,
        // while its gross flows in either direction are real. Its own path is the
        // honest name for it, and its links net to zero so nothing it adds is
        // drawn unless the two directions had different counterparties.
        let mut merged: BTreeMap<(String, String), Dec> = BTreeMap::new();
        for ((account, other), value) in self.totals {
            let line = lines
                .get(&account)
                .cloned()
                .unwrap_or_else(|| humanized_path(&account));
            let slot = merged.entry((line, other)).or_insert_with(Dec::zero);
            *slot = slot.add(value)?;
        }

        let statement_side = self.direction.statement_side();
        let mut totals: BTreeMap<String, Dec> = BTreeMap::new();
        let mut labels: BTreeMap<String, (String, Option<String>, FlowSide)> = BTreeMap::new();
        let mut links: Vec<FlowLink> = Vec::new();
        let mut total = Dec::zero();
        for ((line, other), value) in merged {
            if value.mantissa <= 0 {
                continue;
            }
            let line_key = format!("g:{line}");
            let account_key = format!("a:{other}");
            labels
                .entry(line_key.clone())
                .or_insert((line, None, statement_side));
            labels.entry(account_key.clone()).or_insert_with(|| {
                (
                    humanized_path(&other),
                    Some(other.clone()),
                    match statement_side {
                        FlowSide::Source => FlowSide::Target,
                        FlowSide::Target => FlowSide::Source,
                    },
                )
            });
            for key in [&line_key, &account_key] {
                let slot = totals.entry(key.clone()).or_insert_with(Dec::zero);
                *slot = slot.add(value)?;
            }
            total = total.add(value)?;
            let (source, target) = match statement_side {
                FlowSide::Source => (line_key, account_key),
                FlowSide::Target => (account_key, line_key),
            };
            links.push(FlowLink {
                source,
                target,
                value,
            });
        }

        let mut nodes: Vec<FlowNode> = labels
            .into_iter()
            .map(|(key, (label, account, side))| FlowNode {
                total: totals.get(&key).copied().unwrap_or_else(Dec::zero),
                key,
                label,
                side,
                account,
            })
            .collect();
        // Biggest first, which is the order a Sankey is read in. The key breaks
        // ties so the wire is deterministic for the byte golden.
        nodes.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.key.cmp(&b.key)));
        links.sort_by(|a, b| {
            b.value
                .cmp(&a.value)
                .then_with(|| a.source.cmp(&b.source))
                .then_with(|| a.target.cmp(&b.target))
        });

        Ok(FlowGraph {
            nodes,
            links,
            total,
            section_total,
        })
    }
}

/// One window's per-account net, as written.
type WindowTotals = BTreeMap<String, MixedAmount>;

fn window_totals(txns: &[Transaction], window: &DateRange) -> Result<WindowTotals, ReportError> {
    Ok(account_totals(
        txns,
        &PostingFilter {
            from: Some(&window.from),
            to: Some(&window.to),
            ..PostingFilter::default()
        },
    )?)
}

/// The one commodity everything in the window is written in, when there is
/// exactly one.
///
/// The last resort for the widths' commodity, and the one that matters most in
/// practice: a single-currency journal with no `P` directive at all has no price
/// table and therefore no base commodity, which is the majority of personal
/// books. Falling back to the commodity in play makes those journals draw,
/// without inventing a rate for anything.
fn sole_commodity(totals: &WindowTotals) -> Option<Commodity> {
    let mut found: Option<&Commodity> = None;
    for (commodity, _) in totals.values().flat_map(MixedAmount::iter) {
        match found {
            Some(seen) if seen == commodity => {}
            Some(_) => return None,
            None => found = Some(commodity),
        }
    }
    found.cloned()
}

/// Every on-statement account in the reported window and in the equal-length
/// window before it, per box.
///
/// Unioned with the prior window because that is what the statement beside these
/// diagrams does, and membership decides the untagged group NAME: the shared
/// prefix and the shallowest member are both read off the whole box
/// (`income_statement::section_groups`). One direct posting to `expenses` in the
/// comparison period renames every line of that box, and a diagram that took
/// membership from the drawn window alone would keep the old names while the table
/// under it changed.
fn box_members(
    txns: &[Transaction],
    window: &DateRange,
    current: &WindowTotals,
    section: &impl Fn(&str) -> Option<IsSectionKind>,
) -> Result<BTreeMap<IsSectionKind, BTreeSet<String>>, ReportError> {
    let prior = window_totals(txns, &prior_window(window))?;
    let mut members: BTreeMap<IsSectionKind, BTreeSet<String>> = BTreeMap::new();
    for account in current.keys().chain(prior.keys()) {
        if let Some(kind) = section(account) {
            members.entry(kind).or_default().insert(account.clone());
        }
    }
    Ok(members)
}

/// One transaction's per-account signed net, as written, or `None` when nothing
/// it posts to is drawn by either graph.
///
/// Postings are selected on the effective date (`posting.date` else the
/// transaction's) and nothing else, matching
/// [`account_totals`](super::aggregate::account_totals) posting for posting so
/// the graph reads the same journal the statement does.
///
/// The `drawn` test is what keeps [`ValuationMeta`] honest. Valuing every
/// transaction's every account would report NVDA and TSLA as unpriced on
/// `fixtures/sample.journal` because a stock sale holds them, and the P&L tab
/// would raise its "some holdings are not valued" banner over a statement that
/// shows no holdings at all. A transaction with no statement leg contributes no
/// link, so its commodities are not this report's business.
fn transaction_nets<'a>(
    txn: &'a Transaction,
    window: &DateRange,
    drawn: &impl Fn(&str) -> bool,
) -> Result<Option<BTreeMap<&'a str, MixedAmount>>, ReportError> {
    let mut nets: BTreeMap<&str, MixedAmount> = BTreeMap::new();
    for posting in &txn.postings {
        let date = posting.date.as_deref().unwrap_or(&txn.date);
        if date < window.from.as_str() || date > window.to.as_str() {
            continue;
        }
        let entry = nets.entry(posting.account.0.as_str()).or_default();
        for amount in &posting.amounts {
            entry.accumulate(&amount.commodity, amount.quantity)?;
        }
    }
    Ok(nets.keys().any(|account| drawn(account)).then_some(nets))
}

/// The displayed base-commodity total of the boxes `direction` reads, measured
/// over window totals exactly as the statement measures its own.
fn section_total(
    totals: &WindowTotals,
    window: &DateRange,
    prices: &PriceDb,
    base: &Commodity,
    section: &impl Fn(&str) -> Option<IsSectionKind>,
    direction: Direction,
) -> Result<Dec, ReportError> {
    let mut sum = Dec::zero();
    for (account, ma) in totals {
        if section(account).is_some_and(|kind| direction.claims(kind)) {
            sum = sum.add(value_at(ma, base, prices, &window.to, None)?)?;
        }
    }
    if direction.flip() {
        Ok(sum.neg()?)
    } else {
        Ok(sum)
    }
}

/// Both flow graphs over `[from, to]` (INCLUSIVE).
///
/// See the module doc for the attribution rule and for what the two graphs mean.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn income_statement_flows(
    txns: &[Transaction],
    explicit_prices: &[PriceDirective],
    opts: &FlowOpts,
    declared: &BTreeMap<String, AccountType>,
    sections: &BTreeMap<String, IsSectionKind>,
    groups: &BTreeMap<String, String>,
) -> Result<FlowReport, ReportError> {
    let types = AccountTypes::from_declared(declared.clone());
    let resolve_section = section_resolver(sections, &types);
    let window = DateRange {
        from: opts.from.to_string(),
        to: opts.to.to_string(),
    };

    let prices = PriceDb::build(explicit_prices);
    let totals = window_totals(txns, &window)?;
    // `value_in`, else the statement's own target, else the commodity in play.
    // The last clause is not a nicety: a single-currency journal with no `P`
    // directive has no price table, so the first two are `None` and there would
    // be no diagram at all for the commonest book there is.
    let Some(base) = opts
        .value_in
        .clone()
        .or_else(|| prices.base_commodity().cloned())
        .or_else(|| sole_commodity(&totals))
    else {
        // Several commodities and nothing connecting them. A link's width is one
        // number, and there is no honest one to give it.
        return Ok(FlowReport {
            from: window.from,
            to: window.to,
            base: None,
            inflows: FlowGraph::empty(),
            outflows: FlowGraph::empty(),
            meta: ReportMeta::default(),
        });
    };

    // "On the statement, excluding `other`": the union of what the two graphs
    // draw, and the test that decides whether a transaction is read at all.
    let drawn = |account: &str| {
        resolve_section(account)
            .is_some_and(|kind| Direction::In.claims(kind) || Direction::Out.claims(kind))
    };

    let mut meta = ValuationMeta::default();
    let mut inflows = Edges::new(Direction::In);
    let mut outflows = Edges::new(Direction::Out);
    for txn in txns {
        let Some(raw) = transaction_nets(txn, &window, &drawn)? else {
            continue;
        };
        let nets: Vec<(&str, Dec)> = raw
            .into_iter()
            .map(|(account, ma)| {
                Ok((
                    account,
                    value_at(&ma, &base, &prices, &window.to, Some(&mut meta))?,
                ))
            })
            .collect::<Result<_, ReportError>>()?;
        for edges in [&mut inflows, &mut outflows] {
            let direction = edges.direction;
            edges.absorb(&nets, &|account: &str| {
                resolve_section(account).is_some_and(|kind| direction.claims(kind))
            })?;
        }
    }

    let members = box_members(txns, &window, &totals, &resolve_section)?;
    let inflow_total = section_total(
        &totals,
        &window,
        &prices,
        &base,
        &resolve_section,
        Direction::In,
    )?;
    let outflow_total = section_total(
        &totals,
        &window,
        &prices,
        &base,
        &resolve_section,
        Direction::Out,
    )?;

    meta.unpriced.sort();
    meta.unpriced.dedup();
    Ok(FlowReport {
        from: window.from.clone(),
        to: window.to.clone(),
        base: Some(base),
        inflows: inflows.build(&members, groups, inflow_total)?,
        outflows: outflows.build(&members, groups, outflow_total)?,
        meta: ReportMeta {
            unpriced: meta.unpriced,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(mantissa: i128, places: u32) -> Dec {
        Dec::new(mantissa, places)
    }

    #[test]
    fn shares_sum_to_the_allocated_total_exactly() {
        // $100.00 across three equal legs: 33.33 + 33.33 + 33.34, never 99.99.
        let shares = allocate(d(10_000, 2), &[d(1, 0), d(1, 0), d(1, 0)]).expect("allocates");
        let sum = shares
            .iter()
            .try_fold(Dec::zero(), |acc, share| acc.add(*share))
            .expect("sums");
        assert_eq!(sum, d(10_000, 2));
        assert_eq!(shares[2].cmp(&shares[0]), std::cmp::Ordering::Greater);
    }

    #[test]
    fn a_negative_total_allocates_negative_shares() {
        let shares = allocate(d(-5_000, 2), &[d(1, 0), d(3, 0)]).expect("allocates");
        assert_eq!(shares, vec![d(-1_250, 2), d(-3_750, 2)]);
    }

    #[test]
    fn weights_at_different_scales_are_compared_at_the_finer_one() {
        // 0.5 against 1.5 is one quarter, not one third: a naive mantissa
        // comparison would read 5 against 15 at different scales.
        let shares = allocate(d(400, 2), &[d(5, 1), d(15, 1)]).expect("allocates");
        assert_eq!(shares, vec![d(100, 2), d(300, 2)]);
    }

    #[test]
    fn only_revenue_is_claimed_going_in_and_only_costs_going_out() {
        assert!(Direction::In.claims(IsSectionKind::Revenue));
        assert!(!Direction::In.claims(IsSectionKind::Opex));
        for kind in [
            IsSectionKind::Cogs,
            IsSectionKind::Opex,
            IsSectionKind::Depreciation,
            IsSectionKind::Interest,
            IsSectionKind::Tax,
        ] {
            assert!(Direction::Out.claims(kind), "{kind:?}");
        }
        // The mixed box is in neither: it has no single direction to flow in.
        assert!(!Direction::In.claims(IsSectionKind::Other));
        assert!(!Direction::Out.claims(IsSectionKind::Other));
    }

    #[test]
    fn a_deep_account_keeps_every_segment_below_its_root() {
        assert_eq!(humanized_path("assets:bank:checking"), "Bank: Checking");
        assert_eq!(humanized_path("liabilities:cc:visa"), "Credit cards: Visa");
        assert_eq!(humanized_path("equity"), "Equity");
    }
}
