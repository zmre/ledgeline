//! The startup-metrics Problems checks (plan 37): `recurring-billing-clearing`
//! and `customer-name-variants`, through [`wire::journal_to_saas_diagnostics`]
//! and the combined `/api/diagnostics` payload.

mod common;

use ledgeline_core::{Journal, parse_journal, wire};
use serde_json::Value;

fn journal(text: &str) -> Journal {
    parse_journal(text, "/tmp/saas-diagnostics.journal").expect("journal parses")
}

/// The saas findings for `text`, as raw JSON objects.
fn diagnostics(text: &str) -> Vec<Value> {
    serde_json::to_value(wire::journal_to_saas_diagnostics(&journal(text)))
        .expect("payload serializes")
        .as_array()
        .expect("an array")
        .clone()
}

fn rules(found: &[Value]) -> Vec<&str> {
    found
        .iter()
        .map(|d| d["rule"].as_str().expect("a rule"))
        .collect()
}

const DECLS: &str = "\
account assets:receivable        ; type: A, kpi: receivable
account liabilities:deferred     ; type: L, kpi: deferred-revenue
account revenue:sales            ; type: R, kpi: recurring
account revenue:other            ; type: R, kpi: recurring
";

/// Invoice to revenue, then a monthly move from revenue to deferred: the
/// clearing pattern. One `info` per recurring account, with the count, anchored
/// to the first move.
#[test]
fn clearing_fires_once_per_recurring_account_with_the_count() {
    let text = format!(
        "{DECLS}
2025-01-01 Invoice Acme
    assets:receivable     $1200
    revenue:sales        $-1200  ; customer: Acme

2025-01-31 Move to deferred
    revenue:sales          $1100
    liabilities:deferred  $-1100

2025-02-28 Move to deferred
    revenue:sales          $100
    revenue:other           $50
    liabilities:deferred  $-150

2025-03-01 Recognise only
    liabilities:deferred   $100
    revenue:sales         $-100
"
    );
    let found = diagnostics(&text);
    assert_eq!(
        rules(&found),
        ["recurring-billing-clearing", "recurring-billing-clearing"]
    );
    assert_eq!(found[0]["severity"], "info");
    assert_eq!(found[0]["txnIndex"], 1);
    assert!(found[0].get("account").is_none(), "transaction-anchored");
    let message = found[0]["message"].as_str().expect("message");
    for part in [
        "revenue:sales",
        "2 transactions",
        "2025-01-31 Move to deferred",
    ] {
        assert!(message.contains(part), "{part}: {message}");
    }
    assert_eq!(found[1]["txnIndex"], 2);
    let second = found[1]["message"].as_str().expect("message");
    assert!(second.contains("revenue:other") && second.contains("1 transaction "));
    assert!(wire::DIAGNOSTIC_RULES.contains(&"recurring-billing-clearing"));
}

/// Recognising deferred revenue (deferred debited, revenue credited) is the
/// recommended pattern and never fires; nor does a journal with no recurring
/// role, whatever its shape.
#[test]
fn recognition_and_untagged_books_never_fire() {
    let recognition = format!(
        "{DECLS}
2025-01-01 Annual prepay
    assets:receivable      $1200
    liabilities:deferred  $-1200

2025-01-31 Revenue recognition
    liabilities:deferred   $100
    revenue:sales         $-100  ; customer: Acme
"
    );
    assert!(diagnostics(&recognition).is_empty());
    let untagged = "\
2025-01-31 Move to deferred
    revenue:sales          $100
    liabilities:deferred  $-100
";
    assert!(diagnostics(untagged).is_empty());
}

/// `Contoso Ltd` and `Contoso Ltd.` are one customer spelled two ways: one
/// warning naming both and a transaction for each, anchored to the first.
/// Posting- and transaction-level `customer:` tags are both read.
#[test]
fn name_variants_fire_once_per_group() {
    let text = format!(
        "{DECLS}
2025-01-31 Recognition  ; customer: Contoso Ltd
    liabilities:deferred   $100
    revenue:sales         $-100

2025-02-28 Recognition
    liabilities:deferred   $100
    revenue:sales         $-100  ; customer: Contoso Ltd.

2025-03-31 Recognition
    liabilities:deferred   $300
    revenue:sales         $-100  ; customer: CONTOSO  LTD
    revenue:sales         $-100  ; customer: Acme
    revenue:other         $-100  ; customer: Acme Labs
"
    );
    let found = diagnostics(&text);
    assert_eq!(rules(&found), ["customer-name-variants"], "{found:#?}");
    assert_eq!(found[0]["severity"], "warning");
    assert_eq!(found[0]["txnIndex"], 0);
    let message = found[0]["message"].as_str().expect("message");
    for part in [
        "'Contoso Ltd' (2025-01-31 Recognition)",
        "'Contoso Ltd.' (2025-02-28 Recognition)",
        "'CONTOSO  LTD' (2025-03-31 Recognition)",
        "3 different customers",
    ] {
        assert!(message.contains(part), "{part}: {message}");
    }
    assert!(!message.contains("Acme"), "Acme vs Acme Labs: {message}");
    assert!(wire::DIAGNOSTIC_RULES.contains(&"customer-name-variants"));
}

/// The normalisation drops ONE trailing legal-suffix WORD, so `Acme` vs
/// `Acme Labs` stay apart, `Costco` keeps its `co` (not a word) while
/// `Cost Co` becomes `cost`, and a bare `Co` keeps its name. A value repeated
/// verbatim is one customer, not a variant.
#[test]
fn distinct_customers_are_not_variants() {
    let text = "\
2025-01-01 a  ; customer: Acme
    assets:bank   $1
    revenue:x

2025-01-02 b  ; customer: Acme Labs
    assets:bank   $1
    revenue:x

2025-01-03 c  ; customer: Costco
    assets:bank   $1
    revenue:x

2025-01-04 d  ; customer: Cost Co
    assets:bank   $1
    revenue:x

2025-01-05 e  ; customer: Co
    assets:bank   $1
    revenue:x

2025-01-06 f  ; customer: Acme
    assets:bank   $1
    revenue:x
";
    assert!(diagnostics(text).is_empty(), "{:#?}", diagnostics(text));
}

/// `Acme Inc` / `ACME inc.` / `acme` are one group, and the combined payload
/// carries the findings (after the account tags, before the stock rules).
#[test]
fn suffix_and_case_variants_reach_the_combined_payload() {
    let text = "\
account revenue:x  ; type: R, kpi: recurrent

2025-01-01 a  ; customer: Acme Inc
    assets:bank   $1
    revenue:x

2025-01-02 b  ; customer: ACME inc.
    assets:bank   $1
    revenue:x

2025-01-03 c  ; customer: acme
    assets:bank   $1
    revenue:x
";
    let all: Vec<String> = serde_json::to_value(wire::journal_to_all_diagnostics(&journal(text)))
        .expect("serializes")
        .as_array()
        .expect("an array")
        .iter()
        .map(|d| d["rule"].as_str().expect("a rule").to_string())
        .collect();
    assert_eq!(all, ["account-tag", "customer-name-variants"]);
}

/// sample.journal carries no `kpi:` and no `customer:` variants.
#[test]
fn the_sample_journal_is_clean() {
    let sample = common::fixture_journal();
    assert!(wire::journal_to_saas_diagnostics(&sample).is_empty());
}

/// Postings keyed by `customer-id:` are one customer however the name is
/// spelled, and two ids are two customers however alike their names are:
/// neither is a name-variant finding.
#[test]
fn customer_ids_are_never_name_variants() {
    let same_id = "
2025-01-31 Recognition
    revenue:subs    -$100.00  ; customer-id: C-1, customer: Contoso Ltd
    assets:bank      $100.00

2025-02-28 Recognition
    revenue:subs    -$100.00  ; customer-id: C-1, customer: Contoso Ltd.
    assets:bank      $100.00
";
    assert!(
        !rules(&diagnostics(same_id)).contains(&"customer-name-variants"),
        "same id, two spellings"
    );
    let two_ids = "
2025-01-31 Recognition
    revenue:subs    -$100.00  ; customer-id: C-1, customer: Acme Inc
    assets:bank      $100.00

2025-02-28 Recognition
    revenue:subs    -$100.00  ; customer-id: C-2, customer: ACME
    assets:bank      $100.00
";
    assert!(
        !rules(&diagnostics(two_ids)).contains(&"customer-name-variants"),
        "two ids, similar names"
    );
    // A plain `customer:` alongside still fires against another plain one.
    let mixed = "
2025-01-31 Recognition
    revenue:subs    -$100.00  ; customer-id: C-1, customer: Acme Inc
    assets:bank      $100.00

2025-02-28 Recognition
    revenue:subs    -$100.00  ; customer: Globex
    assets:bank      $100.00

2025-03-31 Recognition
    revenue:subs    -$100.00  ; customer: Globex LLC
    assets:bank      $100.00
";
    let found = diagnostics(mixed);
    let variants: Vec<&Value> = found
        .iter()
        .filter(|d| d["rule"] == "customer-name-variants")
        .collect();
    assert_eq!(variants.len(), 1);
    assert!(!variants[0]["message"].as_str().unwrap().contains("Acme"));
}
