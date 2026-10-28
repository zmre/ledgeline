//! Every account per period, each section read the way that kind of account
//! is read: the Accounts tab on Reports and on Projections (2026-10-28).
//!
//! | Section | Each column | The last column |
//! | --- | --- | --- |
//! | Assets, Liabilities | the BALANCE at the period's end (`hledger balance -H`) | **Δ**: the change over the whole window, closing at the end minus opening at the start |
//! | Equity, Revenue, Expenses, Other | the NET CHANGE within the period (`hledger balance`) | **Total**: the sum of the periods |
//!
//! A balance is the useful reading of an account that holds money; what moved
//! in a month is the useful reading of one that only measures a flow. Summing
//! balances means nothing, so their last column is the window's Δ instead —
//! which is also exactly the sum of their per-period changes, so the two last
//! columns are one computation.
//!
//! Sections come from the account's effective type ([`AccountTypes`]:
//! declared, inherited from an ancestor, then inferred from the name), never
//! from a name test here. Cash is an asset and conversion is equity, as on the
//! balance sheet; gains are revenue. An account no type reaches is "Other",
//! read as a flow. Each section is rolled up and clamped on its own, so a
//! parent never folds two kinds of figure into one row.
//!
//! Amounts are as written, in natural signs (revenue negative), with no
//! valuation — the raw ledger, as `hledger balance` shows it. Each column's
//! figures come from [`super::account_periods`]' own pass ([`pivot`],
//! [`cumulative`], [`shown`]), so this report and that one cannot disagree.

use std::collections::BTreeMap;

use super::ReportError;
use super::account_periods::{before, cumulative, shown};
use super::account_types::{AccountType, AccountTypes};
use super::cash_flow::{bucket_totals, pivot, sum};
use super::mixed_amount::MixedAmount;
use super::periods::{Interval, range_buckets};
use crate::model::Transaction;

/// Which kind of account a section holds, in the order sections are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AccountSection {
    /// Assets, cash included.
    Assets,
    /// Liabilities.
    Liabilities,
    /// Equity, conversion included.
    Equity,
    /// Revenue, gains included.
    Revenue,
    /// Expenses.
    Expenses,
    /// Anything no type reaches.
    Other,
}

impl AccountSection {
    /// Every section, in display order.
    pub const ALL: [Self; 6] = [
        Self::Assets,
        Self::Liabilities,
        Self::Equity,
        Self::Revenue,
        Self::Expenses,
        Self::Other,
    ];

    /// The section `account` belongs to.
    #[must_use]
    pub fn of(account: &str, types: &AccountTypes) -> Self {
        [
            (AccountType::Asset, Self::Assets),
            (AccountType::Liability, Self::Liabilities),
            (AccountType::Equity, Self::Equity),
            (AccountType::Revenue, Self::Revenue),
            (AccountType::Expense, Self::Expenses),
        ]
        .into_iter()
        .find(|(kind, _)| types.is_type(account, *kind))
        .map_or(Self::Other, |(_, section)| section)
    }

    /// How this section's columns read.
    #[must_use]
    pub fn reading(self) -> Reading {
        match self {
            Self::Assets | Self::Liabilities => Reading::Balance,
            Self::Equity | Self::Revenue | Self::Expenses | Self::Other => Reading::Change,
        }
    }

    /// The wire and display code.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Assets => "assets",
            Self::Liabilities => "liabilities",
            Self::Equity => "equity",
            Self::Revenue => "revenue",
            Self::Expenses => "expenses",
            Self::Other => "other",
        }
    }
}

/// What a section's columns hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// The balance at each period's end; the last column is the window's Δ.
    Balance,
    /// The net change within each period; the last column is their Total.
    Change,
}

/// One account's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountReportRow {
    /// Full account name.
    pub account: String,
    /// Number of `:`-separated segments in `account`.
    pub depth: usize,
    /// One figure per period, oldest → newest, read as the section reads.
    pub values: Vec<MixedAmount>,
    /// The last column: the Δ over the window for a balance, the Total of the
    /// periods for a change. Either way, the sum of the row's per-period
    /// changes.
    pub summary: MixedAmount,
}

/// One section: its rows and its total row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountReportSection {
    pub section: AccountSection,
    pub reading: Reading,
    pub rows: Vec<AccountReportRow>,
    /// The section's figure per period, summed over its UNCLAMPED accounts, so
    /// it never moves with `depth`.
    pub totals: Vec<MixedAmount>,
    /// The total row's last column.
    pub summary: MixedAmount,
}

/// The report: one column per period, the sections that have any account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountReport {
    /// Bucket keys, oldest → newest.
    pub buckets: Vec<String>,
    pub sections: Vec<AccountReportSection>,
}

/// Options for [`account_report`].
#[derive(Debug, Clone, Copy)]
pub struct AccountReportOpts<'a> {
    /// Inclusive start of the first column.
    pub from: &'a str,
    /// Inclusive end of the last column.
    pub to: &'a str,
    /// The column grain; `None` is one column over the whole range.
    pub interval: Option<Interval>,
    /// Roll-up depth. Never clamps a section's totals.
    pub depth: usize,
    /// Accounts shown even when every column is zero (hledger's `-E
    /// --declared`), or `None` to hide every all-zero row.
    pub known: Option<&'a [String]>,
    /// How each account is classified.
    pub types: &'a AccountTypes,
}

/// [`AccountReport`] over `txns`.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or bad bucket math.
pub fn account_report(
    txns: &[Transaction],
    opts: &AccountReportOpts<'_>,
) -> Result<AccountReport, ReportError> {
    let ranges: Vec<(String, String)> = range_buckets(opts.from, opts.to, opts.interval)?
        .into_iter()
        .map(|span| (span.from, span.to))
        .collect();
    let direct = bucket_totals(txns, &ranges, &|_| true)?;
    account_report_from(direct, || before(txns, opts.from), opts)
}

/// [`account_report`] from each column's direct totals and the balances
/// before `opts.from` — what a projection's cached real side already holds
/// (plan 29). Asked for only when a balance section has an account.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or bad bucket math.
pub(crate) fn account_report_from(
    direct: Vec<BTreeMap<String, MixedAmount>>,
    opening: impl FnOnce() -> Result<BTreeMap<String, MixedAmount>, ReportError>,
    opts: &AccountReportOpts<'_>,
) -> Result<AccountReport, ReportError> {
    let buckets: Vec<String> = range_buckets(opts.from, opts.to, opts.interval)?
        .into_iter()
        .map(|span| span.key)
        .collect();
    let section_of = |account: &str| AccountSection::of(account, opts.types);
    let only = |section: AccountSection,
                map: &BTreeMap<String, MixedAmount>|
     -> BTreeMap<String, MixedAmount> {
        map.iter()
            .filter(|(account, _)| section_of(account) == section)
            .map(|(account, ma)| (account.clone(), ma.clone()))
            .collect()
    };
    let mut opening = Some(opening);
    let mut opened: Option<BTreeMap<String, MixedAmount>> = None;
    let mut sections = Vec::new();
    for section in AccountSection::ALL {
        let changes: Vec<BTreeMap<String, MixedAmount>> =
            direct.iter().map(|column| only(section, column)).collect();
        let known: Option<Vec<String>> = opts.known.map(|known| {
            known
                .iter()
                .filter(|account| section_of(account) == section)
                .cloned()
                .collect()
        });
        // The per-period changes give every row's last column (a Total, or
        // the window's Δ, which is the same sum) and, for a change section,
        // its columns too.
        let (change_rows, change_totals) = pivot(&changes, opts.depth)?;
        let summaries: BTreeMap<String, MixedAmount> = change_rows
            .iter()
            .map(|row| (row.account.clone(), row.total.clone().unwrap_or_default()))
            .collect();
        let summary = sum(&change_totals)?;
        let (rows, totals) = match section.reading() {
            Reading::Change => (change_rows, change_totals),
            Reading::Balance => {
                if opened.is_none() {
                    opened = Some(
                        opening
                            .take()
                            .map_or_else(|| Ok(BTreeMap::new()), |open| open())?,
                    );
                }
                let start = only(section, opened.as_ref().expect("opened above"));
                pivot(&cumulative(start, changes)?, opts.depth)?
            }
        };
        let rows: Vec<AccountReportRow> = shown(rows, known.as_deref(), opts.depth, buckets.len())
            .into_iter()
            .map(|row| AccountReportRow {
                summary: summaries.get(&row.account).cloned().unwrap_or_default(),
                account: row.account,
                depth: row.depth,
                values: row.values,
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        sections.push(AccountReportSection {
            section,
            reading: section.reading(),
            rows,
            totals,
            summary,
        });
    }
    Ok(AccountReport { buckets, sections })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{mixed, txn, usd};
    use super::*;
    use crate::reports::account_types::AccountType;

    fn usd_ma(cents: i128) -> MixedAmount {
        mixed(&[("$", cents, 2)])
    }

    /// The user's case (2026-10-28): cash that holds six figures, a transfer
    /// between two banks, payroll, and a charge on a card — the transfer's
    /// both sides must show, and the cash must read as BALANCES.
    fn journal() -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2026-05-20",
                vec![
                    ("assets:mercury:checking", vec![usd(15_000_000)]),
                    ("assets:mercury:savings", vec![usd(5_000_000)]),
                    ("equity:opening", vec![usd(-20_000_000)]),
                ],
            ),
            txn(
                2,
                "2026-06-15",
                vec![
                    ("expenses:payroll", vec![usd(6_000_000)]),
                    ("assets:mercury:checking", vec![usd(-6_000_000)]),
                ],
            ),
            txn(
                3,
                "2026-06-20",
                vec![
                    ("assets:mercury:savings", vec![usd(3_000_000)]),
                    ("assets:mercury:checking", vec![usd(-3_000_000)]),
                ],
            ),
            txn(
                4,
                "2026-07-03",
                vec![
                    ("assets:mercury:checking", vec![usd(4_000_000)]),
                    ("revenue:sales", vec![usd(-4_000_000)]),
                ],
            ),
            txn(
                5,
                "2026-07-05",
                vec![
                    ("expenses:rent", vec![usd(500_000)]),
                    ("liabilities:card", vec![usd(-500_000)]),
                ],
            ),
        ]
    }

    fn types() -> AccountTypes {
        AccountTypes::from_declared(
            [
                ("assets".to_string(), AccountType::Asset),
                ("liabilities".to_string(), AccountType::Liability),
                ("equity".to_string(), AccountType::Equity),
                ("revenue".to_string(), AccountType::Revenue),
                ("expenses".to_string(), AccountType::Expense),
            ]
            .into_iter()
            .collect(),
        )
    }

    fn report(depth: usize) -> AccountReport {
        let types = types();
        account_report(
            &journal(),
            &AccountReportOpts {
                from: "2026-06-01",
                to: "2026-07-31",
                interval: Some(Interval::Monthly),
                depth,
                known: None,
                types: &types,
            },
        )
        .expect("report")
    }

    fn row<'a>(report: &'a AccountReport, account: &str) -> &'a AccountReportRow {
        report
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.account == account)
            .unwrap_or_else(|| panic!("a row for {account}"))
    }

    #[test]
    fn sections_come_in_order_each_read_its_own_way() {
        let report = report(3);
        let shape: Vec<(AccountSection, Reading)> = report
            .sections
            .iter()
            .map(|section| (section.section, section.reading))
            .collect();
        assert_eq!(
            shape,
            [
                (AccountSection::Assets, Reading::Balance),
                (AccountSection::Liabilities, Reading::Balance),
                // No Equity: its one posting (the opening) is before the
                // window, so nothing changed in it — and a change section
                // with no row is not shown.
                (AccountSection::Revenue, Reading::Change),
                (AccountSection::Expenses, Reading::Change),
            ]
        );
    }

    /// Assets read as balances, opening included, and both sides of a
    /// transfer between two banks show — as `hledger balance -H -M`.
    #[test]
    fn assets_and_liabilities_are_balances_with_the_window_s_delta() {
        let report = report(3);
        let checking = row(&report, "assets:mercury:checking");
        assert_eq!(checking.values, [usd_ma(6_000_000), usd_ma(10_000_000)]);
        // Δ: closing 100,000 − opening 150,000.
        assert_eq!(checking.summary, usd_ma(-5_000_000));
        let savings = row(&report, "assets:mercury:savings");
        assert_eq!(savings.values, [usd_ma(8_000_000), usd_ma(8_000_000)]);
        assert_eq!(savings.summary, usd_ma(3_000_000));
        let card = row(&report, "liabilities:card");
        assert_eq!(card.values, [MixedAmount::new(), usd_ma(-500_000)]);
        assert_eq!(card.summary, usd_ma(-500_000));
        let assets = &report.sections[0];
        assert_eq!(assets.totals, [usd_ma(14_000_000), usd_ma(18_000_000)]);
        assert_eq!(assets.summary, usd_ma(-2_000_000));
    }

    /// Equity, revenue and expenses read as each period's change, with a Total.
    #[test]
    fn equity_revenue_and_expenses_are_changes_with_a_total() {
        let report = report(3);
        let payroll = row(&report, "expenses:payroll");
        assert_eq!(payroll.values, [usd_ma(6_000_000), MixedAmount::new()]);
        assert_eq!(payroll.summary, usd_ma(6_000_000));
        let sales = row(&report, "revenue:sales");
        assert_eq!(sales.values, [MixedAmount::new(), usd_ma(-4_000_000)]);
        // The opening balance is before the window: no change in it.
        assert!(
            report
                .sections
                .iter()
                .all(|s| s.section != AccountSection::Equity)
        );
    }

    /// The depth clamp rolls up within a section and never moves its totals.
    #[test]
    fn depth_rolls_up_within_each_section() {
        let report = report(1);
        let assets = row(&report, "assets");
        assert_eq!(assets.values, [usd_ma(14_000_000), usd_ma(18_000_000)]);
        assert_eq!(assets.summary, usd_ma(-2_000_000));
        assert_eq!(
            report.sections[0].totals,
            self::report(3).sections[0].totals
        );
    }
}
