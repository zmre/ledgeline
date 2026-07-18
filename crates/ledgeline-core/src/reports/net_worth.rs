//! Net worth over time — port of `web/src/lib/reports/netWorth.ts`.
//!
//! One row per asset/liability account clamped to `depth` (natural signs:
//! liabilities negative), one column per bucket; `totals[i]` = net worth at the
//! end of bucket `i` (always the full depth-1 roots, so it is depth-independent).
//! Every commodity is valued to `value_in ?? prices.base_commodity()` via the
//! latest direct `P` directive ≤ the bucket end — where the price set is the
//! journal's explicit `P` directives PLUS the prices inferred from `@`/`@@` cost
//! annotations (matching hledger `--infer-market-prices`). Commodities still
//! without a direct price are SKIPPED and reported in `meta.unpriced`.

use super::ReportError;
use super::accounts::{RootCategory, categorize};
use super::aggregate::{PostingFilter, account_totals, at_depth, roll_up};
use super::mixed_amount::MixedAmount;
use super::periods::{Interval, bucket_end, compare_iso, last_n_buckets};
use super::prices::{PriceDb, ValuationMeta, infer_market_prices, value_at};
use super::types::{PeriodReport, PeriodRow, ReportMeta};
use crate::model::{Commodity, PriceDirective, Transaction};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

struct BucketData {
    as_of: String,
    /// Asset/liability accounts clamped to the report depth — the report rows.
    rows: BTreeMap<String, MixedAmount>,
    /// The depth-1 asset/liability roots — summed and valued for the total.
    roots: BTreeMap<String, MixedAmount>,
}

/// Value `ma` in `target` (identity when `None`), collapsing to a single-target
/// `MixedAmount` (empty when the result is zero); skips are recorded in `meta`.
fn valued(
    ma: &MixedAmount,
    target: Option<&Commodity>,
    prices: &PriceDb,
    as_of: &str,
    meta: &mut ValuationMeta,
) -> Result<MixedAmount, ReportError> {
    match target {
        None => Ok(ma.clone()),
        Some(t) => {
            let v = value_at(ma, t, prices, as_of, Some(meta))?;
            Ok(if v.is_zero() {
                MixedAmount::new()
            } else {
                MixedAmount::single(t.clone(), v)
            })
        }
    }
}

/// Net worth per bucket, valued at market prices, with asset/liability rows
/// clamped to `depth`. `value_in` overrides the default target
/// (`base_commodity()` of the combined explicit + inferred prices); when there
/// is no target at all balances are reported unvalued.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or bad bucket math.
pub fn net_worth(
    txns: &[Transaction],
    explicit_prices: &[PriceDirective],
    end: &str,
    interval: Interval,
    count: usize,
    depth: usize,
    value_in: Option<Commodity>,
) -> Result<PeriodReport, ReportError> {
    // Explicit `P` directives PLUS prices inferred from `@`/`@@` costs. Inferred
    // go first so an explicit price wins a same-date tie (hledger's precedence).
    let mut all_prices = infer_market_prices(txns)?;
    all_prices.extend_from_slice(explicit_prices);
    let prices = PriceDb::build(&all_prices);

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
        // Keep asset/liability accounts (by root category); rows are clamped to
        // `depth`, roots (depth 1) drive the depth-independent total.
        let asset_liability: BTreeMap<String, MixedAmount> = rolled
            .into_iter()
            .filter(|(account, _)| {
                matches!(
                    categorize(account),
                    RootCategory::Asset | RootCategory::Liability
                )
            })
            .collect();
        per_bucket.push(BucketData {
            as_of,
            rows: at_depth(&asset_liability, depth),
            roots: at_depth(&asset_liability, 1),
        });
    }

    let accounts: BTreeSet<String> = per_bucket
        .iter()
        .flat_map(|bucket| bucket.rows.keys().cloned())
        .collect();

    let mut rows: Vec<PeriodRow> = Vec::with_capacity(accounts.len());
    for account in &accounts {
        let mut values: Vec<MixedAmount> = Vec::with_capacity(per_bucket.len());
        for bucket in &per_bucket {
            let ma = bucket.rows.get(account).cloned().unwrap_or_default();
            values.push(valued(
                &ma,
                target.as_ref(),
                &prices,
                &bucket.as_of,
                &mut meta,
            )?);
        }
        rows.push(PeriodRow {
            account: account.clone(),
            depth: account.split(':').count(),
            values,
        });
    }

    let mut totals: Vec<MixedAmount> = Vec::with_capacity(per_bucket.len());
    for bucket in &per_bucket {
        let mut sum = MixedAmount::new();
        for ma in bucket.roots.values() {
            sum = sum.ma_add(ma)?;
        }
        totals.push(valued(
            &sum,
            target.as_ref(),
            &prices,
            &bucket.as_of,
            &mut meta,
        )?);
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

    fn prices() -> Vec<PriceDirective> {
        vec![
            price("2026-01-31", "EUR", amount("$", 110, 2)),
            price("2026-02-28", "EUR", amount("$", 120, 2)),
        ]
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
            1,
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
        let report =
            net_worth(&sample(), &[], "2026-02-28", Interval::Monthly, 1, 1, None).unwrap();
        let mut expected = MixedAmount::new();
        expected.accumulate(&c("$"), Dec::new(10_000, 2)).unwrap();
        expected.accumulate(&c("EUR"), Dec::new(5000, 2)).unwrap();
        assert_eq!(report.rows[0].values, [expected]);
        assert!(report.meta.is_none());
    }

    #[test]
    fn values_sub_accounts_at_depth() {
        // Depth 2 surfaces sub-accounts; the total stays the depth-1 net worth.
        let report = net_worth(
            &sample(),
            &prices(),
            "2026-02-28",
            Interval::Monthly,
            1,
            2,
            None,
        )
        .unwrap();
        assert_eq!(report.buckets, ["2026-02"]);
        assert_eq!(
            report
                .rows
                .iter()
                .map(|r| (r.account.as_str(), r.depth))
                .collect::<Vec<_>>(),
            [
                ("assets", 1),
                ("assets:bank", 2),
                ("assets:wise", 2),
                ("liabilities", 1),
                ("liabilities:visa", 2),
            ]
        );
        let by = |name: &str| {
            report
                .rows
                .iter()
                .find(|r| r.account == name)
                .unwrap()
                .values[0]
                .clone()
        };
        // Feb 28 (EUR $1.20): checking $100; wise 50 EUR → $60.
        assert_eq!(by("assets:bank"), dollars(10_000, 2));
        assert_eq!(by("assets:wise"), dollars(600_000, 4));
        assert_eq!(by("assets"), dollars(1_600_000, 4));
        assert_eq!(by("liabilities:visa"), dollars(-2000, 2));
        // Net worth: $160 − $20 = $140.
        assert_eq!(report.totals, [dollars(1_400_000, 4)]);
    }
}
