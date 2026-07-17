//! Golden parity: the native report engine vs. committed hledger-CLI output.
//!
//! Mirrors `web/src/lib/reports/golden.test.ts`. Input is the parsed
//! `fixtures/sample.journal` (the same source the committed `fixtures/golden/`
//! were generated from by `scripts/gen-golden.sh`); we run our engine and diff
//! its numbers against the goldens.
//!
//! Reconciliation (identical to the TS adapter):
//! - hledger's `-e DATE` is EXCLUSIVE; our `asOf`/`to` are INCLUSIVE, so
//!   `-e 2026-07-01` ≙ `2026-06-30`.
//! - hledger keeps same-commodity, different-cost-basis lots as separate
//!   `MixedAmount` entries; our engine merges per commodity, so the adapter sums
//!   the golden amounts per commodity with exact `Dec` math.
//! - all comparisons are on CANONICAL `(mantissa, places)` (trailing zeros
//!   stripped, zero commodities dropped) — never floats.

mod common;

use common::fixture_journal;
use ledgeline_core::Dec;
use ledgeline_core::reports::{
    Interval, MixedAmount, PriceDb, SectionedReport, account_decls, balance_sheet, cash_flow,
    cash_predicate, income_statement, net_worth,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

// ---- fixtures ----

fn golden(name: &str) -> Value {
    let path: PathBuf = common::fixtures_dir().join("golden").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

// ---- canonical exact comparison: commodity → (mantissa, places), zeros stripped ----

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

/// A golden `GAmount` → `(commodity, Dec)`.
fn golden_amount(amount: &Value) -> (String, Dec) {
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
    let places = u32::try_from(quantity["decimalPlaces"].as_u64().expect("decimalPlaces")).unwrap();
    (commodity, Dec::new(mantissa, places))
}

/// Sum hledger amounts per commodity (merging cost-basis lots) with exact `Dec`
/// math, then canonicalize — the golden side of every comparison.
fn sum_golden(amounts: &Value) -> Canon {
    let mut merged: BTreeMap<String, Dec> = BTreeMap::new();
    for amount in amounts.as_array().expect("amount array") {
        let (commodity, qty) = golden_amount(amount);
        merged
            .entry(commodity)
            .and_modify(|prev| *prev = prev.add(qty).expect("no overflow"))
            .or_insert(qty);
    }
    merged
        .into_iter()
        .map(|(commodity, dec)| (commodity, canon(dec.mantissa, dec.places)))
        .filter(|(_, (mantissa, _))| *mantissa != 0)
        .collect()
}

/// A section's "leaves": rows that are not an ancestor of another row (what
/// hledger's flat JSON lists).
fn leaf_accounts(rows: &[String]) -> Vec<String> {
    rows.iter()
        .filter(|row| {
            let prefix = format!("{row}:");
            !rows.iter().any(|other| other.starts_with(&prefix))
        })
        .cloned()
        .collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

// ---- CompoundBalanceReport (bs / is) comparison ----

fn check_sectioned(report: &SectionedReport, golden: &Value) {
    for subreport in golden["cbrSubreports"].as_array().expect("cbrSubreports") {
        let title = subreport[0].as_str().expect("subreport title");
        let sub = &subreport[1];
        let section = report
            .sections
            .iter()
            .find(|s| s.title == title)
            .unwrap_or_else(|| panic!("section {title} exists"));

        let row_accounts: Vec<String> = section.rows.iter().map(|r| r.account.clone()).collect();
        let golden_leaves: Vec<String> = sub["prRows"]
            .as_array()
            .expect("prRows")
            .iter()
            .map(|r| r["prrName"].as_str().expect("prrName").to_string())
            .collect();
        assert_eq!(
            sorted(leaf_accounts(&row_accounts)),
            sorted(golden_leaves.clone()),
            "{title} leaf account set"
        );

        for row in sub["prRows"].as_array().unwrap() {
            let name = row["prrName"].as_str().unwrap();
            let mine = section
                .rows
                .iter()
                .find(|r| r.account == name)
                .unwrap_or_else(|| panic!("row {name} exists"));
            assert_eq!(
                canon_map(&mine.inclusive),
                sum_golden(&row["prrAmounts"][0]),
                "{title} row {name}"
            );
        }

        assert_eq!(
            canon_map(&section.total),
            sum_golden(&sub["prTotals"]["prrAmounts"][0]),
            "{title} total"
        );
    }

    assert_eq!(
        canon_map(&report.grand_total),
        sum_golden(&golden["cbrTotals"]["prrAmounts"][0]),
        "grand total"
    );
}

#[test]
fn balance_sheet_depth_1_matches_golden() {
    let journal = fixture_journal();
    let report = balance_sheet(&journal.transactions, "2026-06-30", 1).unwrap();
    check_sectioned(&report, &golden("bs-d1.json"));
}

#[test]
fn balance_sheet_depth_3_matches_golden() {
    let journal = fixture_journal();
    let report = balance_sheet(&journal.transactions, "2026-06-30", 3).unwrap();
    check_sectioned(&report, &golden("bs-d3.json"));
}

/// Depth-2 balance sheet: no committed golden, so the expectations are the
/// CLI-derived values embedded in `golden.test.ts` (hledger 1.52, `--depth 2`).
#[test]
fn balance_sheet_depth_2_matches_embedded_cli_values() {
    let journal = fixture_journal();
    let report = balance_sheet(&journal.transactions, "2026-06-30", 2).unwrap();

    let by_account: BTreeMap<String, Canon> = report
        .sections
        .iter()
        .flat_map(|s| {
            s.rows
                .iter()
                .map(|r| (r.account.clone(), canon_map(&r.inclusive)))
        })
        .collect();

    let entry = |pairs: &[(&str, i128, u32)]| -> Canon {
        pairs
            .iter()
            .map(|(c, m, p)| ((*c).to_string(), canon(*m, *p)))
            .collect()
    };
    let stocks: [(&str, i128, u32); 4] = [
        ("AAPL", 195, 1),
        ("GLD", 5, 0),
        ("TSLA", -2, 0),
        ("VTI", 17, 0),
    ];

    assert_eq!(
        by_account["assets:bank"],
        entry(&[("$", 4_366_781, 2), ("EUR", 56_675, 2)])
    );
    let mut broker = entry(&[("$", 660_975, 2)]);
    broker.extend(entry(&stocks));
    assert_eq!(by_account["assets:broker"], broker);
    assert_eq!(by_account["liabilities:cc"], entry(&[("$", 6211, 2)]));

    let mut assets_total = entry(&[("$", 5_027_756, 2), ("EUR", 56_675, 2)]);
    assets_total.extend(entry(&stocks));
    assert_eq!(canon_map(&report.sections[0].total), assets_total);
    assert_eq!(
        canon_map(&report.sections[1].total),
        entry(&[("$", 6211, 2)])
    );

    let mut grand = entry(&[("$", 5_021_545, 2), ("EUR", 56_675, 2)]);
    grand.extend(entry(&stocks));
    assert_eq!(canon_map(&report.grand_total), grand);
}

#[test]
fn income_statement_depth_2_matches_golden() {
    let journal = fixture_journal();
    let report = income_statement(&journal.transactions, "2026-01-01", "2026-06-30", 2).unwrap();
    check_sectioned(&report, &golden("is-d2.json"));
}

/// hledger's cashflow honors `type: C` declarations. We run BOTH the name-only
/// fallback (default) and the declared predicate built from the real journal
/// declarations: both must reproduce hledger's selection.
#[test]
fn cash_flow_monthly_matches_golden_both_predicates() {
    let journal = fixture_journal();
    let decls = account_decls(&journal);
    let predicate = cash_predicate(&decls);
    let declared: &dyn Fn(&str) -> bool = &predicate;

    for is_cash in [None, Some(declared)] {
        let report = cash_flow(
            &journal.transactions,
            "2026-06-30",
            Interval::Monthly,
            6,
            99,
            is_cash,
        )
        .unwrap();
        assert_eq!(
            report.buckets,
            [
                "2026-01", "2026-02", "2026-03", "2026-04", "2026-05", "2026-06"
            ]
        );

        let g = golden("cf-monthly.json");
        let sub = &g["cbrSubreports"][0][1];
        let row_accounts: Vec<String> = report.rows.iter().map(|r| r.account.clone()).collect();
        let golden_leaves: Vec<String> = sub["prRows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["prrName"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(sorted(leaf_accounts(&row_accounts)), sorted(golden_leaves));

        for row in sub["prRows"].as_array().unwrap() {
            let name = row["prrName"].as_str().unwrap();
            let mine = report
                .rows
                .iter()
                .find(|r| r.account == name)
                .unwrap_or_else(|| panic!("row {name} exists"));
            for (i, bucket) in report.buckets.iter().enumerate() {
                assert_eq!(
                    canon_map(&mine.values[i]),
                    sum_golden(&row["prrAmounts"][i]),
                    "row {name} bucket {bucket}"
                );
            }
        }
        for (i, bucket) in report.buckets.iter().enumerate() {
            assert_eq!(
                canon_map(&report.totals[i]),
                sum_golden(&sub["prTotals"]["prrAmounts"][i]),
                "totals bucket {bucket}"
            );
        }
    }
}

#[test]
fn net_worth_spot_matches_golden() {
    let journal = fixture_journal();
    let prices = PriceDb::build(&journal.prices);
    let report = net_worth(
        &journal.transactions,
        &prices,
        "2026-06-30",
        Interval::Monthly,
        1,
        None,
    )
    .unwrap();

    // GLD and TSLA deliberately have no P directive: hledger leaves them
    // unvalued in place; our engine skips them → meta.unpriced. Filter them from
    // the golden before comparing.
    let unpriced = ["GLD", "TSLA"];
    let meta_unpriced: Vec<String> = report
        .meta
        .as_ref()
        .expect("net worth reports unpriced commodities")
        .unpriced
        .iter()
        .map(|c| c.0.clone())
        .collect();
    assert_eq!(meta_unpriced, unpriced);

    let g = golden("networth-spot.json");
    let golden_rows = g[0].as_array().expect("bal rows");
    let golden_total = &g[1];

    // Group golden leaf rows by root account (assets / liabilities).
    let mut by_root: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for row in golden_rows {
        let account = row[0].as_str().expect("row account");
        let root = account.split(':').next().unwrap().to_string();
        for amount in row[3].as_array().expect("row amounts") {
            by_root
                .entry(root.clone())
                .or_default()
                .push(amount.clone());
        }
    }

    let valued = |amounts: &[Value]| -> Value {
        Value::Array(
            amounts
                .iter()
                .filter(|a| !unpriced.contains(&a["acommodity"].as_str().unwrap_or("")))
                .cloned()
                .collect(),
        )
    };

    let mut my_roots: Vec<String> = report.rows.iter().map(|r| r.account.clone()).collect();
    my_roots.sort();
    let mut golden_root_keys: Vec<String> = by_root.keys().cloned().collect();
    golden_root_keys.sort();
    assert_eq!(my_roots, golden_root_keys);

    for (root, amounts) in &by_root {
        let mine = report
            .rows
            .iter()
            .find(|r| &r.account == root)
            .unwrap_or_else(|| panic!("row {root} exists"));
        assert_eq!(
            canon_map(&mine.values[0]),
            sum_golden(&valued(amounts)),
            "valued {root}"
        );
    }

    let total_amounts: Vec<Value> = golden_total.as_array().expect("total amounts").clone();
    assert_eq!(
        canon_map(&report.totals[0]),
        sum_golden(&valued(&total_amounts)),
        "net worth"
    );
}
