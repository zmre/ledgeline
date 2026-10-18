//! The engine's budget summary reproduces the TypeScript it replaced (plan 26
//! B6).
//!
//! `fixtures/budget-summary/budget-summary.json` was CAPTURED from the SPA's
//! `reports/budgetSummary.ts` plus the bar, label and section logic in
//! `BudgetSummary.svelte` and the totals in `BudgetGaps.svelte`, run over each
//! fixture journal's `/api/budget` and `/api/budget/gaps` wire at a spread of
//! as-of dates, before that code was deleted. A failure here is the port
//! drifting; the fixture is not to be regenerated from Rust.
//!
//! Everything is compared by VALUE (`1.50 == 1.5`), and exactly — except the
//! pace and the two labels quoted from it. The TypeScript snapped the elapsed
//! fraction to six places before multiplying; the engine uses the exact ratio.
//! Those figures may differ by at most `|goal| × 5e-7` plus one unit in the
//! pace's last place, and are compared within exactly that.

mod common;

use common::fixtures_dir;
use ledgeline_core::Dec;
use ledgeline_core::model::Commodity;
use ledgeline_core::parse_journal;
use ledgeline_core::reports::{
    BudgetBar, BudgetOpts, Interval, LabelKind, MixedAmount, account_decls, budget_gaps,
    budget_report, budget_summary, declared_types, gap_totals,
};
use serde_json::Value;

fn golden() -> Value {
    let path = fixtures_dir().join("budget-summary/budget-summary.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("golden readable"))
        .expect("golden is JSON")
}

fn dec_of(value: &Value) -> Dec {
    let mantissa: i128 = value["m"].as_str().expect("m").parse().expect("mantissa");
    let places = u32::try_from(value["p"].as_u64().expect("p")).expect("places");
    Dec::new(mantissa, places)
}

fn mixed_of(value: &Value) -> MixedAmount {
    let mut out = MixedAmount::new();
    for (commodity, qty) in value.as_object().expect("mixed amount object") {
        out.accumulate(&Commodity(commodity.clone()), dec_of(qty))
            .expect("accumulates");
    }
    out.drop_zeros();
    out
}

/// `|a − b| ≤ tol` in every commodity, and the same commodities.
fn close(actual: &MixedAmount, expected: &MixedAmount, tol: &MixedAmount) -> bool {
    let commodities: std::collections::BTreeSet<&Commodity> = actual
        .iter()
        .chain(expected.iter())
        .map(|(c, _)| c)
        .collect();
    commodities.into_iter().all(|commodity| {
        let a = actual.get(commodity).unwrap_or_else(Dec::zero);
        let b = expected.get(commodity).unwrap_or_else(Dec::zero);
        let limit = tol.get(commodity).unwrap_or_else(Dec::zero);
        a.sub(b).and_then(Dec::abs).expect("difference") <= limit
    })
}

/// The pace tolerance for `goal`: `|goal| × 5e-7` plus one unit at six places
/// past the goal's scale, per commodity.
fn pace_tolerance(goal: &MixedAmount) -> MixedAmount {
    let mut out = MixedAmount::new();
    for (commodity, qty) in goal.iter() {
        let tol = qty
            .abs()
            .and_then(|abs| abs.mul(Dec::new(5, 7)))
            .and_then(|scaled| scaled.add(Dec::new(1, qty.places + 6)))
            .expect("tolerance");
        out.accumulate(commodity, tol).expect("accumulates");
    }
    out
}

fn label_kind(kind: &str) -> LabelKind {
    match kind {
        "onPlan" => LabelKind::OnPlan,
        "behindPace" => LabelKind::BehindPace,
        "aboveTarget" => LabelKind::AboveTarget,
        "toGo" => LabelKind::ToGo,
        "over" => LabelKind::Over,
        "aheadOfPace" => LabelKind::AheadOfPace,
        "left" => LabelKind::Left,
        other => panic!("unknown label kind {other}"),
    }
}

fn health_name(bar: &BudgetBar) -> Value {
    match bar.health {
        None => Value::Null,
        Some(health) => serde_json::to_value(health).expect("serializes"),
    }
}

fn assert_bar(at: &str, bar: &BudgetBar, want: &Value) {
    assert_eq!(
        bar.account,
        want["account"].as_str().expect("account"),
        "{at}"
    );
    assert_eq!(
        bar.income,
        want["income"].as_bool().expect("income"),
        "{at}: income"
    );
    assert_eq!(bar.actual, mixed_of(&want["actual"]), "{at}: actual");
    assert_eq!(bar.goal, mixed_of(&want["goal"]), "{at}: goal");
    assert_eq!(
        bar.remaining,
        mixed_of(&want["remaining"]),
        "{at}: remaining"
    );
    let tol = pace_tolerance(&bar.goal);
    assert!(
        close(&bar.pace, &mixed_of(&want["pace"]), &tol),
        "{at}: pace {:?} vs {}",
        bar.pace,
        want["pace"]
    );
    assert_eq!(health_name(bar), want["health"], "{at}: health");
    let kind = label_kind(want["label"]["kind"].as_str().expect("kind"));
    assert_eq!(bar.label.kind, kind, "{at}: label kind");
    match (&bar.label.amount, &want["label"]["amount"]) {
        (None, Value::Null) => {}
        (Some(amount), expected) if !expected.is_null() => {
            let expected = mixed_of(expected);
            if matches!(kind, LabelKind::BehindPace | LabelKind::AheadOfPace) {
                assert!(
                    close(amount, &expected, &tol),
                    "{at}: label amount {amount:?} vs {expected:?}"
                );
            } else {
                assert_eq!(amount, &expected, "{at}: label amount");
            }
        }
        (got, expected) => panic!("{at}: label amount {got:?} vs {expected}"),
    }
}

#[test]
fn budget_summary_matches_the_captured_typescript() {
    let golden = golden();
    let mut compared = 0;
    let mut bars = 0;
    for entry in golden["journals"].as_array().expect("journals") {
        let rel = entry["journal"].as_str().expect("journal");
        let path = fixtures_dir().join(rel);
        let text = std::fs::read_to_string(&path).expect("fixture readable");
        let journal = parse_journal(&text, &path.to_string_lossy()).expect("fixture parses");
        let declared = declared_types(&account_decls(&journal));
        for window in entry["windows"].as_array().expect("windows") {
            let end = window["end"].as_str().expect("end");
            let opts = BudgetOpts {
                end,
                interval: Interval::Monthly,
                count: usize::try_from(window["count"].as_u64().expect("count")).expect("count"),
                depth: usize::try_from(window["depth"].as_u64().expect("depth")).expect("depth"),
                budget_desc: window["budgetDesc"].as_str(),
            };
            let report =
                budget_report(&journal.transactions, &journal.periodic_transactions, &opts)
                    .expect("report");

            let gaps = budget_gaps(
                &journal.transactions,
                &journal.periodic_transactions,
                &declared,
                &opts,
            )
            .expect("gaps");
            let totals = gap_totals(&gaps).expect("gap totals");
            let want = &window["gaps"];
            assert_eq!(
                totals.revenue,
                mixed_of(&want["revenueTotal"]),
                "{rel} {end}: revenue total"
            );
            assert_eq!(
                totals.expense,
                mixed_of(&want["expenseTotal"]),
                "{rel} {end}: expense total"
            );
            assert_eq!(
                totals.headline,
                mixed_of(&want["headline"]),
                "{rel} {end}: headline"
            );

            for case in window["cases"].as_array().expect("cases") {
                let as_of = case["asOf"].as_str().expect("asOf");
                let at = format!("{rel} end={end} depth={} @ {as_of}", opts.depth);
                let summary = budget_summary(&report, &declared, end, as_of).expect("summary");
                assert_eq!(
                    summary.from,
                    window["from"].as_str().expect("from"),
                    "{at}: from"
                );
                let fraction = case["elapsed"].as_f64().expect("elapsed");
                assert!(
                    (summary.elapsed.fraction() - fraction).abs() < 1e-12,
                    "{at}: elapsed"
                );
                let sections = case["sections"].as_array().expect("sections");
                assert_eq!(
                    summary.sections.len(),
                    sections.len(),
                    "{at}: section count"
                );
                for (section, want) in summary.sections.iter().zip(sections) {
                    assert_eq!(
                        serde_json::to_value(section.kind).expect("kind"),
                        want["section"],
                        "{at}: section kind"
                    );
                    assert_bar(&format!("{at} overall"), &section.overall, &want["overall"]);
                    let want_bars = want["bars"].as_array().expect("bars");
                    assert_eq!(section.bars.len(), want_bars.len(), "{at}: bar count");
                    for (bar, want_bar) in section.bars.iter().zip(want_bars) {
                        assert_bar(&at, bar, want_bar);
                        bars += 1;
                    }
                }
                compared += 1;
            }
        }
    }
    assert!(compared >= 150, "too few cases compared: {compared}");
    assert!(bars >= 800, "too few bars compared: {bars}");
}
