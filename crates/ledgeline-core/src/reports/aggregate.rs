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
/// # Errors
/// Returns [`DecError`] on decimal overflow.
pub fn roll_up(
    totals: &BTreeMap<String, MixedAmount>,
) -> Result<BTreeMap<String, MixedAmount>, DecError> {
    let mut out: BTreeMap<String, MixedAmount> = BTreeMap::new();
    for (account, ma) in totals {
        let mut path = String::new();
        for segment in account.split(':') {
            if path.is_empty() {
                path.push_str(segment);
            } else {
                path.push(':');
                path.push_str(segment);
            }
            let combined = match out.get(&path) {
                Some(existing) => existing.ma_add(ma)?,
                None => ma.clone(),
            };
            out.insert(path.clone(), combined);
        }
    }
    Ok(out)
}

/// Keep only accounts with at most `depth` segments.
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
