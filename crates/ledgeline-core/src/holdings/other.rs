//! Other holdings — everything you own that is neither a security nor cash: a
//! house, a car, a partnership interest, a receivable.
//!
//! This is the Holdings page's second tab, and it is a DIFFERENT ENGINE, not a
//! filter over [`super::engine`]. That one is keyed by commodity and its first
//! act is to drop every currency amount (`engine.rs`, the `is_currency` skip in
//! `replay_pools`), so a house booked as `$150,000.00` produces no pool, no
//! symbol and no row — it is not hidden, it is structurally invisible. Here the
//! thing you own IS the account, and its value is that account's balance.
//!
//! What the two tabs share: the scope (`scope_accounts`), the `holdings:` tag
//! that splits them ([`super::classify`]), the `change %` arithmetic
//! (`engine::gain_pct`), and the wire shape of the trend
//! ([`HoldingsSeries`]) — so the chart component draws both with no new code.
//!
//! Valuation reads explicit `P` directives PLUS prices inferred from `@`/`@@`
//! cost annotations, the same set (and the same precedence) as
//! [`net_worth`](crate::reports::net_worth). It deliberately differs from the
//! balance sheet, which is explicit-`P`-only to match `hledger bs -V`: an
//! account whose only price evidence is the annotation on its own purchase
//! should still show that value, because otherwise the common case — one
//! `1 HOUSE @ $150,000` and nothing else — reads as unpriced.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::decimal::Dec;
use crate::model::{AccountDeclaration, AccountName, Commodity, PriceDirective, Transaction};
use crate::reports::{
    AccountType, AccountTypes, Interval, MixedAmount, PostingFilter, PriceDb, ReportError,
    ValuationMeta, account_decls_from, account_totals, bucket_end, bucket_label, compare_iso,
    declared_types, infer_market_prices, last_n_buckets, value_at,
};
use crate::wire::{account_tag_map, inherited_account_tags};

use super::classify::{
    HoldingsClass, ValuationRole, declared_holdings_classes, declared_valuation_roles,
    resolve_holdings_class, resolve_valuation_role,
};
use super::commodities::is_currency;
use super::engine::{FALLBACK_BASE, gain_pct, scope_accounts};
use super::series::{HoldingsPoint, HoldingsSeries};
use super::types::HoldingsScope;

/// One non-stock, non-cash asset account.
#[derive(Debug, Clone, PartialEq)]
pub struct OtherHolding {
    /// Full account path — the row's identity.
    pub account: String,
    /// The nearest declared `name:` tag (own, then ancestors), else the
    /// account's last segment.
    pub name: String,
    /// The balance AS WRITTEN, so the UI can show `1 HOUSE` next to its dollar
    /// value. Empty of anything but the base commodity for a dollar-booked
    /// asset, which is the common case.
    pub commodities: MixedAmount,
    /// Market value at `as_of` in the base commodity; `None` when any held
    /// commodity has no price route to the base (see
    /// [`OtherWarningKind::Unpriced`]).
    pub value: Option<Dec>,
    /// The same balance at COST (hledger `-B`), valued into the base.
    pub cost: Option<Dec>,
    /// `value − reference`, where the reference is `cost` for an all-time
    /// window and the value at the window start otherwise.
    pub change: Option<Dec>,
    /// `change / reference × 100` (display-boundary float); `None` when the
    /// reference is missing or non-positive.
    pub change_pct: Option<f64>,
}

/// Why a row could not be valued (or costed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtherWarningKind {
    /// No chain of prices reaches the base commodity (excluded from totals).
    Unpriced,
    /// The row's VALUE is fine but its at-cost basis holds a commodity with no
    /// route to the base, so `cost` and `change` read `None`. The value still
    /// counts toward the totals; only the cost and change columns go dark.
    ///
    /// Never emitted alongside [`Unpriced`](Self::Unpriced) for the same row: an
    /// unpriceable value already excludes the row wholesale and warns once, and
    /// a second warning about its cost would restate the first.
    UnpricedCost,
}

/// A scope-local warning surfaced alongside the rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherHoldingsWarning {
    /// The affected account.
    pub account: String,
    /// What went wrong.
    pub kind: OtherWarningKind,
    /// Human-readable detail.
    pub message: String,
}

/// Totals over the rows. `cost`/`change`/`change_pct` are PARTIAL on the same
/// rule as [`HoldingsTotals`](super::types::HoldingsTotals): they sum over only
/// the rows carrying the needed input, so one unpriced asset no longer blanks
/// the whole tab. Each is `None` only when its set is empty.
#[derive(Debug, Clone, PartialEq)]
pub struct OtherHoldingsTotals {
    /// Sum of priced values; unpriced rows excluded. An empty tab reports a
    /// real zero.
    pub value: Dec,
    /// Sum of cost over rows that have both a value and a cost.
    pub cost: Option<Dec>,
    /// Sum of `value − reference` over the qualifying rows.
    pub change: Option<Dec>,
    /// `change / reference-sum × 100` over the qualifying rows.
    pub change_pct: Option<f64>,
}

/// The full other-holdings report for a scope at `as_of`.
#[derive(Debug, Clone, PartialEq)]
pub struct OtherHoldingsReport {
    /// Snapshot date.
    pub as_of: String,
    /// Base valuation commodity.
    pub base: String,
    /// Rows, by value desc; unpriced last, then by account.
    pub holdings: Vec<OtherHolding>,
    /// Every account that could be a row, over the whole journal and ignoring
    /// the scope — the scope chooser's option list, sorted.
    ///
    /// The engine hands this over rather than letting the UI derive it because
    /// membership depends on the `holdings:` tag, and the SPA's account feed
    /// carries only `type:`. A UI-side approximation would offer a tagged house
    /// on neither tab and a `holdings: none` account on one it can never appear
    /// in.
    pub accounts: Vec<String>,
    /// Totals over the rows.
    pub totals: OtherHoldingsTotals,
    /// Scope-local warnings.
    pub warnings: Vec<OtherHoldingsWarning>,
}

/// Everything a snapshot reads that does not depend on `as_of`, hoisted so a
/// `count`-point series builds the price database, the type memo and the tag map
/// once rather than once per point (the shape [`super::engine::HoldingsInputs`]
/// exists for, and for the same measured reason).
struct OtherInputs<'a> {
    txns: &'a [Transaction],
    db: PriceDb,
    types: AccountTypes,
    classes: BTreeMap<String, HoldingsClass>,
    roles: BTreeMap<String, ValuationRole>,
    account_tags: HashMap<&'a str, &'a [(String, String)]>,
    /// Each posting-bearing account mapped to the account that OWNS its row.
    /// Precomputed from the whole journal, so the tree shape — and therefore the
    /// row set — does not change as `as_of` moves.
    row_roots: BTreeMap<String, String>,
}

impl<'a> OtherInputs<'a> {
    fn build(
        txns: &'a [Transaction],
        explicit_prices: &[PriceDirective],
        accounts: &'a [AccountDeclaration],
    ) -> Result<Self, ReportError> {
        // Inferred first so an explicit `P` wins a same-date tie — hledger's
        // precedence, and exactly the order `net_worth` combines them in.
        let mut all_prices = infer_market_prices(txns)?;
        all_prices.extend_from_slice(explicit_prices);
        let classes = declared_holdings_classes(accounts)?;
        let types = AccountTypes::from_declared(declared_types(&account_decls_from(accounts)));
        Ok(Self {
            txns,
            db: PriceDb::build(&all_prices),
            roles: declared_valuation_roles(accounts)?,
            account_tags: account_tag_map(accounts),
            row_roots: row_roots(txns, &classes, &types),
            classes,
            types,
        })
    }

    /// The commodity the whole report is denominated in.
    ///
    /// Unlike the stock engine's `choose_base`, there is nothing to optimize
    /// for: that one walks price candidates looking for the one that prices the
    /// most *symbols*, because a portfolio of securities has no inherent
    /// currency. These accounts are mostly denominated in the journal's own
    /// currency already, so the balance sheet's rule — caller's override, else
    /// the price database's base — is both simpler and what the rest of the app
    /// shows.
    fn base(&self, scope: &HoldingsScope) -> Commodity {
        scope
            .value_in
            .clone()
            .or_else(|| self.db.base_commodity().cloned())
            .unwrap_or_else(|| Commodity(FALLBACK_BASE.to_string()))
    }
}

/// The parent path of `account`, or `None` for a single-segment root.
fn parent_of(account: &str) -> Option<&str> {
    account.rfind(':').map(|cut| &account[..cut])
}

/// Map every posting-bearing account to the account that owns its ROW.
///
/// A great many real journals split one asset across sibling accounts, because
/// that is how you carry a cost/market split without booking the asset as its
/// own commodity:
///
/// ```journal
/// assets:home:cost        ; what you paid
/// assets:home:unrealized  ; the mark against it
/// ```
///
/// Those are one house. Reported as two rows they are not merely untidy: they
/// sit apart once the table sorts by value, and each one's cost equals its own
/// balance, so both report a change of zero and the tab's whole subject
/// disappears.
///
/// Two rules, in order:
///
/// 1. **An explicit `holdings:` tag wins.** The nearest tagged ancestor-or-self
///    owns the row. Tagging is how you say "this subtree is one thing" when the
///    shape below is unusual.
/// 2. **Otherwise, roll up to a parent that is purely a container**: it has no
///    postings of its own, and its posting-bearing descendants that could
///    themselves be Other-tab rows are ALL direct children, of which there are
///    at least two.
///
/// Rule 2 is deliberately shallow — applied once, never iterated. That is what
/// keeps `assets:partnerships:angel-continuity` one row while leaving
/// `assets:partnerships` alone: the fund's children are leaves, but
/// `assets:partnerships`'s child is not, so the walk stops. And the "at least
/// two" clause is what stops a lone `assets:vehicles:car` from being relabelled
/// `assets:vehicles`, which would trade a name for nothing.
///
/// Both clauses of rule 2 look ONLY at accounts this tab could show. Merging
/// exists to reunite the halves of one holding, so a sibling that can never be
/// a row here — a cash account, a `holdings: none`/`stocks` subtree, a child
/// rule 1 already claimed — is not a half of anything. Counting one would merge
/// a LONE qualifying child into a row wearing the container's name, whose value
/// (the qualifying subtree only) disagrees with the container's real subtree
/// balance on the balance sheet.
fn row_roots(
    txns: &[Transaction],
    classes: &BTreeMap<String, HoldingsClass>,
    types: &AccountTypes,
) -> BTreeMap<String, String> {
    let posted: BTreeSet<&str> = txns
        .iter()
        .flat_map(|txn| txn.postings.iter())
        .map(|posting| posting.account.0.as_str())
        .collect();

    // Accounts that ever post a non-currency amount: untagged, those are the
    // Stocks tab's material (`is_other_holding`'s commodity rule), so they are
    // never Other-tab merge material either.
    let non_currency: BTreeSet<&str> = txns
        .iter()
        .flat_map(|txn| txn.postings.iter())
        .filter(|posting| {
            posting
                .amounts
                .iter()
                .any(|amount| !is_currency(&amount.commodity.0))
        })
        .map(|posting| posting.account.0.as_str())
        .collect();

    // Whether `account` could be one half of a rule-2 merge: untagged (a tag
    // anywhere up the chain means rule 1 owns it, or the tab excludes it), an
    // exact Asset, and never booked in a non-currency commodity. The date-free
    // counterpart of `is_other_holding`, which rule 2 must approximate here
    // because the tree shape is precomputed once for every `as_of`.
    let mergeable = |account: &str| {
        resolve_holdings_class(account, classes).is_none()
            && types.resolve(account) == Some(AccountType::Asset)
            && !non_currency.contains(account)
    };

    // Per candidate parent: how many MERGEABLE posting-bearing descendants it
    // has, and whether every one of them is a direct child.
    let mut descendants: BTreeMap<&str, (usize, bool)> = BTreeMap::new();
    for account in posted.iter().filter(|account| mergeable(account)) {
        let mut ancestor = parent_of(account);
        while let Some(name) = ancestor {
            let direct = parent_of(account) == Some(name);
            let entry = descendants.entry(name).or_insert((0, true));
            entry.0 += 1;
            entry.1 &= direct;
            ancestor = parent_of(name);
        }
    }

    posted
        .iter()
        .map(|account| {
            // 1 — the nearest explicitly tagged ancestor-or-self.
            let mut name: Option<&str> = Some(account);
            while let Some(current) = name {
                if classes.contains_key(current) {
                    return ((*account).to_string(), current.to_string());
                }
                name = parent_of(current);
            }
            // 2 — a container parent, once, and only for merge material.
            let root = Some(*account)
                .filter(|candidate| mergeable(candidate))
                .and_then(parent_of)
                .filter(|parent| !posted.contains(*parent))
                .filter(|parent| {
                    descendants
                        .get(*parent)
                        .is_some_and(|(count, all_direct)| *count >= 2 && *all_direct)
                })
                .unwrap_or(account);
            ((*account).to_string(), root.to_string())
        })
        .collect()
}

/// One holding's figures at a date, accumulated over its whole subtree.
struct RowFacts {
    account: String,
    commodities: MixedAmount,
    value: Option<Dec>,
    cost: Option<Dec>,
    /// Commodities in the row's BALANCE with no route to the base (value is
    /// `None`).
    unpriced: Vec<Commodity>,
    /// Commodities in the row's at-cost BASIS with no route to the base (cost
    /// is `None`). Disjoint failure from `unpriced`: a written-off asset's cost
    /// residue can be unpriceable while the surviving balance is plain base
    /// currency.
    unpriced_cost: Vec<Commodity>,
}

/// Value `ma` in `base`, refusing a PARTIAL answer: if any commodity has no
/// route to the base, the caller gets `None` plus the offending commodities.
///
/// The balance sheet keeps the unpriced remainder visible in a mixed amount
/// instead, which is right for a report whose columns are mixed amounts. Here a
/// row is one number, and a number that silently omits part of the asset it
/// claims to describe is the "plausible figure instead of an error" failure this
/// codebase already refuses elsewhere.
fn value_or_nothing(
    ma: &MixedAmount,
    base: &Commodity,
    db: &PriceDb,
    as_of: &str,
) -> Result<(Option<Dec>, Vec<Commodity>), ReportError> {
    let mut meta = ValuationMeta::default();
    let priced = value_at(ma, base, db, as_of, Some(&mut meta))?;
    if meta.unpriced.is_empty() {
        Ok((Some(priced), Vec::new()))
    } else {
        Ok((None, meta.unpriced))
    }
}

/// The nearest declared `name:` for `account`, else its last path segment.
fn display_name(account: &str, tags: &HashMap<&str, &[(String, String)]>) -> String {
    let name = AccountName(account.to_string());
    inherited_account_tags(&name, tags)
        .into_iter()
        .find(|(key, value)| key == "name" && !value.trim().is_empty())
        .map(|(_, value)| value.trim().to_string())
        .unwrap_or_else(|| {
            account
                .rsplit(':')
                .next()
                .unwrap_or(account)
                .trim()
                .to_string()
        })
}

/// Whether an account holding `commodities` belongs on this tab, ignoring scope
/// and date.
///
/// Two rules, both of which must hold:
/// 1. an effective account type of EXACTLY [`AccountType::Asset`] — `Cash` folds
///    into `Asset` under `is_account_type`, so the loose test would drag in every
///    `type:C` account, which is precisely what this tab excludes;
/// 2. a `holdings:` class of `other`, or no class AND no non-currency commodity.
///
/// One function because [`rows_at`] and [`candidate_accounts`] must agree: an
/// account the scope chooser offers but the table can never show is a dead
/// option, and one the table shows but the chooser omits cannot be deselected.
fn is_other_holding(inputs: &OtherInputs<'_>, account: &str, commodities: &MixedAmount) -> bool {
    if inputs.types.resolve(account) != Some(AccountType::Asset) {
        return false;
    }
    match resolve_holdings_class(account, &inputs.classes) {
        Some(HoldingsClass::Other) => true,
        Some(HoldingsClass::Stocks | HoldingsClass::None) => false,
        None => commodities
            .iter()
            .all(|(commodity, _)| is_currency(&commodity.0)),
    }
}

/// Every account that could ever be a row, over the WHOLE journal and ignoring
/// the scope — what the scope chooser offers.
///
/// Neither scope- nor date-filtered, deliberately, and for the reason
/// `view.ts`'s `stockAccounts` gives on the Stocks tab: an option that vanishes
/// the moment you deselect it, or when you travel back a month, cannot be used
/// to compose a scope.
///
/// Membership is per-posting EVER-HELD, the `stock_accounts` semantics, not the
/// lifetime NET balance. A disposed asset nets to exactly zero over the whole
/// journal, yet it is still a row at any `as_of` before the disposal — and a
/// row the table shows that the chooser omits cannot be deselected. Summing
/// each posting's ABSOLUTE amount gives `is_other_holding` every commodity the
/// account ever held, not just what survived the netting.
fn candidate_accounts(inputs: &OtherInputs<'_>) -> Result<Vec<String>, ReportError> {
    let mut activity: BTreeMap<&str, MixedAmount> = BTreeMap::new();
    for posting in inputs.txns.iter().flat_map(|txn| txn.postings.iter()) {
        let entry = activity.entry(posting.account.0.as_str()).or_default();
        for amount in &posting.amounts {
            entry.accumulate(&amount.commodity, amount.quantity.abs()?)?;
        }
    }
    // Row ROOTS, deduped — the chooser must offer the same thing the table shows
    // it, or a holding the reader can see is one they cannot deselect. `BTreeSet`
    // both dedupes the siblings that share a root and keeps the list sorted.
    Ok(activity
        .iter()
        .filter(|(account, gross)| {
            let mut commodities = (*gross).clone();
            commodities.drop_zeros();
            !commodities.is_zero() && is_other_holding(inputs, account, &commodities)
        })
        .map(|(account, _)| {
            inputs
                .row_roots
                .get(*account)
                .cloned()
                .unwrap_or_else(|| (*account).to_string())
        })
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect())
}

/// Every in-scope other holding at `as_of`, unsorted: the accounts passing
/// [`is_other_holding`] that also hold a non-zero balance at `as_of` and sit
/// inside the scope.
fn rows_at(
    inputs: &OtherInputs<'_>,
    scope: &HoldingsScope,
    as_of: &str,
    base: &Commodity,
) -> Result<Vec<RowFacts>, ReportError> {
    let in_scope = scope_accounts(scope);
    let totals = |at_cost: bool| {
        account_totals(
            inputs.txns,
            &PostingFilter {
                to: Some(as_of),
                at_cost,
                ..PostingFilter::default()
            },
        )
    };
    let written = totals(false)?;
    let at_cost = totals(true)?;

    // Accumulate per ROW ROOT rather than per account, so an asset split across
    // sibling accounts is one holding. `held` is the whole subtree (the value);
    // `basis` takes only the accounts that are not a mark (the cost). The
    // difference between them IS the unrealized gain, which is the entire reason
    // the two are tracked apart.
    let mut held: BTreeMap<&str, MixedAmount> = BTreeMap::new();
    let mut basis: BTreeMap<&str, MixedAmount> = BTreeMap::new();
    for (account, balance) in &written {
        let mut commodities = balance.clone();
        commodities.drop_zeros();
        if commodities.is_zero()
            || !in_scope(account)
            || !is_other_holding(inputs, account, &commodities)
        {
            continue;
        }
        let root = inputs
            .row_roots
            .get(account)
            .map_or(account.as_str(), String::as_str);
        held.entry(root).or_default().ma_add_assign(&commodities)?;

        // A mark contributes to value and NOT to cost. Everything else is money
        // in, taken at cost (hledger `-B`), which is also what keeps a
        // commodity-booked asset's basis at what was paid for it.
        if resolve_valuation_role(account, &inputs.roles) == ValuationRole::Cost
            && let Some(ma) = at_cost.get(account)
        {
            basis.entry(root).or_default().ma_add_assign(ma)?;
        }
    }

    let mut rows = Vec::with_capacity(held.len());
    for (root, mut commodities) in held {
        commodities.drop_zeros();
        if commodities.is_zero() {
            continue;
        }
        let (value, unpriced) = value_or_nothing(&commodities, base, &inputs.db, as_of)?;
        // No cost side at all means the holding is ALL mark — nothing was ever
        // recorded as money in — so its cost is unknown rather than zero, and it
        // drops out of the cost and change totals instead of claiming an
        // infinite gain. A cost side that EXISTS but cannot be priced is the
        // same `None` with a different cause, so the offending commodities are
        // kept for the warning rather than discarded.
        let (cost, unpriced_cost) = match basis.get(root) {
            Some(ma) => value_or_nothing(ma, base, &inputs.db, as_of)?,
            None => (None, Vec::new()),
        };
        rows.push(RowFacts {
            account: root.to_string(),
            commodities,
            value,
            cost,
            unpriced,
            unpriced_cost,
        });
    }
    Ok(rows)
}

/// Non-stock, non-cash assets for the scoped journal as of `scope.as_of`.
///
/// `scope.gain_since` means here exactly what it means on the Stocks tab
/// (`HoldingsScope::gain_since`): `None` measures change against the all-time
/// cost, `Some(start)` against the account's value at `start`. So a dollar-booked
/// van reports an all-time change of exactly zero — cost IS value — which is the
/// correct answer for an asset nobody has revalued, not a missing feature.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or an unrecognized `holdings:`
/// class.
pub fn other_holdings(
    txns: &[Transaction],
    prices: &[PriceDirective],
    accounts: &[AccountDeclaration],
    scope: &HoldingsScope,
) -> Result<OtherHoldingsReport, ReportError> {
    let inputs = OtherInputs::build(txns, prices, accounts)?;
    let base = inputs.base(scope);
    let rows = rows_at(&inputs, scope, &scope.as_of, &base)?;

    // The windowed reference: each account's value at the window start, priced
    // as of the start.
    //
    // Three cases, and the difference between the last two matters. ABSENT from
    // this map = not held at `start`, which references zero: buying a car
    // mid-window shows the whole car as that window's change, which is what the
    // phrase has to mean if the rows are to sum to the total. Present-and-`Some`
    // = held and priced. Present-and-`None` = held at `start` but UNPRICEABLE
    // then, which propagates null rather than collapsing to zero — treating it
    // as zero would report the asset's entire current value as this window's
    // change, a fabricated number that looks perfectly plausible. Same
    // null-propagation the stock engine documents on `HoldingsScope::gain_since`.
    let opening: BTreeMap<String, Option<Dec>> = match scope.gain_since.as_deref() {
        Some(start) => rows_at(&inputs, scope, start, &base)?
            .into_iter()
            .map(|row| (row.account, row.value))
            .collect(),
        None => BTreeMap::new(),
    };

    let mut holdings = Vec::with_capacity(rows.len());
    let mut warnings = Vec::new();
    let mut total_value = Dec::zero();
    let mut total_cost: Option<Dec> = None;
    let mut total_change: Option<Dec> = None;
    let mut total_reference: Option<Dec> = None;

    for row in rows {
        if !row.unpriced.is_empty() {
            let names: Vec<&str> = row.unpriced.iter().map(|c| c.0.as_str()).collect();
            warnings.push(OtherHoldingsWarning {
                account: row.account.clone(),
                kind: OtherWarningKind::Unpriced,
                message: format!(
                    "{} holds {} with no price in {} as of {} — excluded from totals",
                    row.account,
                    names.join(", "),
                    base.0,
                    scope.as_of
                ),
            });
        } else if !row.unpriced_cost.is_empty() {
            // Only when the value itself is priced: an unpriced value already
            // excluded the row and warned wholesale, and a second warning about
            // its (equally unpriceable) cost would restate the first.
            let names: Vec<&str> = row.unpriced_cost.iter().map(|c| c.0.as_str()).collect();
            warnings.push(OtherHoldingsWarning {
                account: row.account.clone(),
                kind: OtherWarningKind::UnpricedCost,
                message: format!(
                    "{}'s cost includes {} with no price in {} as of {} — cost and change unknown",
                    row.account,
                    names.join(", "),
                    base.0,
                    scope.as_of
                ),
            });
        }

        let reference = match scope.gain_since.as_deref() {
            // Absent = not held at the window start = a real zero; present-and-
            // `None` = held but unpriceable then, and stays `None`.
            Some(_) => opening
                .get(&row.account)
                .copied()
                .unwrap_or_else(|| Some(Dec::zero())),
            None => row.cost,
        };
        let change = match (row.value, reference) {
            (Some(value), Some(reference)) => Some(value.sub(reference)?),
            _ => None,
        };
        let change_pct = match (change, reference) {
            (Some(change), Some(reference)) => gain_pct(change, reference),
            _ => None,
        };

        if let Some(value) = row.value {
            total_value = total_value.add(value)?;
            if let Some(cost) = row.cost {
                total_cost = Some(match total_cost {
                    Some(sum) => sum.add(cost)?,
                    None => cost,
                });
            }
            if let (Some(change), Some(reference)) = (change, reference) {
                total_change = Some(match total_change {
                    Some(sum) => sum.add(change)?,
                    None => change,
                });
                total_reference = Some(match total_reference {
                    Some(sum) => sum.add(reference)?,
                    None => reference,
                });
            }
        }

        holdings.push(OtherHolding {
            name: display_name(&row.account, &inputs.account_tags),
            account: row.account,
            commodities: row.commodities,
            value: row.value,
            cost: row.cost,
            change,
            change_pct,
        });
    }

    // Value desc, unpriced last, then by account — the stock report's ordering,
    // so the two tables read the same way.
    holdings.sort_by(|a, b| match (a.value, b.value) {
        (None, None) => a.account.cmp(&b.account),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.account.cmp(&b.account)),
    });

    let totals = OtherHoldingsTotals {
        value: total_value,
        cost: total_cost,
        change: total_change,
        change_pct: match (total_change, total_reference) {
            (Some(change), Some(reference)) => gain_pct(change, reference),
            _ => None,
        },
    };
    Ok(OtherHoldingsReport {
        as_of: scope.as_of.clone(),
        base: base.0,
        holdings,
        accounts: candidate_accounts(&inputs)?,
        totals,
        warnings,
    })
}

/// Total other-holdings value (and cost) at each of the last `count` period
/// boundaries ending at `scope.as_of`, oldest first.
///
/// Returns the stock tab's [`HoldingsSeries`] unchanged so one chart component
/// draws both trends: `market_value` is the summed row values at each bucket
/// end, `basis` the summed costs.
///
/// The base commodity is resolved ONCE from `scope.as_of` and every point pinned
/// to it, for [`holdings_series`](super::series::holdings_series)' reason: a
/// trend line whose units change partway along is worse than no trend.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow, a bad bucket key, or an
/// unrecognized `holdings:` class.
pub fn other_holdings_series(
    txns: &[Transaction],
    prices: &[PriceDirective],
    accounts: &[AccountDeclaration],
    scope: &HoldingsScope,
    interval: Interval,
    count: usize,
) -> Result<HoldingsSeries, ReportError> {
    let keys = last_n_buckets(&scope.as_of, interval, count)?;
    let inputs = OtherInputs::build(txns, prices, accounts)?;
    let base = inputs.base(scope);

    let mut has_basis = false;
    let mut points = Vec::with_capacity(keys.len());
    for key in &keys {
        // Clamped so the final point never overshoots `scope.as_of`.
        let end = bucket_end(key)?;
        let date = if compare_iso(&end, &scope.as_of) == Ordering::Greater {
            scope.as_of.clone()
        } else {
            end
        };
        // One `account_totals` pass per point. The stock series went to some
        // length to avoid that (PERF-5b) because its per-point cost included
        // rebuilding the price database and re-sorting the journal; here those
        // are hoisted into `OtherInputs` and what remains is the aggregation
        // itself. If a profile ever says otherwise, the fix is `net_worth`'s
        // shape — bucket the postings once and prefix-sum — not a second
        // aggregation primitive.
        let rows = rows_at(&inputs, scope, &date, &base)?;
        let mut market_value = Dec::zero();
        let mut basis: Option<Dec> = None;
        for row in rows {
            if let Some(value) = row.value {
                market_value = market_value.add(value)?;
                if let Some(cost) = row.cost {
                    basis = Some(match basis {
                        Some(sum) => sum.add(cost)?,
                        None => cost,
                    });
                }
            }
        }
        if basis.is_some() {
            has_basis = true;
        }
        points.push(HoldingsPoint {
            date,
            bucket: key.clone(),
            label: bucket_label(key),
            market_value,
            basis,
        });
    }
    Ok(HoldingsSeries {
        base: base.0,
        points,
        has_basis,
    })
}
