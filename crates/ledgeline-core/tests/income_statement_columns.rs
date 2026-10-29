//! The grouped income statement in INTERVAL mode (`plans/24-report-intervals-
//! and-totals.md`): one column per bucket across `[from, to]` plus a Total, no
//! prior comparison.
//!
//! Interval mode is the Total-mode statement over a different window list, so
//! the claims defended here are the ones that could differ between the two:
//!
//! 1. **The buckets tile the range**, the first clamped to `from` and the last
//!    to `to`, and each bucket's figure is exactly the Total-mode statement over
//!    that bucket's own dates.
//! 2. **The Total is the SUM of the buckets** at every level — not a
//!    whole-range valuation. Under market valuation the two differ, and that is
//!    the intended QuickBooks behaviour.
//! 3. **Structure is resolved over the union of every bucket**, so the boxes,
//!    lines and rows are the Total-mode statement's, with zeros in the buckets a
//!    line had no activity in.

mod common;

use ledgeline_core::Journal;
use ledgeline_core::model::Commodity;
use ledgeline_core::reports::{
    Amounts, BucketSpan, Columns, IS_GROUP_TAG, IncomeStatementReport, Interval, IsOpts,
    IsSectionKind, MixedAmount, Valuation, account_decls, account_sections, declared_groups,
    declared_types, income_statement_columns, income_statement_grouped,
};
use ledgeline_core::{Dec, parse_journal};

fn opts(from: &'static str, to: &'static str, value: Valuation) -> IsOpts<'static> {
    IsOpts {
        from,
        to,
        value,
        value_in: None,
        compare: false,
        depth: None,
    }
}

fn columns(
    journal: &Journal,
    from: &'static str,
    to: &'static str,
    value: Valuation,
    interval: Interval,
) -> IncomeStatementReport<Columns> {
    income_statement_columns(
        &journal.transactions,
        &journal.prices,
        &opts(from, to, value),
        interval,
        &declared_types(&account_decls(journal)),
        &account_sections(journal),
        &declared_groups(journal, IS_GROUP_TAG),
    )
    .expect("interval income statement")
}

fn total_mode(
    journal: &Journal,
    from: &'static str,
    to: &'static str,
    value: Valuation,
) -> IncomeStatementReport {
    income_statement_grouped(
        &journal.transactions,
        &journal.prices,
        &opts(from, to, value),
        &declared_types(&account_decls(journal)),
        &account_sections(journal),
        &declared_groups(journal, IS_GROUP_TAG),
    )
    .expect("grouped income statement")
}

fn usd(cents: i128) -> MixedAmount {
    MixedAmount::single(Commodity("$".into()), Dec::new(cents, 2))
}

/// Trailing zeros stripped and zero commodities dropped, so exact-decimal
/// valuation scale cannot fail an equality the VALUES satisfy.
fn canon(ma: &MixedAmount) -> Vec<(String, i128, u32)> {
    ma.iter()
        .filter(|(_, dec)| !dec.is_zero())
        .map(|(symbol, dec)| {
            let (mut mantissa, mut places) = (dec.mantissa, dec.places);
            while places > 0 && mantissa % 10 == 0 {
                mantissa /= 10;
                places -= 1;
            }
            (symbol.0.clone(), mantissa, places)
        })
        .collect()
}

#[track_caller]
fn assert_money(actual: &MixedAmount, want: &MixedAmount, what: &str) {
    assert_eq!(canon(actual), canon(want), "{what}");
}

fn sum(values: &[MixedAmount]) -> MixedAmount {
    values.iter().fold(MixedAmount::new(), |acc, ma| {
        acc.ma_add(ma).expect("no overflow")
    })
}

/// Every figure in a statement, flattened with a label, in presentation order.
fn figures<A: Clone>(report: &IncomeStatementReport<A>) -> Vec<(String, A)> {
    let mut out = Vec::new();
    for section in &report.sections {
        for group in &section.groups {
            out.push((format!("{} group", group.name), group.total.clone()));
            for row in &group.rows {
                out.push((row.account.clone(), row.amounts.clone()));
            }
        }
        out.push((format!("{} total", section.title), section.total.clone()));
        for subtotal in &section.trailing {
            out.push((subtotal.label.clone(), subtotal.total.clone()));
        }
    }
    out.push(("Net income".to_string(), report.net_income.clone()));
    out
}

fn span(key: &str, from: &str, to: &str) -> BucketSpan {
    BucketSpan {
        key: key.to_string(),
        from: from.to_string(),
        to: to.to_string(),
    }
}

/// The columns tile `[from, to]`: the first starts at `from` mid-month, the last
/// ends at `to` mid-month, and there is no prior window.
#[test]
fn buckets_are_clamped_to_the_range_and_there_is_no_prior() {
    let journal = common::fixture_journal();
    let report = columns(
        &journal,
        "2026-01-15",
        "2026-03-10",
        Valuation::Market,
        Interval::Monthly,
    );
    assert_eq!(
        report.buckets,
        [
            span("2026-01", "2026-01-15", "2026-01-31"),
            span("2026-02", "2026-02-01", "2026-02-28"),
            span("2026-03", "2026-03-01", "2026-03-10"),
        ]
    );
    assert_eq!(report.prior, None);
    assert_eq!(report.from, "2026-01-15");
    assert_eq!(report.to, "2026-03-10");
    for (what, figure) in figures(&report) {
        assert_eq!(figure.values.len(), 3, "{what}");
    }
    // And the Total-mode statement carries no buckets at all.
    assert!(
        total_mode(&journal, "2026-01-15", "2026-03-10", Valuation::Market)
            .buckets
            .is_empty()
    );
}

/// Each bucket's figure is the Total-mode statement run over exactly that
/// bucket's dates — the same windowing, valuation and section resolution.
#[test]
fn each_bucket_is_the_statement_over_its_own_dates() {
    let journal = common::fixture_journal();
    let report = columns(
        &journal,
        "2026-01-01",
        "2026-07-08",
        Valuation::Market,
        Interval::Quarterly,
    );
    assert_eq!(report.buckets.len(), 3);
    let (Some(revenue), Some(opex)) = (
        report
            .sections
            .iter()
            .find(|s| s.kind == IsSectionKind::Revenue),
        report
            .sections
            .iter()
            .find(|s| s.kind == IsSectionKind::Opex),
    ) else {
        panic!("revenue and opex boxes");
    };
    let quarters = [
        ("2026-01-01", "2026-03-31"),
        ("2026-04-01", "2026-06-30"),
        ("2026-07-01", "2026-07-08"),
    ];
    for (i, (from, to)) in quarters.into_iter().enumerate() {
        let alone = total_mode(&journal, from, to, Valuation::Market);
        let net: &Amounts = &alone.net_income;
        assert_money(
            &report.net_income.values[i],
            &net.current,
            &format!("net income, bucket {i}"),
        );
        for (section, kind) in [
            (revenue, IsSectionKind::Revenue),
            (opex, IsSectionKind::Opex),
        ] {
            let want = alone
                .sections
                .iter()
                .find(|s| s.kind == kind)
                .map_or_else(MixedAmount::new, |s| s.total.current.clone());
            assert_money(
                &section.total.values[i],
                &want,
                &format!("{kind:?}, bucket {i}"),
            );
        }
    }
}

/// The Total column is the sum of the buckets at EVERY level, on every basis —
/// and the buckets' own totals still sum over members, so the Total does too.
#[test]
fn the_total_is_the_sum_of_the_buckets_everywhere() {
    let journal = common::fixture_journal();
    for value in [Valuation::Market, Valuation::Cost, Valuation::None] {
        for interval in [Interval::Monthly, Interval::Quarterly, Interval::Yearly] {
            let report = columns(&journal, "2025-06-01", "2026-07-08", value, interval);
            for (what, figure) in figures(&report) {
                assert_money(
                    &figure.total,
                    &sum(&figure.values),
                    &format!("{what} ({value:?}, {interval:?})"),
                );
            }
        }
    }
}

/// Off market valuation nothing is revalued, so the sum of the buckets IS the
/// whole-range figure: every line's Total equals the Total-mode statement's.
/// Structure agrees too — the same boxes, lines and rows, in the same order.
#[test]
fn off_market_the_total_equals_the_whole_range_statement() {
    let journal = common::fixture_journal();
    for value in [Valuation::Cost, Valuation::None] {
        let report = columns(
            &journal,
            "2025-06-01",
            "2026-07-08",
            value,
            Interval::Monthly,
        );
        let whole = total_mode(&journal, "2025-06-01", "2026-07-08", value);
        let mine = figures(&report);
        let theirs = figures(&whole);
        assert_eq!(
            mine.iter().map(|(what, _)| what).collect::<Vec<_>>(),
            theirs.iter().map(|(what, _)| what).collect::<Vec<_>>(),
            "same lines in the same order ({value:?})"
        );
        for ((what, columns), (_, amounts)) in mine.iter().zip(&theirs) {
            assert_money(
                &columns.total,
                &amounts.current,
                &format!("{what} ({value:?})"),
            );
        }
    }
}

/// Under MARKET valuation each bucket is valued at its own end, so the Total —
/// their sum — differs from one whole-range valuation. Travel is the line that
/// shows it: 681 EUR of 2025 lodging is `$749.10` at 2025-12-31's `$1.10` in its
/// own bucket, and would be `$789.96` at 2026's `$1.16` valued all at once.
#[test]
fn under_market_valuation_the_total_is_the_sum_not_a_whole_range_valuation() {
    let journal = common::fixture_journal();
    let report = columns(
        &journal,
        "2025-01-01",
        "2026-12-31",
        Valuation::Market,
        Interval::Yearly,
    );
    let travel = report
        .sections
        .iter()
        .flat_map(|section| &section.groups)
        .find(|group| group.name == "Travel")
        .expect("Travel line");
    assert_money(&travel.total.values[0], &usd(74_910), "Travel 2025");
    assert_money(&travel.total.values[1], &usd(65_640), "Travel 2026");
    assert_money(
        &travel.total.total,
        &usd(140_550),
        "Travel Total = 2025 + 2026",
    );

    let whole = total_mode(&journal, "2025-01-01", "2026-12-31", Valuation::Market);
    let whole_travel = whole
        .sections
        .iter()
        .flat_map(|section| &section.groups)
        .find(|group| group.name == "Travel")
        .expect("Travel line");
    assert_ne!(
        canon(&travel.total.total),
        canon(&whole_travel.total.current),
        "the Total is deliberately NOT one valuation of the whole range"
    );
}

/// A line with activity in only one bucket is still ONE row, with zeros in
/// every other bucket — structure is resolved over the union of all of them.
#[test]
fn a_line_active_in_one_bucket_reads_zero_in_the_others() {
    let text = "\
2026-01-10 Rent
    expenses:rent      $1000.00
    assets:bank

2026-03-05 One-off
    expenses:repairs    $250.00
    assets:bank
";
    let journal = parse_journal(text, "inline.journal").expect("parses");
    let report = columns(
        &journal,
        "2026-01-01",
        "2026-03-31",
        Valuation::None,
        Interval::Monthly,
    );
    let repairs = report
        .sections
        .iter()
        .flat_map(|section| &section.groups)
        .find(|group| group.name == "Repairs")
        .expect("Repairs line");
    assert_eq!(
        repairs.total.values,
        [MixedAmount::new(), MixedAmount::new(), usd(25_000)]
    );
    assert_money(&repairs.total.total, &usd(25_000), "Repairs Total");
    assert_money(
        &report.net_income.total,
        &usd(-125_000),
        "net income Total over both expenses",
    );
}
