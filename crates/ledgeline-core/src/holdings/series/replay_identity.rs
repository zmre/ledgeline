//! PERF-5b's correctness gate: the ONE date-ordered replay must produce, at
//! every point, byte-for-byte what the per-point path it replaced produced.
//!
//! The per-point path is transcribed verbatim below ([`per_point_reports`] and
//! [`per_point_series`]) rather than described, so this file is an executable
//! copy of the code that was deleted and stays honest if the replay drifts.
//!
//! **Why the existing suites are not enough.** [`Dec`]'s `PartialEq` compares by
//! VALUE — `Dec::new(15, 1) == Dec::new(150, 2)` — and the golden fixtures
//! canonicalize trailing zeros before they are written. A refactor that lost a
//! decimal's SCALE (say by re-deriving a basis instead of carrying it) would
//! pass `assert_eq!` on the reports and pass all 44 goldens, and only show up
//! later as `$750.0` where the journal says `$750.00`. Everything here is
//! therefore compared as an exact `(mantissa, places)` pair, with `gain_pct`
//! compared as raw `f64` bits.

use std::cmp::Ordering;

use super::*;
use crate::holdings::engine::{compute_holdings, valuation_base};
use crate::holdings::test_helpers::{
    account_decl, amt, buy, buy_no_cost, commodity_tags, pd, posting, scope, txn, usd, with_cost,
};
use crate::holdings::types::{Holding, HoldingsReport, PriceSource, ScopeMode, WarningKind};

// ---------------------------------------------------------------------------
// exact-scale fingerprints
// ---------------------------------------------------------------------------

/// A decimal reduced to the pair that actually identifies it. `Dec::new(15, 1)`
/// and `Dec::new(150, 2)` are `==` but are NOT the same number on the wire.
fn exact(value: Dec) -> (i128, u32) {
    (value.mantissa, value.places)
}

fn exact_opt(value: Option<Dec>) -> Option<(i128, u32)> {
    value.map(exact)
}

/// Every field of a [`Holding`], money by exact scale and the percentage by raw
/// bits (so `-0.0`, a `NaN` payload change, or a last-place drift all show).
#[derive(Debug, PartialEq, Eq)]
struct HoldingBits {
    symbol: String,
    name: String,
    accounts: Vec<String>,
    shares: (i128, u32),
    basis: Option<(i128, u32)>,
    first_basis_date: Option<String>,
    price: Option<((i128, u32), String, PriceSource)>,
    market_value: Option<(i128, u32)>,
    gain: Option<(i128, u32)>,
    gain_pct: Option<u64>,
}

fn holding_bits(holding: &Holding) -> HoldingBits {
    HoldingBits {
        symbol: holding.symbol.clone(),
        name: holding.name.clone(),
        accounts: holding.accounts.clone(),
        shares: exact(holding.shares),
        basis: exact_opt(holding.basis),
        first_basis_date: holding.first_basis_date.clone(),
        price: holding
            .price
            .as_ref()
            .map(|price| (exact(price.qty), price.date.clone(), price.source)),
        market_value: exact_opt(holding.market_value),
        gain: exact_opt(holding.gain),
        gain_pct: holding.gain_pct.map(f64::to_bits),
    }
}

/// Every field of a [`HoldingsReport`], including ordering (`holdings`,
/// `top_gainers`, `top_losers` and `warnings` are all order-significant).
#[derive(Debug, PartialEq, Eq)]
struct ReportBits {
    as_of: String,
    base: String,
    holdings: Vec<HoldingBits>,
    market_value: (i128, u32),
    basis: Option<(i128, u32)>,
    gain: Option<(i128, u32)>,
    gain_pct: Option<u64>,
    top_gainers: Vec<HoldingBits>,
    top_losers: Vec<HoldingBits>,
    warnings: Vec<(String, WarningKind, String)>,
}

fn report_bits(report: &HoldingsReport) -> ReportBits {
    ReportBits {
        as_of: report.as_of.clone(),
        base: report.base.clone(),
        holdings: report.holdings.iter().map(holding_bits).collect(),
        market_value: exact(report.totals.market_value),
        basis: exact_opt(report.totals.basis),
        gain: exact_opt(report.totals.gain),
        gain_pct: report.totals.gain_pct.map(f64::to_bits),
        top_gainers: report.top_gainers.iter().map(holding_bits).collect(),
        top_losers: report.top_losers.iter().map(holding_bits).collect(),
        warnings: report
            .warnings
            .iter()
            .map(|warning| {
                (
                    warning.symbol.clone(),
                    warning.kind,
                    warning.message.clone(),
                )
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// the path that was replaced, transcribed
// ---------------------------------------------------------------------------

/// The pre-PERF-5b inner loop: resolve the base once from `scope.as_of`, then
/// run a FULL `compute_holdings` per point with that base pinned and gain
/// windowing off. Verbatim from the loop `holdings_series` used to contain.
fn per_point_reports(
    txns: &[Transaction],
    prices: &[PriceDirective],
    accounts: &[AccountDeclaration],
    commodity_tags: &[(Commodity, Vec<(String, String)>)],
    scope: &HoldingsScope,
    dates: &[String],
) -> Vec<HoldingsReport> {
    let base_commodity = valuation_base(txns, prices, accounts, scope).expect("valuation base");
    dates
        .iter()
        .map(|date| {
            let point_scope = HoldingsScope {
                accounts: scope.accounts.clone(),
                mode: scope.mode,
                as_of: date.clone(),
                gain_since: None,
                value_in: Some(base_commodity.clone()),
            };
            compute_holdings(txns, prices, accounts, commodity_tags, &point_scope)
                .expect("per-point report")
        })
        .collect()
}

/// The pre-PERF-5b `holdings_series`, transcribed whole.
fn per_point_series(
    txns: &[Transaction],
    prices: &[PriceDirective],
    accounts: &[AccountDeclaration],
    commodity_tags: &[(Commodity, Vec<(String, String)>)],
    scope: &HoldingsScope,
    interval: Interval,
    count: usize,
) -> HoldingsSeries {
    let keys = last_n_buckets(&scope.as_of, interval, count).expect("buckets");
    let base_commodity = valuation_base(txns, prices, accounts, scope).expect("valuation base");
    let mut base = "$".to_string();
    let mut has_basis = false;
    let mut points = Vec::with_capacity(keys.len());
    for key in keys {
        let end = bucket_end(&key).expect("bucket end");
        let date = if compare_iso(&end, &scope.as_of) == Ordering::Greater {
            scope.as_of.clone()
        } else {
            end
        };
        let point_scope = HoldingsScope {
            accounts: scope.accounts.clone(),
            mode: scope.mode,
            as_of: date.clone(),
            gain_since: None,
            value_in: Some(base_commodity.clone()),
        };
        let report = compute_holdings(txns, prices, accounts, commodity_tags, &point_scope)
            .expect("per-point report");
        base = report.base;
        if report.totals.basis.is_some() {
            has_basis = true;
        }
        points.push(HoldingsPoint {
            date,
            bucket: key.clone(),
            label: bucket_label(&key),
            market_value: report.totals.market_value,
            basis: report.totals.basis,
        });
    }
    HoldingsSeries {
        base,
        points,
        has_basis,
    }
}

// ---------------------------------------------------------------------------
// the journal
// ---------------------------------------------------------------------------

/// One journal exercising every Phase-4 behaviour the replay had to preserve,
/// each with a BEFORE and an AFTER inside the swept date range so a point that
/// straddles it can go wrong:
///
/// - HOLD-1 split detection via an `equity:splits` counter-leg (`2024-03-20`);
/// - a cost-less lot tainting a pool `CostlessLot` (`2024-04-05`);
/// - a cost-compatible same-transaction transfer between in-scope accounts,
///   which must move `accounts` and nothing else (`2024-06-01`);
/// - a partial sell reducing the basis half-even (`2024-07-01`);
/// - HOLD-4 sticky-negative taint: sold-before-bought, then re-bought back into
///   the positive, which must STAY tainted (`2024-08-01` / `2024-09-01`);
/// - `UnconvertibleCost`, and a price that only becomes convertible partway
///   through the series once a `EUR` cross-rate appears (`2024-10-01`);
/// - a return of capital reducing a basis (`2024-11-05`) in an account that
///   later takes on a SECOND security (`2025-02-12`) — after which the very same
///   transaction stops being attributable, so points either side of it disagree
///   about a transaction that precedes them both;
/// - an RSU vest, which is never a split, tainting a previously clean pool
///   (`2025-03-15`);
/// - a full sell-out and re-open, resetting `first_basis_date` (`2025-04-20` /
///   `2025-05-20`);
/// - cash out of an account BEFORE it holds anything, which reduces no basis at
///   an early `as_of` and reduces one retroactively at a late one
///   (`2025-06-15` / `2025-09-15`);
/// - an unpriced holding (`ZZZ`).
fn torture_txns() -> Vec<Transaction> {
    vec![
        txn(
            1,
            "2024-01-10",
            vec![
                buy("assets:broker:a", "AAPL", 10, 10000, true),
                posting("assets:broker:cash", vec![usd(-100_000)], &[]),
            ],
            &[],
        ),
        txn(
            2,
            "2024-02-15",
            vec![
                buy("assets:broker:a", "AAPL", 5, 12000, true),
                posting("assets:broker:cash", vec![usd(-60_000)], &[]),
            ],
            &[],
        ),
        // 2-for-1 split: cost-less, symbol-only, absorbed by equity.
        txn(
            3,
            "2024-03-20",
            vec![
                posting("assets:broker:a", vec![amt("AAPL", 15, 0)], &[]),
                posting("equity:splits", vec![amt("AAPL", -15, 0)], &[]),
            ],
            &[],
        ),
        txn(
            4,
            "2024-04-05",
            vec![
                buy_no_cost("assets:broker:b", "VTI", 10),
                posting("equity:opening", vec![amt("VTI", -10, 0)], &[]),
            ],
            &[],
        ),
        txn(
            5,
            "2024-05-01",
            vec![
                posting(
                    "assets:broker:mixed",
                    vec![with_cost(amt("GLD", 4, 0), 80000, false, "$")],
                    &[],
                ),
                posting("assets:broker:cash", vec![usd(-80_000)], &[]),
            ],
            &[],
        ),
        // Pure transfer: zero net, incoming leg bare.
        txn(
            6,
            "2024-06-01",
            vec![
                posting("assets:broker:a", vec![amt("AAPL", -6, 0)], &[]),
                posting("assets:broker:b", vec![amt("AAPL", 6, 0)], &[]),
            ],
            &[],
        ),
        txn(
            7,
            "2024-07-01",
            vec![
                posting(
                    "assets:broker:b",
                    vec![with_cost(amt("AAPL", -5, 0), 13000, true, "$")],
                    &[],
                ),
                posting("assets:broker:cash", vec![usd(65_000)], &[]),
            ],
            &[],
        ),
        // Sold before ever bought: the pool goes short and stays tainted.
        txn(
            8,
            "2024-08-01",
            vec![
                posting("assets:broker:a", vec![amt("SHORTY", -3, 0)], &[]),
                posting("assets:broker:cash", vec![usd(3_000)], &[]),
            ],
            &[],
        ),
        txn(
            9,
            "2024-09-01",
            vec![
                buy("assets:broker:a", "SHORTY", 5, 1000, true),
                posting("assets:broker:cash", vec![usd(-5_000)], &[]),
            ],
            &[],
        ),
        // Cost annotated in EUR, which has no rate to `$` on this date.
        txn(
            10,
            "2024-10-01",
            vec![
                posting(
                    "assets:broker:a",
                    vec![with_cost(amt("EURSYM", 10, 0), 5000, true, "EUR")],
                    &[],
                ),
                posting("assets:broker:cash", vec![amt("EUR", -50000, 2)], &[]),
            ],
            &[],
        ),
        // Return of capital out of a then-single-security account.
        txn(
            11,
            "2024-11-05",
            vec![
                posting("assets:broker:mixed", vec![usd(-5_000)], &[]),
                posting("assets:bank", vec![usd(5_000)], &[]),
            ],
            &[],
        ),
        // …which retroactively stops being attributable right here.
        txn(
            12,
            "2025-02-12",
            vec![
                buy("assets:broker:mixed", "AAPL", 1, 9000, true),
                posting("assets:broker:cash", vec![usd(-9_000)], &[]),
            ],
            &[],
        ),
        // An RSU vest: income is touched, so never a split — a cost-less lot.
        txn(
            13,
            "2025-03-15",
            vec![
                posting("assets:broker:a", vec![amt("AAPL", 2, 0)], &[]),
                posting("income:vesting", vec![amt("AAPL", -2, 0)], &[]),
            ],
            &[],
        ),
        txn(
            14,
            "2025-04-20",
            vec![
                posting("assets:broker:mixed", vec![amt("GLD", -4, 0)], &[]),
                posting("assets:broker:cash", vec![usd(100_000)], &[]),
            ],
            &[],
        ),
        txn(
            15,
            "2025-05-20",
            vec![
                buy("assets:broker:mixed", "GLD", 6, 25000, true),
                posting("assets:broker:cash", vec![usd(-150_000)], &[]),
            ],
            &[],
        ),
        // Cash out of an account that holds nothing yet…
        txn(
            16,
            "2025-06-15",
            vec![
                posting("assets:broker:late", vec![usd(-2_000)], &[]),
                posting("assets:bank", vec![usd(2_000)], &[]),
            ],
            &[],
        ),
        txn(
            17,
            "2025-07-10",
            vec![
                buy_no_cost("assets:broker:b", "ZZZ", 1),
                posting("equity:opening", vec![amt("ZZZ", -1, 0)], &[]),
            ],
            &[],
        ),
        // …which only becomes a return of capital once GOOG lands here.
        txn(
            18,
            "2025-09-15",
            vec![
                buy("assets:broker:late", "GOOG", 3, 5000, true),
                posting("assets:broker:cash", vec![usd(-15_000)], &[]),
            ],
            &[],
        ),
    ]
}

/// Quotes that move over the swept range, plus a `EUR` cross-rate that only
/// appears partway through — before `2025-03-01` `EURSYM` cannot be valued in
/// `$` at all, after it can.
fn torture_prices() -> Vec<PriceDirective> {
    vec![
        pd("2024-01-01", "AAPL", 10500, "$"),
        pd("2024-06-01", "AAPL", 6000, "$"),
        pd("2025-01-01", "AAPL", 7000, "$"),
        pd("2025-08-01", "AAPL", 8000, "$"),
        pd("2024-05-01", "VTI", 21000, "$"),
        pd("2025-06-01", "VTI", 22000, "$"),
        pd("2024-06-01", "GLD", 21000, "$"),
        pd("2025-05-01", "GLD", 26000, "$"),
        pd("2024-09-15", "SHORTY", 1200, "$"),
        pd("2025-09-01", "GOOG", 5500, "$"),
        pd("2025-01-05", "EURSYM", 5000, "EUR"),
        pd("2025-03-01", "EUR", 110, "$"),
    ]
}

fn torture_accounts() -> Vec<AccountDeclaration> {
    vec![
        account_decl("assets:broker", &[("name", "Brokerage")]),
        account_decl("assets:broker:b", &[("name", "Taxable")]),
    ]
}

fn torture_commodity_tags() -> Vec<(Commodity, Vec<(String, String)>)> {
    vec![commodity_tags("AAPL", &[("name", "Apple Inc.")])]
}

/// Month-ends from 2023-07 (before any transaction) to 2025-12, plus the day
/// before, of and after every transition above — the off-by-one a shared replay
/// is most likely to introduce is at a boundary, not in the middle of a month.
fn sweep_dates() -> Vec<String> {
    let mut dates: Vec<String> = last_n_buckets("2025-12-31", Interval::Monthly, 30)
        .expect("monthly buckets")
        .iter()
        .map(|key| bucket_end(key).expect("bucket end"))
        .collect();
    dates.extend(
        [
            "2024-01-09",
            "2024-01-10",
            "2024-03-19",
            "2024-03-20",
            "2024-06-01",
            "2024-08-01",
            "2024-09-01",
            "2024-11-04",
            "2024-11-05",
            "2024-11-06",
            "2025-02-11",
            "2025-02-12",
            "2025-02-13",
            "2025-03-14",
            "2025-03-15",
            "2025-03-16",
            "2025-04-19",
            "2025-04-20",
            "2025-05-20",
            "2025-06-14",
            "2025-06-15",
            "2025-06-16",
            "2025-09-14",
            "2025-09-15",
            "2025-09-16",
        ]
        .iter()
        .map(|date| (*date).to_string()),
    );
    dates.sort();
    dates.dedup();
    dates
}

// ---------------------------------------------------------------------------
// the gate
// ---------------------------------------------------------------------------

#[test]
fn one_replay_reproduces_every_point_of_the_per_point_path() {
    let txns = torture_txns();
    let prices = torture_prices();
    let accounts = torture_accounts();
    let tags = torture_commodity_tags();
    let sc = scope("2025-12-31", ScopeMode::Include, &[]);
    let dates = sweep_dates();

    let (base, replayed) = holdings_at_each(&txns, &prices, &accounts, &tags, &sc, &dates)
        .expect("single-replay series");
    let expected = per_point_reports(&txns, &prices, &accounts, &tags, &sc, &dates);

    assert_eq!(replayed.len(), dates.len());
    assert_eq!(expected.len(), dates.len());
    for ((date, got), want) in dates.iter().zip(&replayed).zip(&expected) {
        assert_eq!(report_bits(got), report_bits(want), "as_of {date}");
    }
    assert_eq!(
        base.0,
        expected.last().expect("at least one point").base.clone()
    );
}

/// The same gate at the public boundary, over three intervals and a scoped
/// variant — `holdings_series` is what `/api/holdings/series` actually calls.
#[test]
fn holdings_series_matches_the_per_point_series() {
    let txns = torture_txns();
    let prices = torture_prices();
    let accounts = torture_accounts();
    let tags = torture_commodity_tags();
    let scopes = [
        scope("2025-12-31", ScopeMode::Include, &[]),
        scope("2025-12-31", ScopeMode::Include, &["assets:broker:a"]),
        scope("2025-12-31", ScopeMode::Exclude, &["assets:broker:mixed"]),
        scope("2025-07-04", ScopeMode::Include, &[]),
    ];
    for sc in &scopes {
        for (interval, count) in [
            (Interval::Monthly, 30),
            (Interval::Monthly, 3),
            (Interval::Quarterly, 9),
            (Interval::Yearly, 3),
        ] {
            let got = holdings_series(&txns, &prices, &accounts, &tags, sc, interval, count)
                .expect("series");
            let want = per_point_series(&txns, &prices, &accounts, &tags, sc, interval, count);
            let label = format!("{:?}/{count} as_of {}", interval, sc.as_of);
            assert_eq!(got.base, want.base, "base for {label}");
            assert_eq!(got.has_basis, want.has_basis, "has_basis for {label}");
            assert_eq!(got.points.len(), want.points.len(), "length for {label}");
            for (got, want) in got.points.iter().zip(&want.points) {
                assert_eq!(got.date, want.date, "date for {label}");
                assert_eq!(got.bucket, want.bucket, "bucket for {label}");
                assert_eq!(got.label, want.label, "label for {label}");
                assert_eq!(
                    exact(got.market_value),
                    exact(want.market_value),
                    "market value at {} for {label}",
                    got.bucket
                );
                assert_eq!(
                    exact_opt(got.basis),
                    exact_opt(want.basis),
                    "basis at {} for {label}",
                    got.bucket
                );
            }
        }
    }
}

/// The gate above is only worth having if the journal actually contains a point
/// pair the shared replay CANNOT serve from one pass. This pins the two: the
/// return of capital on 2024-11-05 reduces GLD's basis for every `as_of` before
/// `assets:broker:mixed` took on a second security, and for none after — the
/// same transaction, read two ways, by two points that both follow it.
#[test]
fn the_journal_really_does_force_two_replays() {
    let txns = torture_txns();
    let prices = torture_prices();
    let sc = scope("2025-12-31", ScopeMode::Include, &[]);
    let basis_of = |as_of: &str| {
        let point = HoldingsScope {
            as_of: as_of.to_string(),
            ..sc.clone()
        };
        compute_holdings(&txns, &prices, &[], &[], &point)
            .expect("report")
            .holdings
            .iter()
            .find(|holding| holding.symbol == "GLD")
            .and_then(|holding| holding.basis)
            .map(exact)
    };
    // $800.00 acquired, less the $50.00 returned.
    assert_eq!(basis_of("2025-02-11"), Some((75_000, 2)));
    // AAPL lands in the same account: the cash is no longer attributable, so
    // the full $800.00 stands.
    assert_eq!(basis_of("2025-02-13"), Some((80_000, 2)));

    // `assets:broker:late` is the other shape — absent from the map early,
    // `Some("GOOG")` late — but its cash left before the GOOG pool existed, and
    // a reduction with no pool to reduce is dropped. Pinned because it is the
    // reason that account, which DOES split the replay, changes no number: the
    // split is conservative, not load-bearing.
    let goog_basis = |as_of: &str| {
        let point = HoldingsScope {
            as_of: as_of.to_string(),
            ..sc.clone()
        };
        compute_holdings(&txns, &prices, &[], &[], &point)
            .expect("report")
            .holdings
            .iter()
            .find(|holding| holding.symbol == "GOOG")
            .and_then(|holding| holding.basis)
            .map(exact)
    };
    assert_eq!(goog_basis("2025-09-14"), None, "not held yet");
    assert_eq!(goog_basis("2025-09-16"), Some((15_000, 2)), "3 @ $50.00");
}

/// A `Dec` that has been re-derived rather than CARRIED loses its scale, and
/// nothing else in the suite can see it: `Dec`'s `PartialEq` is by value and the
/// goldens canonicalize trailing zeros. Pin the scale of the two replayed
/// decimals most at risk — a half-even sell reduction (whose places come from
/// the basis it reduces, not from the division) and a re-opened basis.
#[test]
fn replayed_decimals_keep_their_exact_scale() {
    let txns = torture_txns();
    let prices = torture_prices();
    let sc = scope("2025-12-31", ScopeMode::Include, &[]);
    let dates = [
        "2024-06-30".to_string(),
        "2024-07-31".to_string(),
        "2024-11-30".to_string(),
        "2025-03-31".to_string(),
        "2025-05-31".to_string(),
    ];
    let (_, reports) = holdings_at_each(&txns, &prices, &[], &[], &sc, &dates).expect("series");
    let basis = |index: usize, symbol: &str| {
        reports[index]
            .holdings
            .iter()
            .find(|holding| holding.symbol == symbol)
            .and_then(|holding| holding.basis)
            .map(exact)
    };
    // $1000.00 + $600.00, carried through a split untouched.
    assert_eq!(basis(0, "AAPL"), Some((160_000, 2)));
    // 5 of 30 sold: 1600 × 25/30 = 1333.333…, half-even to the BASIS's own two
    // places. A re-derivation would land on `(1333333, 3)` or `(133333, 2)`'s
    // value with a different scale.
    assert_eq!(basis(1, "AAPL"), Some((133_333, 2)));
    // The return of capital: $800.00 − $50.00, still to the cent.
    assert_eq!(basis(2, "GLD"), Some((75_000, 2)));
    // The RSU vest is a cost-less lot, so AAPL's basis is gone from here on.
    assert_eq!(basis(3, "AAPL"), None);
    // Re-opened after a full sell-out: 6 × $250.00, a fresh exact basis.
    assert_eq!(basis(4, "GLD"), Some((150_000, 2)));
}

/// `as_of` is INCLUSIVE, and the replay is the only thing that decides so.
///
/// This is not covered by the identity gates above, and deliberately so: both
/// sides of those comparisons run the same boundary code, so a boundary that
/// moved by a day would move BOTH and compare equal. It was not covered by the
/// rest of the suite either — the rule used to be a `txn.date > as_of` skip
/// inside each pass, too obvious to test, and turning it into a snapshot
/// boundary is exactly what put it at risk. Expected values here are written
/// out, not derived.
#[test]
fn a_transaction_dated_exactly_on_a_point_is_included() {
    let txns = vec![
        txn(
            1,
            "2025-03-10",
            vec![
                buy("assets:broker:a", "AAPL", 10, 10000, true),
                posting("assets:broker:cash", vec![usd(-100_000)], &[]),
            ],
            &[],
        ),
        txn(
            2,
            "2025-05-31",
            vec![
                buy("assets:broker:a", "AAPL", 5, 10000, true),
                posting("assets:broker:cash", vec![usd(-50_000)], &[]),
            ],
            &[],
        ),
    ];
    let prices = vec![pd("2025-01-01", "AAPL", 10000, "$")];
    let sc = scope("2025-05-31", ScopeMode::Include, &[]);
    let shares_at = |report: &HoldingsReport| {
        report
            .holdings
            .iter()
            .find(|holding| holding.symbol == "AAPL")
            .map(|holding| exact(holding.shares))
    };

    // Single-date path: the day before, the day of, the day after.
    for (as_of, want) in [
        ("2025-03-09", None),
        ("2025-03-10", Some((10, 0))),
        ("2025-03-11", Some((10, 0))),
    ] {
        let point = HoldingsScope {
            as_of: as_of.to_string(),
            ..sc.clone()
        };
        let report = compute_holdings(&txns, &prices, &[], &[], &point).expect("report");
        assert_eq!(shares_at(&report), want, "compute_holdings at {as_of}");
    }

    // Multi-date path, with a boundary landing exactly on each transaction.
    let dates = [
        "2025-03-09".to_string(),
        "2025-03-10".to_string(),
        "2025-03-11".to_string(),
        "2025-05-30".to_string(),
        "2025-05-31".to_string(),
    ];
    let (_, reports) = holdings_at_each(&txns, &prices, &[], &[], &sc, &dates).expect("series");
    let shares: Vec<Option<(i128, u32)>> = reports.iter().map(shares_at).collect();
    assert_eq!(
        shares,
        vec![
            None,
            Some((10, 0)),
            Some((10, 0)),
            Some((10, 0)),
            // The LAST point is also the last transaction's date: a `break` that
            // stopped one transaction early would show 10 here.
            Some((15, 0)),
        ]
    );

    // And through the public series, whose final point clamps to `as_of`.
    let series =
        holdings_series(&txns, &prices, &[], &[], &sc, Interval::Monthly, 3).expect("series");
    assert_eq!(series.points.last().expect("point").date, "2025-05-31");
    assert_eq!(
        exact(series.points.last().expect("point").market_value),
        (150_000, 2),
        "15 shares at $100.00, including the transaction dated on the point"
    );
}

/// The account list on a holding is rebuilt at every boundary from the running
/// per-account tallies, so it has to shrink and grow with the series rather than
/// showing the final state at every point.
#[test]
fn per_point_account_lists_track_the_transfer() {
    let txns = torture_txns();
    let prices = torture_prices();
    let sc = scope("2025-12-31", ScopeMode::Include, &[]);
    let dates = [
        "2024-05-31".to_string(),
        "2024-06-30".to_string(),
        "2025-12-31".to_string(),
    ];
    let (_, reports) = holdings_at_each(&txns, &prices, &[], &[], &sc, &dates).expect("series");
    let accounts_of = |index: usize| {
        reports[index]
            .holdings
            .iter()
            .find(|holding| holding.symbol == "AAPL")
            .map(|holding| holding.accounts.clone())
            .expect("AAPL held")
    };
    assert_eq!(accounts_of(0), vec!["assets:broker:a".to_string()]);
    assert_eq!(
        accounts_of(1),
        vec!["assets:broker:a".to_string(), "assets:broker:b".to_string()]
    );
    assert_eq!(
        accounts_of(2),
        vec![
            "assets:broker:a".to_string(),
            "assets:broker:b".to_string(),
            "assets:broker:mixed".to_string()
        ]
    );
}
