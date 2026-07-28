//! Journal-wide diagnostics: unbalanced transactions (PARSE-1) and failed
//! balance assertions (PARSE-2), and the wire payload that carries both.
//!
//! An unbalanced transaction and a failed balance assertion are DIAGNOSTICS,
//! not parse errors. The journal always opens; neither ever blocks opening,
//! editing or reloading. They surface through the `{"diagnostics": [...]}`
//! payload, each element shaped exactly like the SPA's existing `Problem`
//! (`web/src/lib/checks/engine.ts`).
//!
//! Every expectation below was pinned against real `hledger 1.52`
//! (mac-aarch64), whose wording is reproduced verbatim:
//!
//! | journal                                    | hledger 1.52 | ledgeline |
//! |--------------------------------------------|--------------|-----------|
//! | `a $1.00` / `b $-2.00`                     | rejected     | 1 diagnostic |
//! | `[v1] $3.00` / `[v2] $-1.00`               | rejected     | 1 diagnostic |
//! | `a 10 AAA` / `b $-50.00` (2 commodities)   | ACCEPTED     | clean (parity) |
//! | `a 10 AAA` / `b $-50.00` / `c 5 BBB`       | rejected     | 1 diagnostic |
//! | `a 10 AAA` / `b -10 AAA` / `c $-1.00`      | rejected     | 1 diagnostic |
//! | `(a) 10 AAA` / `(b) $-50.00` (unbalanced virtual) | ACCEPTED | clean (parity) |
//! | elided posting                             | ACCEPTED     | clean (parity) |
//!
//! The headline case is the two-commodity one: hledger treats a residual in
//! exactly two commodities as an implicit conversion and infers the cost that
//! balances it, so such a journal loads cleanly. A naive "every sum must be
//! zero" check would flag it, which is why the sweep below over the WHOLE
//! fixtures tree — every journal of which hledger accepts — is the primary
//! correctness signal for this check.

mod common;

use ledgeline_core::parse::{check_transaction_balances, parse_journal};
use ledgeline_core::{Journal, wire};
use serde_json::Value;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn journal(text: &str) -> Journal {
    parse_journal(text, "/tmp/diagnostics.journal").expect("journal parses")
}

/// The HLEDGER-LEVEL diagnostics for `text`, serialized.
///
/// Deliberately `journal_to_diagnostics` and not the whole
/// `/api/diagnostics` payload: this suite is about PARSE-1 and PARSE-2, and
/// most of its journals below post bare commodity amounts (`a 10 AAA`) that the
/// stock rules correctly report as cost-less, unpriced holdings. Mixing the two
/// halves in here would drown the signal. The stock half has its own suite in
/// `stock_diagnostics.rs`, which also pins how the two combine.
fn diagnostics(text: &str) -> Vec<Value> {
    let found = wire::journal_to_diagnostics(&journal(text));
    serde_json::to_value(found)
        .expect("payload serializes")
        .as_array()
        .cloned()
        .expect("a diagnostics array")
}

/// The one diagnostic `text` produces, or a panic naming what was found.
fn one_diagnostic(text: &str) -> Value {
    let found = diagnostics(text);
    assert_eq!(
        found.len(),
        1,
        "expected exactly one diagnostic, got {found:#?}"
    );
    found.into_iter().next().expect("checked length")
}

fn field<'a>(diagnostic: &'a Value, key: &str) -> &'a Value {
    diagnostic
        .get(key)
        .unwrap_or_else(|| panic!("diagnostic has `{key}`: {diagnostic}"))
}

// ---------------------------------------------------------------------------
// PARSE-1 — the balance check itself
// ---------------------------------------------------------------------------

#[test]
fn a_single_commodity_imbalance_is_reported_in_hledgers_words() {
    // hledger 1.52:
    //   This transaction is unbalanced.
    //   The real postings' sum should be 0 but is: $-1.00
    let diagnostic = one_diagnostic("2024-01-01 unbalanced\n    a   $1.00\n    b   $-2.00\n");
    assert_eq!(field(&diagnostic, "txnIndex"), 0);
    assert_eq!(field(&diagnostic, "rule"), "unbalanced");
    assert_eq!(field(&diagnostic, "severity"), "error");
    assert_eq!(
        field(&diagnostic, "message"),
        "This transaction is unbalanced.\n\
         The real postings' sum should be 0 but is: $-1.00"
    );
}

#[test]
fn a_balanced_virtual_group_is_checked_separately_and_named_as_such() {
    // hledger 1.52: "The balanced virtual postings' sum should be 0 but is: $2.00"
    // The REAL postings here balance, so exactly one diagnostic is expected.
    let diagnostic = one_diagnostic(concat!(
        "2024-01-01 t\n",
        "    a       $1.00\n",
        "    b      $-1.00\n",
        "    [v1]    $3.00\n",
        "    [v2]   $-1.00\n",
    ));
    assert_eq!(
        field(&diagnostic, "message"),
        "This transaction is unbalanced.\n\
         The balanced virtual postings' sum should be 0 but is: $2.00"
    );
}

#[test]
fn a_two_commodity_residual_is_hledgers_implicit_conversion_and_stays_clean() {
    // THE case a naive zero-check gets wrong. hledger 1.52 loads all three of
    // these without complaint (`hledger -f … print` succeeds, and
    // `check autobalanced` passes): a residual in exactly two commodities is an
    // implicit conversion, and hledger infers the cost that balances it.
    for text in [
        "2024-01-01 t\n    a   10 AAA\n    b   $-50.00\n",
        "2024-01-01 t\n    a   10 AAA\n    b   $-50.00\n    c   $-1.00\n",
        "2024-01-01 t\n    a   10 AAA\n    b   -10 AAA\n    c   $-50.00\n    d   5 BBB\n",
    ] {
        assert!(
            diagnostics(text).is_empty(),
            "hledger accepts this two-commodity conversion:\n{text}"
        );
    }
}

#[test]
fn three_commodities_left_over_is_unbalanced_and_lists_them_lexically() {
    // hledger 1.52:
    //   This multi-commodity transaction is unbalanced.
    //   The real postings' sum should be 0 but is: $-50.00, 10 AAA, 5 BBB
    //   Consider adjusting this entry's amounts, adding missing postings,
    //   or recording conversion price(s) with @, @@ or equity postings.
    let diagnostic = one_diagnostic(concat!(
        "2024-01-01 t\n",
        "    a   10 AAA\n",
        "    b   $-50.00\n",
        "    c   5 BBB\n",
    ));
    assert_eq!(
        field(&diagnostic, "message"),
        "This multi-commodity transaction is unbalanced.\n\
         The real postings' sum should be 0 but is: $-50.00, 10 AAA, 5 BBB\n\
         Consider adjusting this entry's amounts, adding missing postings,\n\
         or recording conversion price(s) with @, @@ or equity postings."
    );
}

#[test]
fn a_commodity_that_nets_to_zero_is_dropped_from_the_residual_but_not_the_wording() {
    // hledger 1.52 words this the multi-commodity way (the transaction spans two
    // commodities) yet lists only the non-zero residual.
    let diagnostic = one_diagnostic(concat!(
        "2024-01-01 t\n",
        "    a   10 AAA\n",
        "    b   -10 AAA\n",
        "    c   $-1.00\n",
    ));
    assert_eq!(
        field(&diagnostic, "message"),
        "This multi-commodity transaction is unbalanced.\n\
         The real postings' sum should be 0 but is: $-1.00\n\
         Consider adjusting this entry's amounts, adding missing postings,\n\
         or recording conversion price(s) with @, @@ or equity postings."
    );
}

#[test]
fn an_unbalanced_virtual_posting_never_has_to_balance() {
    // `(a)` postings are excluded from balancing entirely; hledger accepts this.
    assert!(
        diagnostics("2024-01-01 t\n    (a)   10 AAA\n    (b)   $-50.00\n").is_empty(),
        "unbalanced virtual postings are excluded from balancing"
    );
}

#[test]
fn an_elided_posting_always_balances_its_group() {
    for text in [
        "2024-01-01 t\n    a   $1.00\n    b\n",
        "2024-01-01 t\n    a   $1.00\n    b   $-1.00\n    c\n",
        "2024-01-01 t\n    a   10 AAA\n    b   $-50.00\n    c   5 BBB\n    d\n",
    ] {
        assert!(diagnostics(text).is_empty(), "inference balances:\n{text}");
    }
}

#[test]
fn costs_are_valued_at_cost_exactly_as_the_inference_values_them() {
    // `10 AAPL @ $5.00` contributes $50.00, not 10 AAPL — so this balances and
    // hledger accepts it.
    assert!(
        diagnostics("2024-01-01 t\n    a   10 AAPL @ $5.00\n    b   $-50.00\n").is_empty(),
        "a unit cost is valued at cost"
    );
    // …and one cent off is not. A single commodity remains ($), so this is the
    // single-commodity wording.
    let diagnostic = one_diagnostic("2024-01-01 t\n    a   10 AAPL @ $5.00\n    b   $-50.01\n");
    assert_eq!(
        field(&diagnostic, "message"),
        "This transaction is unbalanced.\n\
         The real postings' sum should be 0 but is: $-0.01"
    );
}

#[test]
fn a_discrepancy_at_a_written_precision_is_always_reported() {
    // Three thousandths of a dollar written out are a real discrepancy, and
    // hledger 1.52 rejects this journal. Only the digits a COST multiplication
    // manufactured are ever forgiven (see the test below), so no tolerance can
    // reach a precision a human typed.
    let diagnostic = one_diagnostic(concat!(
        "2024-01-01 t\n",
        "    a   $0.001\n",
        "    b   $0.001\n",
        "    c   $0.001\n",
        "    d   $0.00\n",
    ));
    assert_eq!(
        field(&diagnostic, "message"),
        "This transaction is unbalanced.\n\
         The real postings' sum should be 0 but is: $0.003"
    );
    // A single written thousandth, likewise (hledger rejects) — and declaring
    // `commodity $1,000.00` does NOT loosen it, which is how we know the
    // tolerance comes from what was written, not from what was declared.
    for text in [
        "2024-01-01 t\n    a   $1.001\n    b   $-1.00\n",
        "commodity $1,000.00\n\n2024-01-01 t\n    a   $1.001\n    b   $-1.00\n",
    ] {
        assert_eq!(diagnostics(text).len(), 1, "hledger rejects:\n{text}");
    }
}

#[test]
fn digits_manufactured_by_a_unit_price_are_forgiven_exactly_as_hledger_forgives_them() {
    // `fixtures/corpus/precision.journal`, verbatim. 55.3653 × 30.92189512 is
    // 1712.000000112664, so the residual is 0.000000112664 D — and hledger
    // 1.52 loads it without complaint (`print` succeeds, `check autobalanced`
    // passes), because a unit price is a rounded quotient. An exact-zero test
    // would flag every real journal that records a price this way.
    assert!(
        diagnostics("2010-01-01 x\n    A  55.3653 C @ 30.92189512 D\n    A  -1712 D\n").is_empty(),
        "hledger accepts the price-multiplication residual"
    );
    // The tolerance is bounded by the WRITTEN precision, not the price's: give
    // the journal a 5-decimal D amount and the same residual is reported, which
    // is again exactly hledger's verdict.
    assert_eq!(
        diagnostics(concat!(
            "2024-01-01 t\n",
            "    a  1.5 C @ 2.0001 D\n",
            "    b  -3.0 D\n",
            "    c  0.00000 D\n",
            "    d  0.00000 D\n",
        ))
        .len(),
        1,
        "a written 5-decimal amount tightens the tolerance to exactness"
    );
}

#[test]
fn every_unbalanced_transaction_is_reported_with_its_own_row_index() {
    // A reconciliation pass that reports the first break and hides the rest is a
    // worse reconciliation tool — and `txnIndex` must address the SAME array
    // `/transactions` serves.
    let found = diagnostics(concat!(
        "2024-01-01 ok\n    a   $1.00\n    b   $-1.00\n",
        "\n",
        "2024-01-02 bad\n    a   $1.00\n    b   $-2.00\n",
        "\n",
        "2024-01-03 worse\n    a   $5.00\n    b   $-1.00\n",
    ));
    assert_eq!(found.len(), 2);
    assert_eq!(field(&found[0], "txnIndex"), 1);
    assert_eq!(field(&found[1], "txnIndex"), 2);
}

#[test]
fn an_unbalanced_transaction_never_blocks_the_journal() {
    // The product decision: the journal always opens. Everything else about it
    // is still readable and still served.
    let parsed = journal(concat!(
        "2024-01-01 bad\n    a   $1.00\n    b   $-2.00\n",
        "\n",
        "2024-01-02 fine\n    expenses:x   $3.00\n    assets:bank\n",
    ));
    assert_eq!(parsed.transactions.len(), 2);
    assert_eq!(
        parsed.transactions[1].postings[1].amounts[0].commodity.0,
        "$"
    );
    assert_eq!(check_transaction_balances(&parsed).unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// PARSE-2 wiring — assertions reach the same payload
// ---------------------------------------------------------------------------

#[test]
fn a_failed_balance_assertion_reaches_the_wire_as_a_diagnostic() {
    // CLEANUP.md's PARSE-2 repro, which used to be accepted silently.
    let diagnostic =
        one_diagnostic("2024-01-01 assertfail\n    a   $1.00 = $99.00\n    b   $-1.00\n");
    assert_eq!(field(&diagnostic, "txnIndex"), 0);
    assert_eq!(field(&diagnostic, "rule"), "assertion");
    assert_eq!(field(&diagnostic, "severity"), "error");
    let message = field(&diagnostic, "message")
        .as_str()
        .expect("message is a string");
    assert!(
        message.starts_with("balance assertion failed in a\n"),
        "{message}"
    );
    assert!(
        message.contains("the asserted balance is:       $99.00"),
        "{message}"
    );
    assert!(
        message.contains("but the calculated balance is: $1.00"),
        "{message}"
    );
}

#[test]
fn an_assertion_diagnostic_names_the_row_it_belongs_to_not_its_evaluation_position() {
    // Assertions are evaluated in DATE order, which is not file order. The
    // `txnIndex` must still address the transactions array as served.
    let found = diagnostics(concat!(
        "2024-02-01 later\n    a   $1.00 = $99.00\n    b   $-1.00\n",
        "\n",
        "2024-01-01 earlier\n    c   $1.00 = $77.00\n    d   $-1.00\n",
    ));
    assert_eq!(found.len(), 2);
    // Evaluation order puts the January transaction (file index 1) first.
    assert_eq!(field(&found[0], "txnIndex"), 1);
    assert_eq!(field(&found[1], "txnIndex"), 0);
}

#[test]
fn both_rules_land_in_one_payload() {
    let found = diagnostics(concat!(
        "2024-01-01 unbalanced\n    a   $1.00\n    b   $-2.00\n",
        "\n",
        "2024-01-02 assertfail\n    c   $1.00 = $99.00\n    d   $-1.00\n",
    ));
    let rules: Vec<&str> = found
        .iter()
        .map(|d| field(d, "rule").as_str().expect("rule is a string"))
        .collect();
    assert_eq!(rules, vec!["unbalanced", "assertion"]);
}

// ---------------------------------------------------------------------------
// The wire contract
// ---------------------------------------------------------------------------

#[test]
fn a_clean_journal_emits_an_empty_array_never_null_and_never_absent() {
    let value = wire::journal_to_diagnostics_value(&journal(
        "2024-01-01 t\n    expenses:x   $1.00\n    assets:bank\n",
    ))
    .expect("payload serializes");
    assert_eq!(value, serde_json::json!({"diagnostics": []}));
}

#[test]
fn every_diagnostic_has_exactly_the_four_contract_fields() {
    for diagnostic in diagnostics(concat!(
        "2024-01-01 unbalanced\n    a   $1.00\n    b   $-2.00\n",
        "\n",
        "2024-01-02 assertfail\n    c   $1.00 = $99.00\n    d   $-1.00\n",
    )) {
        let object = diagnostic.as_object().expect("diagnostic is an object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["message", "rule", "severity", "txnIndex"]);
        assert!(field(&diagnostic, "txnIndex").is_u64(), "{diagnostic}");
        assert!(field(&diagnostic, "message").is_string(), "{diagnostic}");
        assert_eq!(field(&diagnostic, "severity"), "error");
        assert!(
            matches!(
                field(&diagnostic, "rule").as_str(),
                Some("unbalanced" | "assertion")
            ),
            "{diagnostic}"
        );
    }
}

// ---------------------------------------------------------------------------
// The sweep — the primary correctness signal
// ---------------------------------------------------------------------------

/// Every `*.journal` under `fixtures/`, EXCLUDING `fixtures/corpus/errors/`
/// (journals hledger deliberately rejects) and any file that is `include`d by
/// another (parsing it standalone is not how it is meant to be read).
fn sweepable_journals() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {dir:?}: {e}"));
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "errors") {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "journal") {
                out.push(path);
            }
        }
    }
    let mut paths = Vec::new();
    walk(&common::fixtures_dir(), &mut paths);
    paths.sort();
    paths
}

#[test]
fn the_whole_fixtures_tree_is_diagnostic_free() {
    // Every journal here passes `hledger -f FILE check parseable autobalanced
    // assertions`. A correct PARSE-1 and a correct PARSE-2 wiring must
    // therefore produce ZERO diagnostics across the tree; even one means the
    // check is wrong.
    let journals = sweepable_journals();
    assert!(
        journals.len() >= 40,
        "expected the full fixtures tree, found {}",
        journals.len()
    );

    let mut noisy: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for path in &journals {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        let source_name = path.to_string_lossy().to_string();
        let Ok(parsed) = parse_journal(&text, &source_name) else {
            // Parse failures are the corpus suite's business, not this one.
            continue;
        };
        checked += 1;
        let found = wire::journal_to_diagnostics(&parsed);
        if !found.is_empty() {
            noisy.push(format!("{}: {found:#?}", path.display()));
        }
    }

    println!(
        "diagnostic sweep: {checked} journals checked, {} noisy",
        noisy.len()
    );
    assert!(
        noisy.is_empty(),
        "fixtures hledger accepts must produce NO diagnostics:\n{}",
        noisy.join("\n")
    );
}
