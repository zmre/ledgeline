//! PARITY GOLDEN for plan 28 Phase 1: the forecast expansion must reproduce
//! the per-bucket engine it replaced.
//!
//! Every figure below was CAPTURED from the engine as it stood before plan 28
//! (commit `33439a7`, the residual-rule accumulators), by running this file's
//! scenarios with `as_of = 2026-09-30` — opening balances through September,
//! first projected bucket October. The new engine is run with `today =
//! 2026-10-15` over a journal whose latest transaction is 2026-09-30, which
//! puts its Decision 5 cutoff on 2026-10-01: the same first bucket and the same
//! opening balances, by a different rule. So any difference here is the
//! EXPANSION's, not the start date's.
//!
//! The scenarios are written as files in the PRE-plan-28 format (unbalanced and
//! balanced virtual postings), because that is the input whose numbers plan 28
//! promises to keep: "a group whose postings do not balance … gets its residual
//! assigned to the default counter-account at load. That keeps today's numbers."

use ledgeline_core::model::Commodity;
use ledgeline_core::parse_journal;
use ledgeline_core::projections::{Projection, ProjectionOpts, Scenario, project, serialize};
use ledgeline_core::reports::{AccountType, Interval, MixedAmount, account_decls, declared_types};
use std::collections::BTreeMap;

/// $50,000 of checking, $8,000 of savings, $100,000 of brokerage (held in
/// dollars, so unpriced), a $500,000 house, and a rent history paid from
/// checking. Latest transaction 2026-09-30.
const JOURNAL: &str = "\
2026-01-01 opening balances
    assets:checking         $50000.00
    assets:savings           $8000.00
    assets:brokerage       $100000.00
    assets:house           $500000.00
    equity:opening

2026-08-01 rent
    expenses:rent            $4200.00
    assets:checking

2026-09-01 rent
    expenses:rent            $4200.00
    assets:checking

2026-09-30 salary
    assets:checking          $4200.00
    revenues:salary
";

/// (name, interval, count, file body)
const CASES: &[(&str, Interval, usize, &str)] = &[
    (
        "burn with a growing salary (unbalanced, legacy)",
        Interval::Monthly,
        14,
        "~ monthly  projection
    (revenues:salary)   $-6000.00  ; growth: 3%/yr
    (expenses:rent)      $4200.00  ; growth: 2%/yr
    (expenses:payroll)  $12000.00
",
    ),
    (
        "a rule that states its own cash leg",
        Interval::Monthly,
        6,
        "~ monthly  rent
    (expenses:rent)      $4200.00
    (assets:checking)   $-4200.00
",
    ),
    (
        "accrual, partial payment and an equity raise",
        Interval::Monthly,
        6,
        "~ monthly  partial
    (expenses:rent)      $4200.00
    (assets:savings)    $-2000.00

~ 2026-11-15  legal, net 60
    (expenses:legal)    $45000.00
    (liabilities:payable)  $-45000.00

~ 2027-01-15  settle legal
    (liabilities:payable)  $45000.00
    (assets:checking)  $-45000.00

~ 2026-12-01  series A
    (assets:checking)  $2000000.00
    (equity:preferred)  $-2000000.00
",
    ),
    (
        "unpriced asset rows: a house that grows and a brokerage that is paid into",
        Interval::Monthly,
        15,
        "~ monthly  projection
    (assets:house)           $0  ; growth: 4%/yr
    (assets:brokerage)  $2000.00  ; growth: 7%/yr
    (expenses:living)   $3000.00
",
    ),
    (
        "a step change and weekly growth, quarterly",
        Interval::Quarterly,
        6,
        "~ monthly to 2027-04-01  projection
    (expenses:payroll)  $50000.00  ; line: payroll

~ monthly from 2027-04-01  projection
    (expenses:payroll)  $120000.00  ; line: payroll, growth: 5%/yr

~ weekly  projection
    (revenues:consulting)  $-2500.00  ; growth: 0.5%/wk
",
    ),
];

fn journal() -> (ledgeline_core::Journal, BTreeMap<String, AccountType>) {
    let journal = parse_journal(JOURNAL, "main.journal").expect("the journal parses");
    let declared = declared_types(&account_decls(&journal));
    (journal, declared)
}

fn run(body: &str, interval: Interval, count: usize) -> Projection {
    let (journal, declared) = journal();
    let file = format!("; Ledgeline projection\n; projection: parity\n\n{body}");
    let scenario: Scenario = serialize::scenario_from_text(&file, "p.journal", "p", &declared)
        .expect("the scenario reads");
    project(
        &scenario,
        &journal.transactions,
        &journal.prices,
        &ProjectionOpts {
            today: "2026-10-15",
            interval,
            count,
            lookback: 0,
            depth: 99,
            declared: &declared,
            value_in: None,
        },
    )
    .expect("the projection runs")
}

/// Whole cents of `$`.
fn cents(ma: &MixedAmount) -> i128 {
    ma.get(&Commodity("$".into()))
        .map_or(0, |dec| dec.rounded(2).expect("cents fit").mantissa)
}

fn summary(projection: &Projection) -> String {
    let series = |values: &[MixedAmount]| values.iter().map(cents).collect::<Vec<_>>();
    format!(
        "buckets={:?}\ncash.opening={}\ncash={:?}\nnw.opening={}\nnw={:?}\nincome={:?}\nrunway={:?}\nassets={:?}",
        projection.buckets,
        cents(&projection.cash.opening),
        series(&projection.cash.values),
        cents(&projection.net_worth.opening),
        series(&projection.net_worth.values),
        series(&projection.net_income.totals),
        projection
            .runway
            .as_ref()
            .map(|r| (r.bucket, r.date.clone(), r.periods)),
        projection
            .assets
            .iter()
            .map(|a| (
                a.account.clone(),
                cents(&a.journal_opening),
                cents(&a.opening),
                cents(&a.growth)
            ))
            .collect::<Vec<_>>(),
    )
}

/// The old engine's answers, captured verbatim (see the module docs).
const EXPECTED: &[&str] = &[
    // 0: burn with a growing salary (unbalanced, legacy)
    "buckets=[\"2026-10\", \"2026-11\", \"2026-12\", \"2027-01\", \"2027-02\", \"2027-03\", \"2027-04\", \"2027-05\", \"2027-06\", \"2027-07\", \"2027-08\", \"2027-09\", \"2027-10\", \"2027-11\"]
cash.opening=5380000
cash=[4360000, 3340000, 2320000, 1300000, 280000, -740000, -1760000, -2780000, -3800000, -4820000, -5840000, -6860000, -7870400, -8880800]
nw.opening=65380000
nw=[64360000, 63340000, 62320000, 61300000, 60280000, 59260000, 58240000, 57220000, 56200000, 55180000, 54160000, 53140000, 52129600, 51119200]
income=[-1020000, -1020000, -1020000, -1020000, -1020000, -1020000, -1020000, -1020000, -1020000, -1020000, -1020000, -1020000, -1010400, -1010400]
runway=Some((5, \"2027-03-31\", 6))
assets=[]",
    // 1: a rule that states its own cash leg
    "buckets=[\"2026-10\", \"2026-11\", \"2026-12\", \"2027-01\", \"2027-02\", \"2027-03\"]
cash.opening=5380000
cash=[4960000, 4540000, 4120000, 3700000, 3280000, 2860000]
nw.opening=65380000
nw=[64960000, 64540000, 64120000, 63700000, 63280000, 62860000]
income=[-420000, -420000, -420000, -420000, -420000, -420000]
runway=None
assets=[]",
    // 2: accrual, partial payment and an equity raise
    "buckets=[\"2026-10\", \"2026-11\", \"2026-12\", \"2027-01\", \"2027-02\", \"2027-03\"]
cash.opening=5380000
cash=[4960000, 4540000, 204120000, 199200000, 198780000, 198360000]
nw.opening=65380000
nw=[64960000, 60040000, 259620000, 259200000, 258780000, 258360000]
income=[-420000, -4920000, -420000, -420000, -420000, -420000]
runway=None
assets=[]",
    // 3: unpriced asset rows
    "buckets=[\"2026-10\", \"2026-11\", \"2026-12\", \"2027-01\", \"2027-02\", \"2027-03\", \"2027-04\", \"2027-05\", \"2027-06\", \"2027-07\", \"2027-08\", \"2027-09\", \"2027-10\", \"2027-11\", \"2027-12\"]
cash.opening=5380000
cash=[4880000, 4380000, 3880000, 3380000, 2880000, 2380000, 1880000, 1380000, 880000, 380000, -120000, -620000, -1120000, -1620000, -2120000]
nw.opening=65380000
nw=[65080000, 64780000, 64480000, 64180000, 63880000, 63580000, 63280000, 62980000, 62680000, 62380000, 62080000, 61780000, 64348000, 64048000, 63748000]
income=[-300000, -300000, -300000, -300000, -300000, -300000, -300000, -300000, -300000, -300000, -300000, -300000, -300000, -300000, -300000]
runway=Some((10, \"2027-08-31\", 11))
assets=[(\"assets:house\", 50000000, 50000000, 2000000), (\"assets:brokerage\", 10000000, 10000000, 868000)]",
    // 4: a step change and weekly growth, quarterly
    "buckets=[\"2026-Q4\", \"2027-Q1\", \"2027-Q2\", \"2027-Q3\", \"2027-Q4\", \"2028-Q1\"]
cash.opening=5380000
cash=[-6270688, -17697030, -49883989, -81815530, -113474546, -144842787]
nw.opening=65380000
nw=[53729312, 42302970, 10116011, -21815530, -53474546, -84842787]
income=[-11650688, -11426342, -32186959, -31931541, -31659016, -31368241]
runway=Some((0, \"2026-12-31\", 1))
assets=[]",
];

#[test]
fn the_expansion_reproduces_the_per_bucket_engine() {
    for (index, (name, interval, count, body)) in CASES.iter().enumerate() {
        let got = summary(&run(body, *interval, *count));
        match EXPECTED.get(index) {
            Some(want) => assert_eq!(&got, want, "{name}"),
            None => println!("CAPTURE {index} {name}\n{got}\n"),
        }
    }
    assert_eq!(
        EXPECTED.len(),
        CASES.len(),
        "every case has a captured answer"
    );
}
