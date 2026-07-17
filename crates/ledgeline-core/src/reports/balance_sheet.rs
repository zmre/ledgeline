//! Balance sheet — port of `web/src/lib/reports/balanceSheet.ts`.

use super::ReportError;
use super::accounts::RootCategory;
use super::aggregate::{PostingFilter, account_totals, at_depth, roll_up};
use super::sections::build_section;
use super::types::SectionedReport;
use crate::model::Transaction;

/// Asset + liability balances as of `as_of` (INCLUSIVE: postings dated ≤
/// `as_of`). Presentation matches `hledger bs`: liabilities are sign-flipped
/// (positive = owed); `grand_total` = assets − liabilities(displayed).
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn balance_sheet(
    txns: &[Transaction],
    as_of: &str,
    depth: usize,
) -> Result<SectionedReport, ReportError> {
    let direct = account_totals(
        txns,
        &PostingFilter {
            to: Some(as_of),
            ..PostingFilter::default()
        },
    )?;
    let clamped = at_depth(&roll_up(&direct)?, depth);
    let assets = build_section("Assets", RootCategory::Asset, &direct, &clamped, false)?;
    let liabilities = build_section(
        "Liabilities",
        RootCategory::Liability,
        &direct,
        &clamped,
        true,
    )?;
    let grand_total = assets.total.ma_add(&liabilities.total.ma_neg()?)?;
    Ok(SectionedReport {
        as_of: Some(as_of.to_string()),
        from: None,
        to: None,
        sections: vec![assets, liabilities],
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
            txn(
                1,
                "2026-01-05",
                vec![
                    ("assets:bank:checking", vec![usd(100_000)]),
                    ("equity:opening", vec![usd(-100_000)]),
                ],
            ),
            txn(
                2,
                "2026-02-10",
                vec![
                    ("expenses:food", vec![usd(2500)]),
                    ("liabilities:cc:visa", vec![usd(-2500)]),
                ],
            ),
            txn(
                3,
                "2026-03-15",
                vec![
                    ("assets:bank:savings", vec![usd(50_000)]),
                    ("assets:bank:checking", vec![usd(-50_000)]),
                ],
            ),
            txn(
                4,
                "2026-04-01",
                vec![
                    ("assets:bank", vec![usd(1000)]),
                    ("income:interest", vec![usd(-1000)]),
                ],
            ),
            txn(
                5,
                "2026-07-01",
                vec![
                    ("assets:bank:checking", vec![usd(99_999)]),
                    ("income:salary", vec![usd(-99_999)]),
                ],
            ),
        ]
    }

    fn usd_ma(cents: i128) -> MixedAmount {
        mixed(&[("$", cents, 2)])
    }

    #[test]
    fn assets_and_sign_flipped_liabilities_as_of_inclusive_date() {
        let report = balance_sheet(&sample(), "2026-06-30", 3).unwrap();
        assert_eq!(report.as_of.as_deref(), Some("2026-06-30"));
        assert_eq!(
            report
                .sections
                .iter()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>(),
            ["Assets", "Liabilities"]
        );
        let assets = &report.sections[0];
        assert_eq!(
            assets
                .rows
                .iter()
                .map(|r| (r.account.as_str(), r.depth))
                .collect::<Vec<_>>(),
            [
                ("assets", 1),
                ("assets:bank", 2),
                ("assets:bank:checking", 3),
                ("assets:bank:savings", 3),
            ]
        );
        assert_eq!(assets.rows[2].inclusive, usd_ma(50_000)); // checking: 1000 − 500, July excluded
        assert_eq!(assets.rows[3].inclusive, usd_ma(50_000));
        assert_eq!(assets.total, usd_ma(101_000));

        let liabilities = &report.sections[1];
        assert_eq!(
            liabilities
                .rows
                .iter()
                .map(|r| r.account.as_str())
                .collect::<Vec<_>>(),
            ["liabilities", "liabilities:cc", "liabilities:cc:visa"]
        );
        assert_eq!(liabilities.rows[0].inclusive, usd_ma(2500)); // displayed positive
        assert_eq!(liabilities.total, usd_ma(2500));

        assert_eq!(report.grand_total, usd_ma(98_500)); // 1010 − 25
    }

    #[test]
    fn distinguishes_own_from_inclusive() {
        let report = balance_sheet(&sample(), "2026-06-30", 2).unwrap();
        let bank = report.sections[0]
            .rows
            .iter()
            .find(|r| r.account == "assets:bank")
            .unwrap();
        assert_eq!(bank.own, usd_ma(1000)); // only the direct $10 posting
        assert_eq!(bank.inclusive, usd_ma(101_000)); // checking + savings + own
        let root = report.sections[0]
            .rows
            .iter()
            .find(|r| r.account == "assets")
            .unwrap();
        assert_eq!(root.own, MixedAmount::new());
        assert_eq!(root.inclusive, usd_ma(101_000));
    }

    #[test]
    fn clamps_to_depth_one() {
        let report = balance_sheet(&sample(), "2026-06-30", 1).unwrap();
        assert_eq!(report.sections[0].rows.len(), 1);
        assert_eq!(report.sections[0].rows[0].account, "assets");
        assert_eq!(report.sections[0].rows[0].own, MixedAmount::new());
        assert_eq!(report.sections[0].rows[0].inclusive, usd_ma(101_000));
        assert_eq!(report.sections[1].rows.len(), 1);
        assert_eq!(report.sections[1].rows[0].account, "liabilities");
        assert_eq!(report.sections[1].rows[0].inclusive, usd_ma(2500));
        assert_eq!(report.grand_total, usd_ma(98_500));
    }

    #[test]
    fn empty_sections_before_all_activity() {
        let report = balance_sheet(&sample(), "2025-12-31", 3).unwrap();
        assert!(report.sections[0].rows.is_empty());
        assert_eq!(report.grand_total, MixedAmount::new());
    }
}
