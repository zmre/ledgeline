//! Cash flow — port of `web/src/lib/reports/cashFlow.ts`.
//!
//! Per-bucket changes (natural signs: inflow positive) in cash-like asset
//! accounts, for the last `count` buckets ending with the bucket containing
//! `end`. The final bucket is truncated at `end`.

use super::ReportError;
use super::account_types::{AccountType, infer_account_type};
use super::aggregate::{at_depth, roll_up};
use super::mixed_amount::MixedAmount;
use super::periods::{Interval, bucket_end, bucket_start, compare_iso, last_n_buckets};
use super::types::{PeriodReport, PeriodRow};
use crate::model::Transaction;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Name-based "cash-like asset" heuristic — the fallback when a journal declares
/// no account types. Delegates to hledger's Cash name inference.
#[must_use]
pub fn is_cash_like(account: &str) -> bool {
    infer_account_type(account) == Some(AccountType::Cash)
}

/// Per-bucket cash flow. `is_cash` overrides the name heuristic (pass the result
/// of [`super::account_types::cash_predicate`] to honor declared `type:` tags).
///
/// `is_cash` must be a pure function of the account name: it is consulted once
/// per DISTINCT account rather than once per account per bucket.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or bad bucket math.
pub fn cash_flow(
    txns: &[Transaction],
    end: &str,
    interval: Interval,
    count: usize,
    depth: usize,
    is_cash: Option<&dyn Fn(&str) -> bool>,
) -> Result<PeriodReport, ReportError> {
    let default_pred = |account: &str| is_cash_like(account);
    let is_cash: &dyn Fn(&str) -> bool = match is_cash {
        Some(pred) => pred,
        None => &default_pred,
    };

    let buckets = last_n_buckets(end, interval, count)?;
    let mut totals: Vec<MixedAmount> = Vec::with_capacity(buckets.len());
    let mut per_bucket: Vec<BTreeMap<String, MixedAmount>> = Vec::with_capacity(buckets.len());

    // Each bucket's inclusive `[start, to]` range, with the last one truncated
    // at `end`. `last_n_buckets` yields CONTIGUOUS, non-overlapping buckets
    // oldest → newest, so the `to` bounds ascend strictly and every posting
    // falls in at most one of them — which is what lets the binary search below
    // replace one `account_totals` re-scan per bucket (PERF-5).
    let ranges: Vec<(String, String)> = buckets
        .iter()
        .map(|key| {
            let start = bucket_start(key)?;
            let end_of_bucket = bucket_end(key)?;
            let to = if compare_iso(end, &end_of_bucket) == Ordering::Less {
                end.to_string()
            } else {
                end_of_bucket
            };
            Ok((start, to))
        })
        .collect::<Result<_, ReportError>>()?;

    // ONE pass over every posting, summing per FULL account name into its own
    // bucket — i.e. exactly what `account_totals(from, to)` would have produced
    // for that bucket, and in the same transaction order, so no number can move.
    let mut direct: Vec<BTreeMap<&str, MixedAmount>> = vec![BTreeMap::new(); ranges.len()];
    let mut cash_like: HashMap<&str, bool> = HashMap::new();
    for txn in txns {
        for posting in &txn.postings {
            let date = posting.date.as_deref().unwrap_or(&txn.date);
            let index = ranges.partition_point(|(_, to)| to.as_str() < date);
            let Some((start, _)) = ranges.get(index) else {
                continue; // after the last bucket
            };
            if date < start.as_str() {
                continue; // before the report span
            }
            let account = posting.account.0.as_str();
            if !*cash_like.entry(account).or_insert_with(|| is_cash(account)) {
                continue;
            }
            let entry = direct[index].entry(account).or_default();
            for amount in &posting.amounts {
                entry.accumulate(&amount.commodity, amount.quantity)?;
            }
        }
    }

    for bucket in &direct {
        // `account_totals` prunes zero commodities in one final sweep.
        let direct: BTreeMap<String, MixedAmount> = bucket
            .iter()
            .map(|(account, ma)| {
                let mut pruned = ma.clone();
                pruned.drop_zeros();
                ((*account).to_string(), pruned)
            })
            .collect();

        let mut total = MixedAmount::new();
        for ma in direct.values() {
            total = total.ma_add(ma)?;
        }
        totals.push(total);
        per_bucket.push(at_depth(&roll_up(&direct)?, depth));
    }

    let accounts: BTreeSet<String> = per_bucket
        .iter()
        .flat_map(|clamped| clamped.keys().cloned())
        .collect();
    let rows = accounts
        .into_iter()
        .map(|account| {
            let depth = account.split(':').count();
            let values = per_bucket
                .iter()
                .map(|clamped| clamped.get(&account).cloned().unwrap_or_default())
                .collect();
            PeriodRow {
                account,
                depth,
                values,
            }
        })
        .collect();

    Ok(PeriodReport {
        buckets,
        rows,
        totals,
        meta: None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{amount, mixed, txn, usd};
    use super::*;
    use crate::decimal::Dec;
    use crate::model::Commodity;

    fn usd_ma(cents: i128) -> MixedAmount {
        mixed(&[("$", cents, 2)])
    }

    fn sample() -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2026-01-10",
                vec![
                    ("assets:bank:checking", vec![usd(10_000)]),
                    ("income:salary", vec![usd(-10_000)]),
                ],
            ),
            txn(
                2,
                "2026-02-05",
                vec![
                    ("expenses:food", vec![usd(3000)]),
                    ("assets:bank:checking", vec![usd(-3000)]),
                ],
            ),
            txn(
                3,
                "2026-02-14",
                vec![
                    ("assets:broker:taxable:aapl", vec![amount("AAPL", 2, 0)]),
                    ("assets:broker:taxable:cash", vec![usd(-40_000)]),
                ],
            ),
            txn(
                4,
                "2026-02-20",
                vec![
                    ("assets:bank:savings", vec![usd(5000)]),
                    ("assets:bank:checking", vec![usd(-5000)]),
                ],
            ),
            txn(
                5,
                "2026-03-10",
                vec![
                    ("assets:bank:checking", vec![usd(7000)]),
                    ("income:salary", vec![usd(-7000)]),
                ],
            ),
            // After `end` (mid-bucket truncation):
            txn(
                6,
                "2026-03-20",
                vec![
                    ("assets:bank:checking", vec![usd(9999)]),
                    ("income:salary", vec![usd(-9999)]),
                ],
            ),
        ]
    }

    #[test]
    fn is_cash_like_matches_hledger_heuristic() {
        assert!(is_cash_like("assets:bank:checking"));
        assert!(is_cash_like("assets:bank:wise:eur"));
        assert!(is_cash_like("assets:broker:taxable:cash"));
        assert!(is_cash_like("asset:savings"));
        assert!(!is_cash_like("assets:broker:taxable:aapl"));
        assert!(!is_cash_like("assets"));
        assert!(!is_cash_like("expenses:bank"));
        assert!(!is_cash_like("liabilities:cc:visa"));
    }

    #[test]
    fn buckets_cash_changes_truncating_last_at_end() {
        let report = cash_flow(&sample(), "2026-03-15", Interval::Monthly, 3, 4, None).unwrap();
        assert_eq!(report.buckets, ["2026-01", "2026-02", "2026-03"]);
        assert_eq!(
            report
                .rows
                .iter()
                .map(|r| r.account.as_str())
                .collect::<Vec<_>>(),
            [
                "assets",
                "assets:bank",
                "assets:bank:checking",
                "assets:bank:savings",
                "assets:broker",
                "assets:broker:taxable",
                "assets:broker:taxable:cash",
            ]
        );
        let by_account = |name: &str| {
            report
                .rows
                .iter()
                .find(|r| r.account == name)
                .unwrap()
                .values
                .clone()
        };
        assert_eq!(
            by_account("assets:bank:checking"),
            [usd_ma(10_000), usd_ma(-8000), usd_ma(7000)] // 03-20 txn beyond end
        );
        assert_eq!(
            by_account("assets:bank:savings"),
            [MixedAmount::new(), usd_ma(5000), MixedAmount::new()]
        );
        assert_eq!(
            by_account("assets:broker:taxable:cash"),
            [MixedAmount::new(), usd_ma(-40_000), MixedAmount::new()]
        );
        assert_eq!(
            by_account("assets"),
            [usd_ma(10_000), usd_ma(-43_000), usd_ma(7000)]
        );
        assert_eq!(
            report.totals,
            [usd_ma(10_000), usd_ma(-43_000), usd_ma(7000)]
        );
    }

    #[test]
    fn clamps_rows_to_depth() {
        let report = cash_flow(&sample(), "2026-03-15", Interval::Monthly, 3, 2, None).unwrap();
        assert_eq!(
            report
                .rows
                .iter()
                .map(|r| r.account.as_str())
                .collect::<Vec<_>>(),
            ["assets", "assets:bank", "assets:broker"]
        );
        assert_eq!(
            report.totals,
            [usd_ma(10_000), usd_ma(-43_000), usd_ma(7000)]
        );
    }

    #[test]
    fn supports_quarterly_buckets() {
        let report = cash_flow(&sample(), "2026-06-30", Interval::Quarterly, 2, 1, None).unwrap();
        assert_eq!(report.buckets, ["2026-Q1", "2026-Q2"]);
        // Q1: 100.00 − 30.00 − 50.00 + 50.00 − 400.00 + 70.00 + 99.99 = −160.01; Q2 empty.
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].account, "assets");
        assert_eq!(report.rows[0].depth, 1);
        assert_eq!(report.rows[0].values, [usd_ma(-16_001), MixedAmount::new()]);
        assert_eq!(report.totals, [usd_ma(-16_001), MixedAmount::new()]);
    }

    #[test]
    fn honors_custom_is_cash_predicate() {
        let pred = |account: &str| account == "assets:broker:taxable:aapl";
        let report = cash_flow(
            &sample(),
            "2026-03-15",
            Interval::Monthly,
            3,
            4,
            Some(&pred),
        )
        .unwrap();
        assert_eq!(
            report
                .rows
                .iter()
                .map(|r| r.account.as_str())
                .collect::<Vec<_>>(),
            [
                "assets",
                "assets:broker",
                "assets:broker:taxable",
                "assets:broker:taxable:aapl",
            ]
        );
        let aapl = MixedAmount::single(Commodity("AAPL".into()), Dec::new(2, 0));
        let by_account = |name: &str| {
            report
                .rows
                .iter()
                .find(|r| r.account == name)
                .unwrap()
                .values
                .clone()
        };
        assert_eq!(
            by_account("assets:broker:taxable:aapl"),
            [MixedAmount::new(), aapl.clone(), MixedAmount::new()]
        );
        assert_eq!(
            report.totals,
            [MixedAmount::new(), aapl, MixedAmount::new()]
        );
    }
}
