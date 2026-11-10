//! Plan 34: Compare vs actuals. A backtest is the projection made on the
//! first actual day `F`, run over the N actual periods, set beside what
//! happened. These pin the anchor (D1), the scenario-as-written table (D2),
//! valuation (D3) and the joined statements.
//!
//! The full-scan equivalence of the joined statements is in
//! `projection_cache.rs`; the hledger parity of the window is in
//! `projection_files.rs`.

use ledgeline_core::Journal;
use ledgeline_core::decimal::Dec;
use ledgeline_core::model::Transaction;
use ledgeline_core::parse_journal;
use ledgeline_core::projections::serialize::scenario_from_text;
use ledgeline_core::projections::{
    AccountsOpts, ComparedStatements, JournalFacts, PROJECTED_KEY_PREFIX, Projection, RealKey,
    RealSide, Scenario, StatementOpts, backtest_warnings, compared_accounts, compared_statements,
    project_backtest, project_cached, projected_accounts, projected_statements,
};
use ledgeline_core::reports::account_report::AccountReport;
use ledgeline_core::reports::income_statement::{IsSectionKind, IsSubtotalKind};
use ledgeline_core::reports::{Interval, MixedAmount, Valuation};

const TODAY: &str = "2026-10-20";

/// Today 2026-10-20, monthly, lookback 3: `C` = 2026-10-01, `F` = 2026-07-01,
/// the window is Jul–Sep 2026.
///
/// - food is paid from checking before `F` and from savings three times
///   inside the window, so a counter-less food line defaults to SAVINGS at
///   `C` and to CHECKING at `F`;
/// - the June card dinner settles on 2026-08-03: a late posting at `F`;
/// - AAPL is repriced inside the window (2026-08-15);
/// - a real transaction after `C` never reaches either block.
const MAIN: &str = "\
commodity $1,000.00
commodity 1,000.0000 AAPL

account assets:checking    ; type: C
account assets:savings     ; type: C
account assets:broker      ; type: A
account assets:house       ; type: A
account liabilities:card   ; type: L
account equity:opening     ; type: E
account equity:gains       ; type: E
account revenues:salary    ; type: R, issection: revenue
account expenses:food      ; type: X
account expenses:rent      ; type: X
account expenses:cogs      ; type: X, issection: cogs

P 2026-01-01 AAPL $100.00
P 2026-05-15 AAPL $110.00
P 2026-08-15 AAPL $130.00

2026-01-01 opening
    assets:checking    $10,000.00
    assets:savings      $5,000.00
    assets:house      $300,000.00
    equity:opening

2026-02-01 buy
    assets:broker      10 AAPL @ $100.00
    assets:checking

2026-03-05 food
    expenses:food          $50.00
    assets:checking

2026-06-20 card dinner, settles in august
    expenses:food          $40.00
    liabilities:card  ; date: 2026-08-03

2026-07-05 food
    expenses:food          $60.00
    assets:savings

2026-07-25 salary
    revenues:salary    $-4,000.00
    assets:checking

2026-08-05 food
    expenses:food          $70.00
    assets:savings

2026-08-25 salary
    revenues:salary    $-4,000.00
    assets:checking

2026-09-05 food
    expenses:food          $80.00
    assets:savings

2026-09-25 salary
    revenues:salary    $-4,500.00
    assets:checking

2026-10-05 after the cutoff
    expenses:food          $11.00
    assets:checking
";

/// Every line names its counter.
const EXPLICIT: &str = "\
~ monthly  salary
    revenues:salary    $-4,000.00
    assets:checking

~ monthly  food
    expenses:food         $100.00
    assets:checking
";

fn journal(text: &str) -> Journal {
    parse_journal(text, "main.journal").expect("journal parses")
}

fn scenario(text: &str, facts: &JournalFacts) -> Scenario {
    scenario_from_text(text, "s.journal", "case", &facts.shape().declared)
        .unwrap_or_else(|e| panic!("scenario must read: {e}"))
}

fn exact<T: std::fmt::Debug>(value: &T) -> String {
    format!("{value:#?}")
}

fn opts() -> StatementOpts {
    StatementOpts {
        valuation: Valuation::Market,
        bs_depth: None,
        is_depth: None,
        cf_depth: 2,
    }
}

/// The two sides for `journal` at `TODAY`, monthly, with `lookback`.
struct Sides {
    facts: JournalFacts,
    main: RealSide,
    backtest: RealSide,
}

impl Sides {
    fn new(journal: &Journal, lookback: usize) -> Self {
        let facts = JournalFacts::new(journal).expect("facts");
        let key = facts
            .key(TODAY, Interval::Monthly, lookback, None)
            .expect("key");
        let main = RealSide::build(journal, &facts, &key).expect("main side");
        let backtest_key = key.backtest().expect("key math").expect("a backtest");
        let backtest = RealSide::build(journal, &facts, &backtest_key).expect("backtest side");
        Self {
            facts,
            main,
            backtest,
        }
    }

    fn project(&self, scenario: &Scenario) -> Projection {
        project_backtest(scenario, &self.main, &self.backtest, 2).expect("backtest")
    }

    fn statements(&self, projection: &Projection) -> ComparedStatements {
        compared_statements(
            &self.main,
            &self.backtest,
            &self.facts,
            &projection.forecast,
            &opts(),
        )
        .expect("compared statements")
    }

    fn accounts(&self, projection: &Projection) -> AccountReport {
        compared_accounts(
            &self.main,
            &self.backtest,
            &self.facts,
            &projection.forecast,
            &AccountsOpts {
                depth: 9,
                empty: false,
            },
        )
        .expect("compared accounts")
    }
}

/// Every generated posting as `date account amount`, for readable asserts.
fn postings(txns: &[Transaction]) -> Vec<String> {
    txns.iter()
        .flat_map(|txn| {
            txn.postings.iter().map(move |posting| {
                let amounts: Vec<String> = posting
                    .amounts
                    .iter()
                    .map(|amount| format!("{}{}", dec(&amount.quantity), amount.commodity.0))
                    .collect();
                format!(
                    "{} {} {}",
                    posting.date.as_deref().unwrap_or(&txn.date),
                    posting.account.0,
                    amounts.join("+")
                )
            })
        })
        .collect()
}

fn dates(projection: &Projection) -> Vec<String> {
    projection
        .forecast
        .txns
        .iter()
        .map(|txn| txn.date.clone())
        .collect()
}

/// A decimal as written: `-40.00`.
fn dec(value: &Dec) -> String {
    let digits = format!(
        "{:0>width$}",
        value.mantissa.unsigned_abs(),
        width = value.places as usize + 1
    );
    let (int, frac) = digits.split_at(digits.len() - value.places as usize);
    let sign = if value.mantissa < 0 { "-" } else { "" };
    if frac.is_empty() {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{frac}")
    }
}

fn units(ma: &MixedAmount, commodity: &str) -> String {
    ma.iter()
        .find(|(c, _)| c.0 == commodity)
        .map_or_else(|| "0".to_string(), |(_, q)| dec(q))
}

// ===========================================================================
// D1: the key and the anchor
// ===========================================================================

#[test]
fn the_backtest_key_is_the_first_lookback_day() {
    let journal = journal(MAIN);
    let facts = JournalFacts::new(&journal).unwrap();
    let cases = [
        (Interval::Monthly, 3, "2026-10-01", "2026-07-01"),
        (Interval::Quarterly, 2, "2026-10-01", "2026-04-01"),
        (Interval::Yearly, 1, "2026-01-01", "2025-01-01"),
        // 2026-10-05 is the latest real day, so the week holding 10-06 is
        // the cutoff (Monday 2026-10-05); two weeks before it is 09-21.
        (Interval::Weekly, 2, "2026-10-05", "2026-09-21"),
    ];
    for (interval, lookback, cutoff, from) in cases {
        let key = facts.key(TODAY, interval, lookback, None).unwrap();
        assert_eq!(key.cutoff, cutoff, "{interval:?}");
        let backtest = key.backtest().unwrap().expect("a backtest");
        assert_eq!(
            backtest,
            RealKey {
                cutoff: from.to_string(),
                interval,
                lookback: 0,
                value_in: None,
            },
            "{interval:?}"
        );
    }
    let none = facts.key(TODAY, Interval::Monthly, 0, None).unwrap();
    assert_eq!(none.backtest().unwrap(), None);
}

#[test]
fn a_side_that_is_not_the_backtest_is_refused() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let facts = &sides.facts;
    let s = scenario(EXPLICIT, facts);
    assert!(project_backtest(&s, &sides.main, &sides.main, 2).is_err());
    let other = RealSide::build(
        &journal,
        facts,
        &facts.key(TODAY, Interval::Monthly, 2, None).unwrap(),
    )
    .unwrap();
    assert!(project_backtest(&s, &other, &sides.backtest, 2).is_err());
}

#[test]
fn real_transactions_inside_the_window_never_reach_the_projected_block() {
    let s = |facts: &JournalFacts| scenario(EXPLICIT, facts);
    let before = journal(MAIN);
    let after = journal(&format!(
        "{MAIN}\n2026-08-10 a windfall inside the window\n    assets:checking  $9,999.00\n    revenues:salary\n"
    ));
    let (a, b) = (Sides::new(&before, 3), Sides::new(&after, 3));
    let (pa, pb) = (a.project(&s(&a.facts)), b.project(&s(&b.facts)));
    // The windfall shifts every generated index by one; nothing else moves.
    assert_eq!(postings(&pa.forecast.txns), postings(&pb.forecast.txns));
    assert_eq!(exact(&pa.cash), exact(&pb.cash));
    let (sa, sb) = (a.statements(&pa), b.statements(&pb));
    // The projected block is byte-identical...
    assert_eq!(
        exact(&sa.balance_sheet[3..].to_vec()),
        exact(&sb.balance_sheet[3..].to_vec())
    );
    // ...and the actual block moved.
    assert_ne!(
        exact(&sa.balance_sheet[..3].to_vec()),
        exact(&sb.balance_sheet[..3].to_vec())
    );
    let net = |c: &ComparedStatements| c.income_statement.net_income.values.clone();
    assert_eq!(net(&sa)[3..], net(&sb)[3..]);
    assert_ne!(net(&sa)[1], net(&sb)[1]);
}

#[test]
fn a_late_posting_before_the_window_is_replayed_into_the_projected_block() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let projection = sides.project(&scenario(EXPLICIT, &sides.facts));
    let report = sides.accounts(&projection);
    let card = report
        .sections
        .iter()
        .flat_map(|section| &section.rows)
        .find(|row| row.account == "liabilities:card")
        .expect("the card row");
    // Jul, Aug, Sep actual; Jul, Aug, Sep as projected. The card is charged
    // in June and settles on Aug 3 in both blocks.
    let values: Vec<String> = card.values.iter().map(|ma| units(ma, "$")).collect();
    assert_eq!(values, ["0", "-40.00", "-40.00", "0", "-40.00", "-40.00"]);
}

#[test]
fn the_actual_block_is_the_forward_runs_lookback_columns() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let s = scenario(EXPLICIT, &sides.facts);
    let forward = project_cached(&s, &sides.main, 6, 2).unwrap();
    let fwd = projected_statements(&sides.main, &sides.facts, &forward.forecast, &opts()).unwrap();
    let compared = sides.statements(&sides.project(&s));

    assert_eq!(compared.actual, fwd.spans[..3]);
    assert_eq!(
        exact(&compared.balance_sheet[..3].to_vec()),
        exact(&fwd.balance_sheet[..3].to_vec())
    );
    assert_eq!(
        compared.income_statement.net_income.values[..3],
        fwd.income_statement.net_income.values[..3]
    );
    for section in &compared.income_statement.sections {
        let other = fwd
            .income_statement
            .sections
            .iter()
            .find(|other| other.kind == section.kind)
            .expect("same box");
        assert_eq!(section.total.values[..3], other.total.values[..3]);
    }
    let cf = &compared.cash_flow;
    assert_eq!(
        cf.net_change.values[..3],
        fwd.cash_flow.net_change.values[..3]
    );
    assert_eq!(
        cf.ending_cash.values[..3],
        fwd.cash_flow.ending_cash.values[..3]
    );
    assert_eq!(
        cf.beginning_cash.values[..3],
        fwd.cash_flow.beginning_cash.values[..3]
    );

    let accounts = AccountsOpts {
        depth: 9,
        empty: false,
    };
    let fwd_accounts =
        projected_accounts(&sides.main, &sides.facts, &forward.forecast, &accounts).unwrap();
    let cmp_accounts = sides.accounts(&sides.project(&s));
    for section in &fwd_accounts.sections {
        let other = cmp_accounts
            .sections
            .iter()
            .find(|other| other.section == section.section)
            .expect("same section");
        assert_eq!(
            section.totals[..3],
            other.totals[..3],
            "{:?}",
            section.section
        );
    }
    // The forward run's real closing cash is the actual block's.
    assert_eq!(
        forward.cash.history,
        compared.cash_flow.ending_cash.values[..3]
    );
}

#[test]
fn the_projected_block_is_a_projection_made_on_the_first_day() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let s = scenario(EXPLICIT, &sides.facts);
    let backtest = sides.project(&s);
    // A run on F itself, lookback 0, count N.
    let on_f = sides
        .facts
        .key("2026-07-01", Interval::Monthly, 0, None)
        .unwrap();
    assert_eq!(&on_f, sides.backtest.key());
    let run = project_cached(&s, &sides.backtest, 3, 2).unwrap();
    // Every counter is explicit, so the generated transactions agree.
    assert_eq!(exact(&backtest.forecast.txns), exact(&run.forecast.txns));
    assert_eq!(exact(&backtest.cash), exact(&run.cash));
    assert_eq!(exact(&backtest.net_worth), exact(&run.net_worth));
    let compared = sides.statements(&backtest);
    let alone =
        projected_statements(&sides.backtest, &sides.facts, &run.forecast, &opts()).unwrap();
    assert_eq!(
        exact(&compared.balance_sheet[3..].to_vec()),
        exact(&alone.balance_sheet)
    );
    assert_eq!(
        compared.income_statement.net_income.values[3..],
        alone.income_statement.net_income.values[..]
    );
    assert_eq!(
        compared.cash_flow.ending_cash.values[3..],
        alone.cash_flow.ending_cash.values[..]
    );
    // The backtest's closing cash is its cash series.
    assert_eq!(
        backtest.cash.values,
        compared.cash_flow.ending_cash.values[3..]
    );
}

#[test]
fn a_counterless_line_takes_todays_default_not_one_inferred_at_the_window_start() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let s = scenario(
        "~ monthly  food\n    (expenses:food)   $100.00\n",
        &sides.facts,
    );
    let forward = project_cached(&s, &sides.main, 3, 2).unwrap();
    let at_f = project_cached(&s, &sides.backtest, 3, 2).unwrap();
    let backtest = sides.project(&s);
    let counter = |p: &Projection| {
        p.forecast.txns[0]
            .postings
            .iter()
            .find(|posting| posting.account.0 != "expenses:food")
            .map(|posting| posting.account.0.clone())
            .expect("a counter")
    };
    // Paid from savings three times in the window: today's default.
    assert_eq!(counter(&forward), "assets:savings");
    // A projection really made on F would only have seen checking...
    assert_eq!(counter(&at_f), "assets:checking");
    // ...and the backtest takes today's, the one the editor shows.
    assert_eq!(counter(&backtest), "assets:savings");
    assert_eq!(
        exact(&backtest.forecast.defaults),
        exact(&forward.forecast.defaults)
    );
}

// ===========================================================================
// D2: the scenario exactly as written
// ===========================================================================

#[test]
fn rules_fire_by_their_own_dates_inside_the_window() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let run = |text: &str| dates(&sides.project(&scenario(text, &sides.facts)));
    let rule =
        |period: &str| format!("~ {period}  r\n    expenses:rent  $1.00\n    assets:checking\n");

    // `from` before F: occurrences inside the window count.
    assert_eq!(
        run(&rule("monthly from 2026-01-01")),
        ["2026-07-01", "2026-08-01", "2026-09-01"]
    );
    // `from` inside the window.
    assert_eq!(
        run(&rule("monthly from 2026-08-01")),
        ["2026-08-01", "2026-09-01"]
    );
    // `from` after the window.
    assert!(run(&rule("monthly from 2026-11-01")).is_empty());
    // `to` before F.
    assert!(run(&rule("monthly to 2026-06-01")).is_empty());
    // A bare rule is phased from F, the span start.
    assert_eq!(
        run(&rule("every 2 weeks")),
        [
            "2026-07-01",
            "2026-07-15",
            "2026-07-29",
            "2026-08-12",
            "2026-08-26",
            "2026-09-09",
            "2026-09-23"
        ]
    );
    // A step pair: whichever segment covers each date.
    let step = "\
~ monthly to 2026-08-01  r
    expenses:rent  $1.00  ; line: r
    assets:checking

~ monthly from 2026-08-01  r
    expenses:rent  $2.00  ; line: r
    assets:checking
";
    let amounts: Vec<String> = sides
        .project(&scenario(step, &sides.facts))
        .forecast
        .txns
        .iter()
        .map(|txn| dec(&txn.postings[0].amounts[0].quantity))
        .collect();
    assert_eq!(amounts, ["1.00", "2.00", "2.00"]);
}

#[test]
fn events_fire_only_inside_the_window() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let event =
        |date: &str| format!("~ {date}  e\n    expenses:rent  $5.00\n    assets:checking\n");
    let text = [
        event("2026-06-15"),
        event("2026-08-15"),
        event("2026-10-15"),
    ]
    .join("\n");
    let backtest = sides.project(&scenario(&text, &sides.facts));
    // The forward run (cutoff C) never shows the August event; the backtest does.
    assert_eq!(dates(&backtest), ["2026-08-15"]);
    let forward = project_cached(&scenario(&text, &sides.facts), &sides.main, 3, 2).unwrap();
    assert_eq!(dates(&forward), ["2026-10-15"]);
}

#[test]
fn disabled_entries_and_auto_rules_behave_as_in_the_forward_run() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let text = "\
~ monthly  rent
    expenses:rent      $1,000.00
    assets:checking

; ~ monthly  disabled rule
;     expenses:food      $1.00
;     assets:checking

; ~ 2026-08-15  disabled event
;     expenses:food      $1.00
;     assets:checking

= expenses:rent
    expenses:food        *0.10
    assets:checking     *-0.10

; = expenses:rent
;     expenses:cogs        *0.50
;     assets:checking     *-0.50
";
    let backtest = sides.project(&scenario(text, &sides.facts));
    assert_eq!(
        postings(&backtest.forecast.txns),
        [
            "2026-07-01 expenses:rent 1000.00$",
            "2026-07-01 expenses:food 100$",
            "2026-07-01 assets:checking -100$",
            "2026-07-01 assets:checking -1000.00$",
            "2026-08-01 expenses:rent 1000.00$",
            "2026-08-01 expenses:food 100$",
            "2026-08-01 assets:checking -100$",
            "2026-08-01 assets:checking -1000.00$",
            "2026-09-01 expenses:rent 1000.00$",
            "2026-09-01 expenses:food 100$",
            "2026-09-01 assets:checking -100$",
            "2026-09-01 assets:checking -1000.00$",
        ]
    );
}

#[test]
fn growth_on_a_flow_is_anchored_at_the_window_start() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    // 12%/mo compounds monthly from F: Jul is the base, Aug +12%, Sep +25.44%.
    let text = "~ monthly  r\n    expenses:rent  $100.00  ; growth: 12%/mo\n    assets:checking\n";
    let amounts: Vec<String> = sides
        .project(&scenario(text, &sides.facts))
        .forecast
        .txns
        .iter()
        .map(|txn| dec(&txn.postings[0].amounts[0].quantity))
        .collect();
    assert_eq!(amounts, ["100.00", "112.00", "125.44"]);
}

#[test]
fn an_opening_override_is_ignored_and_the_account_starts_from_its_real_balance() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let text = "\
~ monthly  house
    (assets:house)     $0.00  ; growth: 1%/mo, opening: 400000.00, gains: equity:gains
";
    let s = scenario(text, &sides.facts);
    let backtest = sides.project(&s);
    let row = &backtest.assets[0];
    // The journal's balance on F, not the override.
    assert_eq!(units(&row.opening, "$"), "300000.00");
    assert_eq!(row.opening, row.journal_opening);
    // No adjustment transaction, and no warning about one; the growth steps
    // still post.
    assert!(
        backtest
            .forecast
            .txns
            .iter()
            .all(|txn| !txn.description.contains("opening adjustment")),
        "{:#?}",
        backtest.forecast.txns
    );
    assert!(
        backtest
            .warnings
            .iter()
            .all(|warning| !warning.contains("opening balance is stated")),
        "{:?}",
        backtest.warnings
    );
    assert!(!backtest.forecast.txns.is_empty());
    // The forward run over the same scenario still applies it.
    let forward = project_cached(&s, &sides.main, 3, 2).unwrap();
    assert_eq!(units(&forward.assets[0].opening, "$"), "400000.00");
    assert!(
        forward
            .forecast
            .txns
            .iter()
            .any(|txn| txn.description.contains("opening adjustment"))
    );
}

#[test]
fn a_priced_row_with_an_override_grows_through_prices_in_the_backtest() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let text = "~ monthly  broker\n    (assets:broker)  $0.00  ; growth: 1%/mo, opening: 5000.00\n";
    let s = scenario(text, &sides.facts);
    let backtest = sides.project(&s);
    assert!(
        backtest
            .forecast
            .prices
            .iter()
            .all(|price| price.commodity.0 == "AAPL"),
    );
    assert!(!backtest.forecast.prices.is_empty(), "grown AAPL prices");
    let forward = project_cached(&s, &sides.main, 3, 2).unwrap();
    assert!(
        forward.forecast.prices.is_empty(),
        "an overridden row compounds as one balance going forward"
    );
}

// ===========================================================================
// D3: valuation
// ===========================================================================

#[test]
fn grown_prices_start_from_the_last_real_price_before_the_window() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let text = "~ monthly  broker\n    (assets:broker)  $0.00  ; growth: 1%/mo\n";
    let backtest = sides.project(&scenario(text, &sides.facts));
    let first = &backtest.forecast.prices[0];
    // One 1% step from May's $110, never from August's $130.
    assert_eq!(first.date, "2026-08-01");
    assert_eq!(dec(&first.price.quantity), "111.1000", "{first:?}");
}

#[test]
fn a_real_price_inside_the_window_values_only_the_actual_block() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let compared = sides.statements(&sides.project(&scenario(EXPLICIT, &sides.facts)));
    let broker = |column: usize| {
        compared.balance_sheet[column]
            .sections
            .iter()
            .flat_map(|section| &section.groups)
            .flat_map(|group| &group.rows)
            .find(|row| row.account == "assets:broker")
            .map(|row| units(&row.inclusive, "$"))
            .expect("broker row")
    };
    // Actual: Jul at $110, Aug and Sep at $130. As projected: $110 throughout.
    assert_eq!(broker(0), "1100.00");
    assert_eq!(broker(1), "1300.00");
    assert_eq!(broker(2), "1300.00");
    assert_eq!(broker(3), "1100.00");
    assert_eq!(broker(4), "1100.00");
    assert_eq!(broker(5), "1100.00");
}

#[test]
fn matching_targets_add_no_warning() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let s = scenario(EXPLICIT, &sides.facts);
    let forward = project_cached(&s, &sides.main, 3, 2).unwrap();
    let backtest = sides.project(&s);
    assert!(backtest_warnings(&forward, &backtest).is_empty());
}

// ===========================================================================
// The joins
// ===========================================================================

#[test]
fn the_income_statement_ladder_is_computed_over_both_blocks() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let text = "\
~ monthly  salary
    revenues:salary    $-4,000.00
    assets:checking

~ monthly  cogs
    expenses:cogs       $1,000.00
    assets:checking
";
    let compared = sides.statements(&sides.project(&scenario(text, &sides.facts)));
    let is = &compared.income_statement;
    assert!(
        is.multi_step,
        "COGS in the projected block makes it multi-step"
    );
    let revenue = is
        .sections
        .iter()
        .find(|section| section.kind == IsSectionKind::Revenue)
        .expect("revenue box");
    let gross = revenue
        .trailing
        .iter()
        .find(|subtotal| subtotal.kind == IsSubtotalKind::GrossProfit)
        .or_else(|| {
            is.sections
                .iter()
                .flat_map(|section| &section.trailing)
                .find(|subtotal| subtotal.kind == IsSubtotalKind::GrossProfit)
        })
        .expect("a Gross profit line");
    // Actual Gross profit is actual revenue (no COGS happened).
    assert_eq!(gross.total.values[..3], revenue.total.values[..3]);
    assert_ne!(gross.total.values[3], revenue.total.values[3]);
}

#[test]
fn cash_flow_sections_agree_across_blocks_under_the_union_investments_set() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let compared = sides.statements(&sides.project(&scenario(EXPLICIT, &sides.facts)));
    let cf = &compared.cash_flow;
    assert_eq!(cf.buckets.len(), 6);
    for section in &cf.sections {
        for line in &section.lines {
            assert_eq!(line.total.values.len(), 6, "{}", line.name);
        }
    }
    // Both blocks start from the real cash on F.
    assert_eq!(cf.beginning_cash.values[0], cf.beginning_cash.values[3]);
    // The investing line (the broker holds AAPL) is one line in both blocks.
    let investing: Vec<&str> = cf
        .sections
        .iter()
        .flat_map(|section| section.lines.iter().flat_map(|line| &line.rows))
        .filter(|row| row.account == "assets:broker")
        .map(|row| row.account.as_str())
        .collect();
    assert!(investing.len() <= 1, "{investing:?}");
}

#[test]
fn projected_column_keys_are_prefixed_and_unique() {
    let journal = journal(MAIN);
    let sides = Sides::new(&journal, 3);
    let compared = sides.statements(&sides.project(&scenario(EXPLICIT, &sides.facts)));
    let keys: Vec<&str> = compared
        .income_statement
        .buckets
        .iter()
        .map(|span| span.key.as_str())
        .collect();
    assert_eq!(
        keys,
        [
            "2026-07",
            "2026-08",
            "2026-09",
            "projected:2026-07",
            "projected:2026-08",
            "projected:2026-09"
        ]
    );
    assert!(
        compared
            .projected
            .iter()
            .all(|span| span.key.starts_with(PROJECTED_KEY_PREFIX))
    );
    let cf: Vec<&str> = compared
        .cash_flow
        .buckets
        .iter()
        .map(|span| span.key.as_str())
        .collect();
    assert_eq!(cf, keys);
    let accounts = sides.accounts(&sides.project(&scenario(EXPLICIT, &sides.facts)));
    assert_eq!(accounts.buckets, keys);
    // Labels' dates are unchanged.
    assert_eq!(compared.projected[0].from, "2026-07-01");
    assert_eq!(compared.projected[2].to, "2026-09-30");
}
