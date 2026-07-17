//! Income statement / P&L — port of `web/src/lib/reports/incomeStatement.ts`.

use super::ReportError;
use super::accounts::RootCategory;
use super::aggregate::{PostingFilter, account_totals, at_depth, roll_up};
use super::sections::build_section;
use super::types::SectionedReport;
use crate::model::Transaction;

/// Revenues + expenses over `[from, to]` (both INCLUSIVE). Presentation matches
/// `hledger is`: revenues are sign-flipped (positive = earned); `grand_total` =
/// revenues(displayed) − expenses = net income.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn income_statement(
    txns: &[Transaction],
    from: &str,
    to: &str,
    depth: usize,
) -> Result<SectionedReport, ReportError> {
    let direct = account_totals(
        txns,
        &PostingFilter {
            from: Some(from),
            to: Some(to),
            ..PostingFilter::default()
        },
    )?;
    let clamped = at_depth(&roll_up(&direct)?, depth);
    let revenues = build_section("Revenues", RootCategory::Revenue, &direct, &clamped, true)?;
    let expenses = build_section("Expenses", RootCategory::Expense, &direct, &clamped, false)?;
    let grand_total = revenues.total.ma_add(&expenses.total.ma_neg()?)?;
    Ok(SectionedReport {
        as_of: None,
        from: Some(from.to_string()),
        to: Some(to.to_string()),
        sections: vec![revenues, expenses],
        grand_total,
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{mixed, txn, usd};
    use super::*;
    use crate::reports::mixed_amount::MixedAmount;

    fn sample() -> Vec<Transaction> {
        vec![
            // Before the range:
            txn(
                1,
                "2025-12-31",
                vec![
                    ("income:salary", vec![usd(-500_000)]),
                    ("assets:bank:checking", vec![usd(500_000)]),
                ],
            ),
            txn(
                2,
                "2026-01-15",
                vec![
                    ("income:salary", vec![usd(-400_000)]),
                    ("assets:bank:checking", vec![usd(400_000)]),
                ],
            ),
            txn(
                3,
                "2026-02-20",
                vec![
                    ("expenses:food:groceries", vec![usd(15_000)]),
                    ("liabilities:cc", vec![usd(-15_000)]),
                ],
            ),
            // "revenues" root categorizes as revenue alongside "income":
            txn(
                4,
                "2026-03-05",
                vec![
                    ("revenues:consulting", vec![usd(-20_000)]),
                    ("assets:bank:checking", vec![usd(20_000)]),
                ],
            ),
            // After the range:
            txn(
                5,
                "2026-07-01",
                vec![
                    ("expenses:food", vec![usd(9999)]),
                    ("assets:bank:checking", vec![usd(-9999)]),
                ],
            ),
        ]
    }

    fn usd_ma(cents: i128) -> MixedAmount {
        mixed(&[("$", cents, 2)])
    }

    #[test]
    fn sign_flipped_revenues_and_natural_expenses_over_inclusive_range() {
        let report = income_statement(&sample(), "2026-01-01", "2026-06-30", 2).unwrap();
        assert_eq!(report.from.as_deref(), Some("2026-01-01"));
        assert_eq!(report.to.as_deref(), Some("2026-06-30"));
        assert_eq!(
            report
                .sections
                .iter()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>(),
            ["Revenues", "Expenses"]
        );

        let revenues = &report.sections[0];
        assert_eq!(
            revenues
                .rows
                .iter()
                .map(|r| (r.account.as_str(), r.inclusive.clone()))
                .collect::<Vec<_>>(),
            [
                ("income", usd_ma(400_000)), // displayed positive; Dec txn out of range
                ("income:salary", usd_ma(400_000)),
                ("revenues", usd_ma(20_000)),
                ("revenues:consulting", usd_ma(20_000)),
            ]
        );
        assert_eq!(revenues.total, usd_ma(420_000)); // sums BOTH revenue roots

        let expenses = &report.sections[1];
        assert_eq!(
            expenses
                .rows
                .iter()
                .map(|r| (r.account.as_str(), r.inclusive.clone()))
                .collect::<Vec<_>>(),
            [
                ("expenses", usd_ma(15_000)), // July txn out of range
                ("expenses:food", usd_ma(15_000)),
            ]
        );
        assert_eq!(expenses.total, usd_ma(15_000));

        assert_eq!(report.grand_total, usd_ma(405_000)); // revenues − expenses
    }

    #[test]
    fn range_boundaries_inclusive_on_both_ends() {
        let report = income_statement(&sample(), "2025-12-31", "2026-07-01", 1).unwrap();
        assert_eq!(report.sections[0].total, usd_ma(920_000)); // 5000 + 4000 + 200
        assert_eq!(report.sections[1].total, usd_ma(24_999)); // 150.00 + 99.99
        assert_eq!(report.grand_total, usd_ma(895_001));
    }
}
