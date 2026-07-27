//! Parser correctness fixes: calendar dates (PARSE-6), indented comment lines
//! (PARSE-7), and the PARSE-9 batch — BOM, unterminated `comment` blocks,
//! assertion-only postings, bracket posting dates, and zero-balance commodities
//! in an inferred amount.
//!
//! Every expectation was pinned against real `hledger 1.52` (mac-aarch64) with
//! `hledger -f FILE print -O json`, and each test names that ground truth.
//!
//! | journal                              | hledger 1.52 | before | now |
//! |--------------------------------------|--------------|--------|-----|
//! | `2024-02-30` / `2023-02-29` / `2024-04-31` | rejected | accepted | rejected |
//! | `    ; subscription: false`          | attached to the posting | dropped | attached |
//! | leading U+FEFF                       | parses | whole file fails | parses |
//! | `comment` with no `end comment`      | swallows the rest | swallows the rest | unchanged |
//! | `    a       = $150.00`              | accepted | `malformed amount: ''` | accepted |
//! | `= 10 AAA @ $5.00`                   | accepted | `malformed amount` | accepted |
//! | `; [2024-01-05=2024-01-06]`          | pdate + pdate2 | ignored | honoured |
//! | inferred `[$0.00, -3 AAPL]`          | both | `[-3 AAPL]` | both |

mod common;

use ledgeline_core::model::{Commodity, PostingType, Tindex};
use ledgeline_core::parse::parse_journal;
use ledgeline_core::reports::{
    Cadence, Subscription, SubscriptionOpts, SubscriptionsReport, detect_subscriptions,
};
use ledgeline_core::{Dec, Journal, JournalEditor};

fn journal(text: &str) -> Journal {
    parse_journal(text, "/tmp/parse_fixes.journal").expect("journal parses")
}

fn error(text: &str) -> String {
    parse_journal(text, "/tmp/parse_fixes.journal")
        .expect_err("journal is rejected")
        .to_string()
}

// ---------------------------------------------------------------------------
// PARSE-6 — calendar-invalid dates
// ---------------------------------------------------------------------------

#[test]
fn a_day_that_does_not_exist_in_its_month_is_rejected() {
    // hledger 1.52 rejects all three with "This is not a valid date, please fix
    // it." Unlike an unbalanced transaction there is no sensible value to carry
    // forward: the bogus ISO string silently rolls forward in bucket maths
    // (`2026-02-31` becomes March 3), corrupting every date-filtered report.
    for date in ["2024-02-30", "2023-02-29", "2024-04-31", "2024-06-31"] {
        let message = error(&format!("{date} t\n    a   $1.00\n    b\n"));
        assert!(
            message.contains("malformed date") && message.contains(date),
            "{date}: {message}"
        );
    }
}

#[test]
fn real_calendar_days_including_leap_days_still_parse() {
    // 2024 is a leap year, 2000 is (÷400), 1900 is not (÷100 but not ÷400).
    for date in ["2024-02-29", "2000-02-29", "2024-01-31", "2024-12-31"] {
        let parsed = journal(&format!("{date} t\n    a   $1.00\n    b\n"));
        assert_eq!(parsed.transactions[0].date, date);
    }
    // …and the century rule bites the other way.
    assert!(error("1900-02-29 t\n    a   $1.00\n    b\n").contains("malformed date"));
}

#[test]
fn the_calendar_check_covers_every_date_the_parser_reads() {
    // `normalize_date` is the single gate for transaction headers, posting
    // `date:` tags and `P` directives — hledger rejects an invalid date in all
    // three, and so must we.
    assert!(
        error("2024-01-01 t\n    a   $1.00  ; date: 2024-02-30\n    b\n").contains("2024-02-30")
    );
    assert!(error("P 2024-02-30 AAPL $1.00\n").contains("2024-02-30"));
    assert!(error("2024-01-01=2024-02-30 t\n    a   $1.00\n    b\n").contains("2024-02-30"));
}

// ---------------------------------------------------------------------------
// PARSE-7 — indented comment lines
// ---------------------------------------------------------------------------

#[test]
fn a_comment_line_attaches_to_the_preceding_posting() {
    // hledger 1.52: tpostings[0].pcomment = "\nsubscription: false\n",
    //               ptags = [["subscription","false"]]
    // Both were empty before: the line was `continue`d past outright.
    let parsed = journal("2024-01-01 t\n    a   $1.00\n    ; subscription: false\n    b\n");
    let postings = &parsed.transactions[0].postings;
    assert_eq!(postings[0].comment, "\nsubscription: false\n");
    assert_eq!(
        postings[0].tags,
        vec![("subscription".to_string(), "false".to_string())]
    );
    // …and not to the posting that follows it.
    assert_eq!(postings[1].comment, "");
    assert!(postings[1].tags.is_empty());
}

#[test]
fn a_comment_line_before_any_posting_attaches_to_the_transaction() {
    // hledger 1.52: tcomment = "\nhdr: one\n", ttags = [["hdr","one"]]
    let parsed = journal("2024-01-01 t\n    ; hdr: one\n    a   $1.00\n    b\n");
    let transaction = &parsed.transactions[0];
    assert_eq!(transaction.comment, "\nhdr: one\n");
    assert_eq!(
        transaction.tags,
        vec![("hdr".to_string(), "one".to_string())]
    );
}

#[test]
fn a_comment_line_extends_an_existing_same_line_comment() {
    // hledger 1.52 models a comment as its lines joined by "\n" with a trailing
    // "\n", the first line being the same-line comment. So an inline comment is
    // simply extended (no leading newline), while a lone continuation line
    // produces one — both spellings verified against `print -O json`.
    let parsed = journal(concat!(
        "2024-01-01 t  ; first: 1\n",
        "    ; second: 2\n",
        "    a   $1.00 ; own: x\n",
        "    ; more: y\n",
        "    b\n",
    ));
    let transaction = &parsed.transactions[0];
    assert_eq!(transaction.comment, "first: 1\nsecond: 2\n");
    assert_eq!(
        transaction.tags,
        vec![
            ("first".to_string(), "1".to_string()),
            ("second".to_string(), "2".to_string())
        ]
    );
    assert_eq!(transaction.postings[0].comment, "own: x\nmore: y\n");
    assert_eq!(
        transaction.postings[0].tags,
        vec![
            ("own".to_string(), "x".to_string()),
            ("more".to_string(), "y".to_string())
        ]
    );
}

#[test]
fn a_bare_semicolon_line_still_contributes_a_line() {
    // hledger 1.52: pcomment = "\n\n" for a lone `;` continuation line.
    let parsed = journal("2024-01-01 t\n    a   $1.00\n    ;\n    b\n");
    assert_eq!(parsed.transactions[0].postings[0].comment, "\n\n");
}

#[test]
fn a_trailing_comment_line_extends_the_transactions_source_span() {
    // hledger 1.52 ends the span on the line AFTER the last line the
    // transaction consumed; a trailing comment line is one of them. The editor
    // addresses transactions by this span, so it must cover everything the
    // parser consumed or an edit deletes the wrong lines.
    let parsed = journal(concat!(
        "2024-01-01 t\n    a   $1.00\n    b\n    ; trailing\n",
        "\n",
        "2024-01-02 u\n    c   $1.00\n    d\n",
    ));
    assert_eq!(parsed.transactions[0].source_span.0.line, 1);
    assert_eq!(parsed.transactions[0].source_span.1.line, 5);
    assert_eq!(parsed.transactions[1].source_span.0.line, 6);
}

#[test]
fn a_periodic_rules_comment_line_attaches_to_its_posting_too() {
    let parsed = journal("~ monthly  budget\n    (a)  $1\n    ; note: here\n");
    let posting = &parsed.periodic_transactions[0].postings[0];
    assert_eq!(posting.comment, "\nnote: here\n");
    assert_eq!(posting.tags, vec![("note".to_string(), "here".to_string())]);
}

#[test]
fn the_editor_still_addresses_transactions_correctly_around_comment_lines() {
    // PARSE-7 made a transaction's `source_span` cover its trailing comment
    // lines, and the editor addresses transactions BY that span. A deletion must
    // therefore still take exactly its own lines and leave every other byte
    // alone — the journal is the user's irreplaceable primary record.
    let text = concat!(
        "2024-01-01 t  ; first: 1\n",
        "    ; second: 2\n",
        "    a   $1.00 ; own: x\n",
        "    ; more: y\n",
        "    b\n",
        "\n",
        "2024-01-02 u\n    c   $2.00\n    d\n",
    );

    let mut second =
        JournalEditor::from_text("/tmp/parse_fixes_edit.journal", text).expect("the journal opens");
    second
        .delete_transaction(Tindex(2))
        .expect("the second transaction deletes");
    assert_eq!(
        second.text(),
        concat!(
            "2024-01-01 t  ; first: 1\n",
            "    ; second: 2\n",
            "    a   $1.00 ; own: x\n",
            "    ; more: y\n",
            "    b\n",
        ),
        "deleting the LAST transaction must leave the first byte-identical, \
         comment lines included"
    );

    let mut first =
        JournalEditor::from_text("/tmp/parse_fixes_edit.journal", text).expect("the journal opens");
    first
        .delete_transaction(Tindex(1))
        .expect("the first transaction deletes");
    assert_eq!(
        first.text(),
        "2024-01-02 u\n    c   $2.00\n    d\n",
        "deleting the FIRST transaction must take its comment lines with it and \
         orphan none of them onto the next"
    );
}

#[test]
fn the_subscription_override_works_from_a_continuation_line() {
    // THE user-visible payoff of PARSE-7. `reports/subscriptions.rs` reads the
    // `subscription` tag off the transaction OR any of its postings; written on
    // its own line — a very natural place for it — the tag used to be dropped
    // before the detector ever saw it, so a `subscription:false` silently did
    // nothing.
    fn monthly_charges(tag_line: &str) -> String {
        // Twelve identical monthly charges: exactly what detection looks for.
        (1..=12)
            .map(|month| {
                format!(
                    "2026-{month:02}-15 Streamly | plan\n    expenses:subscriptions   $9.99\n\
                     {tag_line}    assets:bank\n\n"
                )
            })
            .collect()
    }
    fn report(text: &str) -> SubscriptionsReport {
        detect_subscriptions(
            &journal(text),
            &SubscriptionOpts {
                as_of: "2026-12-31",
                min_monthly: 5,
                ..SubscriptionOpts::default()
            },
        )
        .expect("detection succeeds")
    }
    fn find<'a>(rows: &'a [Subscription], payee: &str) -> Option<&'a Subscription> {
        rows.iter().find(|row| row.payee == payee)
    }

    // Untagged, it is detected — so the suppression below is doing real work.
    let detected = report(&monthly_charges(""));
    let streamly = find(&detected.monthly, "Streamly").expect("plainly detectable");
    assert_eq!(streamly.cadence, Cadence::Monthly);
    assert_eq!(streamly.typical_amount, Dec::new(999, 2));

    // `subscription: false` on its own line now takes it off the list.
    let suppressed = report(&monthly_charges("    ; subscription: false\n"));
    assert!(
        find(&suppressed.monthly, "Streamly").is_none(),
        "a continuation-line subscription:false must suppress the charge"
    );
    assert!(find(&suppressed.annual, "Streamly").is_none());

    // …and `subscription: true` on its own line forces a charge on, the same way.
    let forced = report(concat!(
        "2026-01-01 Wildly | varying\n",
        "    expenses:misc   $18.00\n",
        "    ; subscription: true\n",
        "    assets:bank\n",
    ));
    let wildly = find(&forced.monthly, "Wildly").expect("tagged onto the list");
    assert!(wildly.manual, "flagged as hand-added, not inferred");
}

// ---------------------------------------------------------------------------
// PARSE-9 — BOM
// ---------------------------------------------------------------------------

#[test]
fn a_leading_byte_order_mark_no_longer_fails_the_whole_file() {
    // U+FEFF is not `char::is_whitespace`, so it survived `trim_start` and the
    // first-char dispatch fell through to `other =>`, failing the ENTIRE file
    // with `unsupported directive: '\u{feff}2024-01-01'`. hledger parses it, and
    // Windows/Excel-exported journals routinely carry one.
    let parsed = journal("\u{feff}2024-01-01 t\n    expenses:x   $1.00\n    assets:bank\n");
    assert_eq!(parsed.transactions.len(), 1);
    assert_eq!(parsed.transactions[0].date, "2024-01-01");
    // A BOM in front of a directive works too.
    assert_eq!(journal("\u{feff}account assets:bank\n").accounts.len(), 1);
}

#[test]
fn a_byte_order_mark_is_stripped_from_an_included_file_as_well() {
    let dir = std::env::temp_dir().join("ledgeline_parse_fixes_bom");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let sub = dir.join("sub.journal");
    let main = dir.join("main.journal");
    std::fs::write(
        &sub,
        "\u{feff}2024-02-01 sub\n    expenses:foo   $5.00\n    assets:bank\n",
    )
    .expect("write sub");
    std::fs::write(&main, "include sub.journal\n").expect("write main");

    let text = std::fs::read_to_string(&main).expect("read main");
    let parsed = parse_journal(&text, &main.to_string_lossy()).expect("journal parses");
    assert_eq!(parsed.transactions.len(), 1);
}

// ---------------------------------------------------------------------------
// PARSE-9 — unterminated `comment` block
// ---------------------------------------------------------------------------

#[test]
fn an_unterminated_comment_block_swallows_the_rest_of_the_file_exactly_as_hledger_does() {
    // DELIBERATE PARITY, and the one item here left as-is. hledger 1.52 accepts
    // a `comment` block with no `end comment` and silently drops everything
    // after it — verified: the journal below prints ONLY the first transaction,
    // exit code 0. Erroring instead would make a journal hledger loads refuse to
    // open, and the frozen diagnostics contract has only `unbalanced` and
    // `assertion` rules, so there is no channel to warn on either. Pinned here
    // so the behaviour is at least deliberate and tested rather than accidental
    // and undiscovered.
    let parsed = journal(concat!(
        "2024-01-01 kept\n    a   $1.00\n    b\n",
        "\n",
        "comment\n",
        "some text\n",
        "\n",
        "2024-02-01 swallowed\n    c   $2.00\n    d\n",
    ));
    assert_eq!(parsed.transactions.len(), 1);
    assert_eq!(parsed.transactions[0].description, "kept");

    // A terminated block behaves the ordinary way.
    let terminated = journal(concat!(
        "comment\n",
        "some text\n",
        "end comment\n",
        "\n",
        "2024-02-01 kept\n    c   $2.00\n    d\n",
    ));
    assert_eq!(terminated.transactions.len(), 1);
}

// ---------------------------------------------------------------------------
// PARSE-9 — assertion-only postings and costs on an asserted amount
// ---------------------------------------------------------------------------

#[test]
fn a_posting_with_only_an_assertion_is_an_elided_posting() {
    // `    a       = $150.00` is the reconcile-to-statement idiom. It used to be
    // `PARSE ERROR: malformed amount: ''`, which made a whole real journal
    // unreadable. hledger accepts it; the balancing pass infers the amount.
    let parsed = journal(concat!(
        "2024-01-01 open\n    a   $150.00\n    b\n",
        "\n",
        "2024-01-02 reconcile\n    a       = $160.00\n    b   $-10.00\n",
    ));
    let posting = &parsed.transactions[1].postings[0];
    assert_eq!(posting.amounts.len(), 1);
    assert_eq!(posting.amounts[0].quantity, Dec::new(1000, 2));
    let assertion = posting
        .balance_assertion
        .as_ref()
        .expect("the assertion survives");
    assert_eq!(assertion.amount.quantity, Dec::new(16000, 2));
    assert!(!assertion.total);
    assert!(!assertion.inclusive);
    // hledger agrees on the inferred $10.00 and the assertion then holds.
    assert!(
        ledgeline_core::assertions::check_balance_assertions(&parsed)
            .expect("no overflow")
            .is_empty()
    );
}

#[test]
fn an_asserted_amount_may_carry_a_cost() {
    // `= 10 AAA @ $5.00`: the assertion text used to be handed to the
    // commodity/number splitter, which rejected the whole journal. hledger
    // records the cost on `baamount.acost` and IGNORES it when evaluating, both
    // of which are reproduced here.
    let parsed = journal("2024-01-01 t\n    a   10 AAA = 10 AAA @ $5.00\n    b   $-50.00\n");
    let assertion = parsed.transactions[0].postings[0]
        .balance_assertion
        .as_ref()
        .expect("assertion parsed");
    assert_eq!(assertion.amount.commodity, Commodity("AAA".to_string()));
    assert_eq!(assertion.amount.quantity, Dec::new(10, 0));
    let cost = assertion.amount.cost.as_ref().expect("cost recorded");
    assert_eq!(cost.amount.quantity, Dec::new(500, 2));
    // The cost plays no part in the evaluation: the account holds 10 AAA.
    assert!(
        ledgeline_core::assertions::check_balance_assertions(&parsed)
            .expect("no overflow")
            .is_empty()
    );
}

// ---------------------------------------------------------------------------
// PARSE-9 — bracket posting dates
// ---------------------------------------------------------------------------

#[test]
fn bracket_posting_dates_are_honoured_in_all_three_spellings() {
    // hledger 1.52, verified with `print -O json`. Only `date:`/`date2:` tags
    // were recognised before, so a posting written this way stayed bucketed
    // under the transaction date in every periodic report.
    for (comment, date, date2) in [
        ("[2024-01-05]", Some("2024-01-05"), None),
        ("[=2024-01-06]", None, Some("2024-01-06")),
        (
            "[2024-01-05=2024-01-06]",
            Some("2024-01-05"),
            Some("2024-01-06"),
        ),
    ] {
        let parsed = journal(&format!(
            "2024-01-01 t\n    a   $1.00  ; {comment}\n    b\n"
        ));
        let posting = &parsed.transactions[0].postings[0];
        assert_eq!(posting.date.as_deref(), date, "{comment}");
        assert_eq!(posting.date2.as_deref(), date2, "{comment}");
        // The brackets stay in the comment text and produce no tag, as hledger
        // leaves them.
        assert_eq!(posting.comment, format!("{comment}\n"));
        assert!(posting.tags.is_empty(), "{comment}");
    }
}

#[test]
fn a_bracket_date_is_found_mid_comment_and_on_a_continuation_line() {
    // hledger finds the group anywhere in the comment, including one written on
    // a continuation line.
    let inline = journal("2024-01-01 t\n    a   $1.00  ; note [2024-01-05] more\n    b\n");
    assert_eq!(
        inline.transactions[0].postings[0].date.as_deref(),
        Some("2024-01-05")
    );
    let continued = journal("2024-01-01 t\n    a   $1.00\n    ; [2024-01-07]\n    b\n");
    assert_eq!(
        continued.transactions[0].postings[0].date.as_deref(),
        Some("2024-01-07")
    );
    // A `date:` tag on a continuation line works too — it could not before,
    // because dates were read before the line was merged.
    let tagged = journal("2024-01-01 t\n    a   $1.00\n    ; date: 2024-01-05\n    b\n");
    assert_eq!(
        tagged.transactions[0].postings[0].date.as_deref(),
        Some("2024-01-05")
    );
}

#[test]
fn prose_in_brackets_is_not_a_date() {
    // hledger accepts `; see [note]` and sets no posting date. A bracket group
    // that IS all date characters must still parse, so an invalid one is the
    // same hard error hledger raises.
    let parsed = journal("2024-01-01 t\n    a   $1.00  ; see [note] here\n    b\n");
    assert!(parsed.transactions[0].postings[0].date.is_none());
    assert!(error("2024-01-01 t\n    a   $1.00  ; [2024-02-30]\n    b\n").contains("2024-02-30"));
}

// ---------------------------------------------------------------------------
// PARSE-9 — zero-balance commodities in an inferred amount
// ---------------------------------------------------------------------------

#[test]
fn an_inferred_posting_keeps_a_commodity_that_nets_to_zero() {
    // hledger 1.52 emits `[$0.00, -3 AAPL]` where the zero used to be filtered
    // out, leaving `[-3 AAPL]`. Wire parity only — the balances were never
    // affected — but the wire is a contract.
    let parsed = journal(concat!(
        "2024-01-01 t\n",
        "    a   $1.00\n",
        "    a   3 AAPL\n",
        "    b   $-1.00\n",
        "    c\n",
    ));
    let inferred = &parsed.transactions[0].postings[3].amounts;
    assert_eq!(inferred.len(), 2);
    assert_eq!(inferred[0].commodity, Commodity("$".to_string()));
    assert_eq!(inferred[0].quantity, Dec::new(0, 2));
    assert_eq!(inferred[1].commodity, Commodity("AAPL".to_string()));
    assert_eq!(inferred[1].quantity, Dec::new(-3, 0));

    // …and where the ONLY commodity nets to zero, hledger still emits `[$0.00]`
    // rather than an empty amount list.
    let sole = journal("2024-01-01 t\n    a   $1.00\n    b   $-1.00\n    c\n");
    let amounts = &sole.transactions[0].postings[2].amounts;
    assert_eq!(amounts.len(), 1);
    assert_eq!(amounts[0].quantity, Dec::new(0, 2));
    assert_eq!(sole.transactions[0].postings[2].ptype, PostingType::Regular);
}
