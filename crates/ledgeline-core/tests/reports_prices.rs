//! Transitive market-price valuation, pinned against hledger 1.52 (RPT-3).
//!
//! Every case below records the exact `hledger -f <journal> bal … --value=…`
//! invocation and the output it produced, so the expectations are hledger's
//! numbers rather than our own. The journals are inline because each one exists
//! to pin a single graph rule and is only meaningful next to that output.
//!
//! Rules covered: a multi-hop chain, a chain needing a reversed edge, a forward
//! chain preferred over a reverse edge, a cycle (must terminate), two
//! equal-length chains (tie-break), a genuinely unpriced commodity, and the
//! non-terminating reciprocal that used to drop a whole cash leg.

use ledgeline_core::model::Commodity;
use ledgeline_core::reports::{
    Interval, MixedAmount, NetWorthOpts, PostingFilter, PriceDb, ValuationMeta, account_decls,
    account_totals, declared_types, infer_market_prices, net_worth, value_at,
};
use ledgeline_core::{Dec, Journal, parse_journal};

fn journal(text: &str) -> Journal {
    parse_journal(text, "inline.journal").expect("journal parses")
}

/// The price set `net_worth` uses: costs first, then explicit `P` directives, so
/// an explicit price wins a same-date tie.
fn price_db(journal: &Journal) -> PriceDb {
    let mut all = infer_market_prices(&journal.transactions).expect("cost inference succeeds");
    all.extend_from_slice(&journal.prices);
    PriceDb::build(&all)
}

/// Everything under `assets` at `as_of`, as one mixed amount — the balance
/// `hledger bal assets` reports before valuation.
fn assets(journal: &Journal, as_of: &str) -> MixedAmount {
    account_totals(
        &journal.transactions,
        &PostingFilter {
            to: Some(as_of),
            ..PostingFilter::default()
        },
    )
    .expect("totals compute")
    .iter()
    .filter(|(account, _)| account.starts_with("assets"))
    .try_fold(MixedAmount::new(), |acc, (_, ma)| acc.ma_add(ma))
    .expect("summing assets does not overflow")
}

/// `hledger bal assets --value=end,<target>`, as a single quantity plus the
/// commodities that could not be valued.
fn valued_assets(text: &str, target: &str, as_of: &str) -> (Dec, Vec<Commodity>) {
    let journal = journal(text);
    let db = price_db(&journal);
    let mut meta = ValuationMeta::default();
    let value = value_at(
        &assets(&journal, as_of),
        &Commodity(target.to_string()),
        &db,
        as_of,
        Some(&mut meta),
    )
    .expect("valuation does not overflow");
    (value, meta.unpriced)
}

const GBP_CHAIN: &str = "\
P 2026-01-01 GBP 1.20 EUR
P 2026-01-02 EUR $1.10

2026-02-01 open
    assets:uk   100.00 GBP
    equity:open
";

/// RPT-3. GBP has no directive pricing it in `$`; hledger chains through EUR.
///
/// ```text
/// $ hledger -f chain.journal bal assets --value=end,'$'
///              $132.00  assets:uk
/// ```
#[test]
fn a_two_hop_chain_values_a_commodity_with_no_direct_price() {
    let (value, unpriced) = valued_assets(GBP_CHAIN, "$", "2026-02-28");
    assert_eq!(value, Dec::new(13200, 2));
    assert!(unpriced.is_empty());
}

/// The same journal through the report the bug was reported against: net worth
/// used to come back `$0` with `meta.unpriced = ["GBP"]`.
#[test]
fn net_worth_values_the_chained_commodity() {
    let journal = journal(GBP_CHAIN);
    let report = net_worth(
        &journal.transactions,
        &journal.prices,
        &NetWorthOpts {
            end: "2026-02-28",
            interval: Interval::Monthly,
            count: 1,
            depth: 1,
            value_in: Some(Commodity("$".to_string())),
            declared: &declared_types(&account_decls(&journal)),
        },
    )
    .expect("net worth computes");
    assert_eq!(
        report.totals[0].get(&Commodity("$".to_string())),
        Some(Dec::new(13200, 2))
    );
    assert_eq!(report.meta, None);
}

/// ```text
/// $ hledger -f hop3.journal bal assets --value=end,D
///                 210 D  assets:a
/// ```
#[test]
fn a_three_hop_chain_composes() {
    let (value, unpriced) = valued_assets(
        "\
P 2026-01-01 A 2 B
P 2026-01-01 B 3 C
P 2026-01-01 C 5 D

2026-02-01 open
    assets:a   7 A
    equity:open
",
        "D",
        "2026-02-28",
    );
    assert_eq!(value, Dec::new(210, 0));
    assert!(unpriced.is_empty());
}

/// Only `A→B` and `C→B` are declared, so hledger falls back to its
/// forward-plus-reverse graph and goes `A>B 2`, `B>C 0.25`.
///
/// ```text
/// $ hledger -f rev.journal bal assets --value=end,C
///                    C5  assets:a
/// ```
#[test]
fn a_chain_may_traverse_a_reversed_edge() {
    let (value, unpriced) = valued_assets(
        "\
P 2026-01-01 A 2 B
P 2026-01-01 C 4 B

2026-02-01 open
    assets:a   10 A
    equity:open
",
        "C",
        "2026-02-28",
    );
    assert_eq!(value, Dec::new(5, 0));
    assert!(unpriced.is_empty());
}

/// A forward chain of any length beats a one-hop reverse: `A>B>C` = 6, not
/// `1/(C>A)` = 0.2.
///
/// ```text
/// $ hledger -f pref.journal bal assets --value=end,C
///                  6 C  assets:a
/// ```
#[test]
fn a_forward_chain_beats_a_reverse_edge() {
    let (value, _) = valued_assets(
        "\
P 2026-01-01 A 2 B
P 2026-01-01 B 3 C
P 2026-01-01 C 5 A

2026-02-01 open
    assets:a   1 A
    equity:open
",
        "C",
        "2026-02-28",
    );
    assert_eq!(value, Dec::new(6, 0));
}

/// A cycle in the price graph must terminate, and a target it cannot reach is
/// still reported as unpriced (hledger leaves the amount in its own commodity).
///
/// ```text
/// $ hledger -f cycle.journal bal assets --value=end,ZZZ
///                  1 A  assets:a
/// ```
#[test]
fn a_cycle_terminates_and_leaves_an_unreachable_commodity_unpriced() {
    let (value, unpriced) = valued_assets(
        "\
P 2026-01-01 A 2 B
P 2026-01-01 B 3 C
P 2026-01-01 C 5 A

2026-02-01 open
    assets:a   1 A
    equity:open
",
        "ZZZ",
        "2026-02-28",
    );
    assert_eq!(value, Dec::zero());
    assert_eq!(unpriced, vec![Commodity("A".to_string())]);
}

/// Two equal-length chains, `X>M>Z` = 10 and `X>N>Z` = 21. hledger orders its
/// graph edges by `(from, to)` and takes the left-most complete path, so `M`
/// wins — whichever order the directives were declared in.
///
/// ```text
/// $ hledger -f tie1.journal bal assets --value=end,Z   # M declared first
///                 10 Z  assets:a
/// $ hledger -f tie2.journal bal assets --value=end,Z   # N declared first
///                 10 Z  assets:a
/// ```
#[test]
fn equal_length_chains_break_the_tie_as_hledger_does() {
    for prices in [
        "\
P 2026-01-01 X 2 M
P 2026-01-01 X 3 N
P 2026-01-01 M 5 Z
P 2026-01-01 N 7 Z
",
        "\
P 2026-01-01 N 7 Z
P 2026-01-01 M 5 Z
P 2026-01-01 X 3 N
P 2026-01-01 X 2 M
",
    ] {
        let text = format!(
            "{prices}
2026-02-01 open
    assets:a   1 X
    equity:open
"
        );
        let (value, _) = valued_assets(&text, "Z", "2026-02-28");
        assert_eq!(value, Dec::new(10, 0));
    }
}

/// Every forward edge is extended before every reverse edge at the same depth:
/// `A>F 2` then the reversed `F>Z 0.2`, not the reversed `A>R 1/3` then `R>Z 7`.
///
/// ```text
/// $ hledger -f order.journal bal assets --value=end,Z
///                  4 Z  assets:a
/// ```
#[test]
fn forward_edges_are_tried_before_reverse_edges() {
    let (value, _) = valued_assets(
        "\
P 2026-01-01 A 2 F
P 2026-01-01 Z 5 F
P 2026-01-01 R 3 A
P 2026-01-01 R 7 Z

2026-02-01 open
    assets:a   10 A
    equity:open
",
        "Z",
        "2026-02-28",
    );
    assert_eq!(value, Dec::new(4, 0));
}

/// The MEDIUM half of RPT-3: `1/220` never terminates, so no reverse `P` is
/// inferred and the `-$2,200.00` cash leg used to vanish, leaving `10 AAPL`.
/// hledger nets the two legs to zero.
///
/// ```text
/// $ hledger -f aapl.journal bal --value=end,AAPL --infer-market-prices
///              10 AAPL  assets:broker
///             -10 AAPL  assets:cash
///     --------------------
///                    0
/// ```
#[test]
fn a_non_terminating_reciprocal_no_longer_drops_the_cash_leg() {
    let (value, unpriced) = valued_assets(
        "\
2026-01-05 buy
    assets:broker   10 AAPL @ $220.00
    assets:cash
",
        "AAPL",
        "2026-02-28",
    );
    assert!(unpriced.is_empty(), "the $ leg must be valued, not skipped");
    // hledger's 1/220 runs to `Data.Decimal`'s 255-place ceiling and nets to
    // ~1e-250; ours rounds at 10 places, so the residue is ~1e-7 AAPL — six
    // orders of magnitude below a share.
    assert!(value.abs().expect("abs") < Dec::new(1, 6));
}

/// A commodity nothing prices at all is still skipped rather than guessed.
#[test]
fn an_unpriced_commodity_is_still_reported_as_unpriced() {
    let (value, unpriced) = valued_assets(
        "\
P 2026-01-01 EUR $1.10

2026-02-01 open
    assets:eur    100.00 EUR
    assets:doge   5.00000000 DOGE
    equity:open
",
        "$",
        "2026-02-28",
    );
    assert_eq!(value, Dec::new(11000, 2));
    assert_eq!(unpriced, vec![Commodity("DOGE".to_string())]);
}
