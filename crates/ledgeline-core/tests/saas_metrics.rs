//! Startup metrics (`plans/37-startup-metrics.md`), pinned against
//! `fixtures/saas/`: hledger 1.52's own output (`expected/*.csv`, captured by
//! `capture.sh`, never from Rust), the README's hand-computed figures, the
//! bridge and revenue invariants (fixture and proptest), visibility, edges and
//! the Top Transactions skip rule.

mod common;

use common::fixtures_dir;
use ledgeline_core::model::Commodity;
use ledgeline_core::reports::{
    Bridge, COVERAGE_MIN_DEFAULT, CfOpts, CfSectionKind, CfTags, DIMENSION_KEYS_DEFAULT,
    HiddenReason, InsightsOpts, Interval, RunwayBasis, SaasOpts, SaasReport, account_decls,
    cash_flow_statement, declared_types, income_statement, insights, saas_metrics,
};
use ledgeline_core::{Dec, Journal, auto, parse_journal};
use proptest::prelude::*;
use std::collections::BTreeMap;

const YOY_START: &str = "2024-07-01";
const YOY_END: &str = "2026-06-30";

fn fixture_text(name: &str) -> String {
    std::fs::read_to_string(fixtures_dir().join("saas").join(name)).expect("fixture readable")
}

fn parse(text: &str) -> Journal {
    auto::apply_journal(&parse_journal(text, "startup.journal").expect("journal parses"))
}

fn fixture(name: &str) -> Journal {
    parse(&fixture_text(name))
}

fn default_dims() -> Vec<String> {
    DIMENSION_KEYS_DEFAULT
        .iter()
        .map(|key| (*key).to_string())
        .collect()
}

fn run(journal: &Journal, start: &str, end: &str, matrix: bool) -> SaasReport {
    run_as_of(journal, start, end, matrix, None)
}

fn run_as_of(
    journal: &Journal,
    start: &str,
    end: &str,
    matrix: bool,
    as_of: Option<&str>,
) -> SaasReport {
    let dims = default_dims();
    saas_metrics(
        journal,
        &SaasOpts {
            start,
            end,
            dims: &dims,
            coverage_min: COVERAGE_MIN_DEFAULT,
            top_n: 5,
            with_customer_matrix: matrix,
            as_of,
        },
    )
    .expect("saas metrics compute")
}

fn yoy(journal: &Journal) -> SaasReport {
    run(journal, YOY_START, YOY_END, false)
}

/// Exact decimal from text (`"731.25"`).
fn dec(text: &str) -> Dec {
    Dec::parse(text, '.').expect("decimal literal")
}

fn close(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("value present");
    assert!(
        (actual - expected).abs() < 1e-9,
        "{actual} != {expected} (within 1e-9)"
    );
}

// ---------------------------------------------------------------------------
// hledger's CSV
// ---------------------------------------------------------------------------

/// One `-O csv` balance report: header months and `account → values`.
struct Csv {
    months: Vec<String>,
    rows: BTreeMap<String, Vec<Dec>>,
}

/// Parse hledger's CSV. `0` and blank are zero; `$-150.00` is negative.
fn csv(name: &str) -> Csv {
    let text = fixture_text(&format!("expected/{name}.csv"));
    let mut lines = text.lines().map(|line| {
        line[1..line.len() - 1]
            .split("\",\"")
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    let header = lines.next().expect("header row");
    let rows = lines
        .map(|cells| {
            let values = cells[1..]
                .iter()
                .map(|cell| {
                    let cleaned = cell.replace(['$', ','], "");
                    if cleaned.is_empty() || cleaned == "0" {
                        Dec::zero()
                    } else {
                        dec(&cleaned)
                    }
                })
                .collect();
            (cells[0].clone(), values)
        })
        .collect();
    Csv {
        months: header[1..].to_vec(),
        rows,
    }
}

/// The fixture's whole history as one span: 2025-01 … 2026-06.
fn whole(journal: &Journal) -> SaasReport {
    run(journal, "2025-01-01", "2026-06-30", true)
}

#[test]
fn customer_matrix_equals_hledgers_pivot_cell_for_cell() {
    let expected = csv("customer-mrr");
    let report = whole(&fixture("startup.journal"));
    let months: Vec<&str> = report.months.iter().map(|m| m.month.as_str()).collect();
    assert_eq!(months, expected.months);

    let matrix = report.customer_matrix.expect("matrix requested");
    let ours: BTreeMap<&str, &Vec<Dec>> = matrix
        .iter()
        .map(|row| (row.customer.as_str(), &row.values))
        .collect();
    let theirs: BTreeMap<&str, &Vec<Dec>> = expected
        .rows
        .iter()
        .filter(|(account, _)| !account.is_empty() && account.as_str() != "Total:")
        .map(|(account, values)| (account.as_str(), values))
        .collect();
    assert_eq!(ours, theirs);
    // hledger's untagged row is our unattributed column.
    let unattributed: Vec<Dec> = report.months.iter().map(|m| m.unattributed).collect();
    assert_eq!(&unattributed, &expected.rows[""]);
}

#[test]
fn monthly_series_equal_hledger() {
    let report = whole(&fixture("startup.journal"));
    let column = |pick: &dyn Fn(&ledgeline_core::reports::SaasMonth) -> Dec| -> Vec<Dec> {
        report.months.iter().map(pick).collect()
    };
    assert_eq!(column(&|m| m.mrr), csv("mrr").rows["Total:"]);
    assert_eq!(
        column(&|m| m.cogs.expect("cogs available")),
        csv("cogs").rows["Total:"]
    );
    assert_eq!(column(&|m| m.cash_end), csv("cash").rows["Total:"]);
    assert_eq!(
        column(&|m| m.deferred_end.expect("deferred role")),
        csv("deferred").rows["Total:"]
    );
    // Billings per month = revenue + Δ deferred.
    let mut previous = Dec::zero();
    let billings: Vec<Dec> = report
        .months
        .iter()
        .map(|m| {
            let end = m.deferred_end.expect("deferred role");
            let value = m.revenue.add(end).unwrap().sub(previous).unwrap();
            previous = end;
            value
        })
        .collect();
    assert_eq!(billings, csv("billings").rows["Total:"]);
}

/// Each segment's MRR per month, from a two-month span ending in that month
/// (its current half is the month itself).
fn segment_series(journal: &Journal, key: &str) -> BTreeMap<String, Vec<Dec>> {
    let mut out: BTreeMap<String, Vec<Dec>> = BTreeMap::new();
    let ends = [
        "2025-01-31",
        "2025-02-28",
        "2025-03-31",
        "2025-04-30",
        "2025-05-31",
        "2025-06-30",
        "2025-07-31",
        "2025-08-31",
        "2025-09-30",
        "2025-10-31",
        "2025-11-30",
        "2025-12-31",
        "2026-01-31",
        "2026-02-28",
        "2026-03-31",
        "2026-04-30",
        "2026-05-31",
        "2026-06-30",
    ];
    for (i, end) in ends.iter().enumerate() {
        let start = format!("{}-01", shift_month(&end[..7], -1));
        let report = run(journal, &start, end, false);
        let breakdown = report.segments.iter().find(|s| s.key == key);
        let mut total = Dec::zero();
        for row in breakdown.map(|b| b.rows.as_slice()).unwrap_or_default() {
            out.entry(row.value.clone())
                .or_insert_with(|| vec![Dec::zero(); ends.len()])[i] = row.mrr_curr;
            total = total.add(row.mrr_curr).unwrap();
        }
        // Σ segments' MRR = total MRR, every month.
        if breakdown.is_some() {
            assert_eq!(
                total,
                report.current.mrr.unwrap(),
                "{key} sums to MRR in {end}"
            );
        }
    }
    out
}

fn shift_month(month: &str, by: i64) -> String {
    let (year, mon): (i64, i64) = (month[..4].parse().unwrap(), month[5..7].parse().unwrap());
    let index = year * 12 + mon - 1 + by;
    format!(
        "{:04}-{:02}",
        index.div_euclid(12),
        index.rem_euclid(12) + 1
    )
}

#[test]
fn segment_and_class_mrr_equal_hledgers_pivots() {
    let journal = fixture("startup.journal");
    for (key, file) in [("segment", "segment-mrr"), ("class", "class-mrr")] {
        let expected = csv(file);
        let ours = segment_series(&journal, key);
        for (value, values) in &expected.rows {
            if value == "Total:" {
                continue;
            }
            let ours_key = if value.is_empty() {
                "(unassigned)"
            } else {
                value
            };
            let zeros = vec![Dec::zero(); values.len()];
            assert_eq!(
                ours.get(ours_key).unwrap_or(&zeros),
                values,
                "{key}={ours_key}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Hand-computed expectations (fixtures/saas/README.md)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn bridge(
    new: &str,
    expansion: &str,
    contraction: &str,
    churn: &str,
    reactivation: &str,
    other: &str,
    logos: (u32, u32, u32),
) -> Bridge {
    Bridge {
        new: dec(new),
        expansion: dec(expansion),
        contraction: dec(contraction),
        churn: dec(churn),
        reactivation: dec(reactivation),
        other: dec(other),
        new_logos: logos.0,
        churned_logos: logos.1,
        reactivated_logos: logos.2,
    }
}

#[test]
fn the_bridge_matches_the_readme_every_month() {
    let report = whole(&fixture("startup.journal"));
    let expected = [
        bridge("2350", "0", "0", "0", "0", "0", (6, 0, 0)),
        bridge("600", "0", "0", "600", "0", "0", (1, 1, 0)),
        bridge("1000", "0", "0", "600", "600", "0", (1, 1, 1)),
        bridge("0", "0", "0", "0", "0", "100", (0, 0, 0)),
        bridge("0", "0", "0", "0", "0", "-100", (0, 0, 0)),
        bridge("250", "0", "0", "0", "0", "0", (1, 0, 0)),
        bridge("0", "300", "0", "0", "0", "0", (0, 0, 0)),
        bridge("0", "0", "0", "350", "0", "-150", (0, 1, 0)),
        bridge("0", "0", "0", "250", "350", "150", (0, 1, 1)),
        bridge("1500", "0", "0", "200", "0", "0", (1, 1, 0)),
        bridge("0", "0", "150", "0", "0", "0", (0, 0, 0)),
        bridge("0", "0", "0", "0", "250", "0", (0, 0, 1)),
        bridge("0", "500", "0", "0", "0", "0", (0, 0, 0)),
        bridge("0", "0", "0", "0", "0", "0", (0, 0, 0)),
        bridge("0", "300", "0", "0", "0", "0", (0, 0, 0)),
        bridge("0", "0", "0", "0", "0", "100", (0, 0, 0)),
        bridge("0", "0", "0", "0", "0", "-100", (0, 0, 0)),
        bridge("0", "0", "0", "0", "0", "0", (0, 0, 0)),
    ];
    for (month, want) in report.months.iter().zip(expected) {
        assert_eq!(month.bridge, Some(want), "{}", month.month);
    }
}

#[test]
fn current_half_metrics_match_the_readme() {
    let report = yoy(&fixture("startup.journal"));
    let c = &report.current;
    assert_eq!(c.month.as_deref(), Some("2026-06"));
    assert_eq!(c.months, 12);
    assert_eq!(c.mrr, Some(dec("5850")));
    assert_eq!(c.arr, Some(dec("70200")));
    assert_eq!(c.usage, Some(dec("100")));
    close(c.mrr_growth_pct, 62.5);
    close(
        c.cmgr_pct,
        ((5850.0_f64 / 3600.0).powf(1.0 / 12.0) - 1.0) * 100.0,
    );
    assert_eq!(c.net_new_arr, Some(dec("27000")));
    assert_eq!(
        c.bridge,
        Some(bridge("1500", "1100", "150", "800", "600", "0", (1, 3, 2)))
    );
    assert_eq!(c.customers, Some(8));
    close(c.customer_growth_pct, 0.0);
    close(c.logo_churn_pct, 37.5);
    assert_eq!(c.arpa, Some(dec("731.25")));
    assert_eq!(c.acv, Some(dec("18000")));
    close(c.nrr_pct, 4350.0 / 3600.0 * 100.0);
    close(c.grr_pct, 3250.0 / 3600.0 * 100.0);
    close(c.quick_ratio, 3200.0 / 950.0);
    close(c.top1_pct, 1500.0 / 5850.0 * 100.0);
    close(c.top5_pct, 5100.0 / 5850.0 * 100.0);
    close(c.top10_pct, 100.0);
    assert_eq!(c.revenue, dec("66550"));
    assert_eq!(c.gross_profit, Some(dec("54480")));
    close(c.gross_margin_pct, 54480.0 / 66550.0 * 100.0);
    assert_eq!(c.net_burn, dec("43720"));
    assert_eq!(c.net_burn_avg3, Some(dec("6543.33")));
    assert_eq!(c.cash, Some(dec("432160")));
    // A1.1: runway is on the adjusted burn (see the runway-basis test).
    close(c.runway_months, 432_160.0 * 3.0 / 11_080.0);
    assert_eq!(c.cash_out_date.as_deref(), Some("2036-03-31"));
    // A3: on the adjusted burn (Σ 44,070 over 2025-07 … 2026-06).
    assert_eq!(c.net_burn_adjusted, Some(dec("44070")));
    close(c.burn_multiple, 44070.0 / 27000.0);
    assert_eq!(c.burn_multiple_basis, Some(RunwayBasis::Adjusted));
    close(
        c.rule_of_40,
        (66550.0 / 18450.0 - 1.0) * 100.0 - 43720.0 / 66550.0 * 100.0,
    );
    assert_eq!(c.sm, Some(dec("33000")));
    assert_eq!(c.rnd, Some(dec("48000")));
    assert_eq!(c.ga, Some(dec("18000")));
    close(c.sm_pct, 33000.0 / 66550.0 * 100.0);
    assert_eq!(c.cac, Some(dec("33000")));
    close(
        c.cac_payback_months,
        33000.0 / (2600.0 / 12.0 * (54480.0 / 66550.0)),
    );
    assert_eq!(c.deferred_revenue, Some(dec("10400")));
    assert_eq!(c.billings, Some(dec("68950")));
    close(c.dso_days, 4650.0 / 66550.0 * 365.0);
    close(c.dpo_days, 1170.0 / 111_070.0 * 365.0);
}

#[test]
fn previous_half_metrics_match_the_readme() {
    let report = yoy(&fixture("startup.journal"));
    let p = &report.previous;
    assert_eq!(p.month.as_deref(), Some("2025-06"));
    assert_eq!(p.mrr, Some(dec("3600")));
    assert_eq!(p.arr, Some(dec("43200")));
    // MRR(m − k) is 2024-06, before the journal: no growth rates.
    assert_eq!(p.mrr_growth_pct, None);
    assert_eq!(p.cmgr_pct, None);
    assert_eq!(p.net_new_arr, Some(dec("43200")));
    assert_eq!(
        p.bridge,
        Some(bridge("4200", "0", "0", "1200", "600", "0", (9, 2, 1)))
    );
    assert_eq!(p.customers, Some(8));
    assert_eq!(p.customer_growth_pct, None);
    assert_eq!(p.logo_churn_pct, None);
    assert_eq!(p.arpa, Some(dec("450")));
    assert_eq!(p.acv, Some(dec("5600")));
    // m − 12 is before the journal's first transaction.
    assert_eq!(p.nrr_pct, None);
    assert_eq!(p.grr_pct, None);
    close(p.quick_ratio, 4.0);
    close(p.top1_pct, 1000.0 / 3600.0 * 100.0);
    assert_eq!(p.revenue, dec("18450"));
    assert_eq!(p.gross_profit, Some(dec("14760")));
    close(p.gross_margin_pct, 80.0);
    assert_eq!(p.net_burn, dec("24120"));
    assert_eq!(p.net_burn_avg3, Some(dec("6576.67")));
    assert_eq!(p.cash, Some(dec("475880")));
    assert_eq!(p.cash_out_date.as_deref(), Some("2033-12-01"));
    assert_eq!(p.net_burn_adjusted, Some(dec("29520")));
    close(p.burn_multiple, 29520.0 / 43200.0);
    assert_eq!(p.burn_multiple_basis, Some(RunwayBasis::Adjusted));
    assert_eq!(p.rule_of_40, None, "no revenue before the previous half");
    assert_eq!(p.cac, Some(dec("1333.33")));
    assert_eq!(p.deferred_revenue, Some(dec("8000")));
    assert_eq!(p.billings, Some(dec("26450")));
    close(p.dso_days, 2600.0 / 18450.0 * 365.0);
}

#[test]
fn segments_bridge_customers_and_concentration() {
    let report = yoy(&fixture("startup.journal"));
    assert_eq!(report.available.dimensions, ["segment", "product", "class"]);
    let segment = &report.segments[0];
    assert_eq!(segment.key, "segment");
    let rows: Vec<(&str, Dec)> = segment
        .rows
        .iter()
        .map(|row| (row.value.as_str(), row.mrr_curr))
        .collect();
    assert_eq!(
        rows,
        [
            ("mid", dec("2900")),
            ("enterprise", dec("2200")),
            ("smb", dec("750"))
        ]
    );
    let smb = &segment.rows[2];
    close(smb.nrr_pct, 50.0);
    // Orbit's move: churn in smb, new in enterprise; expansion at the total.
    let smb_bridge = smb.bridge.expect("tier C");
    assert_eq!(smb_bridge.churn, dec("1200")); // Fjord 350 + Lumen 250 + Pinecrest 200 + Orbit 400
    let enterprise = segment.rows[1].bridge.expect("tier C");
    assert_eq!(enterprise.new, dec("900"));
    assert_eq!(enterprise.new_logos, 1);
    // Class: unassigned last.
    let class = report.segments.iter().find(|s| s.key == "class").unwrap();
    assert_eq!(class.rows.last().unwrap().value, "(unassigned)");

    let moves = report.bridge_customers.expect("tier C");
    let churned: Vec<&str> = moves.churn.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(churned, ["Fjord Studio", "Lumen Labs", "Pinecrest Dental"]);
    assert_eq!(moves.churn[2].segment.as_deref(), Some("smb"));
    assert_eq!(moves.new[0].name, "Granite Health");
    assert_eq!(moves.expansion[0].segment.as_deref(), Some("enterprise"));
    assert_eq!(report.top_customers[0].name, "Granite Health");
    assert_eq!(report.top_customers.len(), 8);
}

// ---------------------------------------------------------------------------
// Invariants
// ---------------------------------------------------------------------------

/// The bridge identity, Σ C + unattributed = MRR, and customers = |{C > 0}|
/// in every month; GRR within [0, 100] and ≤ NRR in both halves.
fn assert_invariants(report: &SaasReport) {
    let mut previous_mrr: Option<Dec> = None;
    for (i, month) in report.months.iter().enumerate() {
        if let (Some(bridge), Some(prev)) = (month.bridge, previous_mrr) {
            let rebuilt = prev
                .add(bridge.new)
                .and_then(|v| v.add(bridge.expansion))
                .and_then(|v| v.sub(bridge.contraction))
                .and_then(|v| v.sub(bridge.churn))
                .and_then(|v| v.add(bridge.reactivation))
                .and_then(|v| v.add(bridge.other))
                .unwrap();
            assert_eq!(rebuilt, month.mrr, "bridge identity in {}", month.month);
        }
        previous_mrr = Some(month.mrr);
        if let Some(matrix) = &report.customer_matrix {
            let attributed = matrix
                .iter()
                .try_fold(Dec::zero(), |acc, row| acc.add(row.values[i]))
                .unwrap();
            assert_eq!(
                attributed.add(month.unattributed).unwrap(),
                month.mrr,
                "Σ C + unattributed = MRR in {}",
                month.month
            );
            let positive = matrix
                .iter()
                .filter(|row| row.values[i] > Dec::zero())
                .count();
            assert_eq!(month.customers, Some(positive as u32));
        }
    }
    for half in [&report.current, &report.previous] {
        if let (Some(grr), Some(nrr)) = (half.grr_pct, half.nrr_pct) {
            assert!((0.0..=100.0 + 1e-9).contains(&grr), "0 ≤ GRR ≤ 100: {grr}");
            assert!(grr <= nrr + 1e-9, "GRR {grr} ≤ NRR {nrr}");
        }
    }
}

#[test]
fn invariants_hold_on_the_fixture() {
    let journal = fixture("startup.journal");
    assert_invariants(&whole(&journal));
    assert_invariants(&run(&journal, YOY_START, YOY_END, true));
}

/// Revenue-sum identity: over any window, Σ month revenue = the income
/// statement's revenue, and MRR + usage never exceed it on this fixture.
fn assert_revenue_identity(journal: &Journal, from: &str, to: &str) {
    let report = run(journal, from, to, false);
    let declared = declared_types(&account_decls(journal));
    let statement = income_statement(&journal.transactions, from, to, 1, &declared).unwrap();
    let base = Commodity("$".to_string());
    let expected = statement.sections[0]
        .total
        .get(&base)
        .unwrap_or_else(Dec::zero);
    let ours = report
        .months
        .iter()
        .try_fold(Dec::zero(), |acc, m| acc.add(m.revenue))
        .unwrap();
    assert_eq!(ours, expected, "revenue over {from}..{to}");
}

#[test]
fn revenue_sum_identity_against_the_income_statement() {
    let journal = fixture("startup.journal");
    assert_revenue_identity(&journal, "2025-01-01", "2026-06-30");
    assert_revenue_identity(&journal, "2025-07-01", "2025-12-31");
    let base = Commodity("$".to_string());
    let declared = declared_types(&account_decls(&journal));
    let report = whole(&journal);
    for month in &report.months {
        let from = format!("{}-01", month.month);
        let to = format!("{}-{}", month.month, last_day(&month.month));
        let statement = income_statement(&journal.transactions, &from, &to, 1, &declared).unwrap();
        let services = journal
            .transactions
            .iter()
            .flat_map(|t| t.postings.iter().map(move |p| (t, p)))
            .filter(|(t, p)| p.account.0 == "revenue:services" && t.date >= from && t.date <= to)
            .try_fold(Dec::zero(), |acc, (_, p)| acc.sub(p.amounts[0].quantity))
            .unwrap();
        // Σ MRR + Σ usage + non-recurring revenue = income-statement revenue.
        let sum = month.mrr.add(month.usage).unwrap().add(services).unwrap();
        assert_eq!(
            sum,
            statement.sections[0]
                .total
                .get(&base)
                .unwrap_or_else(Dec::zero),
            "{}",
            month.month
        );
    }
}

fn last_day(month: &str) -> &'static str {
    match &month[5..7] {
        "02" => "28",
        "04" | "06" | "09" | "11" => "30",
        _ => "31",
    }
}

#[test]
fn virtual_auto_and_dated_postings_follow_the_income_statement() {
    let text = r"
account assets:bank       ; type: C
account revenue:subs      ; type: R, kpi: recurring

= revenue:subs
    (revenue:subs:bonus)    *0.1

2026-01-15 Acme | Invoice
    assets:bank           $100.00
    revenue:subs         -$100.00  ; customer: Acme

2026-01-20 Beta | Invoice
    assets:bank           $50.00
    revenue:subs         -$50.00  ; customer: Beta, date: 2026-02-03

2026-02-10 Adjustment
    (revenue:subs)        -$7.00
";
    let journal = parse(text);
    let report = run(&journal, "2026-01-01", "2026-02-28", false);
    // Acme 100 and Beta 50 with their 10% auto legs, the virtual 7 and its own
    // auto leg 0.70. Beta's `date:` moves its 50 into February.
    let total = report.months[0].mrr.add(report.months[1].mrr).unwrap();
    assert_eq!(total, dec("172.70"));
    assert!(
        report.months[1].mrr >= dec("57"),
        "Beta's dated posting is in February"
    );
    assert_revenue_identity(&journal, "2026-01-01", "2026-02-28");
    assert_revenue_identity(&journal, "2026-02-01", "2026-02-28");
}

#[test]
fn net_burn_is_minus_operating_plus_investing() {
    let journal = fixture("startup.journal");
    let report = whole(&journal);
    let statement = cash_flow_statement(
        &journal.transactions,
        &CfOpts {
            from: "2025-01-01",
            to: "2026-06-30",
            interval: Some(Interval::Monthly),
            depth: 1,
        },
        &CfTags::from_journal(&journal),
    )
    .unwrap();
    let base = Commodity("$".to_string());
    let section = |kind: CfSectionKind, i: usize| {
        statement
            .sections
            .iter()
            .find(|s| s.kind == kind)
            .and_then(|s| s.total.values[i].get(&base))
            .unwrap_or_else(Dec::zero)
    };
    for (i, month) in report.months.iter().enumerate() {
        let expected = section(CfSectionKind::Operating, i)
            .add(section(CfSectionKind::Investing, i))
            .unwrap()
            .neg()
            .unwrap();
        assert_eq!(month.net_burn, expected, "{}", month.month);
    }
    // The funding round is financing, not negative burn.
    assert_eq!(report.months[0].net_burn, dec("5150"));
}

/// One random customer posting: (customer index or untagged, segment, amount
/// in dollars, possibly negative or zero).
type Cell = (Option<u8>, u8, i32);

fn journal_from(cells: &[Vec<Cell>]) -> String {
    let mut text = String::from(
        "account assets:receivable  ; type: A, kpi: receivable\n\
         account revenue:subs  ; type: R, kpi: recurring\n\n",
    );
    for (month, postings) in cells.iter().enumerate() {
        let date = format!("{}-{:02}-05", 2024 + month / 12, month % 12 + 1);
        for (customer, segment, amount) in postings {
            let tags = match customer {
                Some(c) => format!("customer: C{c}, segment: s{segment}"),
                None => format!("segment: s{segment}"),
            };
            // Two recurring legs so an untagged posting is never inferred.
            let (debit, credit) = (money(*amount), money(-amount));
            text.push_str(&format!(
                "{date} | Charge\n    assets:receivable  {debit}\n    revenue:subs  {credit}  ; {tags}\n    revenue:subs  $0.00\n\n"
            ));
        }
    }
    text
}

fn money(dollars: i32) -> String {
    if dollars < 0 {
        format!("-${}.00", -dollars)
    } else {
        format!("${dollars}.00")
    }
}

fn cells_strategy() -> impl Strategy<Value = Vec<Vec<Cell>>> {
    let cell = (
        prop_oneof![3 => (0u8..6).prop_map(Some), 1 => Just(None)],
        0u8..3,
        prop_oneof![1 => Just(0i32), 1 => -80i32..0, 4 => 1i32..400],
    );
    prop::collection::vec(prop::collection::vec(cell, 0..6), 2..16)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn invariants_hold_on_random_books(cells in cells_strategy()) {
        let journal = parse(&journal_from(&cells));
        let months = cells.len();
        let end_month = months - 1;
        let end = format!("{}-{:02}-{}", 2024 + end_month / 12, end_month % 12 + 1,
            last_day(&format!("2000-{:02}", end_month % 12 + 1)));
        let dims = vec!["segment".to_string()];
        let report = saas_metrics(&journal, &SaasOpts {
            start: "2024-01-01",
            end: &end,
            dims: &dims,
            coverage_min: Dec::zero(),
            top_n: 5,
            with_customer_matrix: true,
            as_of: None,
        }).unwrap();
        if report.available.customers {
            assert_invariants(&report);
            // With no expansion against m − 12, NRR = GRR.
            if let (Some(nrr), Some(grr)) = (report.current.nrr_pct, report.current.grr_pct) {
                let matrix = report.customer_matrix.as_ref().unwrap();
                let m = report.months.iter().position(|mo| Some(&mo.month) == report.current.month.as_ref()).unwrap();
                let expanded = m >= 12 && matrix.iter().any(|row| {
                    row.values[m - 12] > Dec::zero() && row.values[m] > row.values[m - 12]
                });
                if !expanded {
                    prop_assert!((nrr - grr).abs() < 1e-9);
                }
            }
        }
        // Σ segments' MRR = total MRR.
        if let (Some(breakdown), Some(mrr)) = (report.segments.first(), report.current.mrr) {
            let total = breakdown.rows.iter().try_fold(Dec::zero(), |acc, row| acc.add(row.mrr_curr)).unwrap();
            prop_assert_eq!(total, mrr);
        }
    }
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

#[test]
fn sample_journal_has_nothing_available() {
    let report = yoy(&common::fixture_journal());
    let available = &report.available;
    assert!(!available.recurring && !available.customers && !available.business);
    assert!(available.dimensions.is_empty());
    assert_eq!(available.customers_hidden_reason, None);
    assert!(!report.months.is_empty());
    assert_eq!(report.current.mrr, None);
    assert_eq!(report.current.bridge, None);
    assert!(report.segments.is_empty() && report.top_customers.is_empty());
}

#[test]
fn startup_has_everything() {
    let available = yoy(&fixture("startup.journal")).available;
    assert!(available.recurring && available.customers && available.business && available.cogs);
    assert_eq!(available.customers_hidden_reason, None);
    close(available.coverage_pct, 78_600.0 / 78_800.0 * 100.0);
    assert!(!available.inferred_customers);
}

#[test]
fn clearing_books_hide_customers_as_billing_clearing() {
    let report = run(
        &fixture("clearing.journal"),
        "2025-01-01",
        "2025-03-31",
        false,
    );
    let mrr: Vec<Dec> = report.months.iter().map(|m| m.mrr).collect();
    assert_eq!(mrr, [dec("2100"), dec("2100"), dec("2100")]);
    assert!(report.available.recurring);
    assert!(!report.available.customers);
    assert_eq!(
        report.available.customers_hidden_reason,
        Some(HiddenReason::BillingClearing)
    );
    assert_eq!(report.bridge_customers, None);
}

#[test]
fn payee_inference_attributes_invoices_but_never_the_recognition_entry() {
    let report = run(&fixture("payees.journal"), "2025-01-01", "2025-03-31", true);
    assert!(report.available.customers);
    assert!(report.available.inferred_customers);
    let mrr: Vec<Dec> = report.months.iter().map(|m| m.mrr).collect();
    assert_eq!(mrr, [dec("850"), dec("950"), dec("550")]);
    let unattributed: Vec<Dec> = report.months.iter().map(|m| m.unattributed).collect();
    assert_eq!(unattributed, [dec("50"), dec("50"), dec("50")]);
    let names: Vec<&str> = report
        .customer_matrix
        .as_ref()
        .unwrap()
        .iter()
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(names, ["Aspen Dental", "Cedar Studio"]);
    assert!(
        report
            .bridge_customers
            .unwrap()
            .new
            .iter()
            .all(|m| m.inferred)
    );
}

/// Strip `customer:` from `who`'s lines and blank their payees, so neither
/// the tag nor the payee fallback attributes them.
fn strip(text: &str, who: &[&str]) -> String {
    text.lines()
        .map(|line| {
            let mut line = line.to_string();
            for name in who {
                line = line.replace(&format!("customer: {name}"), &format!("memo: {name}"));
                if line.starts_with(|c: char| c.is_ascii_digit()) {
                    line = line.replace(&format!(" {name} |"), " |");
                }
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn stripping_every_customer_reports_no_customer_tags() {
    let everyone = [
        "Bluebird Analytics",
        "Northwind Logistics",
        "Pinecrest Dental",
        "Harbor Yoga",
        "Lumen Labs",
        "Granite Health",
        "Orbit Freight",
        "Fjord Studio",
        "Contoso Ltd.",
        "Contoso Ltd",
    ];
    let report = yoy(&parse(&strip(&fixture_text("startup.journal"), &everyone)));
    assert!(report.available.recurring);
    assert!(!report.available.customers);
    assert_eq!(
        report.available.customers_hidden_reason,
        Some(HiddenReason::NoCustomerTags)
    );
    close(report.available.coverage_pct, 0.0);
}

#[test]
fn stripping_a_fifth_reports_low_coverage() {
    // Granite (13,500) + Pinecrest (1,800) of 78,800, plus the 200 untagged.
    let report = yoy(&parse(&strip(
        &fixture_text("startup.journal"),
        &["Granite Health", "Pinecrest Dental"],
    )));
    assert_eq!(
        report.available.customers_hidden_reason,
        Some(HiddenReason::LowCoverage)
    );
    close(report.available.coverage_pct, 63_300.0 / 78_800.0 * 100.0);
    assert_eq!(report.current.customers, None);
    assert!(report.current.mrr.is_some());
}

// ---------------------------------------------------------------------------
// Edges
// ---------------------------------------------------------------------------

#[test]
fn a_span_shorter_than_a_month_has_no_point_metrics() {
    let report = run(
        &fixture("startup.journal"),
        "2026-06-05",
        "2026-06-20",
        false,
    );
    assert_eq!(report.current.month, None);
    assert_eq!(report.current.mrr, None);
    assert_eq!(report.current.cash, None);
    assert_eq!(report.previous.runway_months, None);
    assert!(report.months[0].partial);
}

#[test]
fn a_journal_starting_inside_the_span_has_no_previous_nrr() {
    let report = whole(&fixture("startup.journal"));
    assert_eq!(report.previous.month.as_deref(), Some("2025-09"));
    assert_eq!(report.previous.nrr_pct, None);
    assert!(report.current.nrr_pct.is_some());
}

const SMALL: &str = r"
account assets:bank     ; type: C
account revenue:subs    ; type: R, kpi: recurring
account expenses:ops    ; type: X
";

#[test]
fn burning_while_mrr_shrinks_has_no_burn_multiple() {
    let text = format!(
        "{SMALL}
2026-01-05 Acme | Invoice
    assets:bank   $500.00
    revenue:subs  -$500.00  ; customer: Acme
2026-01-06 Rent
    expenses:ops  $900.00
    assets:bank   -$900.00
2026-02-05 Acme | Invoice
    assets:bank   $300.00
    revenue:subs  -$300.00  ; customer: Acme
2026-02-06 Rent
    expenses:ops  $900.00
    assets:bank   -$900.00
"
    );
    let report = run(&parse(&text), "2026-01-01", "2026-02-28", false);
    assert!(report.current.net_burn > Dec::zero());
    assert!(report.current.net_new_arr.unwrap() < Dec::zero());
    assert_eq!(report.current.burn_multiple, None);
    assert!(report.current.runway_months.is_some());
}

#[test]
fn a_profitable_month_has_no_runway() {
    let text = format!(
        "{SMALL}
2026-01-05 Acme | Invoice
    assets:bank   $500.00
    revenue:subs  -$500.00  ; customer: Acme
2026-02-05 Acme | Invoice
    assets:bank   $500.00
    revenue:subs  -$500.00  ; customer: Acme
"
    );
    let report = run(&parse(&text), "2026-01-01", "2026-02-28", false);
    assert!(report.current.net_burn < Dec::zero());
    assert_eq!(report.current.runway_months, None);
    assert_eq!(report.current.cash_out_date, None);
    assert_eq!(report.current.burn_multiple, None);
}

#[test]
fn foreign_recurring_revenue_is_valued_or_reported_unvalued() {
    let invoices = "
2026-01-05 Acme | Invoice
    assets:bank   $500.00
    revenue:subs  -$500.00  ; customer: Acme
2026-01-06 Euro | Invoice
    assets:bank   100 EUR
    revenue:subs  -100 EUR  ; customer: Euro
";
    let unpriced = parse(&format!("{SMALL}{invoices}"));
    let report = run(&unpriced, "2026-01-01", "2026-01-31", false);
    assert_eq!(report.months[0].mrr, dec("500"));
    assert_eq!(
        report.unvalued.get(&Commodity("EUR".to_string())),
        Some(dec("100"))
    );
    let priced = parse(&format!("{SMALL}P 2026-01-01 EUR $1.10\n{invoices}"));
    let report = run(&priced, "2026-01-01", "2026-01-31", false);
    assert_eq!(report.months[0].mrr, dec("610"));
    assert!(report.unvalued.is_empty());
}

// ---------------------------------------------------------------------------
// Top Transactions skip rule
// ---------------------------------------------------------------------------

/// Box 10 over a one-month current half, as `(description, amount)`.
fn top_txns(body: &str) -> Vec<(String, Dec)> {
    let text = format!(
        "account assets:bank                   ; type: C
account assets:receivable             ; type: A, kpi: receivable
account liabilities:card              ; type: L
account liabilities:deferred-revenue  ; type: L, kpi: deferred-revenue
account revenue:subs                  ; type: R, kpi: recurring
account revenue:salary                ; type: R
account expenses:food                 ; type: X
account expenses:taxes                ; type: X
account equity:retained               ; type: E
{body}"
    );
    let report = insights(
        &parse(&text),
        &InsightsOpts {
            start: "2025-12-01",
            end: "2026-01-31",
            cost_exclude: &[],
            change_min: Dec::zero(),
        },
    )
    .unwrap();
    report
        .top_txns
        .iter()
        .map(|t| (t.description.clone(), t.amount))
        .collect()
}

fn one(description: &str, amount: &str) -> Vec<(String, Dec)> {
    vec![(description.to_string(), dec(amount))]
}

#[test]
fn top_transactions_show_a_card_purchase_as_its_expense() {
    let found = top_txns(
        "2026-01-05 Grocer
    expenses:food        $87.20
    liabilities:card    -$87.20
",
    );
    assert_eq!(found, one("Grocer", "87.20"));
}

#[test]
fn top_transactions_show_a_paycheck_as_its_gross_revenue() {
    let found = top_txns(
        "2026-01-27 Acme | Salary
    revenue:salary     -$5,000.00
    expenses:taxes        $900.00
    assets:bank         $4,100.00
",
    );
    assert_eq!(found, one("Acme | Salary", "5000"));
}

#[test]
fn top_transactions_drop_payments_moves_and_closing_entries() {
    let found = top_txns(
        "2026-01-03 Acme | Payment
    assets:bank            $9,000.00
    assets:receivable     -$9,000.00

2026-01-04 Pure move
    revenue:subs                   $9,000.00
    liabilities:deferred-revenue  -$9,000.00

2026-01-05 Prepayment
    assets:bank                    $7,000.00
    liabilities:deferred-revenue  -$7,000.00

2026-01-31 Closing
    revenue:salary         $5,000.00
    equity:retained       -$5,000.00
",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn top_transactions_show_an_invoice_as_its_revenue() {
    let found = top_txns(
        "2026-01-02 Acme | Invoice
    assets:receivable   $9,000.00
    revenue:subs       -$9,000.00
",
    );
    assert_eq!(found, one("Acme | Invoice", "9000"));
}

#[test]
fn top_transactions_show_a_combined_entry_as_only_what_it_recognises() {
    // Invoice booked to revenue, moved to deferred, one month recognised.
    let found = top_txns(
        "2026-01-31 Combined recognise and move
    revenue:subs                   $1,200.00
    liabilities:deferred-revenue  -$1,200.00
    liabilities:deferred-revenue     $100.00
    revenue:subs                    -$100.00
",
    );
    assert_eq!(found, one("Combined recognise and move", "100"));
}

// ---------------------------------------------------------------------------
// Review fixes: billing clearing, as-of, inferred per month
// ---------------------------------------------------------------------------

/// Correctly booked deferred revenue (invoice to deferred, recognition from
/// deferred with `customer:`) with one small correction debit inside the span
/// and one outside it. The corrections are moves to deferred, but no customer
/// revenue sits on a clearing posting, so customers stay available.
#[test]
fn well_booked_deferred_revenue_with_corrections_keeps_customers() {
    let mut text = String::from(
        "account assets:receivable  ; type: A, kpi: receivable
account liabilities:deferred-revenue  ; type: L, kpi: deferred-revenue
account revenue:subs  ; type: R, kpi: recurring

2024-06-15 Correction before the span
    revenue:subs                     $10.00
    liabilities:deferred-revenue    -$10.00

2025-01-01 Acme | Annual invoice
    assets:receivable              $1,200.00
    liabilities:deferred-revenue  -$1,200.00  ; customer: Acme

2025-01-01 Beta | Annual invoice
    assets:receivable                $600.00
    liabilities:deferred-revenue    -$600.00  ; customer: Beta

2025-03-20 Correction in the span
    revenue:subs                      $5.00
    liabilities:deferred-revenue     -$5.00
",
    );
    for month in 1..=6 {
        text.push_str(&format!(
            "
2025-{month:02}-28 Revenue recognition
    liabilities:deferred-revenue    $150.00
    revenue:subs                   -$100.00  ; customer: Acme
    revenue:subs                    -$50.00  ; customer: Beta
"
        ));
    }
    let report = run(&parse(&text), "2025-01-01", "2025-06-30", false);
    assert!(report.available.recurring);
    assert_eq!(report.available.customers_hidden_reason, None);
    assert!(report.available.customers);
    assert_eq!(report.current.customers, Some(2));
}

/// The clearing fixture with `customer:` moved from the invoices onto the
/// recognition lines is correctly attributed MRR: customers are available,
/// and the untagged invoices are not attributed to their payee ("Invoice").
#[test]
fn clearing_books_with_customers_on_recognition_lines_show_customers() {
    let text = fixture_text("clearing.journal")
        .replace("  ; customer: Alder Bakery", "")
        .replace("  ; customer: Birchway Freight", "")
        .replace(
            "    revenue:sales                        -$100.00",
            "    revenue:sales                        -$100.00  ; customer: Alder Bakery",
        )
        .replace(
            "    revenue:sales:enterprise           -$2,000.00",
            "    revenue:sales:enterprise           -$2,000.00  ; customer: Birchway Freight",
        );
    assert_eq!(
        text.matches("customer: Alder Bakery").count(),
        3,
        "recognition lines tagged"
    );
    let report = run(&parse(&text), "2025-01-01", "2025-03-31", true);
    assert_eq!(report.available.customers_hidden_reason, None);
    assert!(report.available.customers);
    assert!(!report.available.inferred_customers);
    let names: Vec<&str> = report
        .customer_matrix
        .as_ref()
        .unwrap()
        .iter()
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(names, ["Alder Bakery", "Birchway Freight"]);
}

/// A span reaching past "today": a month only counts once it has finished.
#[test]
fn as_of_stops_the_half_at_the_last_finished_month() {
    // Books closed through 2026-12, so only as-of limits the half.
    let mut text = fixture_text("startup.journal");
    for month in 7..=12 {
        text.push_str(&format!(
            "
2026-{month:02}-01 Bluebird Analytics | Invoice 2026-{month:02}
    assets:receivable                        $800.00
    revenue:subscriptions:mid               -$800.00  ; customer: Bluebird Analytics
"
        ));
    }
    let journal = parse(&text);
    let report = run_as_of(
        &journal,
        "2025-01-01",
        "2026-12-31",
        false,
        Some("2026-11-10"),
    );
    assert_eq!(report.current.month.as_deref(), Some("2026-10"));
    assert_eq!(report.current.months, 10);
    let partial: Vec<&str> = report
        .months
        .iter()
        .filter(|m| m.partial)
        .map(|m| m.month.as_str())
        .collect();
    assert_eq!(partial, ["2026-11", "2026-12"]);
    assert!(report.months.iter().all(|m| !m.open));
    assert_eq!(report.available.closed_through.as_deref(), Some("2026-12"));
    // Without it, the span's own end decides.
    let unbounded = run(&journal, "2025-01-01", "2026-12-31", false);
    assert_eq!(unbounded.current.month.as_deref(), Some("2026-12"));
    // An as-of after the span's end changes nothing.
    let startup = fixture("startup.journal");
    assert_eq!(
        run_as_of(&startup, YOY_START, YOY_END, false, Some("2026-07-08")),
        yoy(&startup)
    );
}

/// Payee-inferred in 2024, tagged from 2025: the current half's moves are
/// measured on tagged months, so they are not flagged as inferred.
#[test]
fn inferred_is_judged_per_month() {
    let mut text = String::from(
        "account assets:receivable  ; type: A, kpi: receivable
account revenue:subs  ; type: R, kpi: recurring
",
    );
    for year in [2024, 2025] {
        for month in 1..=12 {
            let (amount, tag) = match (year, month) {
                (2024, _) => ("100", String::new()),
                (_, m) if m < 6 => ("100", "  ; customer: Acme".to_string()),
                _ => ("150", "  ; customer: Acme".to_string()),
            };
            text.push_str(&format!(
                "
{year}-{month:02}-01 Acme | Invoice
    assets:receivable   ${amount}.00
    revenue:subs       -${amount}.00{tag}
"
            ));
        }
    }
    let report = run(&parse(&text), "2024-01-01", "2025-12-31", false);
    assert!(report.available.customers);
    assert!(!report.available.inferred_customers);
    let moves = report.bridge_customers.expect("tier C");
    assert_eq!(moves.expansion.len(), 1);
    assert!(!moves.expansion[0].inferred);
    // The previous half is all inferred; its moves would be.
    let earlier = run(&parse(&text), "2023-01-01", "2024-12-31", false);
    assert!(earlier.available.inferred_customers);
    assert!(earlier.bridge_customers.unwrap().new[0].inferred);
}

// ---------------------------------------------------------------------------
// Amendment A1.1: burn adjusted for prepayments and receivables
// ---------------------------------------------------------------------------

#[test]
fn adjusted_burn_matches_the_readme_and_smooths_the_prepayment() {
    let report = whole(&fixture("startup.journal"));
    let adjusted: Vec<Dec> = report
        .months
        .iter()
        .map(|m| m.net_burn_adjusted.expect("deferred and receivable roles"))
        .collect();
    let expected = [
        "5150", "5620", "4620", "4720", "4840", "4570", "4220", "4780", "4430", "-820", "4590",
        "4310", "3860", "3960", "3660", "3620", "3740", "3720",
    ]
    .map(dec);
    assert_eq!(adjusted, expected);
    // 2025-03: Northwind's $12,000 prepayment lands as cash. Raw burn swings
    // to −8,730; adjusted burn stays in line with its neighbours.
    assert_eq!(report.months[2].net_burn, dec("-8730"));
    assert_eq!(report.months[2].net_burn_adjusted, Some(dec("4620")));
}

#[test]
fn adjusted_burn_identity_holds_over_any_range() {
    let report = whole(&fixture("startup.journal"));
    let months = &report.months;
    for first in 0..months.len() {
        for last in first..months.len() {
            let sum = |pick: &dyn Fn(usize) -> Dec| {
                (first..=last)
                    .try_fold(Dec::zero(), |acc, i| acc.add(pick(i)))
                    .unwrap()
            };
            let adjusted = sum(&|i| months[i].net_burn_adjusted.unwrap());
            let raw = sum(&|i| months[i].net_burn);
            let balance =
                |i: Option<usize>, pick: &dyn Fn(usize) -> Dec| i.map_or_else(Dec::zero, pick);
            let start = first.checked_sub(1);
            let deferred = |i: usize| months[i].deferred_end.unwrap();
            let ar = |i: usize| months[i].ar_end.unwrap();
            let expected = raw
                .add(deferred(last))
                .and_then(|v| v.sub(balance(start, &deferred)))
                .and_then(|v| v.sub(ar(last)))
                .and_then(|v| v.add(balance(start, &ar)))
                .unwrap();
            assert_eq!(adjusted, expected, "months {first}..={last}");
        }
    }
}

#[test]
fn runway_uses_adjusted_burn_when_it_exists() {
    let report = yoy(&fixture("startup.journal"));
    let c = &report.current;
    assert_eq!(c.runway_basis, Some(RunwayBasis::Adjusted));
    assert_eq!(c.net_burn_avg3, Some(dec("6543.33")));
    assert_eq!(c.net_burn_adjusted_avg3, Some(dec("3693.33")));
    close(c.runway_months, 432_160.0 * 3.0 / 11_080.0);
    assert_eq!(c.cash_out_date.as_deref(), Some("2036-03-31"));
    // A3: so does the burn multiple; raw net burn is still reported.
    close(c.burn_multiple, 44070.0 / 27000.0);
    assert_eq!(c.net_burn, dec("43720"));
    let p = &report.previous;
    assert_eq!(p.net_burn_adjusted_avg3, Some(dec("4710")));
    close(p.runway_months, 475_880.0 * 3.0 / 14_130.0);
    assert_eq!(p.cash_out_date.as_deref(), Some("2033-12-01"));
}

#[test]
fn runway_falls_back_to_cash_burn_without_deferred_or_receivables() {
    let text = format!(
        "{SMALL}
2026-01-05 Acme | Invoice
    assets:bank   $500.00
    revenue:subs  -$500.00  ; customer: Acme
2026-01-06 Rent
    expenses:ops  $900.00
    assets:bank   -$900.00
"
    );
    let report = run(&parse(&text), "2025-12-01", "2026-01-31", false);
    assert_eq!(report.current.runway_basis, Some(RunwayBasis::Cash));
    assert_eq!(report.current.net_burn_adjusted_avg3, None);
    assert!(report.months.iter().all(|m| m.net_burn_adjusted.is_none()));
}

// ---------------------------------------------------------------------------
// Amendment A1.2: customers from billings
// ---------------------------------------------------------------------------

#[test]
fn clearing_books_count_customers_from_billings() {
    let report = run(
        &fixture("clearing.journal"),
        "2025-01-01",
        "2025-02-28",
        false,
    );
    assert!(!report.available.customers);
    assert!(report.available.billing_customers);
    let current = report.current.billing_customers.expect("from billings");
    assert_eq!(current.customers, 2);
    close(current.customer_growth_pct, 0.0);
    assert_eq!((current.new_logos, current.lapsed_logos), (0, 0));
    close(current.logo_churn_pct, 0.0);
    assert_eq!(current.acv, Some(dec("12600")));
    assert!(current.window_short);
    assert!(!current.inferred);
    let previous = report.previous.billing_customers.expect("from billings");
    assert_eq!(previous.customers, 2);
    assert_eq!(previous.new_logos, 2);
    assert_eq!(previous.customer_growth_pct, None);
    // The MRR customer metrics stay hidden.
    assert_eq!(report.current.customers, None);
    assert_eq!(report.current.nrr_pct, None);
    assert_eq!(report.current.bridge, None);
}

/// Invoices to deferred revenue (the billings) and untagged recognition.
fn deferred_books(invoices: &[(&str, &str, &str)]) -> String {
    let mut text = String::from(
        "account assets:receivable             ; type: A, kpi: receivable
account liabilities:deferred-revenue  ; type: L, kpi: deferred-revenue
account revenue:subs                  ; type: R, kpi: recurring
",
    );
    for (date, description, tags) in invoices {
        text.push_str(&format!(
            "
{date} {description}
    assets:receivable              $1,200.00
    liabilities:deferred-revenue  -$1,200.00{tags}
"
        ));
    }
    for year in [2024, 2025] {
        for month in 1..=12 {
            text.push_str(&format!(
                "
{year}-{month:02}-28 Revenue recognition
    liabilities:deferred-revenue  $100.00
    revenue:subs                 -$100.00
"
            ));
        }
    }
    text
}

#[test]
fn a_customer_lapses_twelve_months_after_its_last_billing() {
    let mut invoices = vec![("2024-01-15", "Invoice | Alpha", "  ; customer: Alpha")];
    let months: Vec<String> = (1..=12)
        .map(|m| format!("2024-{m:02}-10"))
        .chain((1..=6).map(|m| format!("2025-{m:02}-10")))
        .collect();
    for date in &months {
        invoices.push((date.as_str(), "Invoice | Beta", "  ; customer: Beta"));
    }
    let report = run(
        &parse(&deferred_books(&invoices)),
        "2024-07-01",
        "2025-06-30",
        false,
    );
    assert!(report.available.billing_customers);
    let previous = report.previous.billing_customers.unwrap();
    assert_eq!(previous.customers, 2, "Alpha is active through 2024-12");
    let current = report.current.billing_customers.unwrap();
    assert_eq!(current.customers, 1);
    assert_eq!(current.lapsed_logos, 1, "Alpha lapses in 2025-01");
    close(current.logo_churn_pct, 50.0);
    assert!(!current.window_short);
}

#[test]
fn payee_inferred_billings_set_the_inferred_flag() {
    let report = run(
        &parse(&deferred_books(&[("2025-01-10", "Zeta Co | Invoice", "")])),
        "2024-07-01",
        "2025-06-30",
        false,
    );
    let current = report.current.billing_customers.expect("from billings");
    assert_eq!(current.customers, 1);
    assert!(current.inferred);
    assert!(report.available.inferred_customers);
}

#[test]
fn billing_customers_hide_when_nothing_is_billed() {
    let report = run(
        &parse(&deferred_books(&[])),
        "2024-07-01",
        "2025-06-30",
        false,
    );
    assert!(report.available.recurring);
    assert!(!report.available.billing_customers);
    assert_eq!(report.current.billing_customers, None);
}

#[test]
fn billing_customers_never_show_beside_mrr_customers() {
    let report = yoy(&fixture("startup.journal"));
    assert!(report.available.customers);
    assert!(!report.available.billing_customers);
    assert_eq!(report.current.billing_customers, None);
}

// ---------------------------------------------------------------------------
// Amendment A2: the data horizon
// ---------------------------------------------------------------------------

/// A span running past the books' last close: months after the last
/// recurring posting are open, never complete, so m stays at the last closed
/// month instead of reading the unclosed months as total churn.
#[test]
fn months_after_the_last_recurring_posting_are_open_not_churn() {
    let journal = fixture("startup.journal");
    let report = run_as_of(
        &journal,
        "2024-11-01",
        "2026-10-31",
        false,
        Some("2026-11-10"),
    );
    assert_eq!(report.available.closed_through.as_deref(), Some("2026-06"));
    let c = &report.current;
    assert_eq!(c.month.as_deref(), Some("2026-06"));
    let open: Vec<&str> = report
        .months
        .iter()
        .filter(|m| m.open)
        .map(|m| m.month.as_str())
        .collect();
    assert_eq!(open, ["2026-07", "2026-08", "2026-09", "2026-10"]);
    assert!(report.months.iter().filter(|m| m.open).all(|m| m.partial));
    // Point-in-time figures are the 2026-06 ones, as over the YoY span.
    let closed = yoy(&journal).current;
    assert_eq!(c.mrr, closed.mrr);
    assert_eq!(c.arr, closed.arr);
    assert_eq!(c.customers, closed.customers);
    assert_eq!(c.nrr_pct, closed.nrr_pct);
    assert_eq!(c.grr_pct, closed.grr_pct);
    assert_eq!(c.top1_pct, closed.top1_pct);
    assert_eq!(c.cash, closed.cash);
    assert_eq!(c.deferred_revenue, closed.deferred_revenue);
    assert_eq!(c.runway_months, closed.runway_months);
    // No phantom churn: the half (2025-11 … 2026-06) churns nobody.
    assert_eq!(c.logo_churn_pct, Some(0.0));
}

#[test]
fn a_journal_whose_last_recognition_is_mid_span_stops_there() {
    let mut text = String::from(SMALL);
    for month in 1..=3 {
        text.push_str(&format!(
            "
2026-{month:02}-05 Acme | Invoice
    assets:bank   $500.00
    revenue:subs  -$500.00  ; customer: Acme
"
        ));
    }
    let report = run_as_of(
        &parse(&text),
        "2025-07-01",
        "2026-06-30",
        false,
        Some("2026-07-08"),
    );
    assert_eq!(report.available.closed_through.as_deref(), Some("2026-03"));
    assert_eq!(report.current.month.as_deref(), Some("2026-03"));
    assert_eq!(report.current.months, 3);
    assert_eq!(report.current.mrr, Some(dec("500")));
    let open: Vec<&str> = report
        .months
        .iter()
        .filter(|m| m.open)
        .map(|m| m.month.as_str())
        .collect();
    assert_eq!(open, ["2026-04", "2026-05", "2026-06"]);
}

#[test]
fn without_recurring_revenue_there_is_no_horizon() {
    let text = "
account assets:bank     ; type: C
account expenses:ops    ; type: X
account equity:funding  ; type: E

2026-01-02 Seed
    assets:bank       $10,000.00
    equity:funding   -$10,000.00
2026-03-06 Rent
    expenses:ops        $900.00
    assets:bank        -$900.00
";
    let report = run_as_of(
        &parse(text),
        "2025-11-01",
        "2026-10-31",
        false,
        Some("2026-11-10"),
    );
    assert_eq!(report.available.closed_through, None);
    assert!(report.months.iter().all(|m| !m.open));
    assert_eq!(report.current.month.as_deref(), Some("2026-10"));
    assert_eq!(report.current.cash, Some(dec("9100")));
}

// ---------------------------------------------------------------------------
// Amendment A3: burn multiple basis
// ---------------------------------------------------------------------------

#[test]
fn burn_multiple_uses_cash_burn_without_deferred_or_receivables() {
    let text = format!(
        "{SMALL}
2026-01-05 Acme | Invoice
    assets:bank   $500.00
    revenue:subs  -$500.00  ; customer: Acme
2026-01-06 Rent
    expenses:ops  $900.00
    assets:bank   -$900.00
2026-02-05 Acme | Invoice
    assets:bank   $600.00
    revenue:subs  -$600.00  ; customer: Acme
2026-02-06 Rent
    expenses:ops  $900.00
    assets:bank   -$900.00
"
    );
    let report = run(&parse(&text), "2026-01-01", "2026-02-28", false);
    let c = &report.current;
    assert_eq!(c.net_burn_adjusted, None);
    assert_eq!(c.net_burn, dec("300"));
    assert_eq!(c.net_new_arr, Some(dec("1200")));
    close(c.burn_multiple, 300.0 / 1200.0);
    assert_eq!(c.burn_multiple_basis, Some(RunwayBasis::Cash));
}

#[test]
fn burn_multiple_hides_when_adjusted_burn_is_not_positive() {
    // Receivables resolve, so the adjusted basis applies even though the
    // half's raw cash burn is positive: an unpaid invoice earns its revenue.
    let text = "
account assets:bank        ; type: C
account assets:receivable  ; type: A, kpi: receivable
account revenue:subs       ; type: R, kpi: recurring
account expenses:ops       ; type: X

2026-01-05 Acme | Invoice
    assets:receivable   $500.00
    revenue:subs       -$500.00  ; customer: Acme
2026-02-05 Acme | Invoice
    assets:receivable  $2,000.00
    revenue:subs      -$2,000.00  ; customer: Acme
2026-02-06 Rent
    expenses:ops        $900.00
    assets:bank        -$900.00
";
    let report = run(&parse(text), "2026-01-01", "2026-02-28", false);
    let c = &report.current;
    assert_eq!(c.net_burn, dec("900"), "cash went out, none came in");
    assert_eq!(c.net_burn_adjusted, Some(dec("-1100")));
    assert!(c.net_new_arr.unwrap() > Dec::zero());
    assert_eq!(c.burn_multiple, None);
    assert_eq!(c.burn_multiple_basis, None);
}
