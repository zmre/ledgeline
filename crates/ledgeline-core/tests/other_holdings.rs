//! The Other holdings tab (`plans/14-other-holdings.md`) against
//! `fixtures/reports/other-holdings.journal` and hledger 1.52 ground truth.
//!
//! Every value below was read off the `hledger` binary in the dev shell, and the
//! command that produced it is quoted beside the assertion. Four claims are
//! being defended:
//!
//! 1. **Membership is by DECLARATION, never by name.** Cash is out because it is
//!    `type:C`, the brokerage is out because it holds a security, and the petty
//!    receivable is out because it says `holdings: none` — not because any of
//!    them is spelled a particular way.
//! 2. **The two tabs partition.** A commodity-booked house tagged
//!    `holdings: other` leaves the Stocks tab entirely. Nothing is on both.
//! 3. **Change means what the Stocks tab means by it**: against cost for an
//!    all-time window, against the opening value for a bounded one. A
//!    dollar-booked asset nobody revalued reports exactly zero, not null.
//! 4. **Totals are partial and honest** — summed over the rows that carry the
//!    input, with unpriced rows excluded and warned about.

mod common;

use ledgeline_core::holdings::{
    HoldingsScope, ScopeMode, compute_holdings, other_holdings, other_holdings_series,
};
use ledgeline_core::reports::Interval;
use ledgeline_core::{Dec, Journal, parse_journal};
use std::collections::BTreeSet;

const AS_OF: &str = "2026-06-30";

fn fixture() -> Journal {
    let path = common::fixtures_dir().join("reports/other-holdings.journal");
    let text = std::fs::read_to_string(&path).expect("read other-holdings.journal");
    parse_journal(&text, &path.to_string_lossy()).expect("parse other-holdings.journal")
}

fn scope(as_of: &str, gain_since: Option<&str>) -> HoldingsScope {
    HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include,
        as_of: as_of.to_string(),
        gain_since: gain_since.map(str::to_string),
        value_in: None,
    }
}

fn report(
    journal: &Journal,
    scope: &HoldingsScope,
) -> ledgeline_core::holdings::OtherHoldingsReport {
    other_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        scope,
    )
    .expect("other holdings compute succeeds")
}

/// `$X.YZ` as an exact `Dec`, so expectations read like the hledger output they
/// were copied from.
fn usd(text: &str) -> Dec {
    Dec::parse(text, '.').expect("expectation parses")
}

// ---------------------------------------------------------------------------
// Membership
// ---------------------------------------------------------------------------

/// Ground truth (hledger 1.52):
///   hledger -f other-holdings.journal bal type:A -e 2026-07-01 --value=end,'$'
///     $25,000.00  assets:bank:checking      <- type:C, excluded
///      $5,400.00  assets:broker:taxable     <- holds VTI, excluded (Stocks tab)
///     $75,000.00  assets:partners:acme      <- OTHER
///    $455,000.00  assets:property:house     <- OTHER (holdings: other)
///      $1,000.00  assets:receivable:petty   <- holdings: none, excluded
///     $24,500.00  assets:vehicles:van       <- OTHER
#[test]
fn only_declared_non_cash_non_security_assets_are_rows() {
    let journal = fixture();
    let report = report(&journal, &scope(AS_OF, None));

    let accounts: Vec<&str> = report.holdings.iter().map(|h| h.account.as_str()).collect();
    assert_eq!(
        accounts,
        vec![
            "assets:property:house",
            "assets:partners:acme",
            "assets:vehicles:van",
        ],
        "rows are value-desc and exclude cash, securities and `holdings: none`"
    );
    assert_eq!(report.base, "$");
    assert_eq!(report.as_of, AS_OF);
}

/// The `holdings:` tag is the only thing standing between the house and the
/// Stocks tab: HOUSE is a non-currency commodity, so without the tag the stock
/// engine would claim it. A position counted on both tabs is worse than one
/// counted on neither.
#[test]
fn a_tagged_house_leaves_the_stocks_tab_entirely() {
    let journal = fixture();
    let stocks = compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope(AS_OF, None),
    )
    .expect("stock holdings compute succeeds");

    let symbols: Vec<&str> = stocks.holdings.iter().map(|h| h.symbol.as_str()).collect();
    assert_eq!(symbols, vec!["VTI"], "HOUSE is the Other tab's row now");

    // ...and it is genuinely on the other side of the split.
    let other = report(&journal, &scope(AS_OF, None));
    assert!(
        other
            .holdings
            .iter()
            .any(|h| h.account == "assets:property:house")
    );

    // The split reaches the SCOPE CHOOSER too, in both directions. The house
    // holds a non-currency commodity, so a UI-side "any account holding a
    // non-currency commodity" rule — which is what the SPA had — would offer it
    // on the Stocks tab, where selecting it can only ever produce an empty
    // report. Each tab offers exactly its own accounts, and the two lists are
    // disjoint.
    assert!(
        !stocks
            .accounts
            .contains(&"assets:property:house".to_string()),
        "the Stocks chooser must not offer a tagged house: {:?}",
        stocks.accounts
    );
    assert_eq!(
        stocks.accounts,
        vec!["assets:broker:taxable".to_string()],
        "only the genuine security account"
    );
    for account in &stocks.accounts {
        assert!(
            !other.accounts.contains(account),
            "{account} is offered on both tabs"
        );
    }
}

/// `type:C` is a SUBTYPE of Asset under `is_account_type`, so the loose
/// membership test would have dragged every cash account onto this tab. The rule
/// is an exact match on `Asset`, and this is the test that pins it.
#[test]
fn cash_is_excluded_even_though_it_is_an_asset_subtype() {
    let journal = fixture();
    let report = report(&journal, &scope(AS_OF, None));
    assert!(
        !report
            .holdings
            .iter()
            .any(|h| h.account.starts_with("assets:bank")),
        "checking is type:C and belongs to neither tab"
    );
}

// ---------------------------------------------------------------------------
// Value, cost and change
// ---------------------------------------------------------------------------

/// Ground truth (hledger 1.52):
///   value: bal assets:property:house -e 2026-07-01 --value=end,'$'  ->  $455,000.00
///   cost:  bal assets:property:house -e 2026-07-01 -B               ->  $400,000.00
#[test]
fn a_revalued_house_shows_its_gain_over_cost() {
    let journal = fixture();
    let report = report(&journal, &scope(AS_OF, None));
    let house = &report.holdings[0];

    assert_eq!(house.account, "assets:property:house");
    assert_eq!(house.name, "Family home", "the declared `name:` tag wins");
    assert_eq!(house.value, Some(usd("455000.00")));
    assert_eq!(house.cost, Some(usd("400000.00")));
    assert_eq!(house.change, Some(usd("55000.00")));
    let pct = house.change_pct.expect("a known cost gives a percentage");
    assert!(
        (pct - 13.75).abs() < 1e-9,
        "55000/400000 = 13.75%, got {pct}"
    );

    // The as-written balance survives, so the UI can print "1 HOUSE".
    assert_eq!(
        house
            .commodities
            .get(&ledgeline_core::model::Commodity("HOUSE".into())),
        Some(Dec::new(1, 0))
    );
}

/// A dollar-booked asset nobody revalued: cost IS value, so the all-time change
/// is a real zero. Not `None` — there is nothing unknown here — and not omitted.
///
/// Ground truth: bal assets:vehicles:van -e 2026-07-01  ->  $24,500.00
/// ($32,000.00 bought, $7,500.00 depreciated.)
#[test]
fn a_depreciated_van_reports_an_honest_zero_all_time_change() {
    let journal = fixture();
    let report = report(&journal, &scope(AS_OF, None));
    let van = report
        .holdings
        .iter()
        .find(|h| h.account == "assets:vehicles:van")
        .expect("the van is a row");

    assert_eq!(van.name, "Delivery van");
    assert_eq!(van.value, Some(usd("24500.00")));
    assert_eq!(van.cost, Some(usd("24500.00")));
    assert_eq!(van.change, Some(Dec::zero()));
}

/// A bounded window measures against the account's value at the window start,
/// exactly as `HoldingsScope::gain_since` does on the Stocks tab.
///
/// Between 2026-04-01 and 2026-06-30 the fixture does three things: the house
/// reprices $430,000 -> $455,000, the van depreciates $32,000 -> $24,500, and
/// Acme takes a $25,000 follow-on contribution.
#[test]
fn a_window_measures_change_against_the_opening_value() {
    let journal = fixture();
    let report = report(&journal, &scope(AS_OF, Some("2026-04-01")));

    let change_of = |account: &str| {
        report
            .holdings
            .iter()
            .find(|h| h.account == account)
            .unwrap_or_else(|| panic!("{account} is a row"))
            .change
    };

    assert_eq!(change_of("assets:property:house"), Some(usd("25000.00")));
    assert_eq!(change_of("assets:vehicles:van"), Some(usd("-7500.00")));
    assert_eq!(change_of("assets:partners:acme"), Some(usd("25000.00")));
}

/// An asset HELD at the window start but unpriceable then propagates null — it
/// does not collapse to a zero reference.
///
/// Inline rather than in the shared fixture: this needs a commodity with NO
/// price at the window start and one at `as_of`, which is a strange enough shape
/// that putting it in `other-holdings.journal` would perturb every other
/// expectation in this file to prove one point.
///
/// Treating "held but unpriceable then" as a zero reference would report the
/// asset's entire $9,000.00 as the window's change: a fabricated number, and a
/// perfectly plausible-looking one, which is the failure mode this codebase
/// refuses everywhere else.
#[test]
fn an_asset_unpriced_at_the_window_start_has_an_unknown_change() {
    let text = "\
account assets:art:sculpture ; type: A, holdings: other
account equity:opening       ; type: E

; The sculpture's FIRST price lands on the last day, so on 2026-04-01 it is
; genuinely held and genuinely unpriceable.
P 2026-06-30 ART $9,000.00

2026-02-20 Estate sale | bronze sculpture
    assets:art:sculpture   1 ART
    equity:opening
";
    let journal = parse_journal(text, "art.journal").expect("journal parses");
    let report = report(&journal, &scope(AS_OF, Some("2026-04-01")));
    let art = &report.holdings[0];
    assert_eq!(art.account, "assets:art:sculpture");

    // Priced NOW, so it has a value and counts toward the value total...
    assert_eq!(art.value, Some(usd("9000.00")));
    assert_eq!(report.totals.value, usd("9000.00"));
    // ...but the window's change is UNKNOWN — not zero, and not $9,000.00.
    assert_eq!(art.change, None);
    assert_eq!(art.change_pct, None);
    assert_eq!(
        report.totals.change, None,
        "no row carries a reference, so there is no change to report"
    );
}

/// The zero-reference case this must not be confused with: an account simply not
/// held at the window start references a real zero.
#[test]
fn an_asset_not_held_at_the_window_start_references_zero() {
    let text = "\
account assets:vehicles:bike ; type: A
account equity:opening       ; type: E

2026-05-10 Bike shop | cargo bike
    assets:vehicles:bike   $4,000.00
    equity:opening
";
    let journal = parse_journal(text, "bike.journal").expect("journal parses");
    let report = report(&journal, &scope(AS_OF, Some("2026-04-01")));
    let bike = &report.holdings[0];
    assert_eq!(bike.value, Some(usd("4000.00")));
    assert_eq!(
        bike.change,
        Some(usd("4000.00")),
        "bought inside the window, so the whole purchase IS the change"
    );
}

/// An account bought DURING the window references zero, so the whole purchase
/// reads as the window's change. That is what makes the rows sum to the totals.
#[test]
fn an_asset_acquired_inside_the_window_references_zero() {
    let journal = fixture();
    // The van was bought 2026-02-01; a window opening 2026-01-20 predates it.
    let report = report(&journal, &scope(AS_OF, Some("2026-01-20")));
    let van = report
        .holdings
        .iter()
        .find(|h| h.account == "assets:vehicles:van")
        .expect("the van is a row");
    assert_eq!(van.change, Some(usd("24500.00")));
}

// ---------------------------------------------------------------------------
// Totals
// ---------------------------------------------------------------------------

/// Totals are summed over the rows, never recomputed downstream.
///   value: $455,000 + $75,000 + $24,500 = $554,500
///   cost:  $400,000 + $75,000 + $24,500 = $499,500
#[test]
fn totals_sum_the_rows_that_carry_the_input() {
    let journal = fixture();
    let report = report(&journal, &scope(AS_OF, None));

    assert_eq!(report.totals.value, usd("554500.00"));
    assert_eq!(report.totals.cost, Some(usd("499500.00")));
    assert_eq!(report.totals.change, Some(usd("55000.00")));

    // The rows really do add up to it.
    let summed = report
        .holdings
        .iter()
        .filter_map(|h| h.value)
        .try_fold(Dec::zero(), |acc, v| acc.add(v))
        .expect("no overflow");
    assert_eq!(summed, report.totals.value);

    // The percentage divides the SUMMED change by the SUMMED reference, not the
    // mean of the per-row percentages.
    let pct = report.totals.change_pct.expect("a known cost sum");
    let expected = (55_000.0 / 499_500.0) * 100.0;
    assert!(
        (pct - expected).abs() < 1e-9,
        "55000/499500 = {expected}%, got {pct}"
    );
}

// ---------------------------------------------------------------------------
// Scope
// ---------------------------------------------------------------------------

/// The scope bar means the same thing on both tabs.
#[test]
fn excluding_an_account_drops_its_row_and_shrinks_the_totals() {
    let journal = fixture();
    let mut scoped = scope(AS_OF, None);
    scoped.mode = ScopeMode::Exclude;
    scoped.accounts.insert("assets:property:house".to_string());

    let report = report(&journal, &scoped);
    assert!(
        !report
            .holdings
            .iter()
            .any(|h| h.account == "assets:property:house")
    );
    // $75,000 + $24,500, with the house's $455,000 gone.
    assert_eq!(report.totals.value, usd("99500.00"));

    // ...but it is STILL an option in the chooser. An option that disappears the
    // moment you deselect it cannot be deselected twice, or reselected at all.
    assert!(
        report
            .accounts
            .contains(&"assets:property:house".to_string()),
        "the candidate list ignores the scope: {:?}",
        report.accounts
    );
}

/// The scope chooser offers exactly the accounts that can ever be rows — no
/// dead options, and nothing shown in the table that cannot be deselected.
///
/// It is neither scope- nor date-filtered, so time-travelling to before an asset
/// was bought does not make its checkbox vanish while the user is composing a
/// scope.
#[test]
fn the_candidate_account_list_is_scope_and_date_independent() {
    let journal = fixture();

    // A date before the van, Acme and even the house existed.
    let early = report(&journal, &scope("2026-01-10", None));
    assert!(early.holdings.is_empty(), "nothing is held yet");
    assert_eq!(
        early.accounts,
        vec![
            "assets:partners:acme",
            "assets:property:house",
            "assets:vehicles:van",
        ],
        "the chooser still offers all three"
    );

    // Same list at the end of the journal, sorted the same way.
    let late = report(&journal, &scope(AS_OF, None));
    assert_eq!(late.accounts, early.accounts);

    // And it excludes exactly what the table excludes.
    for absent in [
        "assets:bank:checking",
        "assets:broker:taxable",
        "assets:receivable:petty",
    ] {
        assert!(
            !late.accounts.contains(&absent.to_string()),
            "{absent} is not an Other holding"
        );
    }
}

// ---------------------------------------------------------------------------
// Series
// ---------------------------------------------------------------------------

/// The trend is the same total, at each period boundary, in the same wire shape
/// the Stocks trend uses — which is why one chart component draws both.
///
/// Monthly boundaries, Feb..Jun 2026. The house arrives 2026-01-15 at $400,000
/// and reprices to $430,000 on 2026-04-01 and $455,000 on 2026-06-30; the van
/// arrives 2026-02-01 at $32,000 and depreciates on the last day; Acme arrives
/// 2026-02-15 at $50,000 and grows by $25,000 on 2026-05-01.
#[test]
fn the_series_tracks_total_value_at_each_boundary() {
    let journal = fixture();
    let series = other_holdings_series(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &scope(AS_OF, None),
        Interval::Monthly,
        5,
    )
    .expect("series computes");

    assert_eq!(series.base, "$");
    assert_eq!(series.points.len(), 5);
    assert!(series.has_basis);

    let buckets: Vec<&str> = series.points.iter().map(|p| p.bucket.as_str()).collect();
    assert_eq!(
        buckets,
        vec!["2026-02", "2026-03", "2026-04", "2026-05", "2026-06"],
        "oldest first"
    );

    let value_at = |bucket: &str| {
        series
            .points
            .iter()
            .find(|p| p.bucket == bucket)
            .unwrap_or_else(|| panic!("{bucket} is a point"))
            .market_value
    };

    // Feb 28: house $400,000 + van $32,000 + acme $50,000.
    assert_eq!(value_at("2026-02"), usd("482000.00"));
    // Apr 30: the house has repriced to $430,000.
    assert_eq!(value_at("2026-04"), usd("512000.00"));
    // May 31: Acme's follow-on lands.
    assert_eq!(value_at("2026-05"), usd("537000.00"));
    // Jun 30: house $455,000, van depreciated to $24,500, acme $75,000.
    assert_eq!(value_at("2026-06"), usd("554500.00"));

    // The last point equals the report it is a trend of.
    let report = report(&journal, &scope(AS_OF, None));
    assert_eq!(value_at("2026-06"), report.totals.value);
}

/// The final point never overshoots `as_of`, so a mid-month snapshot ends on the
/// snapshot date rather than the month's last day — and everything is read at
/// that date, PRICES INCLUDED. On 2026-06-15 the house is still worth its
/// 2026-04-01 quote, because the $455,000 directive is dated 2026-06-30 and a
/// snapshot may not see a price from its own future.
#[test]
fn the_final_point_is_clamped_to_as_of() {
    let journal = fixture();
    let series = other_holdings_series(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &scope("2026-06-15", None),
        Interval::Monthly,
        2,
    )
    .expect("series computes");

    assert_eq!(series.points.last().expect("a point").date, "2026-06-15");
    // The van depreciates on 2026-06-30, so a 06-15 snapshot still has it whole;
    // the house is at its 2026-04-01 quote for the same reason.
    assert_eq!(
        series.points.last().expect("a point").market_value,
        usd("537000.00"),
        "house $430,000 + van $32,000 + acme $75,000"
    );
}
