//! Every account, per period — hledger's `balance -M` (and `-H`, `-E`).
//!
//! The plain multi-period balance report: one row per account (a tree, rolled
//! up and clamped to `depth`), one column per `interval` period over
//! `[from, to]`, amounts as written and in natural signs (revenue negative).
//! No valuation, no sections, no sign flips: it is the raw ledger the official
//! statements are built from, which is the point of reading it beside them.
//!
//! Two readings of the same columns ([`BalanceMode`]):
//!
//! - **Change** — what moved in each period (`hledger balance -M`). Each row
//!   carries a Total, the sum of its periods.
//! - **Historical** — the balance at the end of each period, everything before
//!   `from` included (`hledger balance -M -H`). Snapshots mean nothing summed,
//!   so there is no Total.
//!
//! Rows whose every column is zero are hidden unless `known` is given, in which
//! case every account it names is a row too (hledger's `-E --declared`): the
//! way to see what a projection never touches. A parent whose own figure is
//! zero but which has a non-zero descendant is always kept, so the tree never
//! loses a level.
//!
//! The per-period pass is the cash flow report's own ([`bucket_totals`]) with
//! every account admitted, and the pivot is its [`pivot`], so the two cannot
//! disagree about which posting lands in which column.

use std::collections::{BTreeMap, BTreeSet};

use super::ReportError;
use super::aggregate::{PostingFilter, account_totals};
use super::cash_flow::{bucket_totals, pivot, sum};
use super::mixed_amount::MixedAmount;
use super::periods::{Interval, add_days, range_buckets};
use super::types::{PeriodReport, PeriodRow};
use crate::model::Transaction;

/// What a column of [`account_periods`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalanceMode {
    /// The movement within the period (hledger's default).
    Change,
    /// The balance at the period's end (hledger `-H`, `--historical`).
    Historical,
}

/// Options for [`account_periods`].
#[derive(Debug, Clone, Copy)]
pub struct AccountPeriodsOpts<'a> {
    /// Inclusive start of the first column.
    pub from: &'a str,
    /// Inclusive end of the last column.
    pub to: &'a str,
    pub interval: Interval,
    /// Roll-up depth. Never clamps the per-column totals.
    pub depth: usize,
    pub mode: BalanceMode,
    /// Accounts shown even when every column is zero, or `None` to hide every
    /// all-zero row.
    pub known: Option<&'a [String]>,
}

/// Every account's change or balance per period over `txns`.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or bad bucket math.
pub fn account_periods(
    txns: &[Transaction],
    opts: &AccountPeriodsOpts<'_>,
) -> Result<PeriodReport, ReportError> {
    let ranges: Vec<(String, String)> = range_buckets(opts.from, opts.to, Some(opts.interval))?
        .into_iter()
        .map(|span| (span.from, span.to))
        .collect();
    let direct = bucket_totals(txns, &ranges, &|_| true)?;
    account_periods_from(direct, || before(txns, opts.from), opts)
}

/// Every account's balance strictly before `from`: what Balance mode opens
/// on. A projection's real side keeps it (plan 29).
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub(crate) fn before(
    txns: &[Transaction],
    from: &str,
) -> Result<BTreeMap<String, MixedAmount>, ReportError> {
    let before = add_days(from, -1);
    Ok(account_totals(
        txns,
        &PostingFilter {
            to: Some(&before),
            ..PostingFilter::default()
        },
    )?)
}

/// [`account_periods`] from each column's direct totals (`direct[i]` is the
/// cash flow pass's [`bucket_totals`] for the `i`-th [`range_buckets`] span of
/// `[opts.from, opts.to]`) and, asked for only in Balance mode, the balances
/// before `opts.from` ([`before`]). Everything after the posting pass is this
/// one function, so a seeded caller cannot present the columns differently.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or bad bucket math.
pub(crate) fn account_periods_from(
    direct: Vec<BTreeMap<String, MixedAmount>>,
    opening: impl FnOnce() -> Result<BTreeMap<String, MixedAmount>, ReportError>,
    opts: &AccountPeriodsOpts<'_>,
) -> Result<PeriodReport, ReportError> {
    let buckets: Vec<String> = range_buckets(opts.from, opts.to, Some(opts.interval))?
        .into_iter()
        .map(|span| span.key)
        .collect();
    let columns = match opts.mode {
        BalanceMode::Change => direct,
        BalanceMode::Historical => cumulative(opening()?, direct)?,
    };
    let (rows, totals) = pivot(&columns, opts.depth)?;
    let rows = shown(rows, opts.known, opts.depth, buckets.len());
    let (rows, grand_total) = match opts.mode {
        BalanceMode::Change => (rows, Some(sum(&totals)?)),
        BalanceMode::Historical => (
            rows.into_iter()
                .map(|row| PeriodRow { total: None, ..row })
                .collect(),
            None,
        ),
    };
    Ok(PeriodReport {
        buckets,
        rows,
        totals,
        grand_total,
        meta: None,
    })
}

/// Each column's closing balance: everything before `from` (`running`), plus
/// every column's movement up to and including it.
pub(super) fn cumulative(
    mut running: BTreeMap<String, MixedAmount>,
    direct: Vec<BTreeMap<String, MixedAmount>>,
) -> Result<Vec<BTreeMap<String, MixedAmount>>, ReportError> {
    direct
        .into_iter()
        .map(|column| {
            for (account, ma) in column {
                running.entry(account).or_default().ma_add_assign(&ma)?;
            }
            Ok(running
                .iter()
                .map(|(account, ma)| {
                    let mut ma = ma.clone();
                    ma.drop_zeros();
                    (account.clone(), ma)
                })
                .collect())
        })
        .collect()
}

/// The rows a reader sees: those with a non-zero figure somewhere in their
/// subtree, plus — when `known` is given — every known account, clamped to
/// `depth` with its ancestors, as an all-zero row where nothing else put one.
pub(super) fn shown(
    rows: Vec<PeriodRow>,
    known: Option<&[String]>,
    depth: usize,
    columns: usize,
) -> Vec<PeriodRow> {
    let Some(known) = known else {
        let live: BTreeSet<String> = rows
            .iter()
            .filter(|row| row.values.iter().any(|ma| !ma.is_zero()))
            .flat_map(|row| ancestors(&row.account, depth))
            .collect();
        return rows
            .into_iter()
            .filter(|row| live.contains(&row.account))
            .collect();
    };
    let mut by_account: BTreeMap<String, PeriodRow> = rows
        .into_iter()
        .map(|row| (row.account.clone(), row))
        .collect();
    for account in known.iter().flat_map(|account| ancestors(account, depth)) {
        by_account
            .entry(account.clone())
            .or_insert_with(|| PeriodRow {
                depth: account.split(':').count(),
                account,
                values: vec![MixedAmount::new(); columns],
                kind: None,
                total: Some(MixedAmount::new()),
            });
    }
    by_account.into_values().collect()
}

/// `account` and each of its ancestors, at most `depth` segments deep:
/// `("a:b:c", 2)` → `a`, `a:b`.
fn ancestors(account: &str, depth: usize) -> Vec<String> {
    let segments: Vec<&str> = account.split(':').collect();
    (1..=segments.len().min(depth))
        .map(|n| segments[..n].join(":"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{mixed, txn, usd};
    use super::*;

    fn usd_ma(cents: i128) -> MixedAmount {
        mixed(&[("$", cents, 2)])
    }

    /// Salary in January and March, food in February: three monthly columns.
    fn sample() -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2025-12-15",
                vec![
                    ("assets:bank:checking", vec![usd(50_000)]),
                    ("equity:opening", vec![usd(-50_000)]),
                ],
            ),
            txn(
                2,
                "2026-01-10",
                vec![
                    ("assets:bank:checking", vec![usd(10_000)]),
                    ("income:salary", vec![usd(-10_000)]),
                ],
            ),
            txn(
                3,
                "2026-02-05",
                vec![
                    ("expenses:food", vec![usd(3000)]),
                    ("assets:bank:checking", vec![usd(-3000)]),
                ],
            ),
            txn(
                4,
                "2026-03-10",
                vec![
                    ("assets:bank:checking", vec![usd(10_000)]),
                    ("income:salary", vec![usd(-10_000)]),
                ],
            ),
        ]
    }

    fn opts(mode: BalanceMode, known: Option<&[String]>) -> AccountPeriodsOpts<'_> {
        AccountPeriodsOpts {
            from: "2026-01-01",
            to: "2026-03-31",
            interval: Interval::Monthly,
            depth: 3,
            mode,
            known,
        }
    }

    fn row<'r>(report: &'r PeriodReport, account: &str) -> &'r PeriodRow {
        report
            .rows
            .iter()
            .find(|row| row.account == account)
            .unwrap_or_else(|| panic!("no row {account}"))
    }

    #[test]
    fn change_is_each_periods_movement_with_a_total() {
        let report = account_periods(&sample(), &opts(BalanceMode::Change, None)).expect("report");
        assert_eq!(report.buckets, ["2026-01", "2026-02", "2026-03"]);
        let checking = row(&report, "assets:bank:checking");
        assert_eq!(
            checking.values,
            [usd_ma(10_000), usd_ma(-3000), usd_ma(10_000)]
        );
        assert_eq!(checking.total, Some(usd_ma(17_000)));
        // The roll-up: `assets` carries its subtree.
        assert_eq!(row(&report, "assets").values, checking.values);
        // Before the window: not a column, and not a row.
        assert!(
            report
                .rows
                .iter()
                .all(|row| row.account != "equity:opening")
        );
        assert!(report.grand_total.is_some());
        // Balanced transactions: every column nets to zero.
        assert!(report.totals.iter().all(MixedAmount::is_zero));
    }

    #[test]
    fn historical_is_the_closing_balance_including_everything_before() {
        let report =
            account_periods(&sample(), &opts(BalanceMode::Historical, None)).expect("report");
        let checking = row(&report, "assets:bank:checking");
        assert_eq!(
            checking.values,
            [usd_ma(60_000), usd_ma(57_000), usd_ma(67_000)]
        );
        assert_eq!(checking.total, None);
        assert_eq!(report.grand_total, None);
        // Opening equity has no movement in the window but a balance in it.
        assert_eq!(
            row(&report, "equity:opening").values,
            vec![usd_ma(-50_000); 3]
        );
        assert_eq!(
            row(&report, "expenses:food").values,
            [MixedAmount::new(), usd_ma(3000), usd_ma(3000)]
        );
    }

    #[test]
    fn depth_clamps_rows_and_never_the_totals() {
        let shallow = AccountPeriodsOpts {
            depth: 1,
            ..opts(BalanceMode::Change, None)
        };
        let report = account_periods(&sample(), &shallow).expect("report");
        let accounts: Vec<&str> = report.rows.iter().map(|row| row.account.as_str()).collect();
        assert_eq!(accounts, ["assets", "expenses", "income"]);
        let full = account_periods(&sample(), &opts(BalanceMode::Change, None)).expect("report");
        assert_eq!(report.totals, full.totals);
    }

    #[test]
    fn empty_accounts_are_hidden_unless_known() {
        let known = vec![
            "assets:bank:savings".to_string(),
            "expenses:travel:air".to_string(),
        ];
        let hidden = account_periods(&sample(), &opts(BalanceMode::Change, None)).expect("report");
        assert!(
            hidden
                .rows
                .iter()
                .all(|row| row.account != "assets:bank:savings")
        );

        let shown =
            account_periods(&sample(), &opts(BalanceMode::Change, Some(&known))).expect("report");
        let savings = row(&shown, "assets:bank:savings");
        assert_eq!(savings.values, vec![MixedAmount::new(); 3]);
        assert_eq!(savings.depth, 3);
        // Its ancestors come with it, and the list stays sorted as a tree.
        row(&shown, "expenses:travel");
        let accounts: Vec<&str> = shown.rows.iter().map(|row| row.account.as_str()).collect();
        let mut sorted = accounts.clone();
        sorted.sort_unstable();
        assert_eq!(accounts, sorted);
        // Known accounts are clamped to the depth like any other row.
        let shallow = AccountPeriodsOpts {
            depth: 2,
            ..opts(BalanceMode::Change, Some(&known))
        };
        let clamped = account_periods(&sample(), &shallow).expect("report");
        assert!(clamped.rows.iter().all(|row| row.depth <= 2));
        row(&clamped, "expenses:travel");
    }

    #[test]
    fn a_zero_parent_with_a_live_child_keeps_its_place() {
        let txns = vec![txn(
            1,
            "2026-01-10",
            vec![
                ("assets:bank:checking", vec![usd(500)]),
                ("assets:bank:savings", vec![usd(-500)]),
            ],
        )];
        let report = account_periods(&txns, &opts(BalanceMode::Change, None)).expect("report");
        let accounts: Vec<&str> = report.rows.iter().map(|row| row.account.as_str()).collect();
        assert_eq!(
            accounts,
            [
                "assets",
                "assets:bank",
                "assets:bank:checking",
                "assets:bank:savings"
            ]
        );
        assert!(row(&report, "assets").values[0].is_zero());
    }

    #[test]
    fn an_account_that_nets_to_zero_is_empty() {
        let txns = vec![
            txn(
                1,
                "2026-01-10",
                vec![
                    ("assets:bank:checking", vec![usd(500)]),
                    ("income:refund", vec![usd(-500)]),
                ],
            ),
            txn(
                2,
                "2026-01-20",
                vec![
                    ("assets:bank:checking", vec![usd(-500)]),
                    ("income:refund", vec![usd(500)]),
                ],
            ),
        ];
        let report = account_periods(&txns, &opts(BalanceMode::Change, None)).expect("report");
        assert!(report.rows.is_empty());
    }
}
