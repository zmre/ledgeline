//! The three STOCK diagnostics — `stock-missing-basis`, `stock-negative`,
//! `stock-unpriced` — and the transaction attribution that anchors them.
//!
//! These used to be computed in the browser, by a second copy of the
//! average-cost pools that had drifted from this engine (DRY-1). The SPA now
//! reads them off `/api/diagnostics` in the same `Problem` shape the unbalanced
//! and assertion findings already use, so what has to hold is:
//!
//! 1. the right symbols are reported, with the engine's current semantics
//!    (split detection, sticky-negative taint, taint reset on a clean close);
//! 2. each one names the right TRANSACTION, as a 0-based position into the
//!    `/transactions` array — the anchor the deleted TS rules used to compute;
//! 3. the hledger-level diagnostics are untouched by any of it.
//!
//! The hledger-level half has its own suite in `diagnostics.rs`, including the
//! whole-fixtures-tree sweep; this one deliberately overlaps only at the seam.

mod common;

use ledgeline_core::model::Tindex;
use ledgeline_core::{Journal, parse_journal, wire};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn journal(text: &str) -> Journal {
    parse_journal(text, "/tmp/stock-diagnostics.journal").expect("journal parses")
}

/// The stock half of the payload for `text`, as `(txn_index, rule)` pairs.
fn anchors(text: &str) -> Vec<(usize, String)> {
    anchors_of(&journal(text))
}

fn anchors_of(journal: &Journal) -> Vec<(usize, String)> {
    serde_json::to_value(wire::journal_to_stock_diagnostics(journal))
        .expect("payload serializes")
        .as_array()
        .expect("an array")
        .iter()
        .map(|diagnostic| {
            (
                usize::try_from(
                    diagnostic["txnIndex"]
                        .as_u64()
                        .expect("txnIndex is a number"),
                )
                .expect("txnIndex fits usize"),
                diagnostic["rule"]
                    .as_str()
                    .expect("rule is a string")
                    .to_string(),
            )
        })
        .collect()
}

/// The rules reported for `text`, deduped in report order.
fn rules(text: &str) -> Vec<String> {
    anchors(text).into_iter().map(|(_, rule)| rule).collect()
}

// ---------------------------------------------------------------------------
// The mapping: one WarningKind → one rule, always a warning
// ---------------------------------------------------------------------------

/// A cost-less lot, a short position and an unpriced holding — one of each, so
/// all three rules appear in one payload and their order is pinned.
const ONE_OF_EACH: &str = concat!(
    "2024-01-10 costless buy\n    assets:broker   10 GLD\n    equity:opening\n",
    "\n",
    "2024-02-10 sell what was never bought\n    assets:broker   -2 TSLA\n    assets:cash   $100.00\n",
    "\n",
    "P 2024-03-01 TSLA $50.00\n",
);

#[test]
fn each_warning_kind_maps_to_its_own_rule() {
    assert_eq!(
        rules(ONE_OF_EACH),
        vec!["stock-missing-basis", "stock-negative", "stock-unpriced"]
    );
}

#[test]
fn every_stock_diagnostic_is_a_warning_never_an_error() {
    // The TS rules these replace emitted `severity: "warning"` for all three. A
    // missing cost basis describes a journal hledger itself accepts, so
    // promoting it to `error` would light the badge red on a healthy ledger.
    for diagnostic in
        serde_json::to_value(wire::journal_to_stock_diagnostics(&journal(ONE_OF_EACH)))
            .expect("payload serializes")
            .as_array()
            .expect("an array")
    {
        assert_eq!(diagnostic["severity"], "warning", "{diagnostic}");
    }
}

#[test]
fn every_stock_diagnostic_has_exactly_the_four_contract_fields() {
    for diagnostic in
        serde_json::to_value(wire::journal_to_stock_diagnostics(&journal(ONE_OF_EACH)))
            .expect("payload serializes")
            .as_array()
            .expect("an array")
    {
        let object = diagnostic.as_object().expect("diagnostic is an object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["message", "rule", "severity", "txnIndex"]);
        assert!(field_is_string(diagnostic, "message"), "{diagnostic}");
        assert!(diagnostic["txnIndex"].is_u64(), "{diagnostic}");
    }
}

fn field_is_string(diagnostic: &Value, key: &str) -> bool {
    diagnostic[key]
        .as_str()
        .is_some_and(|s| !s.trim().is_empty())
}

// ---------------------------------------------------------------------------
// Transaction attribution — the blocker DRY-1 named
// ---------------------------------------------------------------------------

#[test]
fn missing_basis_names_every_offending_lot_not_just_the_first() {
    // The deleted TS rule emitted one Problem PER cost-less lot, so a symbol
    // vested without a cost twice flags BOTH rows. One warning, two anchors.
    //
    // Both lots are funded from REVENUE rather than equity on purpose: a
    // cost-less top-up of an already-open position booked against equity is
    // spelled exactly like a stock split, and `is_redenomination` reads it as
    // one (its "known ambiguity"). A vest is unambiguously an acquisition.
    let found = anchors(concat!(
        "2024-01-10 first vest\n    assets:broker   10 GLD\n    income:rsu   -10 GLD\n",
        "\n",
        "2024-02-10 priced buy of something else\n    assets:broker   10 VTI @ $200.00\n    assets:cash\n",
        "\n",
        "2024-03-10 second vest\n    assets:broker   5 GLD\n    income:rsu   -5 GLD\n",
    ));
    let basis: Vec<usize> = found
        .iter()
        .filter(|(_, rule)| rule == "stock-missing-basis")
        .map(|(index, _)| *index)
        .collect();
    assert_eq!(
        basis,
        vec![0, 2],
        "both GLD lots, not the VTI buy: {found:?}"
    );
}

#[test]
fn missing_basis_forgets_the_lots_a_clean_close_sold() {
    // Bought cost-lessly, sold out IN FULL, bought back at a known cost: nothing
    // held has an unknown basis, so there is no finding AND no stale row anchor
    // pointing at the transaction that opened the old position.
    let found = anchors(concat!(
        "2024-01-10 gift\n    assets:broker   10 ROUND\n    equity:opening\n",
        "\n",
        "2024-02-10 sell in full\n    assets:broker   -10 ROUND @ $90.00\n    assets:cash\n",
        "\n",
        "2024-03-10 buy back\n    assets:broker   5 ROUND @ $200.00\n    assets:cash\n",
    ));
    assert!(
        !found.iter().any(|(_, rule)| rule == "stock-missing-basis"),
        "{found:?}"
    );
}

#[test]
fn negative_shares_anchors_to_the_transaction_that_crossed_zero() {
    // Buy 5, sell 10 (the crossing), sell 2 more. The row to flag is the sell
    // that took the running total negative, not the latest one.
    let found = anchors(concat!(
        "2024-01-10 buy\n    assets:broker   5 SHT @ $10.00\n    assets:cash\n",
        "\n",
        "2024-02-10 oversell\n    assets:broker   -10 SHT @ $10.00\n    assets:cash\n",
        "\n",
        "2024-03-10 sell more\n    assets:broker   -2 SHT @ $10.00\n    assets:cash\n",
    ));
    let negative: Vec<usize> = found
        .iter()
        .filter(|(_, rule)| rule == "stock-negative")
        .map(|(index, _)| *index)
        .collect();
    assert_eq!(negative, vec![1], "the crossing, not the last touch");
}

#[test]
fn negative_shares_falls_back_to_the_latest_touch_when_the_pool_opened_short() {
    // Never bought at all: the very first transaction is both the crossing and
    // the latest touch, and the fallback must not leave the finding unanchored.
    let found = anchors("2024-01-10 sell\n    assets:broker   -3 NVR\n    assets:cash   $30.00\n");
    assert!(
        found.contains(&(0, "stock-negative".to_string())),
        "{found:?}"
    );
}

#[test]
fn unpriced_anchors_to_the_latest_transaction_touching_the_symbol() {
    // Pricing is a property of the POSITION, not of a lot, so the newest row
    // that mentions the symbol is the one to look at.
    let found = anchors(concat!(
        "2024-01-10 buy\n    assets:broker   10 GLD\n    equity:opening\n",
        "\n",
        "2024-02-10 sell some, still held\n    assets:broker   -2 GLD\n    equity:opening\n",
    ));
    let unpriced: Vec<usize> = found
        .iter()
        .filter(|(_, rule)| rule == "stock-unpriced")
        .map(|(index, _)| *index)
        .collect();
    assert_eq!(unpriced, vec![1]);
}

#[test]
fn the_wire_index_is_a_position_not_the_one_based_tindex() {
    // The SPA translates `txnIndex` through the served array to the
    // transaction's own 1-based `tindex` (normalizeDiagnostics). Emitting the
    // tindex here would shift every finding one row down the table.
    let text = "2024-01-10 sell\n    assets:broker   -3 NVR\n    assets:cash   $30.00\n";
    let parsed = journal(text);
    assert_eq!(parsed.transactions[0].index, Tindex(1));
    assert_eq!(anchors_of(&parsed)[0].0, 0);
}

// ---------------------------------------------------------------------------
// Engine semantics the TypeScript copy did not have
// ---------------------------------------------------------------------------

#[test]
fn a_two_for_one_split_is_not_a_cost_less_acquisition() {
    // THE divergence DRY-1 was opened for. The TS pools had no split detection,
    // so the incoming shares read as a cost-less lot and the drawer contradicted
    // the Holdings page for the same journal.
    let found = anchors(concat!(
        "account equity:splits    ; type: E\n",
        "\n",
        "2024-01-10 buy\n    assets:broker   10 SPL @ $100.00\n    assets:cash\n",
        "\n",
        "2024-02-10 two-for-one split\n    assets:broker   10 SPL\n    equity:splits   -10 SPL\n",
    ));
    assert!(
        !found.iter().any(|(_, rule)| rule == "stock-missing-basis"),
        "a split keeps its basis: {found:?}"
    );
}

#[test]
fn a_share_leg_funded_from_revenue_does_not_net_the_position_to_zero() {
    // `is_holding_account`: the income leg funds the vest, it does not hold the
    // shares. Counting it would net the acquisition to zero and report the later
    // sale as a short. FE-2 — the TS copy grew this filter late.
    let found = anchors(concat!(
        "account vesting:grants    ; type: R\n",
        "\n",
        "2024-01-10 rsu vest\n    assets:broker   10 ACME\n    vesting:grants   -10 ACME\n",
        "\n",
        "2024-06-01 sell in full\n    assets:broker   -10 ACME @ $100.00\n    assets:cash\n",
    ));
    assert!(
        !found.iter().any(|(_, rule)| rule == "stock-negative"),
        "{found:?}"
    );
}

#[test]
fn a_transfer_between_two_holding_accounts_is_not_an_acquisition() {
    let found = anchors(concat!(
        "2024-01-10 buy\n    assets:broker:a   10 VTI @ $200.00\n    assets:cash\n",
        "\n",
        "2024-02-10 move\n    assets:broker:a   -4 VTI\n    assets:broker:b   4 VTI\n",
    ));
    assert!(
        !found.iter().any(|(_, rule)| rule == "stock-missing-basis"),
        "the incoming leg has no cost but nothing was acquired: {found:?}"
    );
}

// ---------------------------------------------------------------------------
// The as-of choice
// ---------------------------------------------------------------------------

#[test]
fn a_future_dated_cost_less_lot_is_reported_today() {
    // The TS rules valued at `today()`, so a future-dated transaction was
    // invisible to them. A journal-wide diagnostic is cached under an ETag and
    // must not change at midnight, so it covers the WHOLE journal — and a
    // mistyped entry is a mistake now, not on its settlement date.
    let found =
        anchors("9998-01-10 far future buy\n    assets:broker   10 GLD\n    equity:opening\n");
    assert!(
        found.iter().any(|(_, rule)| rule == "stock-missing-basis"),
        "{found:?}"
    );
}

// ---------------------------------------------------------------------------
// Composition with the hledger-level diagnostics
// ---------------------------------------------------------------------------

#[test]
fn the_errors_lead_and_the_stock_warnings_follow() {
    // The drawer groups by rule in FIRST-APPEARANCE order, so a journal that
    // both fails to balance and holds an unpriced security shows the hard error
    // at the top.
    let parsed = journal(concat!(
        "2024-01-01 unbalanced\n    a   $1.00\n    b   $-2.00\n",
        "\n",
        "2024-01-10 costless buy\n    assets:broker   10 GLD\n    equity:opening\n",
    ));
    let all: Vec<String> = serde_json::to_value(wire::journal_to_all_diagnostics(&parsed))
        .expect("payload serializes")
        .as_array()
        .expect("an array")
        .iter()
        .map(|d| d["rule"].as_str().expect("rule is a string").to_string())
        .collect();
    assert_eq!(all[0], "unbalanced");
    assert!(all.iter().any(|rule| rule.starts_with("stock-")), "{all:?}");
    assert_eq!(all.len(), wire::journal_to_diagnostics(&parsed).len() + 2);
}

#[test]
fn a_journal_with_no_securities_adds_nothing() {
    assert!(
        wire::journal_to_stock_diagnostics(&journal(
            "2024-01-01 t\n    expenses:x   $1.00\n    assets:bank\n",
        ))
        .is_empty()
    );
}

// ---------------------------------------------------------------------------
// The real fixture — the end-to-end expectation the SPA test mirrors
// ---------------------------------------------------------------------------

#[test]
fn sample_journal_reports_its_three_deliberate_stock_records() {
    // fixtures/sample.journal plants exactly these (see its header comment):
    // the 2025-08-20 GLD gift with no cost and no price directive, and the
    // 2026-06-22 TSLA sell of a position that was never entered. The positions
    // are 0-based, so the SPA resolves them to `tindex` 100 and 180 — the same
    // rows the deleted TypeScript rules flagged.
    //
    // web/src/lib/checks/stock-diagnostics.test.ts asserts the other end of this
    // from the captured payload; keeping both means a change to either side
    // fails somewhere rather than drifting.
    let found = anchors_of(&common::fixture_journal());
    assert_eq!(
        found,
        vec![
            (99, "stock-missing-basis".to_string()),
            (179, "stock-negative".to_string()),
            (99, "stock-unpriced".to_string()),
        ]
    );
}

#[test]
fn the_committed_sample_capture_still_describes_the_engine() {
    // fixtures/api/ledgeline/diagnostics.json is what the SPA test consumes. If
    // this fails, re-capture it (the recipe is in that test's header).
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(common::fixtures_dir().join("api/ledgeline/diagnostics.json"))
            .expect("capture readable"),
    )
    .expect("capture is JSON");
    let actual =
        wire::journal_to_diagnostics_value(&common::fixture_journal()).expect("payload serializes");
    assert_eq!(actual, expected);
}

// ---------------------------------------------------------------------------
// The wire ⇄ SPA allow-list
// ---------------------------------------------------------------------------
//
// `diagnostic_rules_match_the_spa_allow_list` used to live here and read
// `web/src/lib/api/normalize.ts`. It passed under `cargo test` and FAILED under
// `nix build .#tests` — which is what CI runs — because that derivation's source
// is `craneLib.cleanCargoSource`, so `web/` is not in it at all.
//
// The assertion now lives in `web/src/lib/checks/stock-diagnostics.test.ts`,
// which reads BOTH files: vitest always runs from a full checkout, so it works
// in CI and locally. It is mutation-checked — widening DIAGNOSTIC_RULES on
// either side fails it.
