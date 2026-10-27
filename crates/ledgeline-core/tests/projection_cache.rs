//! Plan 29: a projection run from a cached real side is IDENTICAL to the full
//! scan it replaces.
//!
//! The reference is the full-scan path: `project()` (every real read a scan),
//! and the statements and Accounts reading over `merged_transactions` /
//! `merged_prices` (`reference_statements`, `reference_accounts` — the run
//! endpoint's code before plan 29). The candidate is `RealSide::build` once,
//! then `project_cached`, `projected_statements` and `projected_accounts`.
//!
//! "Identical" is compared through `Debug`, which prints every `Dec`'s
//! mantissa AND scale: `Dec`'s `==` is by value, so `$1.5` and `$1.50` would
//! pass an `assert_eq!` on the values and still differ on the wire.

use std::path::PathBuf;

use ledgeline_core::Journal;
use ledgeline_core::model::Commodity;
use ledgeline_core::parse_journal;
use ledgeline_core::projections::serialize::scenario_from_text;
use ledgeline_core::projections::{
    AccountsOpts, JournalFacts, ProjectionOpts, RealSide, Scenario, StatementOpts, project,
    project_cached, projected_accounts, projected_statements, reference_accounts,
    reference_statements,
};
use ledgeline_core::reports::{BalanceMode, Interval, Valuation};
use proptest::prelude::*;

fn exact<T: std::fmt::Debug>(value: &T) -> String {
    format!("{value:#?}")
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn read(relative: &str) -> String {
    let path = fixtures().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn journal_from(text: &str, name: &str) -> Journal {
    parse_journal(text, name).unwrap_or_else(|e| panic!("{name} must parse: {e}"))
}

fn scenario_from(text: &str, name: &str, facts: &JournalFacts) -> Scenario {
    scenario_from_text(text, name, "case", &facts.shape().declared)
        .unwrap_or_else(|e| panic!("{name} must read: {e}"))
}

/// The forecast periods for an interval: enough to cross a growth step and a
/// year, few enough to keep a daily run small.
fn count_for(interval: Interval) -> usize {
    match interval {
        Interval::Daily => 40,
        Interval::Weekly => 20,
        Interval::Monthly => 15,
        Interval::Quarterly => 6,
        Interval::Yearly => 3,
    }
}

const INTERVALS: [Interval; 5] = [
    Interval::Daily,
    Interval::Weekly,
    Interval::Monthly,
    Interval::Quarterly,
    Interval::Yearly,
];
const LOOKBACKS: [usize; 3] = [0, 2, 7];
const DEPTHS: [usize; 3] = [1, 2, usize::MAX];
const VALUATIONS: [Valuation; 3] = [Valuation::Market, Valuation::Cost, Valuation::None];

/// One run's inputs.
struct Run<'a> {
    label: String,
    journal: &'a Journal,
    facts: &'a JournalFacts,
    real: &'a RealSide,
    scenario: &'a Scenario,
    today: &'a str,
    interval: Interval,
    lookback: usize,
    count: usize,
    value_in: Option<Commodity>,
}

impl Run<'_> {
    /// Every reading, cached vs reference, at every depth and basis.
    fn assert_identical(&self) {
        let label = &self.label;
        for depth in DEPTHS {
            let reference = project(
                self.scenario,
                &self.journal.transactions,
                &self.journal.prices,
                &ProjectionOpts {
                    today: self.today,
                    interval: self.interval,
                    count: self.count,
                    lookback: self.lookback,
                    depth,
                    declared: &self.facts.shape().declared,
                    value_in: self.value_in.clone(),
                },
            )
            .unwrap_or_else(|e| panic!("{label}: reference project: {e}"));
            let cached = project_cached(self.scenario, self.real, self.count, depth)
                .unwrap_or_else(|e| panic!("{label}: cached project: {e}"));
            assert_eq!(
                exact(&cached),
                exact(&reference),
                "{label} depth {depth}: project"
            );

            for valuation in VALUATIONS {
                // The balance sheet's own "unclamped" is `None`, not a depth.
                let bs_depth = (depth != usize::MAX).then_some(depth);
                {
                    let opts = StatementOpts {
                        valuation,
                        bs_depth,
                        cf_depth: depth,
                    };
                    let want =
                        reference_statements(self.journal, self.facts, &reference.forecast, &opts)
                            .unwrap_or_else(|e| panic!("{label}: reference statements: {e}"));
                    let got = projected_statements(self.real, self.facts, &cached.forecast, &opts)
                        .unwrap_or_else(|e| panic!("{label}: cached statements: {e}"));
                    assert_eq!(exact(&got), exact(&want), "{label} {opts:?}: statements");
                }
            }
            for mode in [BalanceMode::Change, BalanceMode::Historical] {
                for empty in [false, true] {
                    let opts = AccountsOpts { mode, depth, empty };
                    let want = reference_accounts(self.journal, &reference.forecast, &opts)
                        .unwrap_or_else(|e| panic!("{label}: reference accounts: {e}"));
                    let got = projected_accounts(self.real, self.facts, &cached.forecast, &opts)
                        .unwrap_or_else(|e| panic!("{label}: cached accounts: {e}"));
                    assert_eq!(exact(&got), exact(&want), "{label} {opts:?}: accounts");
                }
            }
        }
    }
}

/// Every scenario in `scenarios` over `journal`, at every interval and
/// lookback, ONE real side per (interval, lookback, valueIn) shared by all of
/// them — what the server does across edits.
fn check_journal(
    name: &str,
    journal: &Journal,
    scenarios: &[(&str, String)],
    today: &str,
    values_in: &[Option<&str>],
) {
    let facts = JournalFacts::new(journal).expect("facts");
    let scenarios: Vec<(&str, Scenario)> = scenarios
        .iter()
        .map(|(file, text)| (*file, scenario_from(text, file, &facts)))
        .collect();
    for interval in INTERVALS {
        for lookback in LOOKBACKS {
            for value_in in values_in {
                let value_in = value_in.map(|symbol| Commodity(symbol.to_string()));
                let key = facts
                    .key(today, interval, lookback, value_in.clone())
                    .expect("key");
                let real = RealSide::build(journal, &facts, &key).expect("real side");
                for (file, scenario) in &scenarios {
                    Run {
                        label: format!(
                            "{name}/{file} {interval:?} lookback {lookback} valueIn {value_in:?}"
                        ),
                        journal,
                        facts: &facts,
                        real: &real,
                        scenario,
                        today,
                        interval,
                        lookback,
                        count: count_for(interval),
                        value_in: value_in.clone(),
                    }
                    .assert_identical();
                }
            }
        }
    }
}

/// Every scenario of the hledger parity suite over its main journal: explicit
/// counters at every cadence, events, legacy (counter-less) rules, an asset
/// row with gains, a step change, EUR lines and grown prices.
#[test]
fn the_parity_fixtures_are_identical_cached() {
    let main = journal_from(&read("projections-parity/main.journal"), "main.journal");
    let scenarios: Vec<(&str, String)> = [
        "flows.journal",
        "events.journal",
        "legacy.journal",
        "assets.journal",
        "step.journal",
        "commodities.journal",
        "growth.journal",
    ]
    .into_iter()
    .map(|file| (file, read(&format!("projections-parity/{file}"))))
    .collect();
    check_journal(
        "parity",
        &main,
        &scenarios,
        "2026-10-20",
        &[None, Some("EUR")],
    );
}

/// `sample.journal` (the e2e fixture: EUR, priced stocks, a mortgage, a
/// contra-asset, `bsgroup:`/`bsterm:`) with the asset-growth fixture (priced
/// growth, an `opening:` override, legacy virtual rows) and a scenario of
/// legacy flows, explicit counters and events in two commodities.
#[test]
fn the_sample_journal_is_identical_cached() {
    let sample = journal_from(&read("sample.journal"), "sample.journal");
    let mixed = "\
~ monthly  groceries
    (expenses:food:groceries)   $640.00

~ monthly  salary
    income:salary              $-6,500.00  ; growth: 3%/yr
    assets:bank:checking

~ monthly  travel
    (expenses:travel)            150.00 EUR

~ every 2 weeks  dining
    expenses:food:restaurants    $95.00
    liabilities:cc:visa

~ 2026-12-15  bonus
    income:salary              $-4,000.00

~ 2027-02-01  car repair
    expenses:transport            $900.00
    assets:bank:checking
";
    check_journal(
        "sample",
        &sample,
        &[
            (
                "asset-growth-scenario.journal",
                read("asset-growth-scenario.journal"),
            ),
            ("mixed.journal", mixed.to_string()),
        ],
        "2026-07-08",
        &[None, Some("EUR")],
    );
}

/// The expense-typed-as-asset repro and its fix, each with a seeded-style
/// scenario that posts to the umbrella account.
#[test]
fn the_expense_as_asset_fixtures_are_identical_cached() {
    let scenario = "\
~ monthly  G&A
    (expenses:ga)          $500.00
    (expenses:ga:rent)   $3,000.00

~ monthly  sales
    revenues:sales      $-21,000.00  ; growth: 2%/yr
    assets:mercury:checking
";
    for file in ["fixed.journal", "typo.journal"] {
        let journal = journal_from(&read(&format!("projections-expense-as-asset/{file}")), file);
        check_journal(
            file,
            &journal,
            &[("scenario.journal", scenario.to_string())],
            "2026-03-10",
            &[None],
        );
    }
}

/// Everything the seed/tail split has to get right, in one journal:
///
/// - a posting dated AFTER the cutoff on a transaction dated before it (the
///   late posting), on a cash account and on a card;
/// - a posting dated BEFORE its transaction's date;
/// - a real transaction after the cutoff, left out of every statement;
/// - a closing entry (revenue/expense against equity), skipped by the cash
///   flow statement;
/// - an unbalanced virtual posting, skipped by the cash flow statement;
/// - a costed cash posting (the exchange-rate line) and a stock bought at
///   cost, priced by `P` and grown by an asset row;
/// - a commodity that nets to zero inside an account, whose scale must
///   survive into a later column.
const EDGES: &str = "\
commodity $1,000.00
commodity 1,000.00 EUR

account assets:checking   ; type: C
account assets:wise       ; type: C
account assets:broker     ; type: A, bsterm: noncurrent
account assets:house      ; type: A, bsgroup: Property
account liabilities:card  ; type: L, cfgroup: Card
account equity:opening    ; type: E
account equity:retained   ; type: E
account revenues:salary   ; type: R, issection: revenue
account expenses:food     ; type: X, isgroup: Living
account expenses:rent     ; type: X
account expenses:fees     ; type: X, issection: other

P 2026-01-01 AAPL $100.00
P 2026-01-01 EUR $1.10
P 2026-08-15 AAPL $110.00
P 2026-09-20 EUR $1.12

2026-01-01 opening
    assets:checking   $10,000.00
    assets:house     $300,000.00
    equity:opening

2026-02-01 buy
    assets:broker     5 AAPL @ $100.00
    assets:checking

2026-03-01 change money
    assets:wise       500.00 EUR @ $1.1
    assets:checking

2026-04-01 salary
    revenues:salary   $-4,000.00
    assets:checking

2026-05-01 virtual note
    expenses:fees     $1.005
    assets:checking
    (assets:checking)  $3.00

2026-05-31 close the books
    revenues:salary    $4,000.00
    expenses:fees     $-1.005
    equity:retained

2026-06-01 zero then dollars
    assets:wise        1.000 EUR
    assets:wise       -1.000 EUR
    assets:checking    $0.000
    expenses:food      $0.000

2026-08-28 card dinner, settles next month
    expenses:food      $62.50
    liabilities:card   ; date: 2026-10-03

2026-09-02 rent, booked early
    expenses:rent      $1,500.00  ; date: 2026-08-31
    assets:checking

2026-09-25 transfer landing after the cutoff
    assets:wise         100.00 EUR @ $1.12  ; date: 2026-10-02
    assets:checking

2026-10-04 after the cutoff
    expenses:food      $11.00
    assets:checking
";

const EDGE_SCENARIO: &str = "\
~ monthly  rent
    expenses:rent      $1,500.00  ; growth: 2%/yr
    assets:checking

~ monthly  food
    (expenses:food)      $400.00

~ monthly  salary
    revenues:salary   $-4,100.00
    assets:checking

~ monthly  EUR spend
    (expenses:food)      80.00 EUR

~ monthly  assets
    (assets:broker)        $0.00  ; growth: 7%/yr
    (assets:house)         $0.00  ; growth: 3%/yr, gains: equity:retained

~ 2026-11-20  bonus
    revenues:salary   $-2,000.00
    assets:checking
";

#[test]
fn the_edge_journal_is_identical_cached() {
    let journal = journal_from(EDGES, "edges.journal");
    check_journal(
        "edges",
        &journal,
        &[("edges-scenario.journal", EDGE_SCENARIO.to_string())],
        "2026-10-20",
        &[None, Some("$"), Some("EUR")],
    );
}

/// An empty scenario still has every reading: the real side, projected flat.
#[test]
fn an_empty_scenario_is_identical_cached() {
    let journal = journal_from(EDGES, "edges.journal");
    check_journal(
        "edges",
        &journal,
        &[("empty.journal", "; projection: empty\n".to_string())],
        "2026-10-20",
        &[None],
    );
}

/// A journal with no transactions at all.
#[test]
fn an_empty_journal_is_identical_cached() {
    let journal = journal_from("account assets:cash  ; type: C\n", "empty.journal");
    check_journal(
        "empty",
        &journal,
        &[("edges-scenario.journal", EDGE_SCENARIO.to_string())],
        "2026-10-20",
        &[None],
    );
}

#[test]
fn a_forecast_of_another_window_is_refused_not_misread() {
    let journal = journal_from(EDGES, "edges.journal");
    let facts = JournalFacts::new(&journal).expect("facts");
    let scenario = scenario_from(EDGE_SCENARIO, "s.journal", &facts);
    let monthly = RealSide::build(
        &journal,
        &facts,
        &facts.key("2026-10-20", Interval::Monthly, 2, None).unwrap(),
    )
    .unwrap();
    let other = RealSide::build(
        &journal,
        &facts,
        &facts.key("2026-10-20", Interval::Monthly, 3, None).unwrap(),
    )
    .unwrap();
    let projection = project_cached(&scenario, &other, 6, 2).unwrap();
    let opts = StatementOpts {
        valuation: Valuation::Market,
        bs_depth: None,
        cf_depth: 2,
    };
    assert!(projected_statements(&monthly, &facts, &projection.forecast, &opts).is_err());
}

// ===========================================================================
// Random scenarios over generated journals
// ===========================================================================

const ACCOUNTS: &str = "\
commodity $1,000.00
commodity 1,000.00 EUR
account assets:checking      ; type: C
account assets:savings       ; type: C
account assets:wise          ; type: C
account assets:broker        ; type: A
account assets:house         ; type: A
account liabilities:card     ; type: L
account equity:opening       ; type: E
account equity:retained      ; type: E
account revenues:salary      ; type: R
account revenues:interest    ; type: R
account expenses:food        ; type: X
account expenses:food:out    ; type: X
account expenses:rent        ; type: X
P 2025-01-01 EUR $1.05
P 2025-01-01 AAPL $90.00
";

/// A date in 2025-01..2026-12 from a day index.
fn date(day: u32) -> String {
    let day = day % 720;
    let (year, rest) = (2025 + day / 360, day % 360);
    format!("{year}-{:02}-{:02}", rest / 30 + 1, rest % 30 % 28 + 1)
}

#[derive(Debug, Clone)]
enum RealTxn {
    Flow {
        day: u32,
        from: usize,
        to: usize,
        cents: i64,
        posting_day: Option<u32>,
    },
    Exchange {
        day: u32,
        eur: u32,
    },
    Stock {
        day: u32,
        shares: u32,
        price: u32,
    },
    Price {
        day: u32,
        eur_mills: u32,
        aapl: u32,
    },
    Virtual {
        day: u32,
        cents: i64,
    },
    Close {
        day: u32,
        cents: i64,
    },
}

const FLOW_ACCOUNTS: [&str; 10] = [
    "assets:checking",
    "assets:savings",
    "assets:house",
    "liabilities:card",
    "equity:opening",
    "revenues:salary",
    "revenues:interest",
    "expenses:food",
    "expenses:food:out",
    "expenses:rent",
];

fn real_txn() -> impl Strategy<Value = RealTxn> {
    prop_oneof![
        6 => (0u32..720, 0usize..10, 0usize..10, -500_000i64..500_000, proptest::option::weighted(0.15, 0u32..720))
            .prop_map(|(day, from, to, cents, posting_day)| RealTxn::Flow { day, from, to, cents, posting_day }),
        1 => (0u32..720, 1u32..900).prop_map(|(day, eur)| RealTxn::Exchange { day, eur }),
        1 => (0u32..720, 1u32..20, 50u32..200).prop_map(|(day, shares, price)| RealTxn::Stock { day, shares, price }),
        1 => (0u32..720, 900u32..1300, 50u32..200).prop_map(|(day, eur_mills, aapl)| RealTxn::Price { day, eur_mills, aapl }),
        1 => (0u32..720, -10_000i64..10_000).prop_map(|(day, cents)| RealTxn::Virtual { day, cents }),
        1 => (0u32..720, 1i64..100_000).prop_map(|(day, cents)| RealTxn::Close { day, cents }),
    ]
}

fn dollars(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let cents = cents.unsigned_abs();
    format!("${sign}{}.{:02}", cents / 100, cents % 100)
}

fn journal_text(txns: &[RealTxn]) -> String {
    let mut out = String::from(ACCOUNTS);
    out.push_str("\n2025-01-01 opening\n    assets:checking  $50,000.00\n    equity:opening\n");
    let mut sorted = txns.to_vec();
    sorted.sort_by_key(|txn| match txn {
        RealTxn::Flow { day, .. }
        | RealTxn::Exchange { day, .. }
        | RealTxn::Stock { day, .. }
        | RealTxn::Price { day, .. }
        | RealTxn::Virtual { day, .. }
        | RealTxn::Close { day, .. } => *day,
    });
    for txn in &sorted {
        match txn {
            RealTxn::Flow {
                day,
                from,
                to,
                cents,
                posting_day,
            } => {
                let (from, to) = (FLOW_ACCOUNTS[*from], FLOW_ACCOUNTS[(*to + 1 + *from) % 10]);
                let tag = posting_day
                    .map(|d| format!("  ; date: {}", date(d)))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "\n{} flow\n    {from}  {}{tag}\n    {to}\n",
                    date(*day),
                    dollars(*cents)
                ));
            }
            RealTxn::Exchange { day, eur } => out.push_str(&format!(
                "\n{} exchange\n    assets:wise  {eur}.00 EUR @ $1.1\n    assets:checking\n",
                date(*day)
            )),
            RealTxn::Stock { day, shares, price } => out.push_str(&format!(
                "\n{} buy\n    assets:broker  {shares} AAPL @ ${price}.00\n    assets:checking\n",
                date(*day)
            )),
            RealTxn::Price {
                day,
                eur_mills,
                aapl,
            } => out.push_str(&format!(
                "\nP {} EUR ${}.{:03}\nP {} AAPL ${aapl}.00\n",
                date(*day),
                eur_mills / 1000,
                eur_mills % 1000,
                date(*day)
            )),
            RealTxn::Virtual { day, cents } => out.push_str(&format!(
                "\n{} with a virtual\n    expenses:food  $1.00\n    assets:checking\n    (assets:savings)  {}\n",
                date(*day),
                dollars(*cents)
            )),
            RealTxn::Close { day, cents } => out.push_str(&format!(
                "\n{} close\n    revenues:salary  {}\n    expenses:rent  {}\n    equity:retained\n",
                date(*day),
                dollars(*cents),
                dollars(-*cents / 2)
            )),
        }
    }
    out
}

#[derive(Debug, Clone)]
enum Line {
    Flow {
        account: usize,
        counter: Option<usize>,
        cents: i64,
        period: usize,
        growth: Option<u32>,
        eur: bool,
    },
    Asset {
        account: usize,
        contribution: i64,
        rate: u32,
        gains: bool,
    },
    Event {
        day: u32,
        account: usize,
        cents: i64,
        counter: Option<usize>,
    },
}

const LINE_ACCOUNTS: [&str; 6] = [
    "revenues:salary",
    "revenues:interest",
    "expenses:food",
    "expenses:food:out",
    "expenses:rent",
    "liabilities:card",
];
const COUNTERS: [&str; 4] = [
    "assets:checking",
    "assets:savings",
    "assets:wise",
    "liabilities:card",
];
const ASSETS: [&str; 3] = ["assets:broker", "assets:house", "assets:savings"];
const PERIODS: [&str; 8] = [
    "monthly",
    "weekly",
    "every 2 weeks",
    "quarterly",
    "yearly",
    "monthly from 2026-11-15",
    "monthly to 2027-03-01",
    "every 15th day of month",
];

fn scenario_line() -> impl Strategy<Value = Line> {
    prop_oneof![
        6 => (0usize..6, proptest::option::of(0usize..4), -800_000i64..800_000, 0usize..8,
              proptest::option::weighted(0.4, 1u32..12), proptest::bool::weighted(0.2))
            .prop_map(|(account, counter, cents, period, growth, eur)| Line::Flow {
                account, counter, cents, period, growth, eur,
            }),
        2 => (0usize..3, 0i64..50_000, 0u32..10, any::<bool>())
            .prop_map(|(account, contribution, rate, gains)| Line::Asset {
                account, contribution, rate, gains,
            }),
        1 => (560u32..1000, 0usize..6, -900_000i64..900_000, proptest::option::of(0usize..4))
            .prop_map(|(day, account, cents, counter)| Line::Event { day, account, cents, counter }),
    ]
}

fn amount(cents: i64, eur: bool) -> String {
    if eur {
        let sign = if cents < 0 { "-" } else { "" };
        let cents = cents.unsigned_abs();
        format!("{sign}{}.{:02} EUR", cents / 100, cents % 100)
    } else {
        dollars(cents)
    }
}

/// A day index past the corpus (2026-08 onward), as an ISO date.
fn future(day: u32) -> String {
    let (year, rest) = (2026 + day / 360, day % 360);
    format!("{year}-{:02}-{:02}", rest / 30 + 1, rest % 30 % 28 + 1)
}

fn scenario_text(lines: &[Line]) -> String {
    let mut out = String::from("; projection: random\n");
    for (index, line) in lines.iter().enumerate() {
        match line {
            Line::Flow {
                account,
                counter,
                cents,
                period,
                growth,
                eur,
            } => {
                let growth = growth
                    .map(|rate| format!("  ; growth: {rate}%/yr"))
                    .unwrap_or_default();
                let posting = format!(
                    "{}  {}{growth}",
                    LINE_ACCOUNTS[*account],
                    amount(*cents, *eur)
                );
                match counter {
                    Some(counter) => out.push_str(&format!(
                        "\n~ {}  line {index}\n    {posting}\n    {}\n",
                        PERIODS[*period], COUNTERS[*counter]
                    )),
                    None => out.push_str(&format!(
                        "\n~ {}  line {index}\n    ({})  {}{}\n",
                        PERIODS[*period],
                        LINE_ACCOUNTS[*account],
                        amount(*cents, *eur),
                        growth
                    )),
                }
            }
            Line::Asset {
                account,
                contribution,
                rate,
                gains,
            } => {
                let gains = if *gains {
                    ", gains: equity:retained"
                } else {
                    ""
                };
                out.push_str(&format!(
                    "\n~ monthly  asset {index}\n    ({})  {}  ; growth: {rate}%/yr{gains}\n",
                    ASSETS[*account],
                    dollars(*contribution)
                ));
            }
            Line::Event {
                day,
                account,
                cents,
                counter,
            } => {
                let counter = counter
                    .map(|counter| format!("    {}\n", COUNTERS[counter]))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "\n~ {}  event {index}\n    {}  {}\n{counter}",
                    future(*day),
                    LINE_ACCOUNTS[*account],
                    dollars(*cents)
                ));
            }
        }
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, .. ProptestConfig::default() })]

    /// Random journals (late and early posting dates, closing entries,
    /// virtual postings, costed cash and stock, repricing) and random
    /// scenarios (flows with and without counters, growth, EUR, asset rows
    /// with gains, events), at a random today, interval, lookback and count.
    #[test]
    fn a_random_run_is_identical_cached(
        txns in proptest::collection::vec(real_txn(), 0..40),
        lines in proptest::collection::vec(scenario_line(), 0..8),
        today in 540u32..760,
        interval in 0usize..5,
        lookback in 0usize..5,
        count in 1usize..10,
        value_in in proptest::option::of(proptest::bool::ANY),
    ) {
        let text = journal_text(&txns);
        let journal = journal_from(&text, "random.journal");
        let facts = JournalFacts::new(&journal).expect("facts");
        let scenario = scenario_from(&scenario_text(&lines), "random-scenario.journal", &facts);
        let today = date(today);
        let interval = INTERVALS[interval];
        let value_in = value_in.map(|eur| Commodity(if eur { "EUR" } else { "$" }.to_string()));
        let key = facts.key(&today, interval, lookback, value_in.clone()).expect("key");
        let real = RealSide::build(&journal, &facts, &key).expect("real side");
        Run {
            label: format!("random today {today} {interval:?} lookback {lookback} count {count}"),
            journal: &journal,
            facts: &facts,
            real: &real,
            scenario: &scenario,
            today: &today,
            interval,
            lookback,
            count,
            value_in,
        }
        .assert_identical();
    }
}
