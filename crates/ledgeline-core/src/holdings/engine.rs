//! Average-cost stock-holdings engine — port of
//! `web/src/lib/holdings/engine.ts`.
//!
//! One average-cost pool per symbol across the WHOLE scope (not per account): an
//! in-scope→in-scope transfer nets to zero shares and zero basis impact. Basis
//! is kept in the valuation base commodity; a cost-less acquisition lot taints
//! the pool (`basis = None`) — we never guess a basis from price directives.
//!
//! Four movements are NOT ordinary acquisitions and are handled apart, because
//! reading them as one produced a confidently-wrong number:
//! - a **split** re-denominates an open position — it scales `shares` and leaves
//!   `basis`/`first_basis_date` alone (see [`is_redenomination`]);
//! - a **same-transaction round trip** at two different prices is not the pure
//!   transfer its zero net makes it look like (see [`is_pure_transfer`]);
//! - a pool that ever goes **negative** is tainted for good: the lot that was
//!   oversold was never entered, so nothing bought later has a knowable average
//!   cost. A pool that is STILL negative at `as_of` is reported all the same —
//!   its market value is real on the balance sheet, so withholding it left
//!   `Σ market_value + cash` disagreeing with net worth (see
//!   [`compute_holdings`]); presentation is the SPA's business, not the engine's;
//! - a **return of capital** (cash out of a one-security account) reduces basis.
//!
//! Under a `gain_since` window the gain is `mv(as_of) − mv(start) − flows`, so
//! money paid in or taken out moves the baseline instead of masquerading as a
//! gain (see `reference_of` in [`compute_holdings`]).
//!
//! All money math is exact [`Dec`], reusing the same non-normalizing multiply
//! (`reports::prices::mul_raw`) as the valuation engine so the ported numbers
//! line up with the TS `domain/money` representation bit-for-bit. The only
//! rounding is the half-even sell reduction (`div_round_half_even`), matching the
//! TS `divRoundHalfEven`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::decimal::{Dec, DecError};
use crate::model::{AccountDeclaration, Commodity, Cost, CostKind, PriceDirective, Transaction};
use crate::reports::account_types::{account_decls_from, declared_types, resolve_account_type};
use crate::reports::prices::{div_round_half_even, mul_raw, per_unit_from_total, pow10};
use crate::reports::{AccountType, PriceDb, ReportError, account_matches};
use crate::wire::{account_tag_map, inherited_account_tags};

use super::commodities::is_currency;
use super::types::{
    Holding, HoldingPrice, HoldingsReport, HoldingsScope, HoldingsTotals, HoldingsWarning,
    PriceSource, ScopeMode, WarningKind,
};

/// Rescale both operands to a common precision and return the mantissa pair
/// (port of the TS `commonMantissas`).
fn common_mantissas(a: Dec, b: Dec) -> Result<(i128, i128), ReportError> {
    let places = a.places.max(b.places);
    let scale = |x: Dec| -> Result<i128, ReportError> {
        let factor = pow10(places - x.places)?;
        Ok(x.mantissa.checked_mul(factor).ok_or(DecError::Overflow)?)
    };
    Ok((scale(a)?, scale(b)?))
}

/// Average-cost basis left after a sell: `basis × sharesAfter / sharesBefore`,
/// computed exactly on mantissas and rounded HALF-EVEN to the basis's own
/// precision (port of the TS `reduceBasis`).
fn reduce_basis(basis: Dec, shares_after: Dec, shares_before: Dec) -> Result<Dec, ReportError> {
    let (after_m, before_m) = common_mantissas(shares_after, shares_before)?;
    let numerator = basis
        .mantissa
        .checked_mul(after_m)
        .ok_or(DecError::Overflow)?;
    Ok(Dec::new(
        div_round_half_even(numerator, before_m)?,
        basis.places,
    ))
}

/// True when securities can actually be HELD in `account` — i.e. its root is not
/// equity/income/expense. Those three are the funding/disposal counter-side of a
/// share movement (the "source" of the shares, exactly like `equity:opening` for
/// cash), so a share leg posted to them must NOT count toward a symbol's net
/// shares. If it did, a share transfer-in booked against `equity`/`income` would
/// net the acquiring transaction to zero (the shares never enter the pool) and a
/// later sale would drive the per-symbol net negative — even though the balance
/// sheet, which sums only asset + liability accounts, shows it non-negative. This
/// keeps holdings' net shares equal to the balance-sheet net for the symbol.
fn is_holding_account(account: &str, declared: &BTreeMap<String, AccountType>) -> bool {
    !matches!(
        resolve_account_type(account, declared),
        Some(AccountType::Equity | AccountType::Revenue | AccountType::Expense)
    )
}

/// The magnitude of a share count rendered to exactly two decimal places,
/// computed on the exact mantissa (never via `f64`) for the negative-shares
/// warning text. Fractional places beyond two are truncated toward zero — a
/// share deficit below a hundredth of a share needs no finer detail in a
/// human-readable warning. Overflow-saturating, so it is panic-free.
fn abs_shares_2dp(shares: Dec) -> String {
    let magnitude = shares.mantissa.unsigned_abs();
    let hundredths = match shares.places.cmp(&2) {
        Ordering::Less => magnitude.saturating_mul(10u128.saturating_pow(2 - shares.places)),
        Ordering::Equal => magnitude,
        Ordering::Greater => magnitude / 10u128.saturating_pow(shares.places - 2),
    };
    format!("{}.{:02}", hundredths / 100, hundredths % 100)
}

/// A dated per-unit price in the base commodity (port of the TS `DatedPrice`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct DatedPrice {
    qty: Dec,
    date: String,
}

/// Why a pool's basis is unknown. Distinct from the wire-level [`WarningKind`]
/// (which the SPA matches exhaustively and so cannot grow here) purely so the
/// warning MESSAGE can name the actual cause — all three used to be reported
/// identically as "acquired without a cost annotation".
#[derive(Debug, Clone, PartialEq, Eq)]
enum TaintReason {
    /// An acquisition lot arrived with no cost annotation at all.
    CostlessLot,
    /// A lot's cost was annotated in a commodity with no rate to the base.
    UnconvertibleCost(String),
    /// Net shares dipped below zero, so the average cost of whatever is held
    /// afterwards is unknowable (see the sticky taint in [`build_pools`]).
    WentNegative,
}

/// Average-cost pool for one stock symbol. Only the fields consumed by
/// [`compute_holdings`] are tracked (the TS `costlessBuyTxns`/`negativeCrossTxn`/
/// `lastTxnIndex` feed the separate WP-10 check rules, which are out of scope
/// here).
///
/// `Clone` so a replay can FREEZE the running pool at a series boundary and keep
/// going — the pool at each point is the same object the single-date build
/// finished with, not a re-derivation of it.
#[derive(Clone)]
struct SymbolPool {
    /// Net shares over processed postings (may be zero or negative).
    shares: Dec,
    /// Running basis in the base commodity; meaningful only when `taint` is
    /// `None`.
    basis: Dec,
    /// Why the basis is untrustworthy; `None` while it is still exact. First
    /// cause wins, so the message names the EARLIEST thing that went wrong.
    taint: Option<TaintReason>,
    /// True once net shares dipped below zero. Sticky: re-buying does not
    /// restore a knowable average cost, because the lot that was oversold was
    /// never entered in the first place.
    went_negative: bool,
    /// Net value contributed to this symbol inside the gain window
    /// `(gain_since, as_of]` — buy costs minus sell proceeds, in the base
    /// commodity. `None` once an in-window leg could not be valued at all;
    /// always `Some(0)` when no window is set.
    window_flow: Option<Dec>,
    /// Date the current position was opened (reset on each re-open); `None`
    /// until a buy is seen.
    first_basis_date: Option<String>,
    /// Accounts whose own net shares are `> 0`, sorted.
    accounts: Vec<String>,
    /// Latest `name:` tag seen — posting-comment tag first, then the
    /// `commodity`-directive tag (keyed by symbol), then the account's own +
    /// ancestors' declared tag, then the txn tag — else the symbol.
    name: String,
}

impl SymbolPool {
    fn new(symbol: &str) -> Self {
        Self {
            shares: Dec::zero(),
            basis: Dec::zero(),
            taint: None,
            went_negative: false,
            window_flow: Some(Dec::zero()),
            first_basis_date: None,
            accounts: Vec::new(),
            name: symbol.to_string(),
        }
    }

    /// Record `reason` unless the basis is already tainted (first cause wins).
    fn taint_with(&mut self, reason: TaintReason) {
        if self.taint.is_none() {
            self.taint = Some(reason);
        }
    }
}

/// One in-scope stock leg gathered from a posting.
struct LotEntry {
    qty: Dec,
    cost: Option<Cost>,
}

/// The value of the first `name` tag in `tags`, if present.
fn tag_value<'a>(tags: &'a [(String, String)], name: &str) -> Option<&'a str> {
    tags.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Map each `commodity` directive's symbol to its declared `name:` tag, if it
/// has one. A directive with other tags (`CUSIP`/`basis`/`type`) but no `name`
/// is omitted entirely, so the name resolver falls through to the next
/// precedence rung rather than picking a wrong tag. This is the canonical,
/// intentional place a security is named. Mirrors [`crate::wire::account_tag_map`]
/// (built once in [`compute_holdings`], threaded into [`build_pools`]).
fn commodity_name_map(
    commodity_tags: &[(Commodity, Vec<(String, String)>)],
) -> HashMap<&str, &str> {
    commodity_tags
        .iter()
        .filter_map(|(commodity, tags)| {
            tag_value(tags, "name").map(|name| (commodity.0.as_str(), name))
        })
        .collect()
}

/// Journal order: date asc, then txn index asc (input order is never assumed).
fn journal_order(txns: &[Transaction]) -> Vec<&Transaction> {
    let mut ordered: Vec<&Transaction> = txns.iter().collect();
    ordered.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.index.0.cmp(&b.index.0)));
    ordered
}

/// A buy lot's cost in the base commodity, or `None` when it has none (or an
/// unconvertible one). Port of the TS `costInBase`.
fn cost_in_base(
    qty: Dec,
    cost: Option<&Cost>,
    db: &PriceDb,
    base: &Commodity,
    date: &str,
) -> Result<Option<Dec>, ReportError> {
    let Some(cost) = cost else {
        return Ok(None);
    };
    let own = if cost.kind == CostKind::Unit {
        mul_raw(qty, cost.amount.quantity)?
    } else {
        cost.amount.quantity
    };
    if cost.amount.commodity == *base {
        return Ok(Some(own));
    }
    match db.lookup_in(&cost.amount.commodity, base, date) {
        Some(rate) => Ok(Some(mul_raw(own, rate.quantity)?)),
        None => Ok(None),
    }
}

/// A leg's cost in the base commodity, SIGNED by the leg's own direction: a buy
/// is value flowing INTO the pool, a sell value flowing out. `@@` totals are
/// written unsigned in a journal (`-10 VTI @@ $1500.00`), so their sign has to
/// come from the quantity rather than from the annotation.
fn signed_cost_in_base(
    qty: Dec,
    cost: &Cost,
    db: &PriceDb,
    base: &Commodity,
    date: &str,
) -> Result<Option<Dec>, ReportError> {
    let Some(value) = cost_in_base(qty, Some(cost), db, base, date)? else {
        return Ok(None);
    };
    if cost.kind == CostKind::Unit {
        return Ok(Some(value)); // `qty × unit_price` already carries the sign
    }
    let magnitude = value.abs()?;
    Ok(Some(if qty.mantissa < 0 {
        magnitude.neg()?
    } else {
        magnitude
    }))
}

/// The value one in-scope leg moved into (`> 0`) or out of (`< 0`) the pool, in
/// the base commodity — a contribution or a withdrawal for the windowed gain.
/// A cost annotation is authoritative; a bare leg (an external transfer in or
/// out) falls back to the market price on its own date. `None` when neither is
/// available, which makes the whole window's gain honestly unknown rather than
/// quietly wrong.
fn leg_flow(
    entry: &LotEntry,
    commodity: &Commodity,
    db: &PriceDb,
    base: &Commodity,
    date: &str,
) -> Result<Option<Dec>, ReportError> {
    match entry.cost.as_ref() {
        Some(cost) => signed_cost_in_base(entry.qty, cost, db, base, date),
        None => db
            .lookup_in(commodity, base, date)
            .map(|price| mul_raw(entry.qty, price.quantity))
            .transpose()
            .map_err(ReportError::from),
    }
}

/// True when `symbol` is the ONLY commodity whose quantity moves in `txn` — no
/// cash consideration, no second security. A split re-labels one security's
/// share count; nothing else changes hands.
fn moves_only(txn: &Transaction, symbol: &str) -> bool {
    txn.postings
        .iter()
        .flat_map(|posting| &posting.amounts)
        .all(|amount| amount.quantity.is_zero() || amount.commodity.0 == symbol)
}

/// True when any posting of `txn` moves a non-zero quantity of `symbol`.
fn moves_symbol(txn: &Transaction, symbol: &str) -> bool {
    txn.postings
        .iter()
        .flat_map(|posting| &posting.amounts)
        .any(|amount| amount.commodity.0 == symbol && !amount.quantity.is_zero())
}

/// True when every `symbol` amount in `txn` — in any account, in or out of
/// scope — is free of a cost annotation.
fn all_symbol_legs_costless(txn: &Transaction, symbol: &str) -> bool {
    txn.postings
        .iter()
        .flat_map(|posting| &posting.amounts)
        .all(|amount| amount.commodity.0 != symbol || amount.cost.is_none())
}

/// True when any posting of `txn` lands in a revenue or expense account. Those
/// mark compensation (an RSU vest), a fee, or a gift — value really did arrive
/// or leave, so the transaction is never a pure re-denomination.
fn touches_income_or_expense(txn: &Transaction, declared: &BTreeMap<String, AccountType>) -> bool {
    txn.postings.iter().any(|posting| {
        matches!(
            resolve_account_type(&posting.account.0, declared),
            Some(AccountType::Revenue | AccountType::Expense)
        )
    })
}

/// Signed total of `symbol` legs posted to EQUITY accounts in `txn` — the
/// counter-side that absorbs a split booked as `assets +n / equity:splits -n`.
fn equity_share_total(
    txn: &Transaction,
    symbol: &str,
    declared: &BTreeMap<String, AccountType>,
) -> Result<Dec, ReportError> {
    let mut total = Dec::zero();
    for posting in &txn.postings {
        if resolve_account_type(&posting.account.0, declared) != Some(AccountType::Equity) {
            continue;
        }
        for amount in &posting.amounts {
            if amount.commodity.0 == symbol {
                total = total.add(amount.quantity)?;
            }
        }
    }
    Ok(total)
}

/// True when this transaction RE-DENOMINATES an already-open `symbol` position
/// (a stock split, or a reverse split) rather than changing the value held. A
/// split must leave `basis` and `first_basis_date` alone and only scale the
/// share count — treating its cost-less share leg as an acquisition destroys
/// the basis of a cleanly-annotated position.
///
/// The test is deliberately narrow. Everything below must hold:
/// - the position is already open in scope (`shares > 0`) — a cost-less arrival
///   into an EMPTY pool is a transfer-in whose basis genuinely is unknown;
/// - no `symbol` leg anywhere in the transaction carries a cost annotation;
/// - `symbol` is the only commodity that moves (no cash, no second security);
/// - no revenue/expense account is touched (that would be a vest, fee or gift);
/// - and the shares are visibly absorbed rather than sourced: either the
///   in-scope legs contain BOTH signs (the same-transaction `-n`/`+m` spelling)
///   or an equity leg carries the opposite sign to the net (`equity:splits`).
///
/// **Known ambiguity.** An ACATS-style share transfer-in booked against equity
/// (`assets:brokerA +5 TSLA` / `equity:transfers -5 TSLA`) is spelled exactly
/// like the second form. When the pool is EMPTY the two are told apart (a
/// transfer-in taints, as before); when the same symbol is already held they are
/// indistinguishable from the journal alone, and this reads it as a split. That
/// is the trade the fix accepts: silently keeping a correct basis through the
/// common case (splits) at the cost of an over-optimistic basis in the rare one
/// (topping up a position by external transfer with no cost recorded).
fn is_redenomination(
    txn: &Transaction,
    symbol: &str,
    entries: &[LotEntry],
    net: Dec,
    shares_before: Dec,
    declared: &BTreeMap<String, AccountType>,
) -> Result<bool, ReportError> {
    if shares_before.mantissa <= 0
        || net.is_zero()
        || !all_symbol_legs_costless(txn, symbol)
        || !moves_only(txn, symbol)
        || touches_income_or_expense(txn, declared)
    {
        return Ok(false);
    }
    let two_sided = entries.iter().any(|entry| entry.qty.mantissa > 0)
        && entries.iter().any(|entry| entry.qty.mantissa < 0);
    if two_sided {
        return Ok(true);
    }
    let equity = equity_share_total(txn, symbol, declared)?;
    Ok(equity.mantissa != 0 && (equity.mantissa > 0) != (net.mantissa > 0))
}

/// True when a zero-net set of in-scope legs really is a pure MOVE between
/// in-scope accounts (leave the basis alone) rather than a same-transaction
/// round trip that re-prices the position.
///
/// Cost-compatible means one of two things: nothing was re-costed on the way in
/// (every incoming leg is bare, the ordinary transfer spelling), or the legs'
/// costs cancel exactly (the position was moved at a single price). A sell at
/// one price paired with a re-buy at another moves real money and must be
/// processed leg by leg.
fn is_pure_transfer(
    entries: &[LotEntry],
    db: &PriceDb,
    base: &Commodity,
    date: &str,
) -> Result<bool, ReportError> {
    if entries
        .iter()
        .filter(|entry| entry.qty.mantissa > 0)
        .all(|entry| entry.cost.is_none())
    {
        return Ok(true);
    }
    let mut total = Dec::zero();
    for entry in entries {
        let Some(cost) = entry.cost.as_ref() else {
            return Ok(false); // a bare leg alongside costed ones: not comparable
        };
        let Some(value) = signed_cost_in_base(entry.qty, cost, db, base, date)? else {
            return Ok(false); // unconvertible cost: cannot prove it nets out
        };
        total = total.add(value)?;
    }
    Ok(total.is_zero())
}

/// For every in-scope holding account, the single stock symbol it holds, or
/// `None` when it holds several. A currency posting into a one-security account
/// is a basis adjustment (a return of capital); in an account that mixes cash
/// and shares it cannot be attributed to anything and is left alone.
///
/// The DEFINITION of the rule, kept as the oracle
/// [`sole_symbols_at`] is checked against; the replay reads the equivalent
/// [`SoleSymbolFacts`] instead so no `as_of` needs its own journal pass.
#[cfg(test)]
fn sole_symbols_by_account(
    txns: &[Transaction],
    as_of: &str,
    in_scope: &dyn Fn(&str) -> bool,
    declared: &BTreeMap<String, AccountType>,
) -> BTreeMap<String, Option<String>> {
    let mut sole: BTreeMap<String, Option<String>> = BTreeMap::new();
    for txn in txns {
        if txn.date.as_str() > as_of {
            continue;
        }
        for posting in &txn.postings {
            if !in_scope(&posting.account.0) || !is_holding_account(&posting.account.0, declared) {
                continue;
            }
            for amount in &posting.amounts {
                if is_currency(&amount.commodity.0) {
                    continue;
                }
                let slot = sole
                    .entry(posting.account.0.clone())
                    .or_insert_with(|| Some(amount.commodity.0.clone()));
                if slot.as_deref() != Some(amount.commodity.0.as_str()) {
                    *slot = None; // a second security: no longer attributable
                }
            }
        }
    }
    sole
}

/// What [`sole_symbols_by_account`] answers for one account at ANY `as_of`,
/// reduced to two dates.
///
/// This is the ONE thing in the replay that looks past the transaction being
/// processed: the sole-symbol map is resolved from the whole `≤ as_of` range and
/// then applied to every transaction in it, so a single shared replay cannot
/// simply carry it forward — an account that is one-security in July and
/// two-security in September answers differently for a July point and a
/// September one, about the very same July transaction. Reducing it to
/// `(first_date, symbol, ambiguous_from)` makes any date's answer an O(accounts)
/// read, which is what lets [`pool_snapshots`] tell cheaply whether the points
/// agree (they nearly always do) and share one replay.
struct SoleSymbolFacts {
    /// Date of the account's first in-scope non-currency posting. Before it the
    /// account is ABSENT from the map — so a cash leg dated EARLIER than the
    /// account's first security reduces no basis at an early `as_of`, but does
    /// at a later one.
    first_date: String,
    /// The commodity of that first posting: the sole symbol for as long as the
    /// account holds only one.
    symbol: String,
    /// Date a SECOND distinct commodity first appeared, from which the account
    /// answers `Some(None)` — held securities are no longer attributable.
    ambiguous_from: Option<String>,
}

/// [`SoleSymbolFacts`] per account, for every `as_of` up to `as_of`.
///
/// Restricted to the accounts the replay can actually ASK about — those that
/// take a currency leg OUT at some point — because the restriction is what keeps
/// a series to one replay. An ordinary `assets:broker:cash` is asked about
/// constantly but holds no security, so it has no facts and answers "absent"
/// forever; an ordinary `assets:broker:vti` holds one but never pays cash out,
/// so it is never asked. Dropping both leaves most journals with an EMPTY map,
/// which is trivially the same at every point.
fn sole_symbol_facts(
    ordered: &[&Transaction],
    as_of: &str,
    in_scope: &dyn Fn(&str) -> bool,
    declared: &BTreeMap<String, AccountType>,
) -> BTreeMap<String, SoleSymbolFacts> {
    let mut facts: BTreeMap<String, SoleSymbolFacts> = BTreeMap::new();
    let mut asked_about: BTreeSet<&str> = BTreeSet::new();
    for txn in ordered {
        if txn.date.as_str() > as_of {
            break; // `ordered` is date-ascending
        }
        for posting in &txn.postings {
            if !in_scope(&posting.account.0) || !is_holding_account(&posting.account.0, declared) {
                continue;
            }
            for amount in &posting.amounts {
                if is_currency(&amount.commodity.0) {
                    if amount.quantity.mantissa < 0 {
                        asked_about.insert(posting.account.0.as_str());
                    }
                    continue;
                }
                match facts.get_mut(&posting.account.0) {
                    Some(known) => {
                        if known.ambiguous_from.is_none() && known.symbol != amount.commodity.0 {
                            known.ambiguous_from = Some(txn.date.clone());
                        }
                    }
                    None => {
                        facts.insert(
                            posting.account.0.clone(),
                            SoleSymbolFacts {
                                first_date: txn.date.clone(),
                                symbol: amount.commodity.0.clone(),
                                ambiguous_from: None,
                            },
                        );
                    }
                }
            }
        }
    }
    facts.retain(|account, _| asked_about.contains(account.as_str()));
    facts
}

/// [`sole_symbols_by_account`]'s answer at `as_of`, read off precomputed facts
/// instead of re-scanning the journal.
fn sole_symbols_at(
    facts: &BTreeMap<String, SoleSymbolFacts>,
    as_of: &str,
) -> BTreeMap<String, Option<String>> {
    facts
        .iter()
        .filter(|(_, known)| known.first_date.as_str() <= as_of)
        .map(|(account, known)| {
            let sole = match known.ambiguous_from.as_deref() {
                Some(from) if from <= as_of => None,
                _ => Some(known.symbol.clone()),
            };
            (account.clone(), sole)
        })
        .collect()
}

/// Everything a holdings replay reads that does NOT depend on `as_of`: the one
/// date-ordered view of the journal, the price database, security names from
/// account and `commodity` directives, and declared account types (so a share
/// movement's funding leg is recognized by TYPE rather than by what its root is
/// called).
///
/// Hoisting these is half of PERF-5b: a 12-point series used to rebuild all of
/// it — including a fresh `PriceDb` and a fresh sort of the whole journal —
/// twelve times over.
struct HoldingsInputs<'a> {
    txns: &'a [Transaction],
    /// Journal order: date asc, then txn index asc. The only sort the report
    /// needs, shared by the replay and by every `latest_cost_prices` scan
    /// `choose_base` runs per candidate.
    ordered: Vec<&'a Transaction>,
    db: PriceDb,
    account_tags: HashMap<&'a str, &'a [(String, String)]>,
    commodity_names: HashMap<&'a str, &'a str>,
    declared: BTreeMap<String, AccountType>,
}

impl<'a> HoldingsInputs<'a> {
    fn build(
        txns: &'a [Transaction],
        prices: &[PriceDirective],
        accounts: &'a [AccountDeclaration],
        commodity_tags: &'a [(Commodity, Vec<(String, String)>)],
    ) -> Self {
        Self {
            txns,
            ordered: journal_order(txns),
            db: PriceDb::build(prices),
            account_tags: account_tag_map(accounts),
            commodity_names: commodity_name_map(commodity_tags),
            declared: declared_types(&account_decls_from(accounts)),
        }
    }
}

/// The replay state frozen at one `as_of` — exactly what a single-date
/// [`build_pools`]-plus-[`latest_cost_prices`] pair used to hand
/// [`assemble_report`].
#[derive(Default)]
struct PoolSnapshot {
    pools: BTreeMap<String, SymbolPool>,
    cost_prices: BTreeMap<String, DatedPrice>,
}

/// Freeze the running state into a report-ready snapshot: clone the pools and
/// fill in each one's `accounts` (the in-scope accounts whose OWN net shares are
/// still positive) from the per-account tallies — the same final pass the
/// single-date build did, now done once per boundary.
fn freeze(
    pools: &BTreeMap<String, SymbolPool>,
    per_account: &BTreeMap<String, BTreeMap<String, Dec>>,
    cost_prices: &BTreeMap<String, DatedPrice>,
) -> PoolSnapshot {
    let mut pools = pools.clone();
    for (symbol, accounts) in per_account {
        if let Some(pool) = pools.get_mut(symbol) {
            // BTreeMap key order is lexical, matching the TS explicit `.sort()`.
            pool.accounts = accounts
                .iter()
                .filter(|(_, shares)| shares.mantissa > 0)
                .map(|(account, _)| account.clone())
                .collect();
        }
    }
    PoolSnapshot {
        pools,
        cost_prices: cost_prices.clone(),
    }
}

/// Build one average-cost pool per stock symbol — plus the latest usable cost
/// price per symbol — from postings whose account passes `in_scope`, snapshotting
/// the running state at each date in `as_ofs`. Port of the TS `buildPools`; see
/// that function's doc for the netting/taint/reduction rules.
///
/// `as_ofs` is ASCENDING, and each snapshot is what a replay stopped at that
/// date would have produced, because every rule below reads only the
/// transaction being processed and the pool state before it — the one exception,
/// the sole-symbol map, is passed in already resolved for these dates (see
/// [`pool_snapshots`]). This is the other half of PERF-5b: 12 points cost one
/// pass, not twelve.
///
/// `window_start`, when set, additionally accumulates each pool's
/// `window_flow`: the value contributed to (or withdrawn from) the symbol over
/// `(window_start, as_of]`, which the windowed gain subtracts so a paycheck
/// contribution is not reported as a gain.
fn replay_pools(
    inputs: &HoldingsInputs<'_>,
    base: &Commodity,
    as_ofs: &[&str],
    window_start: Option<&str>,
    in_scope: &dyn Fn(&str) -> bool,
    sole_symbols: &BTreeMap<String, Option<String>>,
) -> Result<Vec<PoolSnapshot>, ReportError> {
    let HoldingsInputs {
        ordered,
        db,
        account_tags,
        commodity_names,
        declared,
        ..
    } = inputs;
    let mut pools: BTreeMap<String, SymbolPool> = BTreeMap::new();
    // symbol -> account -> net shares.
    let mut per_account: BTreeMap<String, BTreeMap<String, Dec>> = BTreeMap::new();
    // Folded into the same pass rather than run as a second one: it is the same
    // date-ordered walk, and it is the ONLY other input that is a running
    // prefix of the journal.
    let mut cost_prices: BTreeMap<String, DatedPrice> = BTreeMap::new();
    let mut snapshots: Vec<PoolSnapshot> = Vec::with_capacity(as_ofs.len());
    let Some(&last) = as_ofs.last() else {
        return Ok(snapshots);
    };

    for txn in ordered {
        if txn.date.as_str() > last {
            break; // `ordered` is date-ascending, so nothing later can qualify
        }
        // Freeze every boundary this transaction has moved past. A `while`, not
        // an `if`: consecutive points can share a date, and empty buckets are
        // common at the start of a long series.
        while snapshots.len() < as_ofs.len() && txn.date.as_str() > as_ofs[snapshots.len()] {
            snapshots.push(freeze(&pools, &per_account, &cost_prices));
        }
        fold_cost_prices(txn, db, base, &mut cost_prices)?;
        // The gain window is half-open: `mv(window_start)` already includes
        // everything dated ≤ `window_start`, so only later flows are additions.
        let in_window = window_start.is_some_and(|start| txn.date.as_str() > start);

        // Gather this txn's in-scope stock legs per symbol (posting order
        // preserved within each symbol's Vec; symbols are independent pools),
        // plus any return-of-capital basis reductions.
        let mut by_symbol: BTreeMap<String, Vec<LotEntry>> = BTreeMap::new();
        let mut basis_returns: BTreeMap<String, Dec> = BTreeMap::new();
        for posting in &txn.postings {
            // Skip out-of-scope accounts and non-holding (equity/income/expense)
            // legs: the latter are a share movement's funding/disposal counter-
            // side, not a place shares are held (see `is_holding_account`).
            if !in_scope(&posting.account.0) || !is_holding_account(&posting.account.0, declared) {
                continue;
            }
            for amount in &posting.amounts {
                if is_currency(&amount.commodity.0) {
                    // Return of capital: cash paid OUT of an account that holds
                    // exactly one security, in a transaction that moves none of
                    // that security and touches no income/expense account, is a
                    // basis reduction — not a dividend (which lands in a cash
                    // account and correctly leaves shares and basis alone).
                    if amount.quantity.mantissa < 0
                        && let Some(Some(symbol)) = sole_symbols.get(&posting.account.0)
                        && !moves_symbol(txn, symbol)
                        && !touches_income_or_expense(txn, declared)
                    {
                        let reduction = if amount.commodity == *base {
                            Some(amount.quantity)
                        } else {
                            db.lookup_in(&amount.commodity, base, &txn.date)
                                .map(|rate| mul_raw(amount.quantity, rate.quantity))
                                .transpose()?
                        };
                        if let Some(reduction) = reduction {
                            let slot = basis_returns
                                .entry(symbol.clone())
                                .or_insert_with(Dec::zero);
                            *slot = slot.add(reduction)?;
                        }
                    }
                    continue;
                }
                let symbol = amount.commodity.0.clone();
                by_symbol.entry(symbol.clone()).or_default().push(LotEntry {
                    qty: amount.quantity,
                    cost: amount.cost.as_deref().cloned(),
                });

                // Ensure the pool exists; update its name and per-account tally.
                let pool = pools
                    .entry(symbol.clone())
                    .or_insert_with(|| SymbolPool::new(&symbol));
                // Precedence: the posting's own `name:` comment tag, then the
                // `commodity`-directive `name:` (keyed by symbol — the canonical
                // place a security is named), then the account's own + ancestors'
                // declared `name:` (most-specific first), then the txn `name:`.
                let name = tag_value(&posting.tags, "name")
                    .map(str::to_string)
                    .or_else(|| commodity_names.get(symbol.as_str()).map(|&n| n.to_string()))
                    .or_else(|| {
                        inherited_account_tags(&posting.account, account_tags)
                            .into_iter()
                            .find(|(key, _)| key == "name")
                            .map(|(_, value)| value)
                    })
                    .or_else(|| tag_value(&txn.tags, "name").map(str::to_string));
                if let Some(name) = name {
                    pool.name = name;
                }
                let accounts = per_account.entry(symbol).or_default();
                let updated = match accounts.get(&posting.account.0) {
                    Some(prev) => prev.add(amount.quantity)?,
                    None => amount.quantity,
                };
                accounts.insert(posting.account.0.clone(), updated);
            }
        }

        for (symbol, entries) in &by_symbol {
            let mut net = Dec::zero();
            for entry in entries {
                net = net.add(entry.qty)?;
            }
            let Some(pool) = pools.get_mut(symbol) else {
                continue; // unreachable: gathered above
            };
            // A zero net is a move between in-scope accounts — but only when the
            // legs are cost-compatible. A sell-and-re-buy of equal size at
            // DIFFERENT prices also nets to zero and must not be swallowed.
            if net.is_zero() && is_pure_transfer(entries, db, base, &txn.date)? {
                continue; // zero shares, zero basis impact, zero flow
            }
            // A split only re-labels the share count: scale `shares` and leave
            // `basis`/`first_basis_date` (and the flow — no value moved) alone.
            if is_redenomination(txn, symbol, entries, net, pool.shares, declared)? {
                pool.shares = pool.shares.add(net)?;
                if pool.shares.mantissa < 0 {
                    // A "split" that removes more than was ever held is not one;
                    // fall back to the same sticky taint a bare oversell gets.
                    pool.went_negative = true;
                    pool.taint_with(TaintReason::WentNegative);
                }
                continue;
            }
            let commodity = Commodity(symbol.clone());
            for entry in entries {
                let leg_before = pool.shares;
                let leg_after = leg_before.add(entry.qty)?;
                if in_window {
                    let flow = leg_flow(entry, &commodity, db, base, &txn.date)?;
                    pool.window_flow = match (pool.window_flow, flow) {
                        (Some(running), Some(flow)) => Some(running.add(flow)?),
                        _ => None, // an unvaluable in-window leg: gain unknowable
                    };
                }
                if entry.qty.mantissa > 0 {
                    if leg_before.mantissa <= 0 {
                        // (re)opening the position resets its basis date
                        pool.first_basis_date = Some(txn.date.clone());
                    }
                    match cost_in_base(entry.qty, entry.cost.as_ref(), db, base, &txn.date)? {
                        None => pool.taint_with(match entry.cost.as_ref() {
                            None => TaintReason::CostlessLot,
                            Some(cost) => {
                                TaintReason::UnconvertibleCost(cost.amount.commodity.0.clone())
                            }
                        }),
                        Some(lot_cost) => pool.basis = pool.basis.add(lot_cost)?,
                    }
                } else if entry.qty.mantissa < 0 && leg_before.mantissa > 0 {
                    pool.basis = if leg_after.mantissa >= 0 {
                        reduce_basis(pool.basis, leg_after, leg_before)?
                    } else {
                        Dec::new(0, pool.basis.places)
                    };
                }
                if leg_after.mantissa < 0 {
                    // Sticky: once more shares have left than ever arrived, the
                    // opening lot was never entered, so the average cost of
                    // anything bought afterwards is unknowable. Re-buying used
                    // to silently pile a full new cost onto a zeroed basis.
                    pool.went_negative = true;
                    pool.taint_with(TaintReason::WentNegative);
                }
                pool.shares = leg_after;
            }
        }

        // Return of capital reduces the basis of the security it was paid on.
        for (symbol, reduction) in &basis_returns {
            let Some(pool) = pools.get_mut(symbol) else {
                continue;
            };
            let reduced = pool.basis.add(*reduction)?;
            // Capital returned beyond the basis is a realised gain, not a
            // negative cost; clamp rather than report a below-zero basis.
            pool.basis = if reduced.mantissa < 0 {
                Dec::new(0, reduced.places)
            } else {
                reduced
            };
            if in_window && let Some(running) = pool.window_flow {
                // Cash left the position: a withdrawal, like a partial sell.
                pool.window_flow = Some(running.add(*reduction)?);
            }
        }
    }

    while snapshots.len() < as_ofs.len() {
        snapshots.push(freeze(&pools, &per_account, &cost_prices));
    }
    Ok(snapshots)
}

/// [`replay_pools`] over dates that may not agree about the sole-symbol map.
///
/// The map is resolved from the whole `≤ as_of` range, so points that disagree
/// about it would have replayed the SAME transaction differently and cannot
/// share a pass. Consecutive points that agree — nearly always all of them, see
/// [`sole_symbol_facts`] — get one replay between them; a disagreement splits
/// the series there and costs one extra pass, never a wrong number.
fn pool_snapshots(
    inputs: &HoldingsInputs<'_>,
    facts: &BTreeMap<String, SoleSymbolFacts>,
    base: &Commodity,
    as_ofs: &[String],
    window_start: Option<&str>,
    in_scope: &dyn Fn(&str) -> bool,
) -> Result<Vec<PoolSnapshot>, ReportError> {
    let mut snapshots: Vec<PoolSnapshot> = Vec::with_capacity(as_ofs.len());
    let mut run: Vec<&str> = Vec::new();
    let mut run_symbols: BTreeMap<String, Option<String>> = BTreeMap::new();
    for as_of in as_ofs {
        let sole = sole_symbols_at(facts, as_of);
        if !run.is_empty() && sole != run_symbols {
            snapshots.extend(replay_pools(
                inputs,
                base,
                &run,
                window_start,
                in_scope,
                &run_symbols,
            )?);
            run.clear();
        }
        run_symbols = sole;
        run.push(as_of.as_str());
    }
    if !run.is_empty() {
        snapshots.extend(replay_pools(
            inputs,
            base,
            &run,
            window_start,
            in_scope,
            &run_symbols,
        )?);
    }
    Ok(snapshots)
}

/// Latest `P` directive ≤ `as_of` pricing `symbol` in `base` (ties: last
/// declared wins), with its date. Port of the TS `latestDirectivePrice` (scans
/// the raw directive list so it can return the date, unlike `PriceDb::lookup_in`).
///
/// A directive priced in `base` always wins. Failing that, the latest directive
/// in ANY commodity that converts to `base` is used, applying exactly the same
/// conversion [`latest_cost_prices`] already applies to cost annotations — so a
/// `P XYZ 100.00 EUR` + `P EUR $1.10` pair prices XYZ instead of reading as
/// unpriced and dropping it out of the portfolio totals. The security's quote
/// keeps its own date (that is what `HoldingPrice.date` reports); the rate that
/// converts it is the one in force at `as_of`, the valuation date.
fn latest_directive_price(
    prices: &[PriceDirective],
    db: &PriceDb,
    symbol: &str,
    base: &Commodity,
    as_of: &str,
) -> Result<Option<DatedPrice>, ReportError> {
    let mut direct: Option<DatedPrice> = None;
    let mut converted: Option<DatedPrice> = None;
    for directive in prices {
        if directive.commodity.0 != symbol || directive.date.as_str() > as_of {
            continue;
        }
        let newer = |current: &Option<DatedPrice>| {
            current
                .as_ref()
                .is_none_or(|best| directive.date.as_str() >= best.date.as_str())
        };
        if directive.price.commodity == *base {
            if newer(&direct) {
                direct = Some(DatedPrice {
                    qty: directive.price.quantity,
                    date: directive.date.clone(),
                });
            }
        } else if newer(&converted)
            && let Some(rate) = db.lookup_in(&directive.price.commodity, base, as_of)
        {
            converted = Some(DatedPrice {
                qty: mul_raw(directive.price.quantity, rate.quantity)?,
                date: directive.date.clone(),
            });
        }
    }
    Ok(direct.or(converted))
}

/// Fold one transaction's cost annotations into the running "latest usable
/// base-commodity price per symbol" map — the WHOLE journal is scanned (not just
/// in-scope), buys and sells alike. Later dates overwrite earlier ones, so
/// walking the journal in date order leaves the latest annotation ≤ the current
/// date standing: the map is a running prefix, which is what lets a series read
/// it off at each boundary instead of rescanning.
fn fold_cost_prices(
    txn: &Transaction,
    db: &PriceDb,
    base: &Commodity,
    latest: &mut BTreeMap<String, DatedPrice>,
) -> Result<(), ReportError> {
    for posting in &txn.postings {
        for amount in &posting.amounts {
            let Some(cost) = amount.cost.as_deref() else {
                continue;
            };
            if is_currency(&amount.commodity.0) || amount.quantity.is_zero() {
                continue;
            }
            let per_unit = if cost.kind == CostKind::Unit {
                cost.amount.quantity
            } else {
                per_unit_from_total(cost.amount.quantity, amount.quantity)?
            };
            let in_base = if cost.amount.commodity == *base {
                per_unit
            } else {
                match db.lookup_in(&cost.amount.commodity, base, &txn.date) {
                    Some(rate) => mul_raw(per_unit, rate.quantity)?,
                    None => continue,
                }
            };
            latest.insert(
                amount.commodity.0.clone(),
                DatedPrice {
                    qty: in_base,
                    date: txn.date.clone(),
                },
            );
        }
    }
    Ok(())
}

/// Per symbol, the latest cost annotation ≤ `as_of` usable as a base-commodity
/// price. Port of the TS `latestCostPrices`.
///
/// Only [`coverage`] still calls this: it measures a CANDIDATE base, so it runs
/// before the replay has a base to fold prices in. The report's own copy comes
/// out of the replay.
fn latest_cost_prices(
    ordered: &[&Transaction],
    db: &PriceDb,
    base: &Commodity,
    as_of: &str,
) -> Result<BTreeMap<String, DatedPrice>, ReportError> {
    let mut latest: BTreeMap<String, DatedPrice> = BTreeMap::new();
    for txn in ordered {
        if txn.date.as_str() > as_of {
            break; // `ordered` is date-ascending
        }
        fold_cost_prices(txn, db, base, &mut latest)?;
    }
    Ok(latest)
}

/// The valuation commodity used when the journal declares no prices at all and
/// the caller names none either.
const FALLBACK_BASE: &str = "$";

/// Every non-currency commodity with a POSITIVE net quantity in scope at
/// `as_of` — exactly the symbols that become rows in the report, and so exactly
/// the ones whose pricing decides whether the portfolio reads as a number or as
/// zero.
///
/// A deliberately cheap pre-pass: it nets share quantities and nothing else,
/// with none of [`build_pools`]'s basis/taint/split/flow machinery, because it
/// has to run BEFORE a base commodity exists — choosing that base is what it is
/// for.
fn held_symbols(
    txns: &[Transaction],
    as_of: &str,
    in_scope: &dyn Fn(&str) -> bool,
    declared: &BTreeMap<String, AccountType>,
) -> Result<Vec<Commodity>, ReportError> {
    let mut net: BTreeMap<&str, Dec> = BTreeMap::new();
    for txn in txns {
        if txn.date.as_str() > as_of {
            continue;
        }
        for posting in &txn.postings {
            if !in_scope(&posting.account.0) || !is_holding_account(&posting.account.0, declared) {
                continue;
            }
            for amount in &posting.amounts {
                if is_currency(&amount.commodity.0) {
                    continue;
                }
                let slot = net
                    .entry(amount.commodity.0.as_str())
                    .or_insert_with(Dec::zero);
                *slot = slot.add(amount.quantity)?;
            }
        }
    }
    Ok(net
        .into_iter()
        .filter(|(_, shares)| shares.mantissa > 0)
        .map(|(symbol, _)| Commodity(symbol.to_string()))
        .collect())
}

/// How many of `held` the report could actually put a per-unit price on if it
/// valued everything in `target`.
///
/// It runs the SAME two lookups [`compute_holdings`] runs — a `P` directive
/// (direct, or converted through a rate to `target`) and then a cost annotation
/// — so a candidate's measured coverage can never disagree with the report the
/// candidate is being chosen for.
fn coverage(
    held: &[Commodity],
    target: &Commodity,
    ordered: &[&Transaction],
    prices: &[PriceDirective],
    db: &PriceDb,
    as_of: &str,
) -> Result<usize, ReportError> {
    let cost_prices = latest_cost_prices(ordered, db, target, as_of)?;
    let mut covered = 0;
    for symbol in held {
        let priced = symbol == target
            || latest_directive_price(prices, db, &symbol.0, target, as_of)?.is_some()
            || cost_prices.contains_key(&symbol.0);
        if priced {
            covered += 1;
        }
    }
    Ok(covered)
}

/// The commodity a holdings report over `scope` values everything in.
///
/// `scope.value_in` wins outright when set — that is the caller (a `valueIn`
/// query param, or the journal's own `D` directive) saying what the answer
/// should be denominated in, and second-guessing it would just move the
/// surprise somewhere else.
///
/// Otherwise the candidates from [`PriceDb::base_candidates`] are walked in rank
/// order and the one that prices the MOST in-scope holdings wins, ties going to
/// the higher-ranked (more frequent, then lexically smaller) candidate. Rank
/// alone — the old rule — has no idea what it is valuing: three jotted-down
/// `P … 1.15 EUR` travel cross-rates outvote the single `P VTI $120.00` pricing
/// an entire portfolio, EUR becomes the base, nothing connects VTI to EUR, and a
/// $1,200 portfolio reports $0 (HOLD-3). Coverage is what makes the DEFAULT
/// safe, which matters far more than the override: almost nobody will set one.
///
/// This is NOT what hledger does, because hledger never faces the question: its
/// `-V` values each commodity in ITS OWN latest price target, so the same
/// journal yields `$1,200.00` for VTI and leaves the untouched travel currencies
/// in EUR — a multi-commodity result this report's single `base` field cannot
/// represent. Picking the base that prices the most of the portfolio is the
/// closest single-commodity approximation of that.
fn choose_base(
    inputs: &HoldingsInputs<'_>,
    prices: &[PriceDirective],
    as_of: &str,
    value_in: Option<&Commodity>,
    in_scope: &dyn Fn(&str) -> bool,
) -> Result<Commodity, ReportError> {
    if let Some(target) = value_in {
        return Ok(target.clone());
    }
    let db = &inputs.db;
    // One candidate (the overwhelmingly common single-currency journal) or none:
    // there is nothing to choose between, so skip the scan entirely and leave
    // that journal exactly as fast as it was.
    let candidates = db.base_candidates();
    if candidates.len() < 2 {
        return Ok(candidates
            .first()
            .cloned()
            .unwrap_or_else(|| Commodity(FALLBACK_BASE.to_string())));
    }
    let held = held_symbols(inputs.txns, as_of, in_scope, &inputs.declared)?;
    if held.is_empty() {
        return Ok(candidates[0].clone());
    }
    let mut best = &candidates[0];
    let mut best_covered = 0;
    for candidate in candidates {
        let covered = coverage(&held, candidate, &inputs.ordered, prices, db, as_of)?;
        if covered > best_covered {
            best = candidate;
            best_covered = covered;
        }
        if best_covered == held.len() {
            break; // nothing can beat pricing everything
        }
    }
    Ok(best.clone())
}

/// The commodity a holdings report over `scope` will value everything in —
/// [`choose_base`] exposed so callers can pin it.
///
/// [`holdings_series`](crate::holdings::holdings_series) uses it to value every
/// point of a trend in ONE commodity (the scope's holdings, and so the safest
/// base, differ from bucket to bucket), and the HTTP layer uses it to answer
/// "what would this request be denominated in?" without computing the report.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn valuation_base(
    txns: &[Transaction],
    prices: &[PriceDirective],
    accounts: &[AccountDeclaration],
    scope: &HoldingsScope,
) -> Result<Commodity, ReportError> {
    let predicate = scope_predicate(scope);
    // No `commodity` directives: choosing a base never reads a security's name.
    let inputs = HoldingsInputs::build(txns, prices, accounts, &[]);
    choose_base(
        &inputs,
        prices,
        &scope.as_of,
        scope.value_in.as_ref(),
        &predicate,
    )
}

/// True when valuing this scope in `target` prices at least one of the holdings
/// it contains (vacuously true for a scope that holds nothing).
///
/// The HTTP layer's admission test for an explicit `valueIn`: a commodity that
/// prices NOTHING — a typo, or a real commodity with no route to the portfolio —
/// yields a report of all-zero totals and one `unpriced` warning per row, which
/// is precisely the "plausible number instead of an error" failure this review is
/// against. Answering `400` instead costs one cheap netting pass.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn prices_any_held(
    txns: &[Transaction],
    prices: &[PriceDirective],
    accounts: &[AccountDeclaration],
    scope: &HoldingsScope,
    target: &Commodity,
) -> Result<bool, ReportError> {
    let predicate = scope_predicate(scope);
    // No `commodity` directives: measuring coverage never reads a security's name.
    let inputs = HoldingsInputs::build(txns, prices, accounts, &[]);
    let held = held_symbols(txns, &scope.as_of, &predicate, &inputs.declared)?;
    if held.is_empty() {
        return Ok(true);
    }
    Ok(coverage(
        &held,
        target,
        &inputs.ordered,
        prices,
        &inputs.db,
        &scope.as_of,
    )? > 0)
}

/// Account predicate for a scope: `Include` + empty set = everything;
/// `account_matches` subtree semantics. Port of the TS `scopePredicate`.
fn scope_predicate(scope: &HoldingsScope) -> impl Fn(&str) -> bool + '_ {
    let selected: Vec<&str> = scope.accounts.iter().map(String::as_str).collect();
    move |account: &str| {
        let matches = selected.iter().any(|&sel| account_matches(sel, account));
        match scope.mode {
            ScopeMode::Include => selected.is_empty() || matches,
            ScopeMode::Exclude => !matches,
        }
    }
}

/// `gain / reference × 100` as a display-boundary `f64`, or `None` when there is
/// no capital to measure against. A zero reference has always been undefined; a
/// NEGATIVE one (possible once windowed withdrawals exceed the starting value)
/// would silently flip the sign of the percentage, so it is refused too.
fn gain_pct(gain: Dec, reference: Dec) -> Option<f64> {
    if reference.mantissa <= 0 {
        None
    } else {
        Some((gain.floating_point() / reference.floating_point()) * 100.0)
    }
}

/// Stock holdings, average-cost basis, prices, and gains for the scoped journal
/// as of `scope.as_of`. Port of the TS `computeHoldings`.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow (unreachable for realistic
/// journals, but never unwrapped).
pub fn compute_holdings(
    txns: &[Transaction],
    prices: &[PriceDirective],
    accounts: &[AccountDeclaration],
    commodity_tags: &[(Commodity, Vec<(String, String)>)],
    scope: &HoldingsScope,
) -> Result<HoldingsReport, ReportError> {
    let predicate = scope_predicate(scope);
    let inputs = HoldingsInputs::build(txns, prices, accounts, commodity_tags);
    let base_commodity = choose_base(
        &inputs,
        prices,
        &scope.as_of,
        scope.value_in.as_ref(),
        &predicate,
    )?;
    // `gain_since` re-runs the report at the window start, so the facts have to
    // cover the LATER of the two dates; `as_of` is it.
    let facts = sole_symbol_facts(&inputs.ordered, &scope.as_of, &predicate, &inputs.declared);
    report_at(
        &inputs,
        &facts,
        prices,
        &base_commodity,
        &scope.as_of,
        scope.gain_since.as_deref(),
        &predicate,
    )
}

/// [`compute_holdings`] at each of `as_ofs` (ASCENDING), sharing one replay.
///
/// Every report is exactly what [`compute_holdings`] returns for a scope with
/// that `as_of`, `value_in: Some(base)` and `gain_since: None` — the per-point
/// scope [`holdings_series`](super::series::holdings_series) builds — and the
/// pinned `base` is returned alongside, resolved ONCE from `scope.as_of` (see
/// [`valuation_base`], which this replaces for the series so the journal is
/// sorted and the `PriceDb` built once rather than twice).
pub(super) fn holdings_at_each(
    txns: &[Transaction],
    prices: &[PriceDirective],
    accounts: &[AccountDeclaration],
    commodity_tags: &[(Commodity, Vec<(String, String)>)],
    scope: &HoldingsScope,
    as_ofs: &[String],
) -> Result<(Commodity, Vec<HoldingsReport>), ReportError> {
    let predicate = scope_predicate(scope);
    let inputs = HoldingsInputs::build(txns, prices, accounts, commodity_tags);
    let base_commodity = choose_base(
        &inputs,
        prices,
        &scope.as_of,
        scope.value_in.as_ref(),
        &predicate,
    )?;
    let Some(last) = as_ofs.last() else {
        return Ok((base_commodity, Vec::new()));
    };
    let facts = sole_symbol_facts(&inputs.ordered, last, &predicate, &inputs.declared);
    // The trend tracks market value/basis only; gain windowing is a per-snapshot
    // concern and never applies to a series point, so there is no window flow to
    // accumulate and no start snapshot to recurse into.
    let snapshots = pool_snapshots(&inputs, &facts, &base_commodity, as_ofs, None, &predicate)?;
    let reports = as_ofs
        .iter()
        .zip(snapshots)
        .map(|(as_of, snapshot)| {
            assemble_report(
                &inputs,
                prices,
                &base_commodity,
                as_of,
                None,
                &snapshot,
                &BTreeMap::new(),
            )
        })
        .collect::<Result<Vec<_>, ReportError>>()?;
    Ok((base_commodity, reports))
}

/// One holdings snapshot at `as_of` from prebuilt inputs — [`compute_holdings`]
/// minus the per-call setup, so the `gain_since` start snapshot can reuse it
/// instead of rebuilding the whole world one level down.
fn report_at(
    inputs: &HoldingsInputs<'_>,
    facts: &BTreeMap<String, SoleSymbolFacts>,
    prices: &[PriceDirective],
    base_commodity: &Commodity,
    as_of: &str,
    gain_since: Option<&str>,
    in_scope: &dyn Fn(&str) -> bool,
) -> Result<HoldingsReport, ReportError> {
    let sole = sole_symbols_at(facts, as_of);
    let snapshot = replay_pools(
        inputs,
        base_commodity,
        &[as_of],
        gain_since,
        in_scope,
        &sole,
    )?
    .pop()
    .unwrap_or_default(); // one date in, one snapshot out

    // Gain window: when `gain_since` is set, the gain is measured against
    // each position's market value at that start date (a plain all-time snapshot
    // re-run at `start`), not against its all-time cost basis. `symbol → value at
    // start` (`Some(0)` = not held at `start`; `None` = held-but-unpriced there).
    //
    // The start snapshot is pinned to the SAME base rather than choosing its
    // own: the scope holds different symbols at `start` than at `as_of`, so
    // coverage could legitimately favour a different commodity there — and
    // subtracting a value in one commodity from a value in another is not a gain.
    let start_values: BTreeMap<String, Option<Dec>> = match gain_since {
        None => BTreeMap::new(),
        Some(start) => report_at(inputs, facts, prices, base_commodity, start, None, in_scope)?
            .holdings
            .into_iter()
            .map(|holding| (holding.symbol, holding.market_value))
            .collect(),
    };
    assemble_report(
        inputs,
        prices,
        base_commodity,
        as_of,
        gain_since,
        &snapshot,
        &start_values,
    )
}

/// Price one replayed snapshot and turn it into the report for its `as_of`: the
/// per-symbol pricing, warnings, sorting and PARTIAL totals.
///
/// Split out of [`compute_holdings`] because it is the only per-`as_of` work
/// left once the pools are replayed once — and it costs O(symbols), not O(txns),
/// which is why a 60-point series now costs about what a 12-point one does.
fn assemble_report(
    inputs: &HoldingsInputs<'_>,
    prices: &[PriceDirective],
    base_commodity: &Commodity,
    as_of: &str,
    gain_since: Option<&str>,
    snapshot: &PoolSnapshot,
    start_values: &BTreeMap<String, Option<Dec>>,
) -> Result<HoldingsReport, ReportError> {
    let db = &inputs.db;
    let base = base_commodity.0.clone();
    let PoolSnapshot { pools, cost_prices } = snapshot;
    // The capital a holding's gain is measured against: its all-time `basis`
    // (default), or — under `gain_since` — the capital actually at work over the
    // window, `value_at_start + net_contributions`.
    //
    // Subtracting the contributions is what makes the windowed number a GAIN
    // rather than a change in value. Without it, buying $1000 more of a
    // flat-priced fund inside the window reads as +$1000 of gain and selling
    // reads as a loss of the whole proceeds — every account taking a paycheck
    // contribution showed a fabricated YTD figure. Using the same sum as the
    // percentage's denominator makes it a simple-Dietz return: money in and out
    // moves the baseline, not the gain.
    //
    // A position that is NET SHORT at `as_of` has no reference in either mode.
    // Its opening lot was never entered, so neither its all-time cost nor its
    // value at a window start is a real number, and a fabricated one would
    // distort the whole portfolio's return through the denominator. Refusing it
    // here keeps `gain`/`gain_pct` null on the row (so it never reaches
    // `top_gainers`/`top_losers`) and keeps it out of the gain totals — exactly
    // the treatment a tainted row already gets, while its market value still
    // counts.
    let reference_of =
        |symbol: &str, shares: Dec, basis: Option<Dec>| -> Result<Option<Dec>, ReportError> {
            if shares.mantissa <= 0 {
                return Ok(None);
            }
            if gain_since.is_none() {
                return Ok(basis);
            }
            let start = start_values
                .get(symbol)
                .copied()
                .unwrap_or_else(|| Some(Dec::zero()));
            let flow = pools.get(symbol).and_then(|pool| pool.window_flow);
            match (start, flow) {
                (Some(start), Some(flow)) => Ok(Some(start.add(flow)?)),
                // Held-but-unpriced at the start, or an in-window leg that could
                // not be valued: refuse the windowed gain rather than invent one.
                _ => Ok(None),
            }
        };

    let mut holdings: Vec<Holding> = Vec::new();
    let mut warnings: Vec<HoldingsWarning> = Vec::new();
    // A BTreeMap iterates in symbol order — matches the TS explicit symbol sort.
    for (symbol, pool) in pools {
        if pool.shares.is_zero() {
            continue; // fully sold: dropped silently
        }
        // A position that is still NET SHORT at `as_of` is reported like any
        // other, not dropped. The balance sheet carries those shares and values
        // them, so omitting the row made the portfolio total and net worth
        // disagree by exactly the short's market value with nothing on the wire
        // tying them together. Emitting it keeps `Σ market_value` equal to
        // `totals.market_value` by construction and hands the SPA the symbol and
        // the amount it needs to hide the row behind an accurate note.
        //
        // `basis` and `gain` stay `None` — the opening lot was never entered, so
        // there is no cost to report (`taint` is already `WentNegative` here, by
        // the same rule that taints a pool which merely dipped below zero).
        let short = pool.shares.mantissa < 0;
        if short {
            warnings.push(HoldingsWarning {
                symbol: symbol.clone(),
                kind: WarningKind::NegativeShares,
                message: format!(
                    "{symbol}: net shares are negative (-{deficit} shares) — the opening position was likely never entered, so its basis and gain are unknown; its market value is still counted in the totals",
                    deficit = abs_shares_2dp(pool.shares)
                ),
            });
        }

        let price = match latest_directive_price(prices, db, symbol, base_commodity, as_of)? {
            Some(directive) => Some(HoldingPrice {
                qty: directive.qty,
                date: directive.date,
                source: PriceSource::Directive,
            }),
            None => cost_prices.get(symbol).map(|cost| HoldingPrice {
                qty: cost.qty,
                date: cost.date.clone(),
                source: PriceSource::Cost,
            }),
        };
        if price.is_none() {
            warnings.push(HoldingsWarning {
                symbol: symbol.clone(),
                kind: WarningKind::Unpriced,
                message: format!(
                    "{symbol}: no market price or usable cost annotation — excluded from totals"
                ),
            });
        }
        // A pool that dipped below zero but recovered is shown with a positive
        // share count, and its basis is not knowable, so the warning stays
        // attached to the row. A pool that is STILL short got the sharper
        // "net shares are negative" message above instead — one warning, not two.
        if pool.went_negative && !short {
            warnings.push(HoldingsWarning {
                symbol: symbol.clone(),
                kind: WarningKind::NegativeShares,
                message: format!(
                    "{symbol}: net shares dipped below zero before this date — the opening position was likely never entered, so the average cost of the shares still held is unknown"
                ),
            });
        }
        // `WentNegative` is already reported above; the other two reasons each
        // get their own text (they used to share the cost-less lot's message).
        match &pool.taint {
            None | Some(TaintReason::WentNegative) => {}
            Some(TaintReason::CostlessLot) => warnings.push(HoldingsWarning {
                symbol: symbol.clone(),
                kind: WarningKind::MissingBasis,
                message: format!("{symbol}: acquired without a cost annotation — basis unknown"),
            }),
            Some(TaintReason::UnconvertibleCost(commodity)) => warnings.push(HoldingsWarning {
                symbol: symbol.clone(),
                kind: WarningKind::MissingBasis,
                message: format!(
                    "{symbol}: cost annotated in {commodity}, which has no price in {base} on that date — basis unknown"
                ),
            }),
        }

        let basis = if pool.taint.is_none() {
            Some(pool.basis)
        } else {
            None
        };
        let market_value = match &price {
            Some(p) => Some(mul_raw(pool.shares, p.qty)?),
            None => None,
        };
        // `gain = market_value − reference`, where `reference` is the all-time
        // basis (default) or the window's invested capital (`gain_since`);
        // `basis` itself stays all-time on the row regardless.
        let reference = reference_of(symbol, pool.shares, basis)?;
        let gain = match (market_value, reference) {
            (Some(mv), Some(r)) => Some(mv.sub(r)?),
            _ => None,
        };
        let pct = match (gain, reference) {
            (Some(g), Some(r)) => gain_pct(g, r),
            _ => None,
        };
        holdings.push(Holding {
            symbol: symbol.clone(),
            name: pool.name.clone(),
            accounts: pool.accounts.clone(),
            shares: pool.shares,
            basis,
            first_basis_date: pool.first_basis_date.clone(),
            price,
            market_value,
            gain,
            gain_pct: pct,
        });
    }

    // Market value desc; unpriced last; ties (and unpriced) by symbol asc.
    holdings.sort_by(|a, b| match (a.market_value, b.market_value) {
        (None, None) => a.symbol.cmp(&b.symbol),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.symbol.cmp(&b.symbol)),
    });

    // PARTIAL totals: rather than refusing the whole portfolio when a single row
    // is tainted or unpriced, `basis`/`gain`/`gainPct` sum over only the rows that
    // carry the needed inputs. `basis` sums the "fully-known" rows — a known
    // `basis` AND a market value; `gain` sums `market_value − reference` over the
    // rows that have a `reference` (all-time `basis` by default, or the window's
    // invested capital under `gain_since`) AND a market value; `gainPct` divides
    // that gain by the reference sum over the same rows. Each is `None` only when
    // its set is empty — i.e. every shown holding is excluded (all tainted/unpriced
    // → an honest dash). An EMPTY portfolio keeps a real zero (you own nothing, so
    // your basis is $0, not "unknown"), matching the series' empty-scope behavior.
    // `market_value` is unrestricted: the whole priced portfolio, INCLUDING any
    // net-short row (whose value is negative). That is what makes
    // `totals.market_value + cash` equal the valued balance sheet.
    let empty = holdings.is_empty();
    let mut market_value = Dec::zero();
    let mut basis_total: Option<Dec> = empty.then_some(Dec::zero());
    let mut gain_total: Option<Dec> = empty.then_some(Dec::zero());
    let mut reference_sum: Option<Dec> = empty.then_some(Dec::zero());
    for holding in &holdings {
        let Some(mv) = holding.market_value else {
            continue; // unpriced rows contribute to no total
        };
        market_value = market_value.add(mv)?;
        if let Some(basis) = holding.basis {
            basis_total = Some(match basis_total {
                Some(bt) => bt.add(basis)?,
                None => basis,
            });
        }
        if let Some(reference) = reference_of(&holding.symbol, holding.shares, holding.basis)? {
            gain_total = Some(match gain_total {
                Some(gt) => gt.add(mv.sub(reference)?)?,
                None => mv.sub(reference)?,
            });
            reference_sum = Some(match reference_sum {
                Some(rs) => rs.add(reference)?,
                None => reference,
            });
        }
    }
    let gain_pct_total = match (gain_total, reference_sum) {
        (Some(g), Some(rs)) => gain_pct(g, rs),
        _ => None,
    };

    // Only real signs: gainers gain_pct > 0 (desc), losers gain_pct < 0 (asc).
    // Filtering the already-MV-sorted list + a stable sort matches the TS
    // tie-ordering.
    let mut top_gainers: Vec<Holding> = holdings
        .iter()
        .filter(|h| h.gain_pct.is_some_and(|p| p > 0.0))
        .cloned()
        .collect();
    top_gainers.sort_by(|a, b| {
        b.gain_pct
            .partial_cmp(&a.gain_pct)
            .unwrap_or(Ordering::Equal)
    });
    top_gainers.truncate(5);

    let mut top_losers: Vec<Holding> = holdings
        .iter()
        .filter(|h| h.gain_pct.is_some_and(|p| p < 0.0))
        .cloned()
        .collect();
    top_losers.sort_by(|a, b| {
        a.gain_pct
            .partial_cmp(&b.gain_pct)
            .unwrap_or(Ordering::Equal)
    });
    top_losers.truncate(5);

    Ok(HoldingsReport {
        as_of: as_of.to_string(),
        base,
        holdings,
        totals: HoldingsTotals {
            market_value,
            basis: basis_total,
            gain: gain_total,
            gain_pct: gain_pct_total,
        },
        top_gainers,
        top_losers,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holdings::test_helpers::{
        account_decl, amt, buy, buy_no_cost, commodity_tags, pd, posting, scope, scope_in,
        scope_since, sell, txn, usd, with_cost,
    };
    use crate::holdings::types::HoldingsReport;

    fn only<'a>(report: &'a HoldingsReport, symbol: &str) -> &'a Holding {
        report
            .holdings
            .iter()
            .find(|h| h.symbol == symbol)
            .unwrap_or_else(|| panic!("holding {symbol} should exist"))
    }

    fn run(txns: &[Transaction], prices: &[PriceDirective], sc: &HoldingsScope) -> HoldingsReport {
        compute_holdings(txns, prices, &[], &[], sc).expect("compute_holdings succeeds")
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // ---- average-cost basis ----

    #[test]
    fn accumulates_per_unit_buys_and_reduces_partial_sell() {
        // Deliberately out of journal order: the engine sorts by date, then index.
        let txns = [
            txn(
                3,
                "2025-03-10",
                vec![
                    sell("assets:broker:vti", "VTI", 5),
                    posting("assets:broker:cash", vec![usd(115_000)], &[]),
                ],
                &[],
            ),
            txn(
                1,
                "2025-01-10",
                vec![
                    buy("assets:broker:vti", "VTI", 10, 20000, true),
                    posting("assets:broker:cash", vec![usd(-200_000)], &[]),
                ],
                &[],
            ),
            txn(
                2,
                "2025-02-10",
                vec![
                    buy("assets:broker:vti", "VTI", 10, 22000, true),
                    posting("assets:broker:cash", vec![usd(-220_000)], &[]),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-04-01", "VTI", 25000, "$")],
            &scope("2025-04-30", ScopeMode::Include, &[]),
        );

        assert_eq!(report.base, "$");
        let vti = only(&report, "VTI");
        assert_eq!(vti.shares, Dec::new(15, 0));
        assert_eq!(vti.basis, Some(Dec::new(315_000, 2))); // (2000 + 2200) × 15/20, exact
        let price = vti.price.as_ref().expect("VTI priced");
        assert_eq!(price.date, "2025-04-01");
        assert_eq!(price.source, PriceSource::Directive);
        assert_eq!(price.qty, Dec::new(25000, 2));
        assert_eq!(vti.market_value, Some(Dec::new(3750, 0)));
        assert_eq!(vti.gain, Some(Dec::new(600, 0)));
        assert!(close(vti.gain_pct.unwrap(), (600.0 / 3150.0) * 100.0));
        assert_eq!(vti.accounts, vec!["assets:broker:vti".to_string()]);
        assert_eq!(report.totals.market_value, Dec::new(3750, 0));
        assert_eq!(report.totals.basis, Some(Dec::new(3150, 0)));
        assert_eq!(report.totals.gain, Some(Dec::new(600, 0)));
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn handles_total_cost_buys() {
        let txns = [txn(
            1,
            "2025-01-10",
            vec![buy("assets:broker", "VTI", 4, 85000, false)], // 4 VTI @@ $850.00
            &[],
        )];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-03-01", ScopeMode::Include, &[]),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.basis, Some(Dec::new(85000, 2)));
        assert_eq!(vti.market_value, Some(Dec::new(1000, 0)));
        assert_eq!(vti.gain, Some(Dec::new(150, 0)));
    }

    #[test]
    fn rounds_sell_reductions_half_even() {
        // 2 @@ $1.01 → sell 1 → 0.505 rounds to 0.50 (even); @@ $1.03 → 0.515 → 0.52.
        let txns = [
            txn(1, "2025-01-10", vec![buy("a", "EEE", 2, 101, false)], &[]),
            txn(2, "2025-01-10", vec![buy("a", "OOO", 2, 103, false)], &[]),
            txn(
                3,
                "2025-02-10",
                vec![sell("a", "EEE", 1), sell("a", "OOO", 1)],
                &[],
            ),
        ];
        let report = run(&txns, &[], &scope("2025-03-01", ScopeMode::Include, &[]));
        assert_eq!(only(&report, "EEE").basis, Some(Dec::new(50, 2)));
        assert_eq!(only(&report, "OOO").basis, Some(Dec::new(52, 2)));
    }

    // ---- scoping ----

    fn two_accounts() -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2025-01-10",
                vec![buy("assets:broker:a", "VTI", 10, 20000, true)],
                &[],
            ),
            txn(
                2,
                "2025-01-20",
                vec![buy("assets:broker:b", "VTI", 5, 21000, true)],
                &[],
            ),
            txn(
                3,
                "2025-01-25",
                vec![buy("assets:other:c", "VTI", 2, 22000, true)],
                &[],
            ),
        ]
    }

    #[test]
    fn include_empty_set_means_all_accounts() {
        let txns = two_accounts();
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.shares, Dec::new(17, 0));
        assert_eq!(
            vti.accounts,
            vec![
                "assets:broker:a".to_string(),
                "assets:broker:b".to_string(),
                "assets:other:c".to_string()
            ]
        );
    }

    #[test]
    fn include_matches_whole_subtrees() {
        let txns = two_accounts();
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &["assets:broker"]),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.shares, Dec::new(15, 0));
        assert_eq!(vti.basis, Some(Dec::new(3050, 0)));
        assert_eq!(
            vti.accounts,
            vec!["assets:broker:a".to_string(), "assets:broker:b".to_string()]
        );
    }

    #[test]
    fn exclude_removes_selected_subtrees_only() {
        let txns = two_accounts();
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Exclude, &["assets:broker:b"]),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.shares, Dec::new(12, 0));
        assert_eq!(vti.basis, Some(Dec::new(2440, 0)));
        assert_eq!(
            vti.accounts,
            vec!["assets:broker:a".to_string(), "assets:other:c".to_string()]
        );
    }

    #[test]
    fn in_scope_transfer_nets_to_zero_and_leaves_basis_untouched() {
        let txns = [
            txn(
                1,
                "2025-01-10",
                vec![buy("assets:broker:a", "VTI", 10, 20000, true)],
                &[],
            ),
            txn(
                2,
                "2025-02-10",
                vec![
                    sell("assets:broker:a", "VTI", 4),
                    buy_no_cost("assets:broker:b", "VTI", 4),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.shares, Dec::new(10, 0));
        // the cost-less incoming leg must NOT taint the pool
        assert_eq!(vti.basis, Some(Dec::new(200_000, 2)));
        assert_eq!(
            vti.accounts,
            vec!["assets:broker:a".to_string(), "assets:broker:b".to_string()]
        );
    }

    // ---- taint and pricing ----

    #[test]
    fn costless_buy_taints_the_pool() {
        let txns = [
            txn(
                1,
                "2025-01-10",
                vec![buy_no_cost("assets:broker", "GLD", 10)],
                &[],
            ),
            txn(
                2,
                "2025-01-20",
                vec![buy("assets:broker", "VTI", 10, 20000, true)],
                &[],
            ),
        ];
        let prices = [
            pd("2025-02-01", "GLD", 18000, "$"),
            pd("2025-02-01", "VTI", 22000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );

        let gld = only(&report, "GLD");
        assert_eq!(gld.basis, None);
        assert_eq!(gld.gain, None);
        assert_eq!(gld.gain_pct, None);
        assert_eq!(gld.market_value, Some(Dec::new(1800, 0))); // priced via directive despite taint
        assert_eq!(
            report.warnings,
            vec![HoldingsWarning {
                symbol: "GLD".to_string(),
                kind: WarningKind::MissingBasis,
                message: report.warnings[0].message.clone(),
            }]
        );
        assert!(report.warnings[0].message.contains("GLD"));
        assert_eq!(report.totals.market_value, Dec::new(4000, 0));
        // Partial totals: GLD (tainted) is excluded, but VTI's basis/gain count.
        assert_eq!(report.totals.basis, Some(Dec::new(2000, 0))); // VTI $2000 only
        assert_eq!(report.totals.gain, Some(Dec::new(200, 0))); // VTI mv $2200 − $2000
        assert!(close(report.totals.gain_pct.unwrap(), 10.0)); // 200 / 2000
    }

    #[test]
    fn non_base_cost_converts_via_directive_else_taints() {
        let txns = [
            txn(
                1,
                "2025-01-10",
                vec![posting(
                    "a",
                    vec![with_cost(amt("VTI", 10, 0), 10000, true, "EUR")],
                    &[],
                )],
                &[],
            ), // 10 VTI @ €100
            txn(
                2,
                "2025-01-10",
                vec![posting(
                    "a",
                    vec![with_cost(amt("XYZ", 10, 0), 10000, true, "GBP")],
                    &[],
                )],
                &[],
            ), // no GBP→$ price: taint
        ];
        let prices = [
            pd("2025-01-01", "EUR", 110, "$"),
            pd("2025-02-01", "VTI", 15000, "$"),
            pd("2025-02-01", "XYZ", 15000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert_eq!(only(&report, "VTI").basis, Some(Dec::new(11_000_000, 4))); // €1000 × 1.10
        assert_eq!(only(&report, "XYZ").basis, None);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].symbol, "XYZ");
        assert_eq!(report.warnings[0].kind, WarningKind::MissingBasis);
    }

    #[test]
    fn falls_back_to_latest_cost_annotation_as_price() {
        let txns = [
            txn(
                1,
                "2025-01-10",
                vec![buy("assets:broker", "XXX", 10, 5000, true)],
                &[],
            ), // @ $50
            txn(
                2,
                "2025-03-01",
                vec![buy("assets:broker", "XXX", 4, 26000, false)],
                &[],
            ), // @@ $260 → $65/share
        ];
        let report = run(&txns, &[], &scope("2025-06-30", ScopeMode::Include, &[]));
        let xxx = only(&report, "XXX");
        let price = xxx.price.as_ref().expect("XXX priced from cost");
        assert_eq!(price.date, "2025-03-01");
        assert_eq!(price.source, PriceSource::Cost);
        assert_eq!(price.qty, Dec::new(65, 0));
        assert_eq!(xxx.shares, Dec::new(14, 0));
        assert_eq!(xxx.basis, Some(Dec::new(760, 0)));
        assert_eq!(xxx.market_value, Some(Dec::new(910, 0)));
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn excludes_unpriced_from_totals_and_sorts_them_last() {
        let txns = [
            txn(
                1,
                "2025-01-10",
                vec![buy("assets:broker", "VTI", 10, 20000, true)],
                &[],
            ),
            txn(
                2,
                "2025-01-20",
                vec![buy_no_cost("assets:broker", "NOP", 3)],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 22000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );

        let symbols: Vec<&str> = report.holdings.iter().map(|h| h.symbol.as_str()).collect();
        assert_eq!(symbols, ["VTI", "NOP"]);
        let nop = only(&report, "NOP");
        assert_eq!(nop.price, None);
        assert_eq!(nop.market_value, None);
        assert_eq!(report.totals.market_value, Dec::new(2200, 0));
        // NOP is unpriced (excluded from every total); the basis/gain are VTI's.
        assert_eq!(report.totals.basis, Some(Dec::new(2000, 0)));
        assert_eq!(report.totals.gain, Some(Dec::new(200, 0)));
        let kinds: Vec<(&str, WarningKind)> = report
            .warnings
            .iter()
            .map(|w| (w.symbol.as_str(), w.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("NOP", WarningKind::Unpriced),
                ("NOP", WarningKind::MissingBasis)
            ]
        );
    }

    #[test]
    fn totals_are_partial_and_none_only_when_every_row_is_excluded() {
        // One tainted (cost-less) holding + one fully-known holding: the basis
        // total is the KNOWN holding's basis alone (partial), NOT refused.
        let mixed = [
            txn(
                1,
                "2025-01-10",
                vec![buy("assets:broker", "VTI", 10, 20000, true)],
                &[],
            ),
            txn(
                2,
                "2025-01-20",
                vec![buy_no_cost("assets:broker", "GLD", 5)],
                &[],
            ),
        ];
        let prices = [
            pd("2025-02-01", "VTI", 22000, "$"),
            pd("2025-02-01", "GLD", 18000, "$"),
        ];
        let report = run(
            &mixed,
            &prices,
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert_eq!(
            report.totals.basis,
            Some(Dec::new(2000, 0)),
            "partial = VTI's basis only"
        );
        assert_eq!(report.totals.gain, Some(Dec::new(200, 0)));

        // An ALL-tainted portfolio (both priced, both cost-less) still yields a
        // null basis/gain total — the fully-known set is empty (honest dash).
        let all_tainted = [
            txn(
                1,
                "2025-01-10",
                vec![buy_no_cost("assets:broker", "GLD", 5)],
                &[],
            ),
            txn(
                2,
                "2025-01-20",
                vec![buy_no_cost("assets:broker", "SLV", 5)],
                &[],
            ),
        ];
        let report = run(
            &all_tainted,
            &[
                pd("2025-02-01", "GLD", 18000, "$"),
                pd("2025-02-01", "SLV", 2000, "$"),
            ],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert!(!report.totals.market_value.is_zero(), "both are priced");
        assert_eq!(report.totals.basis, None, "no fully-known row → dash");
        assert_eq!(report.totals.gain, None);
        assert_eq!(report.totals.gain_pct, None);
    }

    // ---- firstBasisDate ----

    #[test]
    fn first_basis_date_simple_buy() {
        let txns = [txn(
            1,
            "2025-01-10",
            vec![buy("a", "VTI", 10, 20000, true)],
            &[],
        )];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert_eq!(
            only(&report, "VTI").first_basis_date.as_deref(),
            Some("2025-01-10")
        );
    }

    #[test]
    fn first_basis_date_resets_on_rebuy() {
        let txns = [
            txn(1, "2025-01-10", vec![buy("a", "VTI", 10, 20000, true)], &[]),
            txn(2, "2025-02-10", vec![sell("a", "VTI", 10)], &[]),
            txn(3, "2025-03-10", vec![buy("a", "VTI", 4, 21000, true)], &[]),
        ];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert_eq!(
            only(&report, "VTI").first_basis_date.as_deref(),
            Some("2025-03-10")
        );
    }

    #[test]
    fn first_basis_date_partial_sell_keeps_original() {
        let txns = [
            txn(1, "2025-01-10", vec![buy("a", "VTI", 10, 20000, true)], &[]),
            txn(2, "2025-02-10", vec![sell("a", "VTI", 4)], &[]),
        ];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert_eq!(
            only(&report, "VTI").first_basis_date.as_deref(),
            Some("2025-01-10")
        );
    }

    #[test]
    fn first_basis_date_buy_more_keeps_earliest() {
        let txns = [
            txn(1, "2025-01-10", vec![buy("a", "VTI", 10, 20000, true)], &[]),
            txn(2, "2025-02-10", vec![buy("a", "VTI", 5, 22000, true)], &[]),
        ];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert_eq!(
            only(&report, "VTI").first_basis_date.as_deref(),
            Some("2025-01-10")
        );
    }

    // ---- row filtering ----

    #[test]
    fn drops_fully_sold_symbol_silently() {
        let txns = [
            txn(1, "2025-01-10", vec![buy("a", "VTI", 10, 20000, true)], &[]),
            txn(2, "2025-02-10", vec![sell("a", "VTI", 10)], &[]),
        ];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 22000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert!(report.holdings.is_empty());
        assert!(report.warnings.is_empty());
    }

    /// REVISED: the engine no longer drops a net-short pool, it reports it (the
    /// balance sheet values those shares, so hiding them broke reconciliation —
    /// see `report_invariants::holdings_reconcile_even_when_a_short_position_is_open`).
    /// The row carries the short share count and no basis/gain; hiding it is the
    /// SPA's job.
    #[test]
    fn reports_negative_pool_with_warning_but_no_basis() {
        let txns = [txn(1, "2025-01-10", vec![sell("a", "SHT", 5)], &[])];
        let report = run(
            &txns,
            &[pd("2025-02-01", "SHT", 1000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        let sht = only(&report, "SHT");
        assert_eq!(sht.shares, Dec::new(-5, 0));
        assert_eq!(sht.market_value, Some(Dec::new(-50, 0)), "-5 × $10.00");
        assert_eq!(sht.basis, None, "the opening lot was never entered");
        assert_eq!(sht.gain, None);
        assert_eq!(sht.gain_pct, None);
        // No account nets positive, so no account "holds" it.
        assert!(sht.accounts.is_empty());
        // The negative value is the whole portfolio total, and is NOT smuggled
        // into the basis/gain totals.
        assert_eq!(report.totals.market_value, Dec::new(-50, 0));
        assert_eq!(report.totals.basis, None);
        assert_eq!(report.totals.gain, None);
        assert!(report.top_gainers.is_empty() && report.top_losers.is_empty());

        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert_eq!(report.warnings[0].symbol, "SHT");
        assert_eq!(report.warnings[0].kind, WarningKind::NegativeShares);
        assert!(report.warnings[0].message.contains("never entered"));
    }

    // ---- asOf time travel ----

    fn aapl_txns() -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2025-01-05",
                vec![posting(
                    "assets:broker",
                    vec![with_cost(amt("AAPL", 10, 0), 10000, true, "$")],
                    &[("name", "Apple Inc.")],
                )],
                &[],
            ),
            txn(
                2,
                "2025-06-05",
                vec![posting(
                    "assets:broker",
                    vec![with_cost(amt("AAPL", 10, 0), 12000, true, "$")],
                    &[],
                )],
                &[("name", "Apple Computer")],
            ),
        ]
    }

    #[test]
    fn early_asof_sees_first_lot_price_and_name() {
        let txns = aapl_txns();
        let prices = [
            pd("2025-01-15", "AAPL", 11000, "$"),
            pd("2025-07-01", "AAPL", 15000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope("2025-03-01", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        assert_eq!(aapl.shares, Dec::new(10, 0));
        assert_eq!(aapl.basis, Some(Dec::new(1000, 0)));
        assert_eq!(aapl.price.as_ref().unwrap().date, "2025-01-15");
        assert_eq!(aapl.price.as_ref().unwrap().qty, Dec::new(11000, 2));
        assert_eq!(aapl.name, "Apple Inc.");
    }

    #[test]
    fn late_asof_sees_both_lots_newer_price_and_txn_name() {
        let txns = aapl_txns();
        let prices = [
            pd("2025-01-15", "AAPL", 11000, "$"),
            pd("2025-07-01", "AAPL", 15000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        assert_eq!(aapl.shares, Dec::new(20, 0));
        assert_eq!(aapl.basis, Some(Dec::new(2200, 0)));
        assert_eq!(aapl.price.as_ref().unwrap().date, "2025-07-01");
        assert_eq!(aapl.price.as_ref().unwrap().qty, Dec::new(15000, 2));
        assert_eq!(aapl.name, "Apple Computer");
    }

    // ---- name resolution: inherited account-directive tags ----

    fn aapl_buy() -> Vec<Transaction> {
        vec![txn(
            1,
            "2024-01-01",
            vec![buy("assets:broker:aapl", "AAPL", 10, 22000, true)],
            &[],
        )]
    }

    fn aapl_prices() -> Vec<PriceDirective> {
        vec![pd("2024-02-01", "AAPL", 22500, "$")]
    }

    #[test]
    fn account_directive_name_used_when_no_posting_or_txn_name() {
        // The repro: the leaf account declares the name; nothing else does.
        let decls = [account_decl(
            "assets:broker:aapl",
            &[("name", "Apple Inc.")],
        )];
        let report = compute_holdings(
            &aapl_buy(),
            &aapl_prices(),
            &decls,
            &[],
            &scope("2024-12-31", ScopeMode::Include, &[]),
        )
        .expect("compute_holdings succeeds");
        assert_eq!(only(&report, "AAPL").name, "Apple Inc.");
    }

    #[test]
    fn posting_comment_name_wins_over_account_directive_name() {
        let txns = [txn(
            1,
            "2024-01-01",
            vec![posting(
                "assets:broker:aapl",
                vec![with_cost(amt("AAPL", 10, 0), 22000, true, "$")],
                &[("name", "Posting Wins")],
            )],
            &[],
        )];
        let decls = [account_decl(
            "assets:broker:aapl",
            &[("name", "Apple Inc.")],
        )];
        let report = compute_holdings(
            &txns,
            &aapl_prices(),
            &decls,
            &[],
            &scope("2024-12-31", ScopeMode::Include, &[]),
        )
        .expect("compute_holdings succeeds");
        assert_eq!(only(&report, "AAPL").name, "Posting Wins");
    }

    #[test]
    fn account_directive_name_wins_over_txn_name() {
        // Precedence check for the middle rung: account beats a txn-level name.
        let txns = [txn(
            1,
            "2024-01-01",
            vec![buy("assets:broker:aapl", "AAPL", 10, 22000, true)],
            &[("name", "Txn Name")],
        )];
        let decls = [account_decl(
            "assets:broker:aapl",
            &[("name", "Apple Inc.")],
        )];
        let report = compute_holdings(
            &txns,
            &aapl_prices(),
            &decls,
            &[],
            &scope("2024-12-31", ScopeMode::Include, &[]),
        )
        .expect("compute_holdings succeeds");
        assert_eq!(only(&report, "AAPL").name, "Apple Inc.");
    }

    #[test]
    fn ancestor_account_name_is_inherited_by_child_with_none() {
        // Only the ANCESTOR `assets:broker` declares a name; the posted leaf
        // `assets:broker:aapl` has no declaration of its own.
        let decls = [account_decl(
            "assets:broker",
            &[("name", "Broker Holdings")],
        )];
        let report = compute_holdings(
            &aapl_buy(),
            &aapl_prices(),
            &decls,
            &[],
            &scope("2024-12-31", ScopeMode::Include, &[]),
        )
        .expect("compute_holdings succeeds");
        assert_eq!(only(&report, "AAPL").name, "Broker Holdings");
    }

    // ---- name resolution: commodity-directive tags ----

    #[test]
    fn commodity_directive_name_used_for_symbol() {
        // The user's exact multi-tag `commodity` directives: the display name
        // lives on the directive; nothing else names the security.
        let commodities = [
            commodity_tags(
                "NAWGX",
                &[
                    ("CUSIP", "92913X811"),
                    ("basis", "64045.66"),
                    ("name", "VOYA GLOBAL HI DIV LOW VOL A"),
                    ("type", "mutualfund"),
                ],
            ),
            commodity_tags(
                "WMT",
                &[
                    ("CUSIP", "931142103"),
                    ("basis", "15358.22"),
                    ("name", "WALMART INC"),
                ],
            ),
            commodity_tags(
                "TEMFX",
                &[("name", "Templeton Foreign"), ("type", "mutualfund")],
            ),
        ];
        let txns = [txn(
            1,
            "2024-01-01",
            vec![
                buy("assets:broker:nawgx", "NAWGX", 10, 1000, true),
                buy("assets:broker:wmt", "WMT", 10, 1000, true),
                buy("assets:broker:temfx", "TEMFX", 10, 1000, true),
            ],
            &[],
        )];
        let report = compute_holdings(
            &txns,
            &[],
            &[],
            &commodities,
            &scope("2024-12-31", ScopeMode::Include, &[]),
        )
        .expect("compute_holdings succeeds");
        assert_eq!(only(&report, "NAWGX").name, "VOYA GLOBAL HI DIV LOW VOL A");
        assert_eq!(only(&report, "WMT").name, "WALMART INC");
        assert_eq!(only(&report, "TEMFX").name, "Templeton Foreign");
    }

    #[test]
    fn posting_name_overrides_commodity_directive_name() {
        // A per-posting `name:` still wins over the commodity directive.
        let commodities = [commodity_tags("WMT", &[("name", "WALMART INC")])];
        let txns = [txn(
            1,
            "2024-01-01",
            vec![posting(
                "assets:broker:wmt",
                vec![with_cost(amt("WMT", 10, 0), 1000, true, "$")],
                &[("name", "Posting Wins")],
            )],
            &[],
        )];
        let report = compute_holdings(
            &txns,
            &[],
            &[],
            &commodities,
            &scope("2024-12-31", ScopeMode::Include, &[]),
        )
        .expect("compute_holdings succeeds");
        assert_eq!(only(&report, "WMT").name, "Posting Wins");
    }

    #[test]
    fn commodity_directive_name_beats_account_directive_name() {
        // The commodity directive is the canonical security name, so it beats an
        // incidental account-directive `name:`.
        let commodities = [commodity_tags("WMT", &[("name", "WALMART INC")])];
        let decls = [account_decl("assets:broker:wmt", &[("name", "Brokerage")])];
        let txns = [txn(
            1,
            "2024-01-01",
            vec![buy("assets:broker:wmt", "WMT", 10, 1000, true)],
            &[],
        )];
        let report = compute_holdings(
            &txns,
            &[],
            &decls,
            &commodities,
            &scope("2024-12-31", ScopeMode::Include, &[]),
        )
        .expect("compute_holdings succeeds");
        assert_eq!(only(&report, "WMT").name, "WALMART INC");
    }

    #[test]
    fn commodity_directive_without_name_falls_through_to_symbol() {
        // Other tags (CUSIP/type) but NO `name` must NOT be mistaken for the
        // display name — with nothing else naming it, the row shows the symbol.
        let commodities = [commodity_tags(
            "WMT",
            &[("CUSIP", "931142103"), ("type", "stock")],
        )];
        let txns = [txn(
            1,
            "2024-01-01",
            vec![buy("assets:broker:wmt", "WMT", 10, 1000, true)],
            &[],
        )];
        let report = compute_holdings(
            &txns,
            &[],
            &[],
            &commodities,
            &scope("2024-12-31", ScopeMode::Include, &[]),
        )
        .expect("compute_holdings succeeds");
        assert_eq!(only(&report, "WMT").name, "WMT");
    }

    // ---- gainers and losers ----

    #[test]
    fn splits_gainers_and_losers_and_caps_at_five() {
        // All bought at $100/share: G1 +60% … G6 +10%, L1 -30% L2 -20% L3 -10%,
        // Z0 flat, T0 tainted (gain_pct None).
        let priced: [(&str, i128); 10] = [
            ("G1", 16000),
            ("G2", 15000),
            ("G3", 14000),
            ("G4", 13000),
            ("G5", 12000),
            ("G6", 11000),
            ("L1", 7000),
            ("L2", 8000),
            ("L3", 9000),
            ("Z0", 10000),
        ];
        let mut txns: Vec<Transaction> = priced
            .iter()
            .enumerate()
            .map(|(i, (symbol, _))| {
                #[allow(clippy::cast_possible_truncation)]
                let index = (i + 1) as u32;
                txn(
                    index,
                    "2025-01-10",
                    vec![buy("a", symbol, 1, 10000, true)],
                    &[],
                )
            })
            .collect();
        #[allow(clippy::cast_possible_truncation)]
        let last = (priced.len() + 1) as u32;
        txns.push(txn(
            last,
            "2025-01-10",
            vec![buy_no_cost("a", "T0", 1)],
            &[],
        ));
        let mut prices: Vec<PriceDirective> = priced
            .iter()
            .map(|(symbol, cents)| pd("2025-02-01", symbol, *cents, "$"))
            .collect();
        prices.push(pd("2025-02-01", "T0", 99900, "$"));

        let report = run(
            &txns,
            &prices,
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        let gainers: Vec<&str> = report
            .top_gainers
            .iter()
            .map(|h| h.symbol.as_str())
            .collect();
        let losers: Vec<&str> = report
            .top_losers
            .iter()
            .map(|h| h.symbol.as_str())
            .collect();
        assert_eq!(gainers, ["G1", "G2", "G3", "G4", "G5"]); // > 0 only, desc, G6 capped off
        assert_eq!(losers, ["L1", "L2", "L3"]); // < 0 only, asc — Z0 and T0 in neither
    }

    #[test]
    fn empty_losers_when_everything_gained() {
        let txns = [
            txn(1, "2025-01-10", vec![buy("a", "AAA", 1, 10000, true)], &[]),
            txn(2, "2025-01-10", vec![buy("a", "BBB", 1, 10000, true)], &[]),
        ];
        let prices = [
            pd("2025-02-01", "AAA", 12000, "$"),
            pd("2025-02-01", "BBB", 11000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        let gainers: Vec<&str> = report
            .top_gainers
            .iter()
            .map(|h| h.symbol.as_str())
            .collect();
        assert_eq!(gainers, ["AAA", "BBB"]);
        assert!(report.top_losers.is_empty());
    }

    // ---- holdings net == balance-sheet net (equity/income share legs) ----

    #[test]
    fn share_transfer_in_via_equity_is_not_read_as_negative() {
        // Shares transferred in from another institution with the source booked
        // in SHARES to equity (ACATS-style): `assets:brokerA +5 TSLA` /
        // `equity:transfers -5 TSLA`. The balance sheet sums only asset+liability
        // accounts, so its TSLA net is +5. Counting the equity leg would net the
        // acquiring txn to zero (shares never pooled); a later sale would then
        // read −5 — a spurious "negative shares". The pool must track the asset
        // leg alone and stay equal to the balance-sheet net.
        let txns = [txn(
            1,
            "2024-06-01",
            vec![
                buy_no_cost("assets:brokerA", "TSLA", 5),
                sell("equity:transfers", "TSLA", 5),
            ],
            &[],
        )];
        let report = run(
            &txns,
            &[pd("2024-07-01", "TSLA", 30000, "$")],
            &scope("2026-06-30", ScopeMode::Include, &[]),
        );
        let tsla = only(&report, "TSLA");
        assert_eq!(tsla.shares, Dec::new(5, 0)); // matches the assets-only net
        assert_eq!(tsla.accounts, vec!["assets:brokerA".to_string()]);
        // A cost-less transfer-in has an unknown basis (tainted) — NOT a bogus
        // negative-shares warning.
        assert_eq!(tsla.basis, None);
        assert!(
            report
                .warnings
                .iter()
                .all(|w| w.kind != WarningKind::NegativeShares),
            "an equity-sourced transfer-in must not read as negative shares"
        );
    }

    #[test]
    fn share_transfer_in_via_equity_then_full_sell_nets_flat() {
        // Same equity-sourced transfer-in, then the whole position is sold. The
        // asset TSLA nets to 0 — dropped silently, exactly like the balance sheet
        // (which never sees the equity leg), with no negative-shares warning.
        let txns = [
            txn(
                1,
                "2024-06-01",
                vec![
                    buy_no_cost("assets:brokerA", "TSLA", 5),
                    sell("equity:transfers", "TSLA", 5),
                ],
                &[],
            ),
            txn(
                2,
                "2025-06-01",
                vec![sell("assets:brokerA", "TSLA", 5)],
                &[],
            ),
        ];
        let report = run(&txns, &[], &scope("2026-06-30", ScopeMode::Include, &[]));
        assert!(report.holdings.iter().all(|h| h.symbol != "TSLA"));
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn income_denominated_share_leg_does_not_count_toward_net() {
        // An RSU vest booked with the income offset in SHARES: the +10 lands in
        // assets, the −10 in income. Only the asset leg is a holding.
        let txns = [txn(
            1,
            "2025-01-10",
            vec![
                buy_no_cost("assets:broker:tsla", "TSLA", 10),
                sell("income:rsu", "TSLA", 10),
            ],
            &[],
        )];
        let report = run(
            &txns,
            &[pd("2025-02-01", "TSLA", 30000, "$")],
            &scope("2026-06-30", ScopeMode::Include, &[]),
        );
        assert_eq!(only(&report, "TSLA").shares, Dec::new(10, 0));
        assert!(
            report
                .warnings
                .iter()
                .all(|w| w.kind != WarningKind::NegativeShares)
        );
    }

    #[test]
    fn negative_shares_warning_states_the_deficit() {
        // A genuine short (sold, never bought) still warns — now spelling out how
        // far below zero the position is.
        let txns = [txn(
            1,
            "2025-01-10",
            vec![sell("assets:brokerA", "SHT", 5)],
            &[],
        )];
        let report = run(&txns, &[], &scope("2025-06-30", ScopeMode::Include, &[]));
        // Two warnings now: the short itself, then `unpriced` — the row is
        // reported, so (like any other reported row) it is checked for a price.
        // The negative-shares warning is pushed first.
        assert_eq!(report.warnings.len(), 2, "{:?}", report.warnings);
        assert_eq!(report.warnings[1].kind, WarningKind::Unpriced);
        let message = &report.warnings[0].message;
        assert_eq!(report.warnings[0].kind, WarningKind::NegativeShares);
        assert!(message.contains("-5.00 shares"), "message was: {message}");
        assert!(message.contains("never entered"));

        // A fractional deficit renders to two decimals too.
        let frac = [txn(
            1,
            "2025-01-10",
            vec![posting("assets:brokerA", vec![amt("FRC", -45, 1)], &[])],
            &[],
        )];
        let report = run(&frac, &[], &scope("2025-06-30", ScopeMode::Include, &[]));
        assert!(
            report.warnings[0].message.contains("-4.50 shares"),
            "message was: {}",
            report.warnings[0].message
        );
    }

    // ---- gain window (`gain_since`) ----

    #[test]
    fn gain_since_windows_gain_against_value_at_start() {
        // 10 VTI @ $200 in Jan; priced $250 from Jun 2025, $300 from Jan 2026.
        let txns = [txn(
            1,
            "2025-01-10",
            vec![buy("assets:broker:vti", "VTI", 10, 20000, true)],
            &[],
        )];
        let prices = [
            pd("2025-06-01", "VTI", 25000, "$"),
            pd("2026-01-01", "VTI", 30000, "$"),
        ];

        // All-time (no window): gain = mv($3000) − basis($2000) = $1000.
        let all_time = run(
            &txns,
            &prices,
            &scope("2026-06-30", ScopeMode::Include, &[]),
        );
        let vti = only(&all_time, "VTI");
        assert_eq!(vti.basis, Some(Dec::new(2000, 0)));
        assert_eq!(vti.market_value, Some(Dec::new(3000, 0)));
        assert_eq!(vti.gain, Some(Dec::new(1000, 0)));

        // Windowed since 2025-07-01: value_at_start = 10 × $250 (latest ≤ start) =
        // $2500 → gain = $3000 − $2500 = $500; basis is unchanged (all-time).
        let windowed = run(
            &txns,
            &prices,
            &scope_since("2026-06-30", ScopeMode::Include, &[], "2025-07-01"),
        );
        let vti = only(&windowed, "VTI");
        assert_eq!(vti.basis, Some(Dec::new(2000, 0)), "basis stays all-time");
        assert_eq!(vti.market_value, Some(Dec::new(3000, 0)));
        assert_eq!(vti.gain, Some(Dec::new(500, 0)), "windowed gain");
        assert!(close(vti.gain_pct.unwrap(), (500.0 / 2500.0) * 100.0));
        // Totals mirror the window (basis stays all-time).
        assert_eq!(windowed.totals.market_value, Dec::new(3000, 0));
        assert_eq!(windowed.totals.basis, Some(Dec::new(2000, 0)));
        assert_eq!(windowed.totals.gain, Some(Dec::new(500, 0)));
    }

    #[test]
    fn gain_since_before_position_opened_measures_against_the_contribution() {
        // REVISED (was `..._counts_full_value_pct_undefined`). The window starts
        // BEFORE the buy, so `value_at_start` is 0 — but the $2000 spent inside
        // the window is a CONTRIBUTION, not a gain. The old expectation
        // (`gain = $2500`, the entire market value, `gain_pct = None`) is exactly
        // the HOLD-2 defect: it reported the money you put in as money you made.
        //
        // Now the reference is `value_at_start + contributions` = $0 + $2000, so
        // the gain is the $500 the position actually appreciated, and the
        // percentage is defined (25%) because there IS capital to measure
        // against — it just arrived mid-window rather than before it.
        let txns = [txn(
            1,
            "2025-03-10",
            vec![buy("assets:broker:vti", "VTI", 10, 20000, true)],
            &[],
        )];
        let report = run(
            &txns,
            &[pd("2025-01-01", "VTI", 25000, "$")],
            &scope_since("2026-06-30", ScopeMode::Include, &[], "2025-01-15"),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.market_value, Some(Dec::new(2500, 0)));
        assert_eq!(
            vti.gain,
            Some(Dec::new(500, 0)),
            "only the appreciation over the $2000 contributed inside the window"
        );
        assert!(close(vti.gain_pct.unwrap(), 25.0));
        assert_eq!(vti.basis, Some(Dec::new(2000, 0)));
        // Totals follow the same reference.
        assert_eq!(report.totals.gain, Some(Dec::new(500, 0)));
    }

    #[test]
    fn windowed_gain_excludes_a_mid_window_contribution() {
        // HOLD-2 case 1: price FLAT at $100 throughout. Hold 10 VTI, buy 10 more
        // inside the window. The old engine reported `mv(asOf) − mv(start)` =
        // $2000 − $1000 = +$1000 (+100%) — the contribution itself.
        let txns = [
            txn(
                1,
                "2025-06-01",
                vec![
                    buy("assets:broker:vti", "VTI", 10, 10000, true),
                    posting("assets:broker:cash", vec![usd(-100_000)], &[]),
                ],
                &[],
            ),
            txn(
                2,
                "2026-03-01",
                vec![
                    buy("assets:broker:vti", "VTI", 10, 10000, true),
                    posting("assets:broker:cash", vec![usd(-100_000)], &[]),
                ],
                &[],
            ),
        ];
        let prices = [
            pd("2025-01-01", "VTI", 10000, "$"),
            pd("2026-01-01", "VTI", 10000, "$"),
            pd("2026-06-30", "VTI", 10000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope_since("2026-06-30", ScopeMode::Include, &[], "2026-01-01"),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.market_value, Some(Dec::new(2000, 0)));
        assert_eq!(vti.gain, Some(Dec::zero()), "a flat price cannot gain");
        assert!(close(vti.gain_pct.unwrap(), 0.0));
        assert_eq!(report.totals.gain, Some(Dec::zero()));
        assert!(report.top_gainers.is_empty(), "nothing actually gained");
    }

    #[test]
    fn windowed_gain_excludes_a_mid_window_withdrawal() {
        // HOLD-2 case 2: price FLAT at $150 throughout. Hold 20 VTI, sell 10
        // inside the window. The old engine reported $1500 − $3000 = −$1500
        // (−50%) — the proceeds, booked as a loss.
        let txns = [
            txn(
                1,
                "2025-06-01",
                vec![
                    buy("assets:broker:vti", "VTI", 20, 10000, true),
                    posting("assets:broker:cash", vec![usd(-200_000)], &[]),
                ],
                &[],
            ),
            txn(
                2,
                "2026-03-01",
                vec![
                    posting(
                        "assets:broker:vti",
                        vec![with_cost(amt("VTI", -10, 0), 15000, true, "$")],
                        &[],
                    ),
                    posting("assets:broker:cash", vec![usd(150_000)], &[]),
                ],
                &[],
            ),
        ];
        let prices = [
            pd("2025-01-01", "VTI", 15000, "$"),
            pd("2026-01-01", "VTI", 15000, "$"),
            pd("2026-06-30", "VTI", 15000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope_since("2026-06-30", ScopeMode::Include, &[], "2026-01-01"),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.market_value, Some(Dec::new(1500, 0)));
        assert_eq!(
            vti.gain,
            Some(Dec::zero()),
            "the $1500 taken out is a withdrawal, not a loss"
        );
        assert!(close(vti.gain_pct.unwrap(), 0.0));
        assert!(report.top_losers.is_empty(), "nothing actually lost");
    }

    #[test]
    fn windowed_gain_refuses_when_an_in_window_leg_cannot_be_valued() {
        // A mid-window lot is bought with a cost annotated in a currency that has
        // no rate to the base, so the contribution's size is unknowable — the
        // windowed gain is refused rather than silently counting the arrival as a
        // gain.
        let txns = [
            txn(
                1,
                "2025-06-01",
                vec![buy("assets:broker:zzz", "ZZZ", 10, 10000, true)],
                &[],
            ),
            txn(
                2,
                "2026-03-01",
                vec![posting(
                    "assets:broker:zzz",
                    vec![with_cost(amt("ZZZ", 10, 0), 10000, true, "GBP")],
                    &[],
                )],
                &[],
            ),
        ];
        // No GBP→$ rate anywhere, so the £1000 cost cannot be sized in dollars.
        let prices = [
            pd("2026-01-01", "ZZZ", 10000, "$"),
            pd("2026-06-30", "ZZZ", 10000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope_since("2026-06-30", ScopeMode::Include, &[], "2026-01-01"),
        );
        let zzz = only(&report, "ZZZ");
        assert_eq!(zzz.market_value, Some(Dec::new(2000, 0)));
        assert_eq!(zzz.gain, None, "an unvaluable in-window flow refuses");
        assert_eq!(zzz.gain_pct, None);
        assert_eq!(report.totals.gain, None);
    }

    // ---- stock splits (HOLD-1) ----

    /// 10 AAPL bought cleanly at $100 — basis $1000 — for the split cases.
    fn opened_aapl() -> Transaction {
        txn(
            1,
            "2025-01-05",
            vec![
                buy("assets:broker:aapl", "AAPL", 10, 10000, true),
                posting("assets:broker:cash", vec![usd(-100_000)], &[]),
            ],
            &[],
        )
    }

    #[test]
    fn split_booked_against_equity_scales_shares_and_keeps_basis() {
        // Spelling A: `assets +10 AAPL` / `equity:splits -10 AAPL`. The share-only
        // leg has no cost, which used to taint the pool: basis and gain both
        // collapsed to null, and took the portfolio totals with them.
        let txns = [
            opened_aapl(),
            txn(
                2,
                "2025-06-01",
                vec![
                    buy_no_cost("assets:broker:aapl", "AAPL", 10),
                    sell("equity:splits", "AAPL", 10),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-07-01", "AAPL", 6000, "$")],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        assert_eq!(aapl.shares, Dec::new(20, 0));
        assert_eq!(aapl.basis, Some(Dec::new(100_000, 2)), "basis survives");
        assert_eq!(aapl.market_value, Some(Dec::new(1200, 0)));
        assert_eq!(aapl.gain, Some(Dec::new(200, 0)));
        assert_eq!(
            aapl.first_basis_date.as_deref(),
            Some("2025-01-05"),
            "a split does not restart the holding period"
        );
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(report.totals.basis, Some(Dec::new(1000, 0)));
        assert_eq!(report.totals.gain, Some(Dec::new(200, 0)));
    }

    #[test]
    fn split_as_a_same_account_pair_scales_shares_and_keeps_basis() {
        // Spelling B: `assets -10 AAPL` / `assets +20 AAPL`. This one used to
        // wipe the basis TWICE — `reduce_basis(basis, 0, 10)` zeroed it and the
        // cost-less `+20` then tainted it and reset `first_basis_date`.
        let txns = [
            opened_aapl(),
            txn(
                2,
                "2025-06-01",
                vec![
                    sell("assets:broker:aapl", "AAPL", 10),
                    buy_no_cost("assets:broker:aapl", "AAPL", 20),
                    sell("equity:splits", "AAPL", 10),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-07-01", "AAPL", 6000, "$")],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        assert_eq!(aapl.shares, Dec::new(20, 0));
        assert_eq!(aapl.basis, Some(Dec::new(100_000, 2)));
        assert_eq!(aapl.first_basis_date.as_deref(), Some("2025-01-05"));
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn reverse_split_scales_down_and_keeps_basis() {
        // 1-for-2: 10 AAPL become 5. The share count halves, the basis does not.
        let txns = [
            opened_aapl(),
            txn(
                2,
                "2025-06-01",
                vec![
                    sell("assets:broker:aapl", "AAPL", 10),
                    buy_no_cost("assets:broker:aapl", "AAPL", 5),
                    buy_no_cost("equity:splits", "AAPL", 5),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-07-01", "AAPL", 24000, "$")],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        assert_eq!(aapl.shares, Dec::new(5, 0));
        assert_eq!(aapl.basis, Some(Dec::new(100_000, 2)));
        assert_eq!(aapl.market_value, Some(Dec::new(1200, 0)));
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn a_bare_sell_is_never_read_as_a_reverse_split() {
        // Guard: a one-sided cost-less DISPOSAL (no cash, no equity counter-leg)
        // is a sale, not a re-denomination — it must still reduce the basis
        // proportionally. Reading it as a split would leave the basis of a
        // half-sold position at its full original value.
        let txns = [
            opened_aapl(),
            txn(
                2,
                "2025-06-01",
                vec![sell("assets:broker:aapl", "AAPL", 5)],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-07-01", "AAPL", 12000, "$")],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        assert_eq!(aapl.shares, Dec::new(5, 0));
        assert_eq!(
            aapl.basis,
            Some(Dec::new(50000, 2)),
            "half the basis leaves"
        );
    }

    #[test]
    fn a_share_leg_paired_with_cash_is_never_read_as_a_split() {
        // Guard: any cash consideration in the transaction means value moved, so
        // the cost-less leg is an acquisition with an unknown basis, not a split.
        let txns = [
            opened_aapl(),
            txn(
                2,
                "2025-06-01",
                vec![
                    buy_no_cost("assets:broker:aapl", "AAPL", 10),
                    posting("assets:broker:cash", vec![usd(-120_000)], &[]),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-07-01", "AAPL", 12000, "$")],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        assert_eq!(aapl.shares, Dec::new(20, 0));
        assert_eq!(aapl.basis, None, "a cost-less purchase still taints");
        assert_eq!(report.warnings[0].kind, WarningKind::MissingBasis);
    }

    #[test]
    fn an_rsu_vest_into_an_open_position_is_never_read_as_a_split() {
        // Guard: the income leg proves shares arrived from outside as
        // compensation, so their basis is genuinely unknown.
        let txns = [
            opened_aapl(),
            txn(
                2,
                "2025-06-01",
                vec![
                    buy_no_cost("assets:broker:aapl", "AAPL", 10),
                    sell("income:rsu", "AAPL", 10),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-07-01", "AAPL", 12000, "$")],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        assert_eq!(aapl.shares, Dec::new(20, 0));
        assert_eq!(aapl.basis, None);
    }

    #[test]
    fn split_detection_ignores_a_second_security_in_the_same_transaction() {
        // Guard: an exchange (`-10 AAPL` / `+5 XYZ`) moves value between two
        // securities and is not a re-denomination of either.
        let txns = [
            opened_aapl(),
            txn(
                2,
                "2025-06-01",
                vec![
                    sell("assets:broker:aapl", "AAPL", 5),
                    buy_no_cost("assets:broker:xyz", "XYZ", 5),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[
                pd("2025-07-01", "AAPL", 12000, "$"),
                pd("2025-07-01", "XYZ", 12000, "$"),
            ],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        assert_eq!(only(&report, "AAPL").shares, Dec::new(5, 0));
        assert_eq!(
            only(&report, "AAPL").basis,
            Some(Dec::new(50000, 2)),
            "the AAPL sale still reduces AAPL's basis"
        );
        assert_eq!(only(&report, "XYZ").basis, None);
    }

    #[test]
    fn equity_transfer_into_an_already_held_symbol_reads_as_a_split() {
        // PINS THE ACCEPTED AMBIGUITY. An ACATS-style transfer-in booked against
        // equity is spelled identically to a split; when the pool is EMPTY the
        // two are distinguishable (see
        // `share_transfer_in_via_equity_is_not_read_as_negative`, which still
        // taints), but when the same symbol is already held they are not, and the
        // engine chooses the split reading. The consequence — basis unchanged for
        // a larger share count — is deliberate and documented on
        // `is_redenomination`; this test exists so the choice cannot drift
        // silently.
        let txns = [
            opened_aapl(),
            txn(
                2,
                "2025-06-01",
                vec![
                    buy_no_cost("assets:broker:aapl", "AAPL", 10),
                    sell("equity:transfers", "AAPL", 10),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-07-01", "AAPL", 12000, "$")],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        assert_eq!(aapl.shares, Dec::new(20, 0));
        assert_eq!(aapl.basis, Some(Dec::new(100_000, 2)));
    }

    // ---- oversold-then-reopened pools (HOLD-4) ----

    #[test]
    fn oversold_then_reopened_pool_reports_an_unknown_basis_and_warns() {
        // Buy 10 @ $100, oversell 15 (the opening lot was never entered), re-buy
        // 10 @ $200 → 5 shares held. The old engine zeroed the basis at the
        // crossing and then piled the full $2000 re-buy on top, reporting
        // `basis $2000` ($400/share) for shares whose true average cost is $200 —
        // and, because the pool is positive again at `as_of`, it showed no
        // warning at all.
        let txns = [
            txn(
                1,
                "2025-01-05",
                vec![
                    buy("assets:broker:vti", "VTI", 10, 10000, true),
                    posting("assets:broker:cash", vec![usd(-100_000)], &[]),
                ],
                &[],
            ),
            txn(
                2,
                "2025-03-01",
                vec![
                    posting(
                        "assets:broker:vti",
                        vec![with_cost(amt("VTI", -15, 0), 12000, true, "$")],
                        &[],
                    ),
                    posting("assets:broker:cash", vec![usd(180_000)], &[]),
                ],
                &[],
            ),
            txn(
                3,
                "2025-05-01",
                vec![
                    buy("assets:broker:vti", "VTI", 10, 20000, true),
                    posting("assets:broker:cash", vec![usd(-200_000)], &[]),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-07-01", "VTI", 30000, "$")],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.shares, Dec::new(5, 0));
        assert_eq!(vti.basis, None, "the average cost is genuinely unknowable");
        assert_eq!(vti.gain, None, "no basis, no gain — not a confident −$500");
        assert_eq!(vti.market_value, Some(Dec::new(1500, 0)));
        let negatives: Vec<&HoldingsWarning> = report
            .warnings
            .iter()
            .filter(|w| w.kind == WarningKind::NegativeShares)
            .collect();
        assert_eq!(negatives.len(), 1, "{:?}", report.warnings);
        assert!(negatives[0].message.contains("dipped below zero"));
        // The taint is reported once, by the warning that explains the cause.
        assert!(
            report
                .warnings
                .iter()
                .all(|w| w.kind != WarningKind::MissingBasis)
        );
        assert_eq!(report.totals.basis, None);
    }

    // ---- FX for directive prices ----

    #[test]
    fn directive_price_in_a_non_base_commodity_is_converted() {
        // `P XYZ 100.00 EUR` + `P EUR $1.10` prices XYZ at $110. The directive
        // path used to demand an exact base match, so XYZ silently fell back to a
        // stale cost annotation (or read as unpriced) while the cost path had
        // been converting all along.
        let txns = [txn(
            1,
            "2025-01-05",
            vec![
                buy("assets:broker:xyz", "XYZ", 10, 9000, true),
                posting("assets:broker:cash", vec![usd(-90000)], &[]),
            ],
            &[],
        )];
        let prices = [
            pd("2025-02-01", "XYZ", 10000, "EUR"),
            pd("2025-02-01", "EUR", 110, "$"),
            pd("2025-02-01", "VTI", 12000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        assert_eq!(report.base, "$");
        let xyz = only(&report, "XYZ");
        let price = xyz.price.as_ref().expect("XYZ priced via the FX chain");
        assert_eq!(price.source, PriceSource::Directive);
        assert_eq!(price.date, "2025-02-01");
        assert_eq!(price.qty, Dec::new(11000, 2)); // €100.00 × 1.10
        assert_eq!(xyz.market_value, Some(Dec::new(1100, 0)));
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn a_base_direct_directive_still_wins_over_a_convertible_one() {
        let txns = [txn(
            1,
            "2025-01-05",
            vec![buy("assets:broker:xyz", "XYZ", 10, 9000, true)],
            &[],
        )];
        let prices = [
            pd("2025-02-01", "XYZ", 10000, "EUR"),
            pd("2025-01-15", "XYZ", 5000, "$"),
            pd("2025-02-01", "EUR", 110, "$"),
            pd("2025-02-01", "VTI", 12000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let price = only(&report, "XYZ").price.as_ref().expect("priced");
        assert_eq!(price.qty, Dec::new(5000, 2), "the $ directive wins");
        assert_eq!(price.date, "2025-01-15");
    }

    #[test]
    fn unconvertible_cost_warning_names_the_commodity() {
        // The taint message used to claim the lot was "acquired without a cost
        // annotation" even when it carried one that simply could not be valued.
        let txns = [
            txn(
                1,
                "2025-01-10",
                vec![posting(
                    "a",
                    vec![with_cost(amt("XYZ", 10, 0), 10000, true, "GBP")],
                    &[],
                )],
                &[],
            ),
            txn(2, "2025-01-10", vec![buy_no_cost("a", "NOP", 10)], &[]),
        ];
        let prices = [
            pd("2025-02-01", "XYZ", 15000, "$"),
            pd("2025-02-01", "NOP", 15000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        let messages: BTreeMap<&str, &str> = report
            .warnings
            .iter()
            .map(|w| (w.symbol.as_str(), w.message.as_str()))
            .collect();
        assert!(
            messages["XYZ"].contains("cost annotated in GBP"),
            "was: {}",
            messages["XYZ"]
        );
        assert!(messages["NOP"].contains("acquired without a cost annotation"));
    }

    // ---- same-transaction round trips ----

    #[test]
    fn same_transaction_round_trip_at_a_new_price_repools_the_basis() {
        // Hold 10 VTI (basis $1000); one transaction sells all 10 at $100 and
        // re-buys 10 at $500. The zero-net shortcut used to swallow the whole
        // transaction, leaving `basis $1000` against a $5000 market value and
        // reporting a +400% gain that never happened.
        let txns = [
            txn(
                1,
                "2025-01-05",
                vec![
                    buy("assets:broker:vti", "VTI", 10, 10000, true),
                    posting("assets:broker:cash", vec![usd(-100_000)], &[]),
                ],
                &[],
            ),
            txn(
                2,
                "2025-03-01",
                vec![
                    posting(
                        "assets:broker:vti",
                        vec![with_cost(amt("VTI", -10, 0), 10000, true, "$")],
                        &[],
                    ),
                    buy("assets:broker:vti", "VTI", 10, 50000, true),
                    posting("assets:broker:cash", vec![usd(-400_000)], &[]),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-07-01", "VTI", 50000, "$")],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.shares, Dec::new(10, 0));
        assert_eq!(vti.basis, Some(Dec::new(500_000, 2)), "re-costed at $500");
        assert_eq!(vti.market_value, Some(Dec::new(5000, 0)));
        assert_eq!(vti.gain, Some(Dec::zero()));
    }

    #[test]
    fn a_transfer_costed_identically_on_both_legs_is_still_a_pure_move() {
        // Guard for the round-trip fix: costs that cancel exactly mean the
        // position merely moved accounts at one price — the basis is untouched.
        let txns = [
            txn(
                1,
                "2025-01-05",
                vec![buy("assets:broker:a", "VTI", 10, 10000, true)],
                &[],
            ),
            txn(
                2,
                "2025-03-01",
                vec![
                    posting(
                        "assets:broker:a",
                        vec![with_cost(amt("VTI", -4, 0), 25000, true, "$")],
                        &[],
                    ),
                    buy("assets:broker:b", "VTI", 4, 25000, true),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-07-01", "VTI", 25000, "$")],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.shares, Dec::new(10, 0));
        assert_eq!(vti.basis, Some(Dec::new(100_000, 2)), "unchanged");
        assert_eq!(
            vti.accounts,
            vec!["assets:broker:a".to_string(), "assets:broker:b".to_string()]
        );
    }

    // ---- return of capital ----

    #[test]
    fn return_of_capital_reduces_the_basis() {
        // Cash paid out of the account that holds AAPL — and nothing else — is a
        // return of the capital invested, so the basis falls from $1000 to $970.
        let txns = [
            opened_aapl(),
            txn(
                2,
                "2025-04-01",
                vec![
                    posting("assets:broker:cash", vec![usd(3000)], &[]),
                    posting("assets:broker:aapl", vec![usd(-3000)], &[]),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-07-01", "AAPL", 11000, "$")],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        assert_eq!(aapl.shares, Dec::new(10, 0), "shares are untouched");
        assert_eq!(aapl.basis, Some(Dec::new(97000, 2)));
        assert_eq!(aapl.gain, Some(Dec::new(130, 0))); // $1100 − $970
    }

    #[test]
    fn an_ordinary_dividend_leaves_shares_and_basis_alone() {
        // Pins the behaviour that is already right: a dividend lands in a CASH
        // account, so it never reaches the security's basis.
        let txns = [
            opened_aapl(),
            txn(
                2,
                "2025-05-01",
                vec![
                    posting("assets:broker:cash", vec![usd(2500)], &[]),
                    posting("income:dividends", vec![usd(-2500)], &[]),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-07-01", "AAPL", 11000, "$")],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        assert_eq!(aapl.shares, Dec::new(10, 0));
        assert_eq!(aapl.basis, Some(Dec::new(100_000, 2)));
    }

    #[test]
    fn cash_in_an_account_holding_two_securities_never_touches_a_basis() {
        // Guard: the adjustment is only attributable when the account holds
        // exactly one security. A mixed account's cash movements are left alone.
        let txns = [
            txn(
                1,
                "2025-01-05",
                vec![
                    buy("assets:broker", "AAPL", 10, 10000, true),
                    buy("assets:broker", "VTI", 10, 10000, true),
                ],
                &[],
            ),
            txn(
                2,
                "2025-04-01",
                vec![
                    posting("assets:bank", vec![usd(3000)], &[]),
                    posting("assets:broker", vec![usd(-3000)], &[]),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[
                pd("2025-07-01", "AAPL", 10000, "$"),
                pd("2025-07-01", "VTI", 10000, "$"),
            ],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        assert_eq!(only(&report, "AAPL").basis, Some(Dec::new(100_000, 2)));
        assert_eq!(only(&report, "VTI").basis, Some(Dec::new(100_000, 2)));
    }

    #[test]
    fn the_cash_leg_of_a_trade_in_the_same_account_is_not_a_basis_return() {
        // Guard: a brokerage that books the share leg and its cash consideration
        // in ONE account would otherwise have every purchase halve its own basis.
        let txns = [txn(
            1,
            "2025-01-05",
            vec![
                buy("assets:broker", "AAPL", 10, 10000, true),
                posting("assets:broker", vec![usd(-100_000)], &[]),
            ],
            &[],
        )];
        let report = run(
            &txns,
            &[pd("2025-07-01", "AAPL", 11000, "$")],
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        assert_eq!(only(&report, "AAPL").basis, Some(Dec::new(100_000, 2)));
    }

    // ---- price staleness ----

    #[test]
    fn a_price_carries_forward_indefinitely_but_reports_its_true_date() {
        // Carry-forward is unbounded by design (a quote does not expire), so a
        // years-old `P` still values the position. The row therefore has to carry
        // the quote's REAL date: that is the only thing a caller can use to tell
        // a live price from a stale one.
        let txns = [txn(
            1,
            "2019-01-05",
            vec![buy("assets:broker:aapl", "AAPL", 10, 10000, true)],
            &[],
        )];
        let report = run(
            &txns,
            &[pd("2019-02-01", "AAPL", 12000, "$")],
            &scope("2026-06-30", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        let price = aapl.price.as_ref().expect("carried forward");
        assert_eq!(price.qty, Dec::new(12000, 2));
        assert_eq!(
            price.date, "2019-02-01",
            "seven years stale, and says so — the age is `as_of − price.date`"
        );
        assert_eq!(report.as_of, "2026-06-30");
        assert_eq!(aapl.market_value, Some(Dec::new(1200, 0)));
    }

    #[test]
    fn gain_since_reprioritizes_gainers_and_totals() {
        // AAA is flat within the window; BBB is up within it. The windowed
        // gainers/totals must reflect the window, not all-time.
        let txns = [
            txn(1, "2024-01-10", vec![buy("a", "AAA", 10, 5000, true)], &[]),
            txn(2, "2025-06-10", vec![buy("a", "BBB", 10, 10000, true)], &[]),
        ];
        let prices = [
            pd("2025-07-01", "AAA", 10000, "$"),
            pd("2025-07-01", "BBB", 10000, "$"),
            pd("2026-06-30", "AAA", 10000, "$"),
            pd("2026-06-30", "BBB", 12000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope_since("2026-06-30", ScopeMode::Include, &[], "2025-07-01"),
        );
        // AAA: start 10×$100, mv 10×$100 → windowed gain 0.
        assert_eq!(only(&report, "AAA").gain, Some(Dec::zero()));
        // BBB: start 10×$100, mv 10×$120 → windowed gain $200.
        assert_eq!(only(&report, "BBB").gain, Some(Dec::new(200, 0)));
        let gainers: Vec<&str> = report
            .top_gainers
            .iter()
            .map(|h| h.symbol.as_str())
            .collect();
        assert_eq!(gainers, ["BBB"], "only BBB gained within the window");
        assert_eq!(report.totals.market_value, Dec::new(2200, 0));
        assert_eq!(
            report.totals.basis,
            Some(Dec::new(1500, 0)),
            "all-time basis"
        );
        assert_eq!(
            report.totals.gain,
            Some(Dec::new(200, 0)),
            "windowed gain total"
        );
    }

    // ---- choosing the valuation commodity (HOLD-3) ----

    /// 10 VTI bought at `$120.00`, priced by one `P VTI $120.00` directive.
    fn usd_portfolio() -> Vec<Transaction> {
        vec![txn(
            1,
            "2026-01-05",
            vec![
                buy("assets:broker:vti", "VTI", 10, 12000, true),
                posting("assets:broker:cash", vec![usd(-120_000)], &[]),
            ],
            &[],
        )]
    }

    /// The single `P VTI $120.00` that prices the portfolio, plus three jotted
    /// travel cross-rates that price nothing it holds.
    fn cross_rate_prices() -> Vec<PriceDirective> {
        vec![
            pd("2026-06-30", "VTI", 12000, "$"),
            pd("2026-07-01", "GBP", 115, "EUR"),
            pd("2026-07-02", "CHF", 115, "EUR"),
            pd("2026-07-03", "SEK", 115, "EUR"),
        ]
    }

    /// HOLD-3. Frequency puts `EUR` first (3 votes to 1) and nothing connects
    /// `VTI` to `EUR`, so the whole $1,200 portfolio used to read `$0` with a
    /// null basis. Coverage is what has to win.
    #[test]
    fn base_prefers_the_commodity_that_actually_prices_the_portfolio() {
        let prices = cross_rate_prices();
        assert_eq!(
            PriceDb::build(&prices).base_commodity(),
            Some(&Commodity("EUR".to_string())),
            "the frequency rule alone still says EUR — that is the bug being overridden"
        );
        let report = run(
            &usd_portfolio(),
            &prices,
            &scope("2026-07-16", ScopeMode::Include, &[]),
        );
        // hledger 1.52: `bal --value=end,'$'` → $1,200.00 assets:broker:vti.
        assert_eq!(report.base, "$");
        assert_eq!(report.totals.market_value, Dec::new(120_000, 2));
        assert_eq!(report.totals.basis, Some(Dec::new(120_000, 2)));
        assert!(
            report.warnings.is_empty(),
            "a fully priced, fully costed position warns about nothing: {:?}",
            report.warnings
        );
    }

    /// When every candidate prices the portfolio equally well, the old ranking
    /// still decides — coverage only breaks the cases it can see a difference in.
    #[test]
    fn base_falls_back_to_frequency_when_coverage_ties() {
        let prices = vec![
            pd("2026-06-30", "VTI", 12000, "$"),
            pd("2026-06-30", "VTI", 11000, "EUR"),
            pd("2026-07-01", "GBP", 115, "EUR"),
        ];
        let report = run(
            &usd_portfolio(),
            &prices,
            &scope("2026-07-16", ScopeMode::Include, &[]),
        );
        assert_eq!(report.base, "EUR", "EUR outvotes $ 2:1 and prices VTI too");
        // hledger 1.52: `bal --value=end,EUR` → 1100.00 EUR assets:broker:vti.
        assert_eq!(report.totals.market_value, Dec::new(110_000, 2));
    }

    /// An explicit `value_in` is used verbatim — including when it prices
    /// nothing. Second-guessing the caller would just move the surprise.
    #[test]
    fn explicit_value_in_is_honoured_over_the_automatic_choice() {
        let txns = usd_portfolio();
        let prices = cross_rate_prices();
        let priced = run(
            &txns,
            &prices,
            &scope_in("2026-07-16", ScopeMode::Include, &[], "$"),
        );
        assert_eq!(priced.base, "$");
        assert_eq!(priced.totals.market_value, Dec::new(120_000, 2));

        let unpriced = run(
            &txns,
            &prices,
            &scope_in("2026-07-16", ScopeMode::Include, &[], "EUR"),
        );
        assert_eq!(unpriced.base, "EUR");
        assert!(unpriced.totals.market_value.is_zero());
        // The HTTP layer refuses this request outright — see `prices_any_held`.
        assert!(
            !prices_any_held(
                &txns,
                &prices,
                &[],
                &scope("2026-07-16", ScopeMode::Include, &[]),
                &Commodity("EUR".to_string()),
            )
            .expect("coverage math does not overflow")
        );
    }

    /// The admission test the `valueIn` param is held to: `$` prices the
    /// portfolio, `EUR` and a typo do not, and a scope holding nothing accepts
    /// anything (there is nothing to leave unpriced).
    #[test]
    fn prices_any_held_answers_the_valuation_admission_question() {
        let txns = usd_portfolio();
        let prices = cross_rate_prices();
        let any = |scope: &HoldingsScope, target: &str| {
            prices_any_held(&txns, &prices, &[], scope, &Commodity(target.to_string()))
                .expect("coverage math does not overflow")
        };
        let held = scope("2026-07-16", ScopeMode::Include, &[]);
        assert!(any(&held, "$"));
        assert!(any(&held, "VTI"), "a commodity always prices itself");
        assert!(!any(&held, "EUR"));
        assert!(!any(&held, "NOPE"));
        // Nothing held at all (before the buy): no holding can be left unpriced.
        let empty = scope("2025-01-01", ScopeMode::Include, &[]);
        assert!(any(&empty, "EUR"));
        assert!(any(&empty, "NOPE"));
    }

    /// The commodity a report will be denominated in, without computing it.
    #[test]
    fn valuation_base_reports_the_choice_the_report_will_make() {
        let txns = usd_portfolio();
        let prices = cross_rate_prices();
        let base = |scope: &HoldingsScope| {
            valuation_base(&txns, &prices, &[], scope)
                .expect("base resolution does not overflow")
                .0
        };
        assert_eq!(base(&scope("2026-07-16", ScopeMode::Include, &[])), "$");
        assert_eq!(
            base(&scope_in("2026-07-16", ScopeMode::Include, &[], "EUR")),
            "EUR"
        );
        // No holdings in scope: nothing to cover, so the ranking answers.
        assert_eq!(base(&scope("2025-01-01", ScopeMode::Include, &[])), "EUR");
    }

    /// With no `P` directives at all there is nothing to rank, and the report
    /// keeps its historical `$` fallback.
    #[test]
    fn base_falls_back_to_dollars_without_any_prices() {
        let report = run(
            &usd_portfolio(),
            &[],
            &scope("2026-07-16", ScopeMode::Include, &[]),
        );
        assert_eq!(report.base, "$");
    }

    // ---- sole-symbol facts (the one rule the replay re-derives) ----

    /// Every shape the sole-symbol rule can take, spread over time so each one
    /// has a BEFORE and an AFTER within the sweep below.
    fn sole_symbol_journal() -> Vec<Transaction> {
        vec![
            // `late` takes cash out BEFORE it has ever held a security. At an
            // early `as_of` it is absent from the map (no reduction); at a late
            // one it is `Some("GLD")` and this very transaction reduces GLD's
            // basis retroactively.
            txn(
                1,
                "2025-01-10",
                vec![
                    posting("assets:broker:late", vec![usd(-1000)], &[]),
                    posting("assets:bank", vec![usd(1000)], &[]),
                ],
                &[],
            ),
            // `one` holds exactly one security, forever.
            txn(
                2,
                "2025-02-10",
                vec![buy("assets:broker:one", "AAPL", 10, 10000, true)],
                &[],
            ),
            txn(
                3,
                "2025-02-20",
                vec![
                    posting("assets:broker:one", vec![usd(-500)], &[]),
                    posting("assets:bank", vec![usd(500)], &[]),
                ],
                &[],
            ),
            // `mixed` is single-security until August, then two.
            txn(
                4,
                "2025-03-10",
                vec![buy("assets:broker:mixed", "VTI", 10, 10000, true)],
                &[],
            ),
            txn(
                5,
                "2025-04-10",
                vec![
                    posting("assets:broker:mixed", vec![usd(-700)], &[]),
                    posting("assets:bank", vec![usd(700)], &[]),
                ],
                &[],
            ),
            // `quiet` holds a security but never pays cash out: never asked
            // about, so the facts drop it.
            txn(
                6,
                "2025-05-10",
                vec![buy("assets:broker:quiet", "GLD", 10, 10000, true)],
                &[],
            ),
            // `sameday` takes on two securities in ONE transaction: ambiguous
            // from that very date, never `Some`.
            txn(
                7,
                "2025-06-10",
                vec![
                    buy("assets:broker:sameday", "AAPL", 1, 10000, true),
                    buy("assets:broker:sameday", "VTI", 1, 10000, true),
                    posting("assets:broker:sameday", vec![usd(-200)], &[]),
                ],
                &[],
            ),
            // A non-holding account: excluded by both implementations.
            txn(
                8,
                "2025-07-10",
                vec![
                    posting("equity:opening", vec![usd(-300)], &[]),
                    posting("assets:bank", vec![usd(300)], &[]),
                ],
                &[],
            ),
            txn(
                9,
                "2025-08-10",
                vec![buy("assets:broker:mixed", "AAPL", 1, 10000, true)],
                &[],
            ),
            txn(
                10,
                "2025-09-10",
                vec![buy("assets:broker:late", "GLD", 3, 10000, true)],
                &[],
            ),
        ]
    }

    /// `sole_symbols_at` reads two dates off a precomputed summary where
    /// `sole_symbols_by_account` re-scans the journal. They must agree at EVERY
    /// date for every account the replay can ask about — this is the only rule
    /// the multi-date replay re-derives rather than moves, and the rule looks
    /// PAST the transaction it is applied to, so an off-by-one here would
    /// silently move a return-of-capital basis reduction.
    #[test]
    fn precomputed_sole_symbols_match_the_rescan_at_every_date() {
        let txns = sole_symbol_journal();
        let sc = scope("2025-12-31", ScopeMode::Include, &[]);
        let predicate = scope_predicate(&sc);
        let declared = BTreeMap::new();
        let ordered = journal_order(&txns);
        let facts = sole_symbol_facts(&ordered, "2025-12-31", &predicate, &declared);

        // Only the accounts that both hold a security AND pay cash out survive;
        // `quiet` (never asked) and `assets:broker:cash`-alikes (never holds)
        // are exactly the ones a series must not be split by.
        let kept: Vec<&str> = facts.keys().map(String::as_str).collect();
        assert_eq!(
            kept,
            [
                "assets:broker:late",
                "assets:broker:mixed",
                "assets:broker:one",
                "assets:broker:sameday",
            ]
        );

        // Sweep the 1st, 10th, 11th and 28th of every month of 2025 — the 10th
        // and 11th straddle every transition date in the journal above.
        for month in 1..=12 {
            for day in [1, 10, 11, 28] {
                let as_of = format!("2025-{month:02}-{day:02}");
                let rescanned = sole_symbols_by_account(&txns, &as_of, &predicate, &declared);
                let precomputed = sole_symbols_at(&facts, &as_of);
                for account in facts.keys() {
                    assert_eq!(
                        precomputed.get(account),
                        rescanned.get(account),
                        "{account} at {as_of}"
                    );
                }
            }
        }
    }

    /// The facts are capped at a date, but a LATER cap must not change any
    /// earlier date's answer — `holdings_at_each` builds one set of facts for
    /// the whole series and evaluates every point against it, while
    /// `compute_holdings` caps at its own `as_of`.
    #[test]
    fn a_later_facts_cap_does_not_change_an_earlier_answer() {
        let txns = sole_symbol_journal();
        let sc = scope("2025-12-31", ScopeMode::Include, &[]);
        let predicate = scope_predicate(&sc);
        let declared = BTreeMap::new();
        let ordered = journal_order(&txns);
        let full = sole_symbol_facts(&ordered, "2025-12-31", &predicate, &declared);
        for month in 1..=12 {
            let as_of = format!("2025-{month:02}-15");
            let capped = sole_symbol_facts(&ordered, &as_of, &predicate, &declared);
            let from_full = sole_symbols_at(&full, &as_of);
            let from_capped = sole_symbols_at(&capped, &as_of);
            // The wider cap may KEEP an account the narrow one drops (it is
            // asked about later), but never disagrees about one they share.
            for (account, sole) in &from_capped {
                assert_eq!(from_full.get(account), Some(sole), "{account} at {as_of}");
            }
        }
    }
}
