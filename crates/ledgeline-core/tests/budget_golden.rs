//! Golden parity for the budget report: the native engine vs. committed hledger
//! `bal -M --budget -O json` output (`fixtures/budget/*.budget.json`, generated
//! by `scripts/gen-budget-golden.sh`).
//!
//! For each fixture we parse the `.journal` with our own parser (exercising the
//! new `~` periodic-rule support), run `budget_report` with the parameters that
//! mirror the script's fixed `-b`/`-e` span, and diff against the golden
//! `PeriodicReport`. Reconciliation matches the other golden suites:
//! - hledger's `-e DATE` is EXCLUSIVE; our `end` is INCLUSIVE, so
//!   `-e 2026-03-01` ≙ `end = 2026-02-28`.
//! - each golden cell is a 2-tuple `[actualMixedAmount, goalMixedAmount|null]`;
//!   a `null` goal ≙ our `BudgetCell::goal == None`.
//! - all number comparisons are on CANONICAL `(mantissa, places)` (trailing
//!   zeros stripped, zero commodities dropped) — never floats.
//! - the FULL row set is compared (including elided-parent survivors and the
//!   synthetic `<unbudgeted>` row), not just the leaves.

mod common;

use ledgeline_core::Dec;
use ledgeline_core::parse_journal;
use ledgeline_core::reports::{BudgetOpts, BudgetReport, Interval, MixedAmount, budget_report};
use serde_json::Value;
use std::collections::BTreeMap;

// ---- fixtures ----

fn budget_journal(name: &str) -> ledgeline_core::Journal {
    let path = common::fixtures_dir().join("budget").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
    parse_journal(&text, &path.to_string_lossy()).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

fn golden(name: &str) -> Value {
    let path = common::fixtures_dir().join("budget").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

// ---- canonical exact comparison (identical to reports_golden.rs) ----

type Canon = BTreeMap<String, (i128, u32)>;

fn canon(mut mantissa: i128, mut places: u32) -> (i128, u32) {
    while places > 0 && mantissa % 10 == 0 {
        mantissa /= 10;
        places -= 1;
    }
    (mantissa, places)
}

fn canon_map(ma: &MixedAmount) -> Canon {
    ma.iter()
        .filter(|(_, dec)| !dec.is_zero())
        .map(|(commodity, dec)| (commodity.0.clone(), canon(dec.mantissa, dec.places)))
        .collect()
}

/// Sum a golden MixedAmount (array of `GAmount`) per commodity with exact `Dec`
/// math, then canonicalize and drop zeros — the golden side of a comparison.
fn sum_golden(amounts: &Value) -> Canon {
    let mut merged: BTreeMap<String, Dec> = BTreeMap::new();
    for amount in amounts.as_array().expect("amount array") {
        let commodity = amount["acommodity"]
            .as_str()
            .expect("acommodity")
            .to_string();
        let quantity = &amount["aquantity"];
        let mantissa = i128::from(
            quantity["decimalMantissa"]
                .as_i64()
                .expect("decimalMantissa"),
        );
        let places =
            u32::try_from(quantity["decimalPlaces"].as_u64().expect("decimalPlaces")).unwrap();
        let dec = Dec::new(mantissa, places);
        merged
            .entry(commodity)
            .and_modify(|prev| *prev = prev.add(dec).expect("no overflow"))
            .or_insert(dec);
    }
    merged
        .into_iter()
        .map(|(commodity, dec)| (commodity, canon(dec.mantissa, dec.places)))
        .filter(|(_, (mantissa, _))| *mantissa != 0)
        .collect()
}

/// The golden cell `[actual, goal|null]` for row `row` at bucket `index`.
fn golden_cell(row: &Value, index: usize) -> (&Value, &Value) {
    let cell = &row["prrAmounts"][index];
    (&cell[0], &cell[1])
}

/// Compare one of our cells against a golden `[actual, goal|null]` pair.
fn assert_cell(
    label: &str,
    actual: &MixedAmount,
    goal: Option<&MixedAmount>,
    golden_actual: &Value,
    golden_goal: &Value,
) {
    assert_eq!(
        canon_map(actual),
        sum_golden(golden_actual),
        "{label} actual"
    );
    match (goal, golden_goal.is_null()) {
        (None, true) => {}
        (Some(goal), false) => {
            assert_eq!(canon_map(goal), sum_golden(golden_goal), "{label} goal");
        }
        (present, _) => panic!(
            "{label} goal presence mismatch: ours={} golden_null={}",
            present.is_some(),
            golden_goal.is_null()
        ),
    }
}

/// Full diff of a computed report against its golden `PeriodicReport`.
fn check(report: &BudgetReport, golden: &Value) {
    let dates = golden["prDates"].as_array().expect("prDates");
    assert_eq!(report.buckets.len(), dates.len(), "bucket count");

    let golden_rows = golden["prRows"].as_array().expect("prRows");

    // Full row set (names) must match exactly — parents and <unbudgeted> too.
    let mut ours: Vec<String> = report.rows.iter().map(|r| r.account.clone()).collect();
    let mut theirs: Vec<String> = golden_rows
        .iter()
        .map(|r| r["prrName"].as_str().expect("prrName").to_string())
        .collect();
    ours.sort();
    theirs.sort();
    assert_eq!(ours, theirs, "row account set");

    for grow in golden_rows {
        let name = grow["prrName"].as_str().unwrap();
        let row = report
            .rows
            .iter()
            .find(|r| r.account == name)
            .unwrap_or_else(|| panic!("row {name} exists"));
        for (index, bucket) in report.buckets.iter().enumerate() {
            let (ga, gg) = golden_cell(grow, index);
            let cell = &row.cells[index];
            assert_cell(
                &format!("row {name} bucket {bucket}"),
                &cell.actual,
                cell.goal.as_ref(),
                ga,
                gg,
            );
        }
    }

    // Totals row.
    let totals = &golden["prTotals"];
    for (index, bucket) in report.buckets.iter().enumerate() {
        let cell = &report.totals[index];
        let (ga, gg) = golden_cell(totals, index);
        assert_cell(
            &format!("totals bucket {bucket}"),
            &cell.actual,
            cell.goal.as_ref(),
            ga,
            gg,
        );
    }
}

fn run(journal: &str, golden_file: &str, opts: &BudgetOpts) {
    let j = budget_journal(journal);
    let report = budget_report(&j.transactions, &j.periodic_transactions, opts).unwrap();
    check(&report, &golden(golden_file));
}

fn monthly<'a>(end: &'a str, count: usize, budget_desc: Option<&'a str>) -> BudgetOpts<'a> {
    BudgetOpts {
        end,
        interval: Interval::Monthly,
        count,
        depth: 99,
        budget_desc,
    }
}

#[test]
fn basic_monthly_goal_and_unbudgeted_matches_golden() {
    // -b 2026-01-01 -e 2026-03-01 → 2 monthly buckets ending 2026-02-28.
    run(
        "basic.journal",
        "basic.budget.json",
        &monthly("2026-02-28", 2, None),
    );
}

#[test]
fn parent_aggregation_matches_golden() {
    run(
        "parents.journal",
        "parents.budget.json",
        &monthly("2026-01-31", 1, None),
    );
}

#[test]
fn unbudgeted_rollup_matches_golden() {
    run(
        "unbudgeted.journal",
        "unbudgeted.budget.json",
        &monthly("2026-01-31", 1, None),
    );
}

#[test]
fn descpat_housing_selection_matches_golden() {
    run(
        "descpat.journal",
        "descpat-housing.budget.json",
        &monthly("2026-01-31", 1, Some("housing")),
    );
}

#[test]
fn descpat_grocer_selection_matches_golden() {
    run(
        "descpat.journal",
        "descpat-grocer.budget.json",
        &monthly("2026-01-31", 1, Some("grocer")),
    );
}

#[test]
fn weekly_rule_in_monthly_report_matches_golden() {
    // Cross-interval: a `~ weekly` $10 goal viewed in a monthly report sums the
    // four Monday occurrences that fall in Jan 2026 -> a $40 goal. Independently
    // hledger-validated (the other fixtures are all `~ monthly` in a monthly
    // report, so they never exercise cross-interval goal accumulation).
    run(
        "weekly.journal",
        "weekly.budget.json",
        &monthly("2026-01-31", 1, None),
    );
}
