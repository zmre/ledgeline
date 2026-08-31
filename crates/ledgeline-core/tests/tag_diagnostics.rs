//! The `account-tag` diagnostic — the five closed-vocabulary `account` tags,
//! and what happens when one of them cannot be read.
//!
//! Four of them (`issection:`, `bsterm:`, `holdings:`, `valuation:`) used to
//! REFUSE an unrecognized value: a `ReportError` that became a `400` and took
//! the whole tab down with it. The fifth (`type:`) was always lenient and was
//! therefore always silent. Both halves were wrong in the same way — one was too
//! loud to be survivable, the other too quiet to be actionable — and this suite
//! pins the settlement:
//!
//! 1. an unreadable value reads as UNDECLARED, so the report still computes on
//!    its documented fallback (`leniency`, below);
//! 2. it is never silent: exactly one `account-tag` warning names the account,
//!    the tag, the value as written and the codes that would have worked;
//! 3. an EMPTY value is still not a declaration and still not a finding;
//! 4. every bad tag in the journal is reported, not just the first, in an order
//!    that is a function of the journal alone.
//!
//! The wire shape is asserted here too, because `account-tag` is the first
//! diagnostic anchored to something other than a transaction and the SPA's
//! decoder branches on exactly that.

mod common;

use ledgeline_core::holdings::{
    HoldingsClass, ValuationRole, declared_holdings_classes, declared_valuation_roles,
};
use ledgeline_core::reports::{
    AccountType, IsSectionKind, account_decls, account_sections, bs_terms, declared_types,
    resolve_account_type,
};
use ledgeline_core::{Journal, parse_journal, wire};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn journal(text: &str) -> Journal {
    parse_journal(text, "/tmp/tag-diagnostics.journal").expect("journal parses")
}

/// The `account-tag` half of the payload, as raw JSON objects.
fn tag_diagnostics(journal: &Journal) -> Vec<Value> {
    serde_json::to_value(wire::journal_to_tag_diagnostics(journal))
        .expect("payload serializes")
        .as_array()
        .expect("an array")
        .clone()
}

/// The messages reported for `text`, in report order.
fn messages(text: &str) -> Vec<String> {
    tag_diagnostics(&journal(text))
        .iter()
        .map(|d| d["message"].as_str().expect("a message").to_string())
        .collect()
}

/// The single message reported for `text` — the common case, and an assertion
/// that there is exactly one.
fn only_message(text: &str) -> String {
    let all = messages(text);
    assert_eq!(all.len(), 1, "expected exactly one finding, got {all:#?}");
    all.into_iter().next().expect("one")
}

/// A journal declaring every one of the five tags, with `{}` where the value of
/// the tag under test goes.
fn one_bad(tag: &str, value: &str) -> String {
    format!("account assets:house  ; {tag}: {value}\n")
}

// ---------------------------------------------------------------------------
// One bad value, one finding — for each of the five tags
// ---------------------------------------------------------------------------

/// Every message names the four things that make it actionable: the account, the
/// tag, the value AS WRITTEN, and the way out.
///
/// The "way out" is the part a diagnostic is worthless without. "…is not one of"
/// with nothing after it tells the user only what they already know.
#[test]
fn each_tag_reports_the_account_the_value_and_the_alternatives() {
    for (tag, bad, codes) in [
        ("type", "expenditure", vec!["A", "Expense", "Gain"]),
        (
            "issection",
            "cost-of-goods-sold",
            vec!["revenue", "cogs", "other"],
        ),
        ("bsterm", "long-ish", vec!["current", "noncurrent"]),
        ("holdings", "real-estate", vec!["stocks", "other", "none"]),
        (
            "valuation",
            "unrealised-gain",
            vec!["cost", "unrealized", "adjustment"],
        ),
    ] {
        let message = only_message(&one_bad(tag, bad));
        assert!(
            message.contains("assets:house"),
            "{tag}: names the account — {message}"
        );
        assert!(
            message.contains(&format!("{tag}: {bad}")),
            "{tag}: quotes the directive as written — {message}"
        );
        for code in codes {
            assert!(
                message.contains(code),
                "{tag}: names the alternative {code:?} — {message}"
            );
        }
    }
}

/// The exact sentence, once, so a reword is a deliberate act rather than a side
/// effect.
///
/// Two halves: the sentence the `400` used to carry, kept verbatim so moving the
/// finding into the drawer lost nothing, then what ignoring the tag cost — which
/// the `400` never needed to say, because its own consequence was "no report at
/// all".
#[test]
fn the_message_carries_the_old_sentence_and_then_what_it_cost() {
    assert_eq!(
        only_message(&one_bad("issection", "cost-of-goods-sold")),
        "account 'assets:house' declares `issection: cost-of-goods-sold`, \
         which is not one of revenue, cogs, opex, depreciation, interest, tax, other; \
         the tag is ignored and the account falls to the default section inference"
    );
}

/// EVERY tag names the fallback it actually took, and each names a DIFFERENT
/// one.
///
/// A generic "the tag is ignored" would be reassuring for `bsterm:` and
/// dangerously misleading for `valuation:`, so the clause has to be per-tag.
/// Asserting the five are distinct is what stops a future edit collapsing them
/// back into one sentence.
#[test]
fn every_message_names_the_fallback_that_was_taken() {
    let clauses: Vec<String> = ["type", "issection", "bsterm", "holdings", "valuation"]
        .into_iter()
        .map(|tag| {
            let message = only_message(&one_bad(tag, "nonsense-value"));
            let (_, consequence) = message
                .split_once("; the tag is ignored and ")
                .unwrap_or_else(|| panic!("{tag}: no consequence clause in {message:?}"));
            consequence.to_string()
        })
        .collect();

    assert_eq!(
        clauses,
        vec![
            "the type is inferred from the account name instead",
            "the account falls to the default section inference",
            "the account falls to the adaptive default grouping",
            "the account is classified mechanically (does it hold a non-currency commodity?)",
            "the account is treated as cost basis, so any unrealized gain on this holding will \
             read as zero",
        ]
    );
}

/// **`valuation:` must say the gain will read ZERO.**
///
/// This is the one consequence that names a NUMBER the user will see rather than
/// a box something landed in. The other four cost a misfiling and the totals
/// still add up; this one silently replaces a real gain with zero, because an
/// account meant to carry a mark-to-market adjustment is folded back into its own
/// basis — and a zero gain is indistinguishable from a holding that genuinely has
/// not moved.
///
/// It is the strongest argument the old refusal had, and leniency only answers it
/// if the message closes the gap between the typo and the zero. A user who sees a
/// zero gain and reads "is not one of cost, unrealized" cannot connect the two.
/// This test exists so nobody edits that sentence back to a bare code list.
#[test]
fn the_valuation_message_says_the_gain_will_read_zero() {
    let message = only_message(&one_bad("valuation", "unrealised-gain"));
    for expected in ["unrealized gain", "zero", "cost basis"] {
        assert!(
            message.contains(expected),
            "the valuation warning must say {expected:?} in words — {message}"
        );
    }
    // And the four that do NOT have this consequence must not claim it, or the
    // phrase stops meaning anything.
    for tag in ["type", "issection", "bsterm", "holdings"] {
        let other = only_message(&one_bad(tag, "nonsense-value"));
        assert!(
            !other.contains("zero"),
            "{tag} does not zero a number and must not say it does — {other}"
        );
    }
}

/// Every finding is a WARNING under the one rule id.
///
/// Warning, not error: the journal is internally consistent and hledger itself
/// accepts it. We ignored a directive we could not read, and the report still
/// renders — that is not the same event as a transaction that does not balance.
#[test]
fn every_finding_is_a_warning_under_one_rule() {
    for tag in ["type", "issection", "bsterm", "holdings", "valuation"] {
        let found = tag_diagnostics(&journal(&one_bad(tag, "nonsense-value")));
        assert_eq!(found.len(), 1, "{tag}");
        assert_eq!(found[0]["rule"], "account-tag", "{tag}");
        assert_eq!(found[0]["severity"], "warning", "{tag}");
    }
}

// ---------------------------------------------------------------------------
// Leniency: the tag reads as absent, so the report still computes
// ---------------------------------------------------------------------------

/// Each reader treats an unreadable value exactly as it treats a missing one.
///
/// This is the half that keeps the tab up. Every one of the five degrades to a
/// documented fallback — `type:` to name inference, `issection:` to the default
/// section, `bsterm:` to the group default, `holdings:` to the mechanical rule,
/// `valuation:` to the cost side — so there is always a correct-by-fallback
/// number to show alongside the warning.
#[test]
fn an_unreadable_value_reads_as_no_declaration() {
    let bad = journal(
        "account expenses:rent    ; type: expenditure, issection: cost-of-goods-sold\n\
         account assets:house     ; bsterm: long-ish, holdings: real-estate, valuation: unrealised-gain\n",
    );

    assert!(account_sections(&bad).is_empty(), "issection");
    assert!(bs_terms(&bad).is_empty(), "bsterm");
    assert!(
        declared_holdings_classes(&bad.accounts).is_empty(),
        "holdings"
    );
    assert!(
        declared_valuation_roles(&bad.accounts).is_empty(),
        "valuation"
    );
    assert!(
        declared_types(&account_decls(&bad)).is_empty(),
        "type: an unreadable code declares nothing"
    );

    // ...and `type:` specifically falls through to NAME inference rather than
    // to nothing at all, which is the fallback that makes it survivable.
    assert_eq!(
        resolve_account_type("expenses:rent", &declared_types(&account_decls(&bad))),
        Some(AccountType::Expense),
        "an unreadable `type:` leaves hledger's name inference to answer"
    );
}

/// The GOOD values in the same journal are still read. A typo on one account
/// must not cost the accounts around it their declarations — that would be the
/// original whole-tab failure in miniature.
#[test]
fn a_bad_tag_does_not_disturb_its_neighbours() {
    let text = "account revenue          ; issection: revenue\n\
                account cogs             ; issection: cost-of-goods-sold\n\
                account expenses:rent    ; issection: opex\n";
    let mixed = journal(text);

    assert_eq!(
        account_sections(&mixed),
        [
            ("expenses:rent".to_string(), IsSectionKind::Opex),
            ("revenue".to_string(), IsSectionKind::Revenue),
        ]
        .into_iter()
        .collect(),
        "only the misspelt account loses its section"
    );
    assert_eq!(messages(text).len(), 1, "and only it is reported");
}

/// The documented SYNONYMS are not findings. `bsterm:` names two codes but
/// accepts seven spellings, and `holdings:`/`valuation:` do the same on a
/// smaller scale — a warning built from the message's word list instead of the
/// parser would fire on every journal that used a documented synonym.
#[test]
fn documented_synonyms_are_not_findings() {
    for (tag, value) in [
        ("bsterm", "long-term"),
        ("bsterm", "shortterm"),
        ("bsterm", "NON-CURRENT"),
        ("holdings", "stock"),
        ("valuation", "basis"),
        ("valuation", "unrealised"),
        ("valuation", "mark"),
        ("type", "expenses"),
        ("type", "income"),
        ("type", "C"),
        ("issection", "  OPEX  "),
    ] {
        assert!(
            messages(&one_bad(tag, value)).is_empty(),
            "`{tag}: {value}` is accepted by the parser and must not warn"
        );
    }
}

// ---------------------------------------------------------------------------
// An empty value is not a declaration, and not a finding
// ---------------------------------------------------------------------------

/// `; issection:` with nothing after it names no section — the reading every one
/// of the five readers already gave it. Warning about a value the user never
/// wrote would be noise, and it would fire on a shape journals use deliberately.
#[test]
fn an_empty_value_is_not_a_finding() {
    for tag in ["type", "issection", "bsterm", "holdings", "valuation"] {
        assert!(
            messages(&one_bad(tag, "")).is_empty(),
            "`{tag}:` with an empty value"
        );
        assert!(
            messages(&one_bad(tag, "   ")).is_empty(),
            "`{tag}:` with a whitespace-only value"
        );
    }
}

// ---------------------------------------------------------------------------
// Collect and continue
// ---------------------------------------------------------------------------

/// EVERY bad tag is reported, not just the first — `check_balance_assertions`'
/// argument, which applies verbatim: a tool that reports the first break and
/// hides the rest is a worse tool. Under the old design this journal produced
/// ONE message and then a blank tab.
///
/// Order is declaration order, then tag order (`type`, `issection`, `bsterm`,
/// `holdings`, `valuation` — broadest classification first). It is a function of
/// the journal alone, so the golden fixtures and this assertion are stable.
#[test]
fn every_bad_tag_is_reported_in_a_deterministic_order() {
    let text = "\
account zzz:last       ; issection: nope-one
account aaa:first      ; valuation: nope-two, type: nope-three, bsterm: nope-four
account mmm:middle     ; holdings: nope-five
";
    let reported: Vec<(String, String)> = tag_diagnostics(&journal(text))
        .iter()
        .map(|d| {
            (
                d["account"].as_str().expect("an account").to_string(),
                d["message"].as_str().expect("a message").to_string(),
            )
        })
        .map(|(account, message)| {
            // Reduce the message to the tag it is about, so the assertion reads
            // as the ORDER contract rather than as five re-quoted sentences.
            let tag = message
                .split_once('`')
                .and_then(|(_, rest)| rest.split_once(':'))
                .map(|(tag, _)| tag.to_string())
                .expect("the message quotes the directive");
            (account, tag)
        })
        .collect();

    assert_eq!(
        reported,
        vec![
            // Declaration order first: `zzz:last` is declared first.
            ("zzz:last".to_string(), "issection".to_string()),
            // Then tag order WITHIN one account, regardless of the order the
            // tags were written in on the line.
            ("aaa:first".to_string(), "type".to_string()),
            ("aaa:first".to_string(), "bsterm".to_string()),
            ("aaa:first".to_string(), "valuation".to_string()),
            ("mmm:middle".to_string(), "holdings".to_string()),
        ]
    );
}

// ---------------------------------------------------------------------------
// The wire shape
// ---------------------------------------------------------------------------

/// An `account`-anchored finding carries `account` and NO `txnIndex`.
///
/// Both anchors are `skip_serializing_if`, so this is the first diagnostic whose
/// JSON does not have a `txnIndex` key at all. The SPA's `toDiagnostic` drops an
/// entry it cannot anchor, so "absent" rather than "null" is the contract both
/// sides are written against.
#[test]
fn an_account_finding_carries_an_account_and_no_txn_index() {
    let bad = journal(&one_bad("holdings", "real-estate"));
    let found = tag_diagnostics(&bad);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["account"], "assets:house");
    assert!(
        !found[0]
            .as_object()
            .expect("an object")
            .contains_key("txnIndex"),
        "an account finding must OMIT txnIndex, not null it: {:?}",
        found[0]
    );

    // The literal bytes, because "the SPA sees no txnIndex key" is a claim about
    // the JSON and not about a `Value` (whose map sorts its keys).
    assert_eq!(
        serde_json::to_string(&wire::journal_to_tag_diagnostics(&bad)).expect("serializes"),
        r#"[{"account":"assets:house","rule":"account-tag","severity":"warning","message":"account 'assets:house' declares `holdings: real-estate`, which is not one of stocks, other, none; the tag is ignored and the account is classified mechanically (does it hold a non-currency commodity?)"}]"#
    );
}

/// The transaction-anchored rules are UNMOVED by the wider struct: `txnIndex`
/// present, `account` omitted, same four keys as before. This is the half the
/// golden fixtures depend on.
#[test]
fn transaction_findings_keep_their_original_four_keys() {
    let unbalanced =
        journal("2026-01-01 broken\n    assets:bank   $10.00\n    expenses:food  $5.00\n");
    let json =
        serde_json::to_string(&wire::journal_to_diagnostics(&unbalanced)).expect("serializes");

    // The key ORDER and the absence of `account` are both part of the contract
    // the golden fixtures pin, so this compares bytes rather than a `Value`.
    assert!(
        json.starts_with(r#"[{"txnIndex":0,"rule":"unbalanced","severity":"error","message":"#),
        "unchanged from before `account` existed: {json}"
    );
    assert!(
        !json.contains("account"),
        "a transaction finding must not carry an `account` key: {json}"
    );
}

/// `account-tag` is in the wire's rule vocabulary. The SPA drops an
/// unrecognized rule SILENTLY, so a rule missing from this list is a finding
/// that vanishes with no error on either side.
#[test]
fn account_tag_is_in_the_rule_vocabulary() {
    assert!(wire::DIAGNOSTIC_RULES.contains(&"account-tag"));
}

// ---------------------------------------------------------------------------
// The regression this was really about
// ---------------------------------------------------------------------------

/// **The whole point.** A journal with a bad `holdings:` gets its tag warning
/// AND its `stock-*` findings.
///
/// This is the failure that made the old design indefensible. A bad `holdings:`
/// made `compute_holdings` return `Err`, and `journal_to_stock_diagnostics`
/// answers an `Err` with an empty vector — so the one journal most in need of an
/// explanation got both Holdings tabs blank, Insights blank, AND a Problems
/// drawer with nothing in it. One typo silently deleted three unrelated
/// findings.
#[test]
fn a_bad_holdings_tag_no_longer_swallows_the_stock_findings() {
    let text = "\
account assets:broker    ; type: A, holdings: real-estate
account equity:opening   ; type: E

2026-01-01 buy with no cost basis
    assets:broker    10 VTI
    equity:opening
";
    let bad = journal(text);

    // The tag finding is there...
    let tags = tag_diagnostics(&bad);
    assert_eq!(tags.len(), 1, "{tags:#?}");
    assert_eq!(tags[0]["account"], "assets:broker");

    // ...and so are the stock findings, which the refusal used to swallow.
    let stock: Vec<String> = serde_json::to_value(wire::journal_to_stock_diagnostics(&bad))
        .expect("serializes")
        .as_array()
        .expect("an array")
        .iter()
        .map(|d| d["rule"].as_str().expect("a rule").to_string())
        .collect();
    assert!(
        stock.contains(&"stock-missing-basis".to_string()),
        "the cost-less lot must still be reported: {stock:?}"
    );

    // The combined payload carries both, tags before stock.
    let all: Vec<String> = serde_json::to_value(wire::journal_to_all_diagnostics(&bad))
        .expect("serializes")
        .as_array()
        .expect("an array")
        .iter()
        .map(|d| d["rule"].as_str().expect("a rule").to_string())
        .collect();
    assert_eq!(all.first().map(String::as_str), Some("account-tag"));
    assert!(all.iter().any(|rule| rule.starts_with("stock-")), "{all:?}");
}

/// A clean journal reports nothing. The fixtures the endpoint and e2e suites
/// count problems against declare only valid codes, so this rule must not move
/// any existing badge count.
#[test]
fn the_fixtures_declare_no_unreadable_tags() {
    let sample = common::fixture_journal();
    assert!(
        wire::journal_to_tag_diagnostics(&sample).is_empty(),
        "sample.journal must stay clean"
    );
}

/// Unrelated tags are none of this rule's business. `bsgroup:` and `isgroup:`
/// are deliberately FREE TEXT — they name a line inside a box the account is
/// already in, so there is no table for them to fail to match — and a rule that
/// warned about them would be warning about correct journals.
#[test]
fn free_text_tags_are_never_findings() {
    assert!(
        messages(
            "account expenses:rent  ; bsgroup: Whatever You Like, isgroup: Anything At All, note: hello\n"
        )
        .is_empty()
    );
}

/// Only the FIRST occurrence of a repeated tag is judged, because that is the
/// one every reader takes — so a finding always describes the value that
/// actually decided something.
#[test]
fn a_repeated_tag_is_judged_where_the_readers_read_it() {
    let text = "account assets:house  ; holdings: real-estate, holdings: stocks\n";
    let found = messages(text);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].contains("real-estate"), "{}", found[0]);

    // And the reader agrees: the first value won, so nothing was declared.
    assert!(declared_holdings_classes(&journal(text).accounts).is_empty());

    // The mirror image: a readable first value is not a finding, even though a
    // later one is junk.
    let reversed = "account assets:house  ; holdings: stocks, holdings: real-estate\n";
    assert!(messages(reversed).is_empty());
    assert_eq!(
        declared_holdings_classes(&journal(reversed).accounts)
            .get("assets:house")
            .copied(),
        Some(HoldingsClass::Stocks)
    );
}

/// The `valuation:` fallback, spelled out: an unreadable role leaves the account
/// on `Cost`. This is the sharpest of the five consequences — it folds a
/// holding's unrealized gain into its own basis and reports the gain as zero —
/// which is exactly why the warning has to be loud, and exactly why it must not
/// be a blank tab instead.
#[test]
fn an_unreadable_valuation_role_falls_back_to_cost() {
    let bad = journal("account assets:home:unrealized  ; type: A, valuation: unrealised-gain\n");
    let roles = declared_valuation_roles(&bad.accounts);
    assert!(roles.is_empty());
    assert_eq!(
        ledgeline_core::holdings::resolve_valuation_role("assets:home:unrealized", &roles),
        ValuationRole::Cost
    );
    assert_eq!(tag_diagnostics(&bad).len(), 1);
}
