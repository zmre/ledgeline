//! Posting aggregation — port of `web/src/lib/domain/aggregate.ts`.

use super::accounts::account_matches;
use super::mixed_amount::MixedAmount;
use crate::decimal::DecError;
use crate::model::{Status, Transaction};
use std::collections::BTreeMap;

/// Filters applied to a posting before it contributes to a total. Absent fields
/// mean "no constraint".
#[derive(Debug, Clone, Default)]
pub struct PostingFilter<'a> {
    /// Inclusive lower bound on the posting's effective date.
    pub from: Option<&'a str>,
    /// Inclusive upper bound on the posting's effective date.
    pub to: Option<&'a str>,
    /// Selected accounts (each matches itself + sub-accounts); empty/absent =
    /// all.
    pub accounts: Option<&'a [String]>,
    /// Required effective status.
    pub status: Option<Status>,
}

/// One pass over all postings, summing per FULL account name.
///
/// The effective posting date is `posting.date ?? txn.date`; the effective
/// status falls back to the transaction's when the posting is unmarked (hledger
/// semantics). Zero commodities are dropped in a single final sweep.
///
/// # Errors
/// Returns [`DecError`] on decimal overflow.
pub fn account_totals(
    txns: &[Transaction],
    filter: &PostingFilter,
) -> Result<BTreeMap<String, MixedAmount>, DecError> {
    let selected = match filter.accounts {
        Some(accounts) if !accounts.is_empty() => Some(accounts),
        _ => None,
    };
    let mut totals: BTreeMap<String, MixedAmount> = BTreeMap::new();
    for txn in txns {
        for posting in &txn.postings {
            let date = posting.date.as_deref().unwrap_or(&txn.date);
            if filter.from.is_some_and(|from| date < from) {
                continue;
            }
            if filter.to.is_some_and(|to| date > to) {
                continue;
            }
            if let Some(want) = filter.status {
                let effective = if posting.status == Status::Unmarked {
                    txn.status
                } else {
                    posting.status
                };
                if effective != want {
                    continue;
                }
            }
            if let Some(sel) = selected
                && !sel.iter().any(|s| account_matches(s, &posting.account.0))
            {
                continue;
            }
            let entry = totals.entry(posting.account.0.clone()).or_default();
            for amount in &posting.amounts {
                entry.accumulate(&amount.commodity, amount.quantity)?;
            }
        }
    }
    for ma in totals.values_mut() {
        ma.drop_zeros();
    }
    Ok(totals)
}

/// Add each account's total into itself and all ancestors (inclusive balances).
///
/// Accumulates in place. The old form read the ancestor's running total back
/// out, `ma_add`ed a fresh map and re-inserted it, cloning the accumulator once
/// per descendant (PERF-5f).
///
/// # Errors
/// Returns [`DecError`] on decimal overflow.
pub fn roll_up(
    totals: &BTreeMap<String, MixedAmount>,
) -> Result<BTreeMap<String, MixedAmount>, DecError> {
    let mut out: BTreeMap<String, MixedAmount> = BTreeMap::new();
    for (account, ma) in totals {
        let mut path = String::new();
        for segment in account.split(':') {
            if !path.is_empty() {
                path.push(':');
            }
            path.push_str(segment);
            out.entry(path.clone()).or_default().ma_add_assign(ma)?;
        }
    }
    Ok(out)
}

/// Keep only accounts with at most `depth` segments.
///
/// `depth == 0` selects nothing, which is "totals only": `hledger --depth 0`
/// likewise shows no per-account detail, collapsing everything into a single
/// `...` row and printing just the totals. Callers must therefore never derive a
/// total from this function's output — every report total here is summed from
/// the unclamped accounts, so `?depth=0` reports hledger's totals rather than
/// zeros (RPT-4).
#[must_use]
pub fn at_depth(
    rolled: &BTreeMap<String, MixedAmount>,
    depth: usize,
) -> BTreeMap<String, MixedAmount> {
    rolled
        .iter()
        .filter(|(account, _)| account.split(':').count() <= depth)
        .map(|(account, ma)| (account.clone(), ma.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decimal::Dec;
    use crate::model::Commodity;

    fn rolled() -> BTreeMap<String, MixedAmount> {
        let mut totals = BTreeMap::new();
        totals.insert(
            "assets:bank:checking".to_string(),
            MixedAmount::single(Commodity("$".into()), Dec::new(1000, 2)),
        );
        roll_up(&totals).unwrap()
    }

    #[test]
    fn at_depth_keeps_accounts_up_to_the_limit() {
        assert_eq!(
            at_depth(&rolled(), 2).keys().collect::<Vec<_>>(),
            ["assets", "assets:bank"]
        );
        assert_eq!(at_depth(&rolled(), 3).len(), 3);
        assert_eq!(at_depth(&rolled(), 9).len(), 3);
    }

    /// `--depth 0` is "totals only" — no per-account rows. Report totals are
    /// summed from the unclamped accounts, so they survive this (RPT-4).
    #[test]
    fn at_depth_zero_selects_nothing() {
        assert!(at_depth(&rolled(), 0).is_empty());
    }
}
