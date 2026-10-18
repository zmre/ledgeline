//! Cash and short-term-liability balances: the journal tab's Balances view and
//! the `negative-cash` check (plan 26 B3; a port of the SPA's former
//! `web/src/lib/reports/cashBalances.ts`).
//!
//! # What this answers, and what it deliberately does not
//!
//! "How much cash do I have right now, where is it, and how stale is each
//! figure." Narrower than the balance sheet on purpose: no securities, no
//! property, no valuation, no FX. Every figure is a balance as WRITTEN in one
//! commodity, so nothing depends on a `P` directive, and a missing price can
//! never make this view read zero.
//!
//! # Classification
//!
//! Two declared tags, never an account NAME — both resolved by the engine
//! ([`Classes`]):
//!
//! - `type:` — cash when the effective type is hledger's Cash subtype, a
//!   liability when it is Liability.
//! - `bsterm:` — non-current accounts are excluded. When nothing in the
//!   ancestry declares a term, [`crate::reports::infer_bs_term`] may GUESS
//!   non-current from the name; such accounts are excluded too but NAMED
//!   ([`CashBalances::guessed_long_term`]) so the guess is never silent.
//!
//! # Semantics kept from the TypeScript (no alignment with the balance sheet)
//!
//! - Postings of EVERY type count — regular, `(virtual)` and `[balanced
//!   virtual]` — which is what hledger's own `balance` does.
//! - `bsgroup:` is not consulted. An account moved onto or off the balance
//!   sheet's Cash line by an explicit group tag is grouped by
//!   `AccountGroups::resolve`, which this does not reimplement; membership here
//!   is by type alone.
//! - Amounts count as written; a cost annotation is ignored.
//! - The effective posting date is `posting.date ?? txn.date`.
//!
//! # Signs
//!
//! None are applied: hledger records a liability you owe as negative and cash
//! you hold as positive, which is the display the view wants. A NEGATIVE cash
//! balance means overdrawn, and the `negative-cash` check flags it.

use super::Classes;
use super::wire::{DeclaredStyles, WireDec, WireStyle, default_style};
use crate::decimal::Dec;
use crate::model::Journal;
use crate::reports::{AccountType, BsTerm};
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

/// Which half of the view an account belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BalanceKind {
    /// Effective type Cash.
    Cash,
    /// Effective type Liability.
    Liability,
}

/// What the classifier decided about one account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Membership {
    In(BalanceKind),
    /// Excluded because a word in its NAME looked long-term.
    GuessedLongTerm,
    Out,
}

/// The account-selection seam (`cashViewClassifier`): the declared type
/// decides eligibility, then the engine-resolved term decides; a guess is
/// distinguished from a tag so the view can say so.
fn membership(account: &str, classes: &Classes) -> Membership {
    let kind = match classes.account_type(account) {
        Some(AccountType::Cash) => BalanceKind::Cash,
        Some(AccountType::Liability) => BalanceKind::Liability,
        _ => return Membership::Out,
    };
    match classes.term(account) {
        (Some(BsTerm::NonCurrent), true) => Membership::GuessedLongTerm,
        (Some(BsTerm::NonCurrent), false) => Membership::Out,
        _ => Membership::In(kind),
    }
}

/// One account's as-written balances across every commodity it holds.
#[derive(Debug, Clone)]
pub struct AccountBalance {
    /// Full account name, exactly as posted to (never rolled up).
    pub account: String,
    /// Which list it belongs in.
    pub kind: BalanceKind,
    /// `(commodity, balance)` in first-posted order. Commodities netting to
    /// zero are KEPT: a zeroed account is a fact worth showing.
    pub amounts: Vec<(String, Dec)>,
    /// Effective date of the most recent counted posting, in any commodity.
    pub as_of: String,
    /// A posting on that latest date carried a balance assertion.
    pub confirmed: bool,
}

/// One pass's whole answer.
#[derive(Debug, Clone, Default)]
pub struct CashBalances {
    /// Accounts in the view, in first-posting order.
    pub balances: Vec<AccountBalance>,
    /// Accounts a name GUESS excluded (never a tag), sorted — only those with
    /// a posting in range.
    pub guessed_long_term: Vec<String>,
}

/// Every cash and short-term-liability account's balance as of `as_of`
/// (inclusive), from the WHOLE journal (balances are cumulative).
///
/// A balance that would overflow `i128` keeps its last representable value
/// rather than aborting the view; a real journal cannot reach it.
#[must_use]
pub fn cash_balances(journal: &Journal, classes: &Classes, as_of: &str) -> CashBalances {
    let mut kinds: HashMap<&str, Membership> = HashMap::new();
    let mut slots: HashMap<&str, usize> = HashMap::new();
    let mut balances: Vec<AccountBalance> = Vec::new();
    let mut guessed: BTreeSet<&str> = BTreeSet::new();
    for txn in &journal.transactions {
        for posting in &txn.postings {
            let date = posting.date.as_deref().unwrap_or(&txn.date);
            if date > as_of {
                continue;
            }
            let account = posting.account.0.as_str();
            let kind = *kinds
                .entry(account)
                .or_insert_with(|| membership(account, classes));
            let kind = match kind {
                Membership::In(kind) => kind,
                Membership::GuessedLongTerm => {
                    guessed.insert(account);
                    continue;
                }
                Membership::Out => continue,
            };
            let slot = *slots.entry(account).or_insert_with(|| {
                balances.push(AccountBalance {
                    account: account.to_string(),
                    kind,
                    amounts: Vec::new(),
                    as_of: date.to_string(),
                    confirmed: false,
                });
                balances.len() - 1
            });
            let entry = &mut balances[slot];
            for amount in &posting.amounts {
                let symbol = amount.commodity.0.as_str();
                match entry.amounts.iter_mut().find(|(c, _)| c == symbol) {
                    Some((_, total)) => *total = total.add(amount.quantity).unwrap_or(*total),
                    None => entry.amounts.push((symbol.to_string(), amount.quantity)),
                }
            }
            let asserted = posting.balance_assertion.is_some();
            match date.cmp(entry.as_of.as_str()) {
                Ordering::Greater => {
                    entry.as_of = date.to_string();
                    entry.confirmed = asserted;
                }
                Ordering::Equal if asserted => entry.confirmed = true,
                _ => {}
            }
        }
    }
    CashBalances {
        balances,
        guessed_long_term: guessed.into_iter().map(str::to_string).collect(),
    }
}

/// Commodities held by any account in the view, most-used (per ACCOUNT) first,
/// ties alphabetical (`balanceCommodities` / `rankCommodities`).
#[must_use]
pub fn balance_commodities(balances: &[AccountBalance]) -> Vec<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for balance in balances {
        for (commodity, _) in &balance.amounts {
            *counts.entry(commodity.as_str()).or_default() += 1;
        }
    }
    let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
    ranked.sort_by(|(a_name, a_count), (b_name, b_count)| {
        b_count.cmp(a_count).then_with(|| a_name.cmp(b_name))
    });
    ranked
        .into_iter()
        .map(|(name, _)| name.to_string())
        .collect()
}

/// One row of the rendered view: one account, one commodity.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceRow {
    /// Full account name.
    pub account: String,
    /// The balance in the summary's commodity.
    pub qty: WireDec,
    /// Last counted posting date.
    pub as_of: String,
    /// Checked by an assertion on that date.
    pub confirmed: bool,
}

/// The whole view for one commodity: two lists and three headline figures.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceSummary {
    /// The commodity every figure is in.
    pub commodity: String,
    /// The style figures in this commodity display with: the first amount of
    /// it in journal order (the SPA's `styleFor` over the whole journal).
    pub style: WireStyle,
    /// Cash accounts, largest magnitude first.
    pub cash: Vec<BalanceRow>,
    /// Liability accounts, largest magnitude first.
    pub liabilities: Vec<BalanceRow>,
    /// Sum of `cash`.
    pub total_cash: WireDec,
    /// Sum of `liabilities` (negative when money is owed).
    pub total_liabilities: WireDec,
    /// `total_cash + total_liabilities`.
    pub net: WireDec,
}

/// Largest magnitude first; ties alphabetical, so the order is total.
fn by_magnitude(a: &(Dec, &AccountBalance), b: &(Dec, &AccountBalance)) -> Ordering {
    let magnitude = |value: Dec| value.abs().unwrap_or(value);
    magnitude(b.0)
        .cmp(&magnitude(a.0))
        .then_with(|| a.1.account.cmp(&b.1.account))
}

/// Project one commodity out of `balances` (`summarize`). An account with no
/// posting at all in `commodity` is omitted; one netting to zero is kept.
#[must_use]
pub fn summarize(balances: &[AccountBalance], commodity: &str, style: WireStyle) -> BalanceSummary {
    let pick = |kind: BalanceKind| -> Vec<(Dec, &AccountBalance)> {
        let mut rows: Vec<(Dec, &AccountBalance)> = balances
            .iter()
            .filter(|balance| balance.kind == kind)
            .filter_map(|balance| {
                balance
                    .amounts
                    .iter()
                    .find(|(c, _)| c == commodity)
                    .map(|(_, qty)| (*qty, balance))
            })
            .collect();
        rows.sort_by(by_magnitude);
        rows
    };
    let total = |rows: &[(Dec, &AccountBalance)]| {
        rows.iter()
            .fold(Dec::zero(), |sum, (qty, _)| sum.add(*qty).unwrap_or(sum))
    };
    let to_rows = |rows: &[(Dec, &AccountBalance)]| -> Vec<BalanceRow> {
        rows.iter()
            .map(|(qty, balance)| BalanceRow {
                account: balance.account.clone(),
                qty: (*qty).into(),
                as_of: balance.as_of.clone(),
                confirmed: balance.confirmed,
            })
            .collect()
    };
    let cash = pick(BalanceKind::Cash);
    let liabilities = pick(BalanceKind::Liability);
    let (total_cash, total_liabilities) = (total(&cash), total(&liabilities));
    BalanceSummary {
        commodity: commodity.to_string(),
        style,
        cash: to_rows(&cash),
        liabilities: to_rows(&liabilities),
        total_cash: total_cash.into(),
        total_liabilities: total_liabilities.into(),
        net: total_cash
            .add(total_liabilities)
            .unwrap_or(total_cash)
            .into(),
    }
}

/// The `/api/balances/cash` payload: every commodity's summary, so switching
/// the currency selector costs no request.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CashBalancesView {
    /// The as-of date the balances were taken at (inclusive).
    pub as_of: String,
    /// Commodities in the view, most-used first — the selector's order; the
    /// first is the default.
    pub commodities: Vec<String>,
    /// One summary per entry of `commodities`, same order.
    pub summaries: Vec<BalanceSummary>,
    /// Accounts excluded on a name guess, sorted.
    pub guessed_long_term: Vec<String>,
}

/// The first display style of each commodity in journal order — the SPA's
/// `styleFor(allTxns, commodity)`.
fn first_styles(journal: &Journal, wanted: &[String]) -> HashMap<String, WireStyle> {
    let declared = DeclaredStyles::of(journal);
    let mut found: HashMap<String, WireStyle> = HashMap::new();
    let amounts = journal
        .transactions
        .iter()
        .flat_map(|txn| &txn.postings)
        .flat_map(|posting| &posting.amounts);
    for amount in amounts {
        if found.len() == wanted.len() {
            break;
        }
        let symbol = &amount.commodity.0;
        if !found.contains_key(symbol) && wanted.contains(symbol) {
            found.insert(
                symbol.clone(),
                WireStyle::from(&declared.display_style(amount)),
            );
        }
    }
    found
}

/// Build the whole `/api/balances/cash` view.
#[must_use]
pub fn cash_balances_view(journal: &Journal, classes: &Classes, as_of: &str) -> CashBalancesView {
    let set = cash_balances(journal, classes, as_of);
    let commodities = balance_commodities(&set.balances);
    let mut styles = first_styles(journal, &commodities);
    let summaries = commodities
        .iter()
        .map(|commodity| {
            let style = styles
                .remove(commodity)
                .unwrap_or_else(|| WireStyle::from(&default_style()));
            summarize(&set.balances, commodity, style)
        })
        .collect();
    CashBalancesView {
        as_of: as_of.to_string(),
        commodities,
        summaries,
        guessed_long_term: set.guessed_long_term,
    }
}
