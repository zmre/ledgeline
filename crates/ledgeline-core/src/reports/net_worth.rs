//! Net worth over time — port of `web/src/lib/reports/netWorth.ts`.
//!
//! One row per top-level asset/liability account (natural signs: liabilities
//! negative), one column per bucket; `totals[i]` = net worth at the end of
//! bucket `i`. Every commodity is valued to `value_in ?? prices.base_commodity()`
//! via the latest direct `P` directive ≤ the bucket end; unpriced commodities
//! are SKIPPED and reported in `meta.unpriced`.

use super::ReportError;
use super::accounts::{RootCategory, categorize};
use super::aggregate::{PostingFilter, account_totals, roll_up};
use super::mixed_amount::MixedAmount;
use super::periods::{Interval, bucket_end, compare_iso, last_n_buckets};
use super::prices::{PriceDb, ValuationMeta, value_at};
use super::types::{PeriodReport, PeriodRow, ReportMeta};
use crate::model::{Commodity, Transaction};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

struct BucketData {
    as_of: String,
    roots: BTreeMap<String, MixedAmount>,
}

/// Net worth per bucket, valued at market prices. `value_in` overrides the
/// default target (`prices.base_commodity()`); when there is no target at all
/// balances are reported unvalued.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or bad bucket math.
pub fn net_worth(
    txns: &[Transaction],
    prices: &PriceDb,
    end: &str,
    interval: Interval,
    count: usize,
    value_in: Option<Commodity>,
) -> Result<PeriodReport, ReportError> {
    let buckets = last_n_buckets(end, interval, count)?;
    let target: Option<Commodity> = value_in.or_else(|| prices.base_commodity().cloned());
    let mut meta = ValuationMeta::default();

    let mut per_bucket: Vec<BucketData> = Vec::with_capacity(buckets.len());
    for key in &buckets {
        let end_of_bucket = bucket_end(key)?;
        let as_of = if compare_iso(end, &end_of_bucket) == Ordering::Less {
            end.to_string()
        } else {
            end_of_bucket
        };
        let rolled = roll_up(&account_totals(
            txns,
            &PostingFilter {
                to: Some(&as_of),
                ..PostingFilter::default()
            },
        )?)?;
        let mut roots: BTreeMap<String, MixedAmount> = BTreeMap::new();
        for (account, ma) in &rolled {
            if account.contains(':') {
                continue;
            }
            if matches!(
                categorize(account),
                RootCategory::Asset | RootCategory::Liability
            ) {
                roots.insert(account.clone(), ma.clone());
            }
        }
        per_bucket.push(BucketData { as_of, roots });
    }

    let accounts: BTreeSet<String> = per_bucket
        .iter()
        .flat_map(|bucket| bucket.roots.keys().cloned())
        .collect();

    let mut rows: Vec<PeriodRow> = Vec::with_capacity(accounts.len());
    for account in &accounts {
        let mut values: Vec<MixedAmount> = Vec::with_capacity(per_bucket.len());
        for bucket in &per_bucket {
            let ma = bucket.roots.get(account).cloned().unwrap_or_default();
            let valued = match &target {
                None => ma,
                Some(t) => {
                    let v = value_at(&ma, t, prices, &bucket.as_of, Some(&mut meta))?;
                    if v.is_zero() {
                        MixedAmount::new()
                    } else {
                        MixedAmount::single(t.clone(), v)
                    }
                }
            };
            values.push(valued);
        }
        rows.push(PeriodRow {
            account: account.clone(),
            depth: 1,
            values,
        });
    }

    let mut totals: Vec<MixedAmount> = Vec::with_capacity(per_bucket.len());
    for i in 0..per_bucket.len() {
        let mut acc = MixedAmount::new();
        for row in &rows {
            acc = acc.ma_add(&row.values[i])?;
        }
        totals.push(acc);
    }

    let meta_out = if meta.unpriced.is_empty() {
        None
    } else {
        let mut unpriced = meta.unpriced;
        unpriced.sort();
        unpriced.dedup();
        Some(ReportMeta { unpriced })
    };

    Ok(PeriodReport {
        buckets,
        rows,
        totals,
        meta: meta_out,
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{amount, price, txn, usd};
    use super::*;
    use crate::decimal::Dec;

    fn c(s: &str) -> Commodity {
        Commodity(s.into())
    }

    fn dollars(mantissa: i128, places: u32) -> MixedAmount {
        MixedAmount::single(c("$"), Dec::new(mantissa, places))
    }

    fn prices() -> PriceDb {
        PriceDb::build(&[
            price("2026-01-31", "EUR", amount("$", 110, 2)),
            price("2026-02-28", "EUR", amount("$", 120, 2)),
        ])
    }

    fn sample() -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2026-01-10",
                vec![
                    ("assets:bank:checking", vec![usd(10_000)]),
                    ("equity:opening", vec![usd(-10_000)]),
                ],
            ),
            txn(
                2,
                "2026-01-20",
                vec![
                    ("assets:wise", vec![amount("EUR", 5000, 2)]), // 50.00 EUR
                    ("equity:opening", vec![usd(-5500)]),
                ],
            ),
            txn(
                3,
                "2026-02-15",
                vec![
                    ("liabilities:visa", vec![usd(-2000)]),
                    ("expenses:food", vec![usd(2000)]),
                ],
            ),
        ]
    }

    #[test]
    fn values_cumulative_balances_at_each_bucket_end() {
        let report = net_worth(
            &sample(),
            &prices(),
            "2026-02-28",
            Interval::Monthly,
            2,
            None,
        )
        .unwrap();
        assert_eq!(report.buckets, ["2026-01", "2026-02"]);
        assert_eq!(
            report
                .rows
                .iter()
                .map(|r| (r.account.as_str(), r.depth))
                .collect::<Vec<_>>(),
            [("assets", 1), ("liabilities", 1)]
        );
        // Jan 31: $100 + 50 EUR × $1.10 = $155; Feb 28: $100 + 50 EUR × $1.20 = $160.
        assert_eq!(
            report.rows[0].values,
            [dollars(1_550_000, 4), dollars(1_600_000, 4)]
        );
        // No liabilities until Feb; natural (negative) sign.
        assert_eq!(
            report.rows[1].values,
            [MixedAmount::new(), dollars(-2000, 2)]
        );
        assert_eq!(
            report.totals,
            [dollars(1_550_000, 4), dollars(1_400_000, 4)]
        );
        assert!(report.meta.is_none());
    }

    #[test]
    fn skips_unpriced_and_reports_meta() {
        let report = net_worth(
            &sample(),
            &prices(),
            "2026-01-25",
            Interval::Monthly,
            1,
            None,
        )
        .unwrap();
        // EUR held but skipped: first price is 01-31, after asOf 01-25.
        assert_eq!(report.rows[0].values, [dollars(10_000, 2)]);
        assert_eq!(
            report.meta,
            Some(ReportMeta {
                unpriced: vec![c("EUR")]
            })
        );
    }

    #[test]
    fn honors_explicit_value_in_target() {
        let report = net_worth(
            &sample(),
            &prices(),
            "2026-01-31",
            Interval::Monthly,
            1,
            Some(c("EUR")),
        )
        .unwrap();
        // $ has no price in EUR → skipped.
        assert_eq!(
            report.rows[0].values,
            [MixedAmount::single(c("EUR"), Dec::new(5000, 2))]
        );
        assert_eq!(
            report.meta,
            Some(ReportMeta {
                unpriced: vec![c("$")]
            })
        );
    }

    #[test]
    fn reports_raw_mixed_when_no_target() {
        let report = net_worth(
            &sample(),
            &PriceDb::build(&[]),
            "2026-02-28",
            Interval::Monthly,
            1,
            None,
        )
        .unwrap();
        let mut expected = MixedAmount::new();
        expected.accumulate(&c("$"), Dec::new(10_000, 2)).unwrap();
        expected.accumulate(&c("EUR"), Dec::new(5000, 2)).unwrap();
        assert_eq!(report.rows[0].values, [expected]);
        assert!(report.meta.is_none());
    }
}
