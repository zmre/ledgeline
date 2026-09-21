//! Which WAY a projection moves — the properties a person states in one
//! sentence.
//!
//! The rest of the projection suite pins mechanics: which bucket a growth step
//! lands in, how a residual implies a cash leg, what an overflow warns. Those
//! are exact-figure tests, and they are all satisfied by an engine that has the
//! sign of every line backwards. This file pins the DIRECTIONS instead:
//!
//!   - spending money makes cash go down, in every bucket, always;
//!   - earning it makes cash go up;
//!   - making a cost grow shortens the runway, and making it shrink lengthens
//!     it;
//!   - a paper gain on an asset is not money in the bank.
//!
//! Every failure here names the property in words, because the number on its
//! own does not say what went wrong. `-709_080` and `+6_709_080` are both just
//! numbers; "growth on an expense must SHORTEN the runway, not lengthen it" is
//! the bug.
//!
//! # These pin the ENGINE, and the engine was not what broke
//!
//! The 2026-09 report ("adding growth to payroll… shows a huge build up of cash
//! into infinity") was a browser-side sign flip; the engine was correct
//! throughout, and `web/src/lib/domain/accountTypes.test.ts` plus
//! `fixtures/account-types/classification-cases.json` are what guard that half.
//! What was missing HERE is that not one exact-figure test in the projection
//! suite states which way anything is supposed to move, so none of them could
//! have noticed. [`a_negative_expense_is_an_inflow_which_is_why_the_sign_matters`]
//! is the join between the two: it feeds the engine exactly what the browser
//! used to send and pins what that does.
//!
//! # Scenarios are written as FILES
//!
//! Each case is the projection journal a user would have on disk, read back
//! through [`serialize::scenario_from_text`]. That covers the file round trip
//! and the `~` rule reading for free, and — more to the point — it makes the
//! test case legible as the thing the user is actually looking at.
//!
//! # The `cogs:` twin
//!
//! [`a_cogs_rooted_chart_projects_identically_to_an_expenses_rooted_one`] is the
//! regression guard for a whole bug class: classification by NAME rather than by
//! declared type. A chart that books costs under `cogs:` with `; type: X`, or
//! names its accounts in a language other than English, must project the same
//! numbers as the English one. The failure mode is not always a zero — on this
//! tab it has been a SIGN FLIP, which is worse, because a projection that reads
//! zero looks broken and one with the sign backwards looks like good news
//! (plan 22, amendment 42).

use ledgeline_core::model::Commodity;
use ledgeline_core::parse_journal;
use ledgeline_core::projections::{Projection, ProjectionOpts, Scenario, project, serialize};
use ledgeline_core::reports::{AccountType, Interval, account_decls, declared_types};
use ledgeline_core::{Dec, Journal};
use std::collections::BTreeMap;

// ===========================================================================
// Harness
// ===========================================================================

/// A journal holding $3,000,000 of cash and nothing else to say.
///
/// `assets:checking` is Cash by hledger's own name heuristic, so the English
/// cases need no declarations at all — which is the point: they are what a
/// journal with no `type:` tags does.
const ENGLISH_JOURNAL: &str = "\
2026-01-01 opening balances
    assets:checking      $3000000.00
    equity:opening      $-3000000.00
";

/// The same money, under a chart of accounts no English heuristic can read.
/// Every account SAYS what it is; nothing about the name does.
const SPANISH_JOURNAL: &str = "\
account activo:banco       ; type: C
account cogs:nomina        ; type: X
account ingresos:consultoria  ; type: R
account patrimonio:inicio  ; type: E

2026-01-01 saldos iniciales
    activo:banco           $3000000.00
    patrimonio:inicio     $-3000000.00
";

const AS_OF: &str = "2026-09-20";

fn journal_of(text: &str) -> (Journal, BTreeMap<String, AccountType>) {
    let journal = parse_journal(text, "main.journal").expect("the test journal parses");
    let declared = declared_types(&account_decls(&journal));
    (journal, declared)
}

/// Read `file` as the projection journal it is, and project it over `count`
/// monthly buckets against `journal_text`.
fn project_file(file: &str, journal_text: &str, count: usize) -> Projection {
    let (journal, declared) = journal_of(journal_text);
    let scenario: Scenario = serialize::scenario_from_text(file, "p.journal", "p", &declared)
        .expect("the projection file reads back");
    project(
        &scenario,
        &journal.transactions,
        &journal.prices,
        &ProjectionOpts {
            as_of: AS_OF,
            interval: Interval::Monthly,
            count,
            depth: 99,
            declared: &declared,
            value_in: None,
        },
    )
    .expect("the projection runs")
}

/// A projection file with one `~ monthly` rule holding `rows`.
fn file(rows: &str) -> String {
    format!("; Ledgeline projection\n; projection: test\n\n~ monthly  projection\n{rows}")
}

/// The `$` figure of each bucket's closing cash, in whole cents.
fn cash(projection: &Projection) -> Vec<i128> {
    series(&projection.cash.values)
}

fn net_worth(projection: &Projection) -> Vec<i128> {
    series(&projection.net_worth.values)
}

fn series(values: &[ledgeline_core::reports::MixedAmount]) -> Vec<i128> {
    values
        .iter()
        .map(|value| {
            value
                .get(&Commodity("$".into()))
                .unwrap_or_else(Dec::zero)
                .mantissa
        })
        .collect()
}

/// Each bucket's net-income total, in whole cents.
fn totals(projection: &Projection) -> Vec<i128> {
    series(&projection.net_income.totals)
}

/// The date cash first closes below zero, or `None`.
fn runway(projection: &Projection) -> Option<String> {
    projection.runway.as_ref().map(|r| r.date.clone())
}

/// The runway as something ORDERABLE, where "never runs out" is the longest
/// runway of all.
///
/// `Option`'s own ordering puts `None` FIRST, which would read "never ran out"
/// as the shortest runway and quietly invert every comparison in this file.
/// A date far past any horizon these tests project says what is meant.
fn runway_key(projection: &Projection) -> String {
    runway(projection).unwrap_or_else(|| "9999-12-31".to_string())
}

/// How a runway reads in a failure message: the date, or the words.
fn runway_words(projection: &Projection) -> String {
    runway(projection).unwrap_or_else(|| "never".to_string())
}

/// Assert `values` falls in EVERY step, naming the property rather than the
/// figures.
fn assert_falls_every_bucket(values: &[i128], property: &str) {
    assert!(
        !values.is_empty(),
        "{property}: nothing was projected at all"
    );
    for (index, pair) in values.windows(2).enumerate() {
        assert!(
            pair[1] < pair[0],
            "{property}\n  but bucket {} went from {} to {} — it did not fall.\n  full series: {values:?}",
            index + 1,
            pair[0],
            pair[1]
        );
    }
}

/// Assert `values` rises in EVERY step.
fn assert_rises_every_bucket(values: &[i128], property: &str) {
    assert!(
        !values.is_empty(),
        "{property}: nothing was projected at all"
    );
    for (index, pair) in values.windows(2).enumerate() {
        assert!(
            pair[1] > pair[0],
            "{property}\n  but bucket {} went from {} to {} — it did not rise.\n  full series: {values:?}",
            index + 1,
            pair[0],
            pair[1]
        );
    }
}

// ===========================================================================
// Spending money lowers cash; earning it raises cash
// ===========================================================================

/// The most basic claim this tab makes. A recurring cost takes money OUT, every
/// bucket, without exception — and by the same amount each time when it is flat.
#[test]
fn a_recurring_expense_takes_cash_down_in_every_bucket() {
    let projection = project_file(
        &file("    (expenses:payroll)    $100000.00\n"),
        ENGLISH_JOURNAL,
        36,
    );
    let cash = cash(&projection);
    assert_falls_every_bucket(
        &cash,
        "a recurring expense must LOWER cash in every bucket (a $100,000/mo payroll \
         cannot make the balance go up)",
    );
    // Flat means flat: the step is the same size every month.
    let steps: Vec<i128> = cash.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert!(
        steps.iter().all(|step| *step == -10_000_000),
        "a FLAT expense must take the same amount out every bucket, but the steps were {steps:?}"
    );
    assert_eq!(
        cash[35],
        300_000_000 - 36 * 10_000_000,
        "36 months of $100,000 must leave exactly 36 × $100,000 less than the opening balance"
    );
}

/// The mirror image, and the one a sign bug breaks in the opposite direction.
#[test]
fn recurring_revenue_puts_cash_up_in_every_bucket() {
    let projection = project_file(
        &file("    (income:consulting)    $-40000.00\n"),
        ENGLISH_JOURNAL,
        36,
    );
    assert_rises_every_bucket(
        &cash(&projection),
        "recurring revenue must RAISE cash in every bucket (a $40,000/mo retainer \
         cannot drain the bank account)",
    );
    assert_eq!(
        runway(&projection),
        None,
        "a scenario with revenue and no costs must never run out of money"
    );
}

/// Net income and cash must agree about which way the month went. A scenario
/// that spends more than it earns is a loss AND a drain, never one without the
/// other.
#[test]
fn net_income_is_negative_when_expenses_exceed_revenue() {
    let projection = project_file(
        &file("    (income:consulting)    $-40000.00\n    (expenses:payroll)      $100000.00\n"),
        ENGLISH_JOURNAL,
        12,
    );
    let totals = totals(&projection);
    assert!(
        totals.iter().all(|total| *total < 0),
        "spending $100,000 against $40,000 of revenue must show a NEGATIVE net income \
         in every bucket, but the totals were {totals:?}"
    );
    assert!(
        totals.iter().all(|total| *total == -6_000_000),
        "the monthly loss must be revenue minus costs, $-60,000; got {totals:?}"
    );
    assert_falls_every_bucket(
        &cash(&projection),
        "a scenario that loses money every month must also LOSE CASH every month",
    );
}

/// The invariant that makes the pair a `PeriodReport` rather than a report whose
/// total contradicts its own rows: `totals[i]` is the sum of the depth-1 rows.
///
/// This is what lets a chart plot inflows, outflows and net from one report
/// without the line disagreeing with the bars above it.
#[test]
fn every_buckets_total_is_the_sum_of_that_buckets_rows() {
    let projection = project_file(
        &file(
            "    (income:consulting)       $-40000.00\n\
             \x20   (income:licensing)        $-9000.00\n\
             \x20   (expenses:payroll)       $100000.00\n\
             \x20   (expenses:rent)            $4200.00\n",
        ),
        ENGLISH_JOURNAL,
        6,
    );
    let totals = totals(&projection);
    for (bucket, total) in totals.iter().enumerate() {
        let summed: i128 = projection
            .net_income
            .rows
            .iter()
            .filter(|row| row.depth == 1)
            .map(|row| {
                row.values[bucket]
                    .get(&Commodity("$".into()))
                    .unwrap_or_else(Dec::zero)
                    .mantissa
            })
            .sum();
        assert_eq!(
            summed, *total,
            "bucket {bucket}: the net-income TOTAL must be the sum of that bucket's \
             top-level rows, or the chart's net line contradicts its own bars"
        );
    }
    // And the orientation is cash-flow: revenue above the axis, costs below.
    let payroll = row_values(&projection, "expenses");
    assert!(
        payroll.iter().all(|value| *value < 0),
        "expenses must read NEGATIVE in the cash-flow-oriented net-income report \
         (an outflow below the axis); got {payroll:?}"
    );
    let income = row_values(&projection, "income");
    assert!(
        income.iter().all(|value| *value > 0),
        "revenue must read POSITIVE in the cash-flow-oriented net-income report \
         (an inflow above the axis); got {income:?}"
    );
}

fn row_values(projection: &Projection, account: &str) -> Vec<i128> {
    projection
        .net_income
        .rows
        .iter()
        .find(|row| row.account == account)
        .map(|row| series(&row.values))
        .unwrap_or_else(|| panic!("no net-income row for {account}"))
}

// ===========================================================================
// Growth moves the runway the way growth should
// ===========================================================================

/// Making a cost grow can only make the money run out SOONER. It cannot turn a
/// cost into income.
///
/// This is the ENGINE half of the 2026-09 report. The engine was never at fault
/// there — see [`a_negative_expense_is_an_inflow_which_is_why_the_sign_matters`]
/// for the shape that actually produced it — but "growth on a cost shortens the
/// runway" is the claim the user made and no test stated it.
#[test]
fn growth_on_an_expense_shortens_the_runway() {
    let flat = project_file(
        &file("    (expenses:payroll)    $100000.00\n"),
        ENGLISH_JOURNAL,
        60,
    );
    let growing = project_file(
        &file("    (expenses:payroll)    $100000.00  ; growth: 3%/yr\n"),
        ENGLISH_JOURNAL,
        60,
    );

    assert_falls_every_bucket(
        &cash(&growing),
        "a GROWING expense must still lower cash in every bucket — growth makes a \
         cost bigger, it does not make it income",
    );

    assert!(
        runway(&growing).is_some(),
        "a $100,000/mo payroll growing at 3%/yr against $3,000,000 of cash must run \
         the money out inside five years, but the projection reported no runway at \
         all — that is the shape of a cost being modelled as income"
    );
    assert!(
        runway_key(&growing) <= runway_key(&flat),
        "adding growth to an expense must SHORTEN the runway, never lengthen it: \
         flat ran out {}, growing {}",
        runway_words(&flat),
        runway_words(&growing)
    );

    // And every bucket is at least as poor as the flat one.
    for (bucket, (grown, level)) in cash(&growing).iter().zip(cash(&flat)).enumerate() {
        assert!(
            *grown <= level,
            "bucket {bucket}: a growing expense must leave no MORE cash than the same \
             expense held flat, but growing closed at {grown} against flat's {level}"
        );
    }
}

/// What the 2026-09 bug actually looked like, and why the browser's sign is
/// load-bearing.
///
/// The engine's contract is that a flow line carries the JOURNAL's sign, so a
/// NEGATIVE amount on an expense account is an inflow — and with growth on it,
/// an accelerating one. That is not a defect here; it is the correct reading of
/// what it was sent. The defect was upstream, in the browser's `signedQuantity`
/// negating a cost because it resolved `income:contractors ; type: expenses` as
/// revenue while the engine resolved it as an expense (plan 22, amendment 42).
///
/// Pinning it makes the contract explicit in both directions: whoever builds a
/// `ScenarioLine` owns the sign, and getting it wrong does not produce a small
/// error, it produces infinite money.
#[test]
fn a_negative_expense_is_an_inflow_which_is_why_the_sign_matters() {
    const CONTRACTORS: &str = "\
account income:contractors  ; type: expenses

2026-01-01 opening balances
    assets:checking      $3000000.00
    equity:opening      $-3000000.00
";
    // Exactly what the browser used to send: the magnitude the user typed,
    // negated, on an account the ENGINE books as an expense.
    let wrong = project_file(
        &file("    (income:contractors)    $-100000.00  ; growth: 3%/yr\n"),
        CONTRACTORS,
        60,
    );
    assert_rises_every_bucket(
        &cash(&wrong),
        "the engine reads a flow line at the JOURNAL's sign, so a negative amount on \
         an expense account IS an inflow — this test documents the shape of the bug, \
         not a defect in the engine",
    );
    assert_eq!(
        runway(&wrong),
        None,
        "…and money that only comes in never runs out, which is what the user saw"
    );

    // The same line, signed the way the journal books a cost, is a burn.
    let right = project_file(
        &file("    (income:contractors)    $100000.00  ; growth: 3%/yr\n"),
        CONTRACTORS,
        60,
    );
    assert_falls_every_bucket(
        &cash(&right),
        "the identical row at the correct sign must burn cash in every bucket — the \
         only difference between a company with a runway and one without was a minus",
    );
    assert!(
        runway(&right).is_some(),
        "…and it must run the money out inside the window"
    );
    // The account is an EXPENSE to the engine despite its revenue-shaped name,
    // which is the whole reason the browser's disagreement mattered.
    assert!(
        totals(&right).iter().all(|total| *total < 0),
        "`income:contractors ; type: expenses` is a COST: net income must be negative. \
         A positive total here would mean the engine classified it by its root name"
    );
}

/// A cost that SHRINKS buys time. The same arithmetic, the other way up — and
/// the test that would catch a growth walk that ignored the rate's sign.
#[test]
fn a_shrinking_expense_lengthens_the_runway() {
    let flat = project_file(
        &file("    (expenses:payroll)    $100000.00\n"),
        ENGLISH_JOURNAL,
        60,
    );
    let shrinking = project_file(
        &file("    (expenses:payroll)    $100000.00  ; growth: -20%/yr\n"),
        ENGLISH_JOURNAL,
        60,
    );
    assert_falls_every_bucket(
        &cash(&shrinking),
        "a shrinking expense is still an expense: it must lower cash in every bucket",
    );
    assert!(
        runway_key(&shrinking) >= runway_key(&flat),
        "a NEGATIVE growth rate makes the cost smaller each year, so the money must \
         last at least as long as it does flat: flat {}, shrinking {}",
        runway_words(&flat),
        runway_words(&shrinking)
    );
}

/// Growing REVENUE buys time, for the same reason growing a cost costs it.
#[test]
fn growth_on_revenue_lengthens_the_runway() {
    let rows = "    (expenses:payroll)      $100000.00\n    (income:consulting)     $-40000.00\n";
    let flat = project_file(&file(rows), ENGLISH_JOURNAL, 72);
    let growing = project_file(
        &file(
            "    (expenses:payroll)      $100000.00\n\
             \x20   (income:consulting)     $-40000.00  ; growth: 25%/yr\n",
        ),
        ENGLISH_JOURNAL,
        72,
    );
    assert!(
        runway_key(&growing) >= runway_key(&flat),
        "revenue growing at 25%/yr must make the money last at least as long as flat \
         revenue does: flat {}, growing {}",
        runway_words(&flat),
        runway_words(&growing)
    );
    assert!(
        runway(&flat).is_some(),
        "the control half of this test must actually run out of money, or it is \
         comparing two scenarios that both last forever and proves nothing"
    );
    for (bucket, (grown, level)) in cash(&growing).iter().zip(cash(&flat)).enumerate() {
        assert!(
            *grown >= level,
            "bucket {bucket}: growing revenue must leave no LESS cash than flat \
             revenue, but growing closed at {grown} against flat's {level}"
        );
    }
}

// ===========================================================================
// The classification guard
// ===========================================================================

/// **The regression guard for classification by name.**
///
/// The same scenario, once under `expenses:`/`income:`/`assets:` and once under
/// `cogs:`/`ingresos:`/`activo:` with a `type:` on each, must produce the same
/// cash, the same net worth, the same runway and the same net-income totals.
/// Only the row NAMES may differ.
///
/// Anything that reads a root name instead of the declared type fails here:
/// the Spanish half either reports zeroes (the account is in no section) or, on
/// this tab, gets its sign flipped and projects a cost as income.
#[test]
fn a_cogs_rooted_chart_projects_identically_to_an_expenses_rooted_one() {
    let english = project_file(
        &file(
            "    (expenses:payroll)      $100000.00  ; growth: 3%/yr\n\
             \x20   (income:consulting)     $-40000.00\n",
        ),
        ENGLISH_JOURNAL,
        60,
    );
    let spanish = project_file(
        &file(
            "    (cogs:nomina)             $100000.00  ; growth: 3%/yr\n\
             \x20   (ingresos:consultoria)    $-40000.00\n",
        ),
        SPANISH_JOURNAL,
        60,
    );

    assert_eq!(
        cash(&spanish),
        cash(&english),
        "a chart that books costs under `cogs: ; type: X` must project the SAME cash \
         as one under `expenses:` — classification is by declared type, never by name"
    );
    assert_eq!(
        net_worth(&spanish),
        net_worth(&english),
        "…and the same net worth"
    );
    assert_eq!(
        runway(&spanish),
        runway(&english),
        "…and the same runway. A different date here means one of the two charts had \
         a line's sign or section decided by its root name"
    );
    assert_eq!(
        totals(&spanish),
        totals(&english),
        "…and the same net income in every bucket"
    );
    assert!(
        totals(&spanish).iter().all(|total| *total < 0),
        "and the shared answer must be a LOSS: $100,000 of costs against $40,000 of \
         revenue. A non-negative total here means the Spanish accounts landed in no \
         section at all and the report is reading zero"
    );
}

// ===========================================================================
// A paper gain is not money
// ===========================================================================

/// An asset row compounds a BALANCE. That is net worth, not cash — nobody can
/// spend an unrealised gain, and a projection that let one pay the rent would
/// hide a burn behind a rising market.
#[test]
fn an_asset_rows_appreciation_never_moves_cash() {
    const INVESTED: &str = "\
2026-01-01 opening balances
    assets:checking          $3000000.00
    assets:brokerage          $200000.00
    equity:opening          $-3200000.00
";
    let burn_only = project_file(
        &file("    (expenses:payroll)    $100000.00\n"),
        INVESTED,
        36,
    );
    // A second `~` rule, so the asset row is its own group — exactly as the
    // table writes one.
    let with_growth = project_file(
        "; Ledgeline projection\n; projection: test\n\n\
         ~ monthly  projection\n    (expenses:payroll)    $100000.00\n\n\
         ~ monthly  projection\n    (assets:brokerage)         $0.00  ; growth: 20%/yr\n",
        INVESTED,
        36,
    );

    assert_eq!(
        cash(&with_growth),
        cash(&burn_only),
        "a 20%/yr gain on a $200,000 brokerage account must not move CASH by one cent \
         — an unrealised gain is not money in the bank"
    );
    assert_eq!(
        runway(&with_growth),
        runway(&burn_only),
        "…so it must not move the runway either. A growing asset must never mask a burn"
    );
    assert_eq!(
        totals(&with_growth),
        totals(&burn_only),
        "…and appreciation is not income, so net income is unmoved too"
    );
    // It IS net worth, though — that is the honest answer, and the reason cash is
    // the series the runway is read off.
    assert!(
        net_worth(&with_growth).last() > net_worth(&burn_only).last(),
        "the gain must still show up in NET WORTH: it is real, it is just not cash"
    );
    assert_falls_every_bucket(
        &cash(&with_growth),
        "and the burn underneath must still be visible in every bucket",
    );
}
