//! Cash flow — port of `web/src/lib/reports/cashFlow.ts`.
//!
//! Per-bucket changes (natural signs: inflow positive) in cash-like asset
//! accounts, for the last `count` buckets ending with the bucket containing
//! `end`. The final bucket is truncated at `end`.
//!
//! [`cash_flow_sources`] answers the companion question — not WHICH cash
//! account moved, but WHY: the same per-bucket cash movement, attributed to
//! the non-cash accounts on the other side of each transaction.

use super::ReportError;
use super::account_types::{AccountType, infer_account_type};
use super::aggregate::{at_depth, roll_up};
use super::mixed_amount::MixedAmount;
use super::periods::{Interval, bucket_span, last_n_buckets};
use super::types::{PeriodReport, PeriodRow};
use crate::decimal::{Dec, DecError, Remainder, allocate};
use crate::model::{Amount, Commodity, Posting, Transaction};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The source row for cash movement no counterparty can explain: a transfer
/// between two cash accounts whose legs land in different buckets, a currency
/// exchange between two cash accounts, or an unbalanced virtual posting. Named
/// rather than dropped so the sources still sum to the cash flow.
pub const UNATTRIBUTED: &str = "(unattributed)";

/// Name-based "cash-like asset" heuristic — the fallback when a journal declares
/// no account types. Delegates to hledger's Cash name inference.
#[must_use]
pub fn is_cash_like(account: &str) -> bool {
    infer_account_type(account) == Some(AccountType::Cash)
}

/// Each bucket's inclusive `[start, to]` range, with the last one truncated at
/// `end`. `last_n_buckets` yields CONTIGUOUS, non-overlapping buckets oldest →
/// newest, so the `to` bounds ascend strictly and every posting falls in at most
/// one of them — which is what lets [`bucket_of`] binary-search rather than
/// re-scan the journal once per bucket (PERF-5).
fn bucket_ranges(buckets: &[String], end: &str) -> Result<Vec<(String, String)>, ReportError> {
    buckets.iter().map(|key| bucket_span(key, end)).collect()
}

/// The bucket a posting dated `date` falls in, or `None` outside the span.
fn bucket_of(ranges: &[(String, String)], date: &str) -> Option<usize> {
    let index = ranges.partition_point(|(_, to)| to.as_str() < date);
    let (start, _) = ranges.get(index)?;
    (date >= start.as_str()).then_some(index)
}

/// Roll each bucket's direct per-account amounts up, clamp to `depth`, and
/// pivot into rows (the union of accounts across buckets, sorted). Also returns
/// each bucket's total, summed over the UNCLAMPED accounts so it does not move
/// with `depth` (RPT-4).
fn pivot(
    direct: &[BTreeMap<String, MixedAmount>],
    depth: usize,
) -> Result<(Vec<PeriodRow>, Vec<MixedAmount>), ReportError> {
    let mut totals: Vec<MixedAmount> = Vec::with_capacity(direct.len());
    let mut per_bucket: Vec<BTreeMap<String, MixedAmount>> = Vec::with_capacity(direct.len());
    for bucket in direct {
        let mut total = MixedAmount::new();
        for ma in bucket.values() {
            total = total.ma_add(ma)?;
        }
        totals.push(total);
        per_bucket.push(at_depth(&roll_up(bucket)?, depth));
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
                kind: None,
            }
        })
        .collect();
    Ok((rows, totals))
}

/// `account_totals` prunes zero commodities in one final sweep; so does this.
fn pruned(bucket: BTreeMap<&str, MixedAmount>) -> BTreeMap<String, MixedAmount> {
    bucket
        .into_iter()
        .map(|(account, mut ma)| {
            ma.drop_zeros();
            (account.to_string(), ma)
        })
        .collect()
}

/// Resolve the caller's cash predicate, defaulting to the name heuristic.
fn cash_test<'a>(
    is_cash: Option<&'a dyn Fn(&str) -> bool>,
    default: &'a dyn Fn(&str) -> bool,
) -> &'a dyn Fn(&str) -> bool {
    is_cash.unwrap_or(default)
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
    let is_cash = cash_test(is_cash, &is_cash_like);
    let buckets = last_n_buckets(end, interval, count)?;
    let ranges = bucket_ranges(&buckets, end)?;

    // ONE pass over every posting, summing per FULL account name into its own
    // bucket — i.e. exactly what `account_totals(from, to)` would have produced
    // for that bucket, and in the same transaction order, so no number can move.
    let mut direct: Vec<BTreeMap<&str, MixedAmount>> = vec![BTreeMap::new(); ranges.len()];
    let mut cash_like: HashMap<&str, bool> = HashMap::new();
    for txn in txns {
        for posting in &txn.postings {
            let date = posting.date.as_deref().unwrap_or(&txn.date);
            let Some(index) = bucket_of(&ranges, date) else {
                continue;
            };
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

    let direct: Vec<BTreeMap<String, MixedAmount>> = direct.into_iter().map(pruned).collect();
    let (rows, totals) = pivot(&direct, depth)?;
    Ok(PeriodReport {
        buckets,
        rows,
        totals,
        meta: None,
    })
}

/// Where each bucket's cash came from and went to: the cash flow of
/// [`cash_flow`], attributed to COUNTERPARTY accounts instead of cash accounts.
///
/// Same window, same cash predicate, same natural signs (money into cash is
/// positive, so an income source reads positive and a spending category
/// negative). Rows are the counterparty accounts rolled up and clamped to
/// `depth` exactly as the cash flow's own rows are; `kind` is `None`.
///
/// # The attribution
///
/// Per transaction, per bucket (a cash posting's own `date:` decides its
/// bucket, as in [`cash_flow`]), per commodity `k`:
///
/// 1. `D` is the sum of the transaction's CASH postings in `k` in that bucket.
///    Zero — a transfer between two cash accounts, say — attributes nothing:
///    cash moving between cash accounts is internal and is not a source.
/// 2. Every NON-CASH posting amount gets a weight in `k`: its own quantity if
///    it is written in `k`, else its cost (`@`/`@@`) if that is in `k` (buying
///    stock with cash: the counterparty is in `AAPL`, the cash in `$`). So a
///    buy with a commission weighs the shares at cost AND the fee, together.
/// 3. Each weighted counterparty receives `D × w / Σw`. In the ordinary
///    balanced case (`Σw == −D`) that is exactly `−w`, with no division — a
///    `$100` grocery bill is `−$100` of groceries. Otherwise the shares are
///    [`allocate`]d: truncated, with the largest weight taking the remainder,
///    so they still sum to `D` EXACTLY.
/// 4. No usable weights (a currency exchange between two cash accounts, or
///    transfer legs split across buckets) → the whole `D` goes to
///    [`UNATTRIBUTED`].
///
/// So per bucket, `Σ sources == cash_flow(...).totals` exactly: every unit of
/// cash movement is attributed once, and only once.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or bad bucket math.
pub fn cash_flow_sources(
    txns: &[Transaction],
    end: &str,
    interval: Interval,
    count: usize,
    depth: usize,
    is_cash: Option<&dyn Fn(&str) -> bool>,
) -> Result<PeriodReport, ReportError> {
    let is_cash = cash_test(is_cash, &is_cash_like);
    let buckets = last_n_buckets(end, interval, count)?;
    let ranges = bucket_ranges(&buckets, end)?;

    let mut direct: Vec<BTreeMap<&str, MixedAmount>> = vec![BTreeMap::new(); ranges.len()];
    let mut cash_like: HashMap<&str, bool> = HashMap::new();
    for txn in txns {
        let mut deltas: BTreeMap<(usize, &Commodity), Dec> = BTreeMap::new();
        let mut counterparties: Vec<&Posting> = Vec::new();
        for posting in &txn.postings {
            let account = posting.account.0.as_str();
            if !*cash_like.entry(account).or_insert_with(|| is_cash(account)) {
                counterparties.push(posting);
                continue;
            }
            let date = posting.date.as_deref().unwrap_or(&txn.date);
            let Some(index) = bucket_of(&ranges, date) else {
                continue;
            };
            for amount in &posting.amounts {
                let delta = deltas
                    .entry((index, &amount.commodity))
                    .or_insert(Dec::zero());
                *delta = delta.add(amount.quantity)?;
            }
        }
        for ((index, commodity), delta) in deltas {
            if delta.is_zero() {
                continue;
            }
            for (account, share) in attribute(delta, commodity, &counterparties)? {
                direct[index]
                    .entry(account)
                    .or_default()
                    .accumulate(commodity, share)?;
            }
        }
    }

    let direct: Vec<BTreeMap<String, MixedAmount>> = direct.into_iter().map(pruned).collect();
    let (rows, totals) = pivot(&direct, depth)?;
    Ok(PeriodReport {
        buckets,
        rows,
        totals,
        meta: None,
    })
}

/// One amount's weight in `commodity`: its own quantity when it is written in
/// `commodity`, else its cost when that is in `commodity`, else none.
fn weight_in(commodity: &Commodity, amount: &Amount) -> Result<Option<Dec>, DecError> {
    if &amount.commodity == commodity {
        return Ok(Some(amount.quantity));
    }
    let (held, quantity) = amount.at_cost()?;
    Ok((held == commodity).then_some(quantity))
}

/// The counterparties' weights in `commodity` (see [`weight_in`]). Zero
/// weights are dropped: they could only ever receive a zero share.
fn weights<'a>(
    commodity: &Commodity,
    counterparties: &[&'a Posting],
) -> Result<Vec<(&'a str, Dec)>, DecError> {
    let mut out = Vec::new();
    for posting in counterparties {
        for amount in &posting.amounts {
            if let Some(quantity) = weight_in(commodity, amount)?
                && !quantity.is_zero()
            {
                out.push((posting.account.0.as_str(), quantity));
            }
        }
    }
    Ok(out)
}

fn sum(weights: &[(&str, Dec)]) -> Result<Dec, DecError> {
    weights
        .iter()
        .try_fold(Dec::zero(), |acc, (_, weight)| acc.add(*weight))
}

/// Split one transaction's cash movement `delta` in `commodity` across its
/// counterparties — see [`cash_flow_sources`] for the rule.
fn attribute<'a>(
    delta: Dec,
    commodity: &Commodity,
    counterparties: &[&'a Posting],
) -> Result<Vec<(&'a str, Dec)>, DecError> {
    let chosen = weights(commodity, counterparties)?;
    let total = sum(&chosen)?;
    if total.is_zero() {
        return Ok(vec![(UNATTRIBUTED, delta)]);
    }
    // The ordinary balanced transaction: each counterparty's share is exactly
    // the negation of what it was posted.
    if delta == total.neg()? {
        return chosen
            .into_iter()
            .map(|(account, weight)| Ok((account, weight.neg()?)))
            .collect();
    }
    let (accounts, weights): (Vec<&str>, Vec<Dec>) = chosen.into_iter().unzip();
    Ok(accounts
        .into_iter()
        .zip(allocate(delta, &weights, Remainder::Largest)?)
        .collect())
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

    // -----------------------------------------------------------------------
    // cash_flow_sources
    // -----------------------------------------------------------------------

    fn row<'a>(report: &'a PeriodReport, account: &str) -> &'a [MixedAmount] {
        &report
            .rows
            .iter()
            .find(|r| r.account == account)
            .unwrap_or_else(|| panic!("no row {account}"))
            .values
    }

    fn accounts(report: &PeriodReport) -> Vec<&str> {
        report.rows.iter().map(|r| r.account.as_str()).collect()
    }

    /// The contract: per bucket, the sources sum to the cash flow's own total.
    fn assert_reconciles(txns: &[Transaction], end: &str, interval: Interval, count: usize) {
        for depth in 0..=5 {
            let flow = cash_flow(txns, end, interval, count, depth, None).unwrap();
            let sources = cash_flow_sources(txns, end, interval, count, depth, None).unwrap();
            assert_eq!(sources.buckets, flow.buckets);
            assert_eq!(sources.totals, flow.totals, "depth {depth}");
        }
    }

    #[test]
    fn attributes_cash_to_counterparties_and_skips_internal_transfers() {
        let report =
            cash_flow_sources(&sample(), "2026-03-15", Interval::Monthly, 3, 4, None).unwrap();
        // The savings transfer is cash↔cash and so is nobody's source; the
        // stock bought with broker cash has no `$` counterparty and no cost, so
        // its $400 is unattributed rather than dropped.
        assert_eq!(
            accounts(&report),
            [
                UNATTRIBUTED,
                "expenses",
                "expenses:food",
                "income",
                "income:salary"
            ]
        );
        assert_eq!(
            row(&report, "income:salary"),
            [usd_ma(10_000), MixedAmount::new(), usd_ma(7000)]
        );
        assert_eq!(
            row(&report, "expenses:food"),
            [MixedAmount::new(), usd_ma(-3000), MixedAmount::new()]
        );
        assert_eq!(
            row(&report, UNATTRIBUTED),
            [MixedAmount::new(), usd_ma(-40_000), MixedAmount::new()]
        );
        assert!(report.rows.iter().all(|r| r.kind.is_none()));
        assert_reconciles(&sample(), "2026-03-15", Interval::Monthly, 3);
    }

    #[test]
    fn splits_a_multi_leg_paycheck_exactly() {
        let txns = vec![txn(
            1,
            "2026-01-31",
            vec![
                ("assets:bank:checking", vec![usd(300_000)]),
                ("assets:bank:savings", vec![usd(50_000)]),
                ("income:salary", vec![usd(-400_000)]),
                ("expenses:taxes", vec![usd(50_000)]),
            ],
        )];
        let report = cash_flow_sources(&txns, "2026-01-31", Interval::Monthly, 1, 2, None).unwrap();
        assert_eq!(row(&report, "income:salary"), [usd_ma(400_000)]);
        assert_eq!(row(&report, "expenses:taxes"), [usd_ma(-50_000)]);
        assert_eq!(report.totals, [usd_ma(350_000)]);
        assert_reconciles(&txns, "2026-01-31", Interval::Monthly, 1);
    }

    #[test]
    fn falls_back_to_cost_when_the_counterparty_is_another_commodity() {
        let mut shares = amount("AAPL", 10, 0);
        shares.cost = Some(Box::new(crate::model::Cost {
            kind: crate::model::CostKind::Unit,
            amount: usd(22_000),
        }));
        let txns = vec![txn(
            1,
            "2026-01-15",
            vec![
                ("assets:broker:aapl", vec![shares]),
                ("assets:broker:cash", vec![usd(-220_000)]),
            ],
        )];
        let report = cash_flow_sources(&txns, "2026-01-31", Interval::Monthly, 1, 3, None).unwrap();
        assert_eq!(row(&report, "assets:broker:aapl"), [usd_ma(-220_000)]);
        assert_reconciles(&txns, "2026-01-31", Interval::Monthly, 1);
    }

    fn shares_at_unit_cost(quantity: i128, unit_cents: i128) -> Amount {
        let mut shares = amount("AAPL", quantity, 0);
        shares.cost = Some(Box::new(crate::model::Cost {
            kind: crate::model::CostKind::Unit,
            amount: usd(unit_cents),
        }));
        shares
    }

    #[test]
    fn a_buy_with_a_commission_splits_between_the_stock_and_the_fee() {
        // checking −$1005 → 10 AAPL @ $100 plus a $5 commission.
        let txns = vec![txn(
            1,
            "2026-01-15",
            vec![
                ("assets:bank:checking", vec![usd(-100_500)]),
                ("assets:broker:aapl", vec![shares_at_unit_cost(10, 10_000)]),
                ("expenses:fees", vec![usd(500)]),
            ],
        )];
        let report = cash_flow_sources(&txns, "2026-01-31", Interval::Monthly, 1, 3, None).unwrap();
        assert_eq!(row(&report, "assets:broker:aapl"), [usd_ma(-100_000)]);
        assert_eq!(row(&report, "expenses:fees"), [usd_ma(-500)]);
        assert!(!accounts(&report).contains(&UNATTRIBUTED));
        assert_reconciles(&txns, "2026-01-31", Interval::Monthly, 1);
    }

    #[test]
    fn a_sell_with_a_commission_splits_between_the_stock_and_the_fee() {
        // 10 AAPL sold @ $150 = $1500, less a $5 commission → checking +$1495.
        let txns = vec![txn(
            1,
            "2026-01-15",
            vec![
                ("assets:broker:aapl", vec![shares_at_unit_cost(-10, 15_000)]),
                ("expenses:fees", vec![usd(500)]),
                ("assets:bank:checking", vec![usd(149_500)]),
            ],
        )];
        let report = cash_flow_sources(&txns, "2026-01-31", Interval::Monthly, 1, 3, None).unwrap();
        assert_eq!(row(&report, "assets:broker:aapl"), [usd_ma(150_000)]);
        assert_eq!(row(&report, "expenses:fees"), [usd_ma(-500)]);
        assert!(!accounts(&report).contains(&UNATTRIBUTED));
        assert_reconciles(&txns, "2026-01-31", Interval::Monthly, 1);
    }

    #[test]
    fn an_amount_in_the_cash_commodity_weighs_as_written_even_with_a_foreign_cost() {
        // A `$` counterparty priced in EUR still weighs its own `$`, not its cost.
        let mut dollars = usd(10_000);
        dollars.cost = Some(Box::new(crate::model::Cost {
            kind: crate::model::CostKind::Total,
            amount: amount("EUR", 9000, 2),
        }));
        let txns = vec![txn(
            1,
            "2026-01-15",
            vec![
                ("assets:bank:checking", vec![usd(-10_000)]),
                ("expenses:travel", vec![dollars]),
            ],
        )];
        let report = cash_flow_sources(&txns, "2026-01-31", Interval::Monthly, 1, 2, None).unwrap();
        assert_eq!(row(&report, "expenses:travel"), [usd_ma(-10_000)]);
        assert_reconciles(&txns, "2026-01-31", Interval::Monthly, 1);
    }

    #[test]
    fn prorates_when_cash_legs_straddle_buckets() {
        // One bill paid in two instalments dated into consecutive months.
        let mut bill = txn(
            1,
            "2026-01-20",
            vec![
                ("assets:bank:checking", vec![usd(-10_000)]),
                ("assets:bank:checking", vec![usd(-5000)]),
                ("expenses:a", vec![usd(9000)]),
                ("expenses:b", vec![usd(6000)]),
            ],
        );
        bill.postings[1].date = Some("2026-02-05".into());
        let txns = vec![bill];
        let report = cash_flow_sources(&txns, "2026-02-28", Interval::Monthly, 2, 2, None).unwrap();
        // Jan: −100 over weights 90/60 → −60/−40; Feb: −50 → −30/−20.
        assert_eq!(row(&report, "expenses:a"), [usd_ma(-6000), usd_ma(-3000)]);
        assert_eq!(row(&report, "expenses:b"), [usd_ma(-4000), usd_ma(-2000)]);
        assert_reconciles(&txns, "2026-02-28", Interval::Monthly, 2);
    }

    #[test]
    fn a_rounded_split_still_sums_to_the_cash_exactly() {
        // Unbalanced on purpose (virtual-posting style): $100 over three equal
        // weights cannot divide evenly; the remainder lands on one of them.
        let txns = vec![txn(
            1,
            "2026-01-10",
            vec![
                ("assets:bank:checking", vec![usd(-10_000)]),
                ("expenses:a", vec![usd(100)]),
                ("expenses:b", vec![usd(100)]),
                ("expenses:c", vec![usd(100)]),
            ],
        )];
        let report = cash_flow_sources(&txns, "2026-01-31", Interval::Monthly, 1, 2, None).unwrap();
        let shares: Vec<MixedAmount> = ["expenses:a", "expenses:b", "expenses:c"]
            .iter()
            .map(|a| row(&report, a)[0].clone())
            .collect();
        assert_eq!(shares, [usd_ma(-3334), usd_ma(-3333), usd_ma(-3333)]);
        assert_reconciles(&txns, "2026-01-31", Interval::Monthly, 1);
    }

    #[test]
    fn a_currency_exchange_between_cash_accounts_is_unattributed() {
        let mut euros = amount("EUR", 10_000, 2);
        euros.cost = Some(Box::new(crate::model::Cost {
            kind: crate::model::CostKind::Total,
            amount: usd(11_000),
        }));
        let txns = vec![txn(
            1,
            "2026-01-10",
            vec![
                ("assets:bank:wise:eur", vec![euros]),
                ("assets:bank:checking", vec![usd(-11_000)]),
            ],
        )];
        let report = cash_flow_sources(&txns, "2026-01-31", Interval::Monthly, 1, 2, None).unwrap();
        assert_eq!(accounts(&report), [UNATTRIBUTED]);
        assert_eq!(
            row(&report, UNATTRIBUTED),
            [mixed(&[("$", -11_000, 2), ("EUR", 10_000, 2)])]
        );
        assert_reconciles(&txns, "2026-01-31", Interval::Monthly, 1);
    }

    #[test]
    fn honours_the_declared_cash_predicate() {
        // With checking NOT cash, the salary deposit is no cash movement at all.
        let pred = |account: &str| account == "assets:bank:savings";
        let report = cash_flow_sources(
            &sample(),
            "2026-03-15",
            Interval::Monthly,
            3,
            3,
            Some(&pred),
        )
        .unwrap();
        // Only the savings leg of the transfer is cash now, so checking is its source.
        assert_eq!(
            accounts(&report),
            ["assets", "assets:bank", "assets:bank:checking"]
        );
        assert_eq!(
            row(&report, "assets:bank:checking"),
            [MixedAmount::new(), usd_ma(5000), MixedAmount::new()]
        );
    }
}
