//! The transaction-review model (plan 32 Phase 1): review state, the
//! similarity key, the two queues, similar transactions and suggestions.
//!
//! Most cases read `fixtures/review/review.journal` as of 2026-10-30 (its
//! header lists what each transaction is for); the rest build a few lines of
//! journal inline so one source or rule is tested alone.

mod common;

use ledgeline_core::auto;
use ledgeline_core::model::{Journal, Tindex};
use ledgeline_core::parse_journal;
use ledgeline_core::review::{
    CERTAIN, MAX_SUGGESTIONS, QueueKind, QueueOpts, ReviewIndex, ReviewState, SIMILAR_CAP, STRONG,
    SuggestionSource, is_unknown, similarity_key, top_chip,
};
use ledgeline_core::view::Classes;
use ledgeline_core::view::checks::local_findings;
use proptest::prelude::*;
use std::path::Path;

const TODAY: &str = "2026-10-30";

fn fixture() -> Journal {
    let path = common::fixtures_dir().join("review/review.journal");
    let text = std::fs::read_to_string(&path).expect("review.journal readable");
    parse_journal(&text, &path.to_string_lossy()).expect("review.journal parses")
}

fn inline(text: &str) -> Journal {
    parse_journal(text, "/review-inline.journal").expect("inline journal parses")
}

fn indexed(journal: &Journal) -> (Classes, ReviewIndex) {
    let classes = Classes::of(journal);
    let index = ReviewIndex::build(journal, &classes, |_| true);
    (classes, index)
}

/// The `tindex` of the one transaction dated `date` with `needle` in its
/// description, so a test reads as the fixture does.
fn find(journal: &Journal, date: &str, needle: &str) -> Tindex {
    let mut hits = journal
        .transactions
        .iter()
        .filter(|txn| txn.date == date && txn.description.contains(needle));
    let hit = hits.next().unwrap_or_else(|| panic!("no {date} {needle}"));
    assert!(hits.next().is_none(), "{date} {needle} is ambiguous");
    hit.index
}

/// `(date, description)` of each queued item, for readable assertions.
fn described(journal: &Journal, items: &[Tindex]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|index| {
            let txn = &journal.transactions[index.0 as usize - 1];
            (txn.date.clone(), txn.description.clone())
        })
        .collect()
}

fn dates(journal: &Journal, items: &[Tindex]) -> Vec<String> {
    described(journal, items)
        .into_iter()
        .map(|(date, _)| date)
        .collect()
}

fn opts(kind: QueueKind) -> QueueOpts<'static> {
    QueueOpts {
        limit: usize::MAX,
        ..QueueOpts::new(kind, TODAY)
    }
}

// ---------------------------------------------------------------------------
// ReviewState (D1)
// ---------------------------------------------------------------------------

#[test]
fn review_state_reads_each_d1_row_from_the_header() {
    let journal = inline(
        "2026-10-01 none\n    expenses:a  $1\n    assets:b\n\n\
         2026-10-02 dated  ; reviewed:2026-10-30\n    expenses:a  $1\n    assets:b\n\n\
         2026-10-03 ignored  ; reviewed:ignore\n    expenses:a  $1\n    assets:b\n\n\
         2026-10-04 other  ; note, reviewed:yes\n    expenses:a  $1\n    assets:b\n\n\
         2026-10-05 continuation\n    ; reviewed:2026-01-02\n    expenses:a  $1\n    assets:b\n",
    );
    let states: Vec<ReviewState> = journal.transactions.iter().map(ReviewState::of).collect();
    assert_eq!(
        states,
        vec![
            ReviewState::Unreviewed,
            ReviewState::Reviewed("2026-10-30".to_string()),
            ReviewState::Ignored,
            ReviewState::ReviewedOther("yes".to_string()),
            ReviewState::Reviewed("2026-01-02".to_string()),
        ]
    );
    assert!(!states[0].is_tagged());
    assert!(states[2].is_tagged() && !states[2].is_reviewed());
    assert!(states[3].is_reviewed());
}

#[test]
fn a_reviewed_tag_on_a_posting_is_ignored() {
    let journal = inline(
        "2026-10-01 posting tagged\n    expenses:a  $1  ; reviewed:2026-10-30\n    assets:b\n\n\
         2026-10-02 posting continuation\n    expenses:a  $1\n    ; reviewed:ignore\n    assets:b\n",
    );
    for txn in &journal.transactions {
        assert_eq!(
            ReviewState::of(txn),
            ReviewState::Unreviewed,
            "{}",
            txn.description
        );
    }
}

// ---------------------------------------------------------------------------
// similarity_key (D10)
// ---------------------------------------------------------------------------

#[test]
fn similarity_key_table() {
    let cases: &[(&str, Option<&str>)] = &[
        // Square, store numbers on either side.
        (
            "SQ *BLUE BOTTLE 4412 OAKLAND",
            Some("sq blue bottle oakland"),
        ),
        (
            "SQ *BLUE BOTTLE 0917 OAKLAND",
            Some("sq blue bottle oakland"),
        ),
        // Toast, with an apostrophe.
        ("TST* MAMA'S KITCHEN 00123", Some("tst mama s kitchen")),
        // Card-terminal prefixes and embedded dates.
        ("POS0923 SAFEWAY #1234", Some("safeway")),
        (
            "PURCHASE 10/12 TRADER JOE'S X1234",
            Some("purchase trader joe s"),
        ),
        (
            "AMAZON.COM*2K4LP0QX3 AMZN.COM/BILL WA",
            Some("amzn com bill wa"),
        ),
        // Case and runs of whitespace are not distinctions.
        ("  Blue   Bottle\tCoffee ", Some("blue bottle coffee")),
        // The note after `|` is not part of the payee.
        ("Landlord | rent for 2026-10", Some("landlord")),
        ("Landlord|rent", Some("landlord")),
        // Nothing left: no key, matches nothing.
        ("12345", None),
        ("#4412 10/12", None),
        ("", None),
        ("| only a note", None),
        ("***", None),
    ];
    for (description, expected) in cases {
        assert_eq!(
            similarity_key(description).as_deref(),
            *expected,
            "{description:?}"
        );
    }
}

/// Descriptions shaped like bank exports: words, store numbers, punctuation.
fn description() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            "[A-Za-z]{1,8}",
            "[0-9]{1,5}",
            "#[0-9]{2,5}",
            "[A-Za-z]{1,3}[0-9]{1,4}",
            "[*'./&-]",
            "[A-Za-zé]{1,6}[.'*][A-Za-z]{1,6}",
        ],
        0..8,
    )
    .prop_map(|tokens| tokens.join(" "))
}

proptest! {
    #[test]
    fn similarity_key_is_idempotent(text in description()) {
        if let Some(key) = similarity_key(&text) {
            prop_assert_eq!(similarity_key(&key), Some(key));
        }
    }

    #[test]
    fn similarity_key_ignores_case(text in description()) {
        prop_assert_eq!(
            similarity_key(&text.to_uppercase()),
            similarity_key(&text.to_lowercase())
        );
    }

    #[test]
    fn similarity_key_ignores_which_digits(text in description(), shift in 1u8..10) {
        let renumbered: String = text
            .chars()
            .map(|c| match c.to_digit(10) {
                Some(d) => char::from(b'0' + ((d as u8 + shift) % 10)),
                None => c,
            })
            .collect();
        prop_assert_eq!(similarity_key(&renumbered), similarity_key(&text));
    }

    #[test]
    fn similarity_key_ignores_a_trailing_number(text in description(), number in "[0-9]{1,6}") {
        prop_assert_eq!(
            similarity_key(&format!("{text} {number}")),
            similarity_key(&text)
        );
    }
}

// ---------------------------------------------------------------------------
// Unknown (D3)
// ---------------------------------------------------------------------------

#[test]
fn a_generated_posting_to_an_unknown_account_makes_the_transaction_unknown() {
    // Amendment A: an `=` rule that books to an uncategorized account has
    // produced an uncategorised posting like any other — review and the
    // Problems drawer both count it.
    let journal = auto::apply_journal(&inline(
        "= expenses:food:coffee\n    (budget:unknown)  *1\n\n\
         2026-10-01 Blue Bottle\n    expenses:food:coffee  $5\n    assets:cash\n",
    ));
    let txn = &journal.transactions[0];
    assert!(
        txn.postings.iter().any(|p| p.generated.is_some()),
        "the rule fired"
    );
    let (classes, index) = indexed(&journal);
    assert!(is_unknown(txn, &classes));
    assert!(index.unknown_at(0));
    let unknowns = index.queue(&journal, &opts(QueueKind::Unknowns));
    assert_eq!(unknowns.total, 1);
    let drawer = local_findings(&journal, &classes, TODAY);
    assert!(
        drawer
            .iter()
            .any(|finding| finding.rule == "uncategorized" && finding.position == Some(0)),
        "the drawer flags the generated posting too"
    );
}

#[test]
fn the_fixtures_auto_rule_makes_a_reviewed_rename_unknown_again() {
    // `= desc:coffee$` in review.journal, applied as `--auto` would.
    let written = fixture();
    let journal = auto::apply_journal(&written);
    let rename = find(&journal, "2026-09-01", "Blue Bottle Coffee");
    let at = rename.0 as usize - 1;
    let (classes, index) = indexed(&journal);
    assert!(is_unknown(&journal.transactions[at], &classes));
    assert!(
        !is_unknown(&written.transactions[at], &classes),
        "only under --auto"
    );
    // Reviewed but unknown: in Unknowns (newest first), still not in Recents.
    let unknowns = index.queue(&journal, &opts(QueueKind::Unknowns));
    assert_eq!(unknowns.total, 9);
    assert!(unknowns.items.contains(&rename));
    let recents = index.queue(&journal, &opts(QueueKind::Recents));
    assert!(!recents.items.contains(&rename));
}

#[test]
fn forecast_transactions_are_never_unknown_here_or_in_the_drawer() {
    // What a `~` rule generates (hledger tags it `generated-transaction`),
    // with an `=` rule's posting on top: nobody's to review or fix.
    let journal = auto::apply_journal(&inline(
        "= expenses:unknown\n    (budget:unknown)  *1\n\n\
         2026-10-01 Rent  ; generated-transaction:~ monthly\n    expenses:unknown  $900\n    assets:cash\n",
    ));
    let txn = &journal.transactions[0];
    let (classes, index) = indexed(&journal);
    assert!(!is_unknown(txn, &classes));
    assert!(!index.unknown_at(0));
    assert_eq!(index.queue(&journal, &opts(QueueKind::Unknowns)).total, 0);
    assert_eq!(index.queue(&journal, &opts(QueueKind::Recents)).total, 0);
    let drawer = local_findings(&journal, &classes, TODAY);
    assert!(
        drawer.iter().all(|finding| finding.position != Some(0)),
        "no transaction rule looks at a forecast: {drawer:?}"
    );
}

#[test]
fn the_fixtures_periodic_rule_adds_nothing_to_review_or_the_drawer() {
    let journal = fixture();
    assert!(
        !journal.periodic_transactions.is_empty(),
        "the `~` rule parsed"
    );
    let (classes, index) = indexed(&journal);
    // The same counts the queue tests below pin, and no drawer finding names
    // anything the rule could have produced.
    assert_eq!(index.queue(&journal, &opts(QueueKind::Unknowns)).total, 8);
    let drawer = local_findings(&journal, &classes, TODAY);
    let uncategorized = drawer
        .iter()
        .filter(|finding| finding.rule == "uncategorized")
        .count();
    let unknown_txns = journal
        .transactions
        .iter()
        .filter(|txn| is_unknown(txn, &classes))
        .count();
    assert_eq!(uncategorized, unknown_txns);
}

// ---------------------------------------------------------------------------
// Queues (D4)
// ---------------------------------------------------------------------------

#[test]
fn unknowns_are_the_window_newest_first_without_ignore_or_future() {
    let journal = fixture();
    let (_, index) = indexed(&journal);
    let unknowns = index.queue(&journal, &opts(QueueKind::Unknowns));
    assert_eq!(
        dates(&journal, &unknowns.items),
        [
            "2026-10-20", // Venmo: reviewed:<date> but still unknown
            "2026-10-14",
            "2026-10-12",
            "2026-10-05",
            "2026-10-03",
            "2026-09-28",
            "2025-12-02", // from the include
            "2025-10-30", // exactly today − 365: inclusive
        ]
    );
    assert_eq!(unknowns.total, 8);
    assert_eq!(unknowns.not_editable, 0);
    // Not there: the ignored one, the 2026-11-15 one, and 2025-10-29.
    let all = described(&journal, &unknowns.items);
    assert!(all.iter().all(|(date, _)| date.as_str() <= TODAY));
    assert!(!all.iter().any(|(_, d)| d.contains("4400")), "ignored");
    assert!(!all.iter().any(|(_, d)| d.contains("Corner Store 78")));
}

#[test]
fn recents_are_every_untagged_transaction_in_the_window() {
    let journal = fixture();
    let (_, index) = indexed(&journal);
    let recents = index.queue(&journal, &opts(QueueKind::Recents));
    assert_eq!(
        dates(&journal, &recents.items),
        [
            "2026-10-25",
            "2026-10-14",
            "2026-10-12",
            "2026-10-05",
            "2026-10-03",
            "2026-09-28",
            "2026-09-14",
            "2026-08-14",
            "2026-07-15",
            "2026-07-14",
            "2026-06-20",
            "2026-06-14",
            "2026-05-14",
            "2026-04-14",
            "2025-12-20",
            "2025-12-02",
            "2025-10-30",
        ]
    );
    assert_eq!(recents.total, 17);
}

#[test]
fn reviewed_but_unknown_is_in_unknowns_but_not_recents() {
    let journal = fixture();
    let (_, index) = indexed(&journal);
    let venmo = find(&journal, "2026-10-20", "Venmo");
    assert!(
        index
            .queue(&journal, &opts(QueueKind::Unknowns))
            .items
            .contains(&venmo)
    );
    assert!(
        !index
            .queue(&journal, &opts(QueueKind::Recents))
            .items
            .contains(&venmo)
    );
}

#[test]
fn an_untagged_unknown_is_in_both_queues() {
    let journal = fixture();
    let (_, index) = indexed(&journal);
    let card = find(&journal, "2026-10-12", "4412");
    for kind in [QueueKind::Unknowns, QueueKind::Recents] {
        assert!(
            index.queue(&journal, &opts(kind)).items.contains(&card),
            "{kind:?}"
        );
    }
}

#[test]
fn any_reviewed_tag_keeps_a_transaction_out_of_recents() {
    let journal = fixture();
    let (_, index) = indexed(&journal);
    let recents = index.queue(&journal, &opts(QueueKind::Recents));
    for (date, needle) in [
        ("2026-10-18", "Safeway"),     // reviewed:yes
        ("2026-08-02", "1100"),        // reviewed:<date>
        ("2026-05-01", "4400"),        // reviewed:ignore
        ("2026-09-01", "Blue Bottle"), // reviewed, renamed
    ] {
        assert!(
            !recents.items.contains(&find(&journal, date, needle)),
            "{date} {needle}"
        );
    }
}

#[test]
fn the_window_boundary_is_inclusive_for_both_queues() {
    let journal = inline(
        "2025-10-29 outside\n    expenses:unknown  $1\n    assets:cash\n\n\
         2025-10-30 boundary\n    expenses:unknown  $1\n    assets:cash\n\n\
         2026-10-30 today\n    expenses:unknown  $1\n    assets:cash\n\n\
         2026-10-31 tomorrow\n    expenses:unknown  $1\n    assets:cash\n",
    );
    let (_, index) = indexed(&journal);
    for kind in [QueueKind::Unknowns, QueueKind::Recents] {
        let queued = index.queue(&journal, &opts(kind));
        assert_eq!(
            dates(&journal, &queued.items),
            ["2026-10-30", "2025-10-30"],
            "{kind:?}"
        );
        // One more day of window reaches the outside one.
        let wider = QueueOpts {
            max_days: 366,
            ..opts(kind)
        };
        assert_eq!(index.queue(&journal, &wider).total, 3, "{kind:?}");
        // Zero is no limit at all; the future stays out regardless.
        let unlimited = QueueOpts {
            max_days: 0,
            ..opts(kind)
        };
        assert_eq!(
            dates(&journal, &index.queue(&journal, &unlimited).items),
            ["2026-10-30", "2025-10-30", "2025-10-29"],
            "{kind:?}"
        );
    }
}

#[test]
fn max_days_zero_reaches_old_unknowns() {
    let journal = fixture();
    let (_, index) = indexed(&journal);
    let unlimited = QueueOpts {
        max_days: 0,
        ..opts(QueueKind::Unknowns)
    };
    let queued = index.queue(&journal, &unlimited);
    assert_eq!(queued.total, 10);
    assert_eq!(
        dates(&journal, &queued.items[8..]),
        ["2025-10-29", "2024-03-10"]
    );
}

#[test]
fn limit_and_offset_page_the_queue_but_not_its_total() {
    let journal = fixture();
    let (_, index) = indexed(&journal);
    let all = index.queue(&journal, &opts(QueueKind::Recents));
    let first = index.queue(
        &journal,
        &QueueOpts {
            limit: 5,
            ..opts(QueueKind::Recents)
        },
    );
    assert_eq!(first.items, all.items[..5]);
    assert_eq!(first.total, 17);
    let second = index.queue(
        &journal,
        &QueueOpts {
            limit: 5,
            offset: 15,
            ..opts(QueueKind::Recents)
        },
    );
    assert_eq!(second.items, all.items[15..]);
    // The D4 defaults: 50 Recents, Unknowns paged at 100, a 365-day window.
    let defaults = QueueOpts::new(QueueKind::Recents, TODAY);
    assert_eq!(
        (defaults.limit, defaults.max_days, defaults.offset),
        (50, 365, 0)
    );
    assert_eq!(QueueOpts::new(QueueKind::Unknowns, TODAY).limit, 100);
}

#[test]
fn a_read_only_include_is_counted_as_not_editable() {
    let journal = fixture();
    let classes = Classes::of(&journal);
    let read_only = |path: &Path| !path.ends_with("review-2025.journal");
    let index = ReviewIndex::build(&journal, &classes, read_only);

    let unknowns = index.queue(&journal, &opts(QueueKind::Unknowns));
    assert_eq!((unknowns.total, unknowns.not_editable), (7, 1));
    assert!(!dates(&journal, &unknowns.items).contains(&"2025-12-02".to_string()));

    let recents = index.queue(&journal, &opts(QueueKind::Recents));
    assert_eq!((recents.total, recents.not_editable), (15, 2));
}

// ---------------------------------------------------------------------------
// Similar (D11)
// ---------------------------------------------------------------------------

#[test]
fn similar_is_the_same_key_unreviewed_at_any_date_newest_first() {
    let journal = fixture();
    let (classes, index) = indexed(&journal);
    let card = find(&journal, "2026-10-12", "4412");
    let similar = index.similar(&journal, &classes, card);
    assert_eq!(
        dates(&journal, &similar.candidates),
        [
            "2026-11-15", // future-dated: "at any date"
            "2026-10-03",
            "2026-09-28",
            "2026-07-15", // categorised but unreviewed: a rename still applies
            "2026-06-20",
            "2025-12-02", // in the include
            "2024-03-10", // outside the queue window
        ]
    );
    assert_eq!(similar.total, 7);
    // Not self, not reviewed (1100), not ignored (4400), not another key.
    assert!(!similar.candidates.contains(&card));
    let shown = described(&journal, &similar.candidates);
    assert!(
        !shown
            .iter()
            .any(|(_, d)| d.contains("1100") || d.contains("4400"))
    );
    assert!(!shown.iter().any(|(_, d)| d.contains("BERKELEY")));
}

#[test]
fn similar_precedent_is_the_modal_counter_account() {
    let journal = fixture();
    let (classes, index) = indexed(&journal);
    let similar = index.similar(&journal, &classes, find(&journal, "2026-10-12", "4412"));
    let precedent = similar.precedent.expect("categorised history exists");
    assert_eq!(precedent.account, "expenses:food:coffee");
    // 2200, 1100 and the renamed `payee-key:` one, of four categorised.
    assert_eq!((precedent.count, precedent.of), (3, 4));
}

#[test]
fn similar_is_capped_at_500() {
    let body: String = (0..SIMILAR_CAP + 20)
        .map(|n| {
            format!(
                "2026-{:02}-{:02} SQ *CAFE {n} OAKLAND\n    expenses:unknown  $1\n    assets:cash\n\n",
                1 + n % 9,
                1 + n % 28
            )
        })
        .collect();
    let journal = inline(&body);
    let (classes, index) = indexed(&journal);
    let similar = index.similar(&journal, &classes, Tindex(1));
    assert_eq!(similar.total, SIMILAR_CAP + 19);
    assert_eq!(similar.candidates.len(), SIMILAR_CAP);
    let shown = dates(&journal, &similar.candidates);
    assert!(
        shown.windows(2).all(|pair| pair[0] >= pair[1]),
        "newest first"
    );
}

#[test]
fn a_description_with_no_key_has_no_similar() {
    let journal = inline(
        "2026-10-01 12345\n    expenses:unknown  $1\n    assets:cash\n\n\
         2026-10-02 12345\n    expenses:food  $1\n    assets:cash\n",
    );
    let (classes, index) = indexed(&journal);
    let similar = index.similar(&journal, &classes, Tindex(1));
    assert!(similar.candidates.is_empty() && similar.precedent.is_none());
}

// ---------------------------------------------------------------------------
// Suggest (D13a)
// ---------------------------------------------------------------------------

#[test]
fn exact_key_precedent_weighs_reviewed_double_and_offers_the_rename() {
    let journal = fixture();
    let (classes, index) = indexed(&journal);
    let suggestions = index.suggest(&journal, &classes, find(&journal, "2026-10-12", "4412"));
    let top = top_chip(&suggestions).expect("a strong precedent");
    assert_eq!(top.account, "expenses:food:coffee");
    assert_eq!(top.source, SuggestionSource::ExactKey);
    // coffee: 2200 (1) + 1100 reviewed (2) + rename reviewed (2) = 5 of 6.
    assert_eq!(top.confidence, 5 * CERTAIN / 6);
    assert_eq!(top.count, 3);
    assert_eq!(top.description.as_deref(), Some("Blue Bottle Coffee"));
    let runner_up = &suggestions[1];
    assert_eq!(runner_up.account, "expenses:food:groceries");
    assert_eq!(runner_up.confidence, CERTAIN / 6);
    assert_eq!(runner_up.description, None);
}

#[test]
fn reviewed_weight_can_outvote_a_majority() {
    // Two unreviewed coffees against one reviewed snack: 2 vs 2, and the tie
    // goes to the account more transactions used. A second reviewed snack
    // tips it.
    let base = "2026-01-01 Corner Cafe 1\n    expenses:coffee  $1\n    assets:cash\n\n\
                2026-01-02 Corner Cafe 2\n    expenses:coffee  $1\n    assets:cash\n\n\
                2026-01-03 Corner Cafe 3  ; reviewed:2026-01-04\n    expenses:snacks  $1\n    assets:cash\n\n";
    let target = "2026-02-01 Corner Cafe 9\n    expenses:unknown  $1\n    assets:cash\n";
    let tied = inline(&format!("{base}{target}"));
    let (classes, index) = indexed(&tied);
    let suggestions = index.suggest(&tied, &classes, Tindex(4));
    assert_eq!(suggestions[0].account, "expenses:coffee");
    assert_eq!(suggestions[0].confidence, STRONG);
    assert_eq!(suggestions[1].confidence, STRONG);

    let tipped = inline(&format!(
        "{base}2026-01-05 Corner Cafe 4  ; reviewed:yes\n    expenses:snacks  $1\n    assets:cash\n\n{target}"
    ));
    let (classes, index) = indexed(&tipped);
    let suggestions = index.suggest(&tipped, &classes, Tindex(5));
    assert_eq!(suggestions[0].account, "expenses:snacks");
    assert_eq!(suggestions[0].confidence, 4 * CERTAIN / 6);
}

#[test]
fn payee_prefix_is_used_only_without_exact_history_and_is_discounted() {
    let journal = fixture();
    let (classes, index) = indexed(&journal);
    let berkeley = find(&journal, "2026-10-05", "BERKELEY");
    let suggestions = index.suggest(&journal, &classes, berkeley);
    assert_eq!(suggestions[0].account, "expenses:food:coffee");
    assert_eq!(suggestions[0].source, SuggestionSource::PayeePrefix);
    // `sq blue bottle` is 3 of 4 tokens; the Oakland history is 3/4 coffee
    // (the payee-key rename no longer HAS that key): 75% × 80% × 3/4.
    assert_eq!(suggestions[0].confidence, 4_500);
    assert!(
        top_chip(&suggestions).is_none(),
        "below 50%: alternatives only"
    );
}

#[test]
fn a_shared_single_word_is_not_a_prefix() {
    let journal = inline(
        "2026-01-01 AMAZON BOOKS\n    expenses:books  $1\n    assets:cash\n\n\
         2026-02-01 AMAZON FRESH\n    expenses:unknown  $1\n    assets:cash\n",
    );
    let (classes, index) = indexed(&journal);
    assert!(index.suggest(&journal, &classes, Tindex(2)).is_empty());
}

#[test]
fn a_renamed_subscription_is_matched_by_funding_amount_and_day() {
    let journal = fixture();
    let (classes, index) = indexed(&journal);
    let suggestions = index.suggest(&journal, &classes, find(&journal, "2026-10-14", "NFLX"));
    let top = top_chip(&suggestions).expect("a strong recurring match");
    assert_eq!(top.account, "expenses:streaming");
    assert_eq!(top.source, SuggestionSource::Recurring);
    assert_eq!(top.count, 6);
}

#[test]
fn a_recurring_match_needs_the_same_funding_account_and_amount() {
    let months: String = (4..=9)
        .map(|month| {
            format!("2026-{month:02}-14 Netflix\n    expenses:streaming  $15.49\n    liabilities:visa\n\n")
        })
        .collect();
    let declarations = "account expenses:streaming  ; type: X\naccount expenses:unknown  ; type: X\n\
                        account liabilities:visa  ; type: L\naccount liabilities:amex  ; type: L\n\n";
    for (charge, matches) in [
        ("    expenses:unknown  $15.49\n    liabilities:visa\n", true),
        // Paid from another card.
        (
            "    expenses:unknown  $15.49\n    liabilities:amex\n",
            false,
        ),
        // 2% dearer.
        (
            "    expenses:unknown  $15.80\n    liabilities:visa\n",
            false,
        ),
    ] {
        let journal = inline(&format!(
            "{declarations}{months}2026-10-15 STREAMCO 77\n{charge}"
        ));
        let (classes, index) = indexed(&journal);
        let target = Tindex(u32::try_from(journal.transactions.len()).expect("small"));
        let found = index
            .suggest(&journal, &classes, target)
            .iter()
            .any(|s| s.source == SuggestionSource::Recurring);
        assert_eq!(found, matches, "{charge}");
    }
}

#[test]
fn sources_rank_by_confidence_and_one_account_appears_once() {
    // Weak exact-key history (a split vote) under a strong recurring match
    // for the same account: the account appears once, at the recurring
    // confidence, and the chip goes to it.
    let months: String = (4..=9)
        .map(|month| {
            format!("2026-{month:02}-14 Netflix\n    expenses:streaming  $15.49\n    liabilities:visa\n\n")
        })
        .collect();
    let journal = inline(&format!(
        "account expenses:streaming  ; type: X\naccount expenses:unknown  ; type: X\n\
         account expenses:misc  ; type: X\naccount liabilities:visa  ; type: L\n\n{months}\
         2026-01-03 STREAMCO 1\n    expenses:streaming  $3.00\n    liabilities:visa\n\n\
         2026-01-04 STREAMCO 2\n    expenses:misc  $3.00\n    liabilities:visa\n\n\
         2026-02-05 STREAMCO 3\n    expenses:misc  $3.00\n    liabilities:visa\n\n\
         2026-10-14 STREAMCO 9\n    expenses:unknown  $15.49\n    liabilities:visa\n"
    ));
    let (classes, index) = indexed(&journal);
    let target = Tindex(u32::try_from(journal.transactions.len()).expect("small"));
    let suggestions = index.suggest(&journal, &classes, target);
    let accounts: Vec<(&str, SuggestionSource)> = suggestions
        .iter()
        .map(|s| (s.account.as_str(), s.source))
        .collect();
    assert_eq!(
        accounts,
        [
            ("expenses:misc", SuggestionSource::ExactKey),
            ("expenses:streaming", SuggestionSource::Recurring),
        ]
    );
    // misc 2/3 beats the recurring 60%; streaming's exact 1/3 lost to its
    // own recurring 60%.
    assert_eq!(suggestions[0].confidence, 2 * CERTAIN / 3);
    assert_eq!(suggestions[1].confidence, 6_000);
    assert!(suggestions.len() <= MAX_SUGGESTIONS);
}

#[test]
fn the_top_chip_needs_fifty_percent() {
    // One coffee, one snack, one tea: a three-way split is 33% each.
    let journal = inline(
        "2026-01-01 Corner Cafe 1\n    expenses:coffee  $1\n    assets:cash\n\n\
         2026-01-02 Corner Cafe 2\n    expenses:snacks  $1\n    assets:cash\n\n\
         2026-01-03 Corner Cafe 3\n    expenses:tea  $1\n    assets:cash\n\n\
         2026-02-01 Corner Cafe 9\n    expenses:unknown  $1\n    assets:cash\n",
    );
    let (classes, index) = indexed(&journal);
    let suggestions = index.suggest(&journal, &classes, Tindex(4));
    assert_eq!(suggestions.len(), 3);
    assert!(top_chip(&suggestions).is_none());
    assert!(suggestions.iter().all(|s| !s.is_strong()));
}

#[test]
fn an_all_digit_description_gets_no_suggestion() {
    let journal = inline(
        "2026-01-01 12345\n    expenses:coffee  $1\n    assets:cash\n\n\
         2026-02-01 12345\n    expenses:unknown  $1\n    assets:cash\n",
    );
    let (classes, index) = indexed(&journal);
    assert!(index.suggest(&journal, &classes, Tindex(2)).is_empty());
}

#[test]
fn queries_on_a_missing_transaction_are_empty() {
    let journal = fixture();
    let (classes, index) = indexed(&journal);
    assert!(index.suggest(&journal, &classes, Tindex(9_999)).is_empty());
    assert!(
        index
            .similar(&journal, &classes, Tindex(9_999))
            .candidates
            .is_empty()
    );
}

// ---------------------------------------------------------------------------
// Autocomplete vocabularies (D15)
// ---------------------------------------------------------------------------

#[test]
fn payees_rank_by_use_then_recency_and_split_off_the_note() {
    let journal = inline(
        "2026-01-01 Safeway | weekly\n    expenses:food  $1\n    assets:cash\n\n\
         2026-03-01 Safeway\n    expenses:food  $1\n    assets:cash\n\n\
         2026-02-01 Corner Store\n    expenses:food  $1\n    assets:cash\n\n\
         2026-04-01 Bakery\n    expenses:food  $1\n    assets:cash\n\n\
         2026-05-01 \n    expenses:food  $1\n    assets:cash\n",
    );
    let payees: Vec<(String, usize, String)> = ledgeline_core::review::payees(&journal)
        .into_iter()
        .map(|use_| (use_.payee, use_.count, use_.last))
        .collect();
    assert_eq!(
        payees,
        vec![
            ("Safeway".to_string(), 2, "2026-03-01".to_string()),
            ("Bakery".to_string(), 1, "2026-04-01".to_string()),
            ("Corner Store".to_string(), 1, "2026-02-01".to_string()),
        ]
    );
}

#[test]
fn tag_uses_count_header_tags_and_rank_their_values() {
    let journal = inline(
        "2026-01-01 A  ; trip:paris, reviewed:2026-01-02\n    expenses:food  $1  ; posted:no\n    assets:cash\n\n\
         2026-01-02 B  ; trip:paris, _hidden:x\n    expenses:food  $1\n    assets:cash\n\n\
         2026-01-03 C  ; trip:rome, flag:\n    expenses:food  $1\n    assets:cash\n",
    );
    let tags = ledgeline_core::review::tag_uses(&journal);
    let names: Vec<(&str, usize)> = tags.iter().map(|t| (t.name.as_str(), t.count)).collect();
    assert_eq!(names, vec![("trip", 3), ("flag", 1), ("reviewed", 1)]);
    assert_eq!(
        tags[0].values,
        vec![("paris".to_string(), 2), ("rome".to_string(), 1)]
    );
    assert!(tags[1].values.is_empty(), "an empty value is not offered");
}

// ---------------------------------------------------------------------------
// similarity_key: Unicode NFKC (amendment B)
// ---------------------------------------------------------------------------

#[test]
fn similarity_key_folds_compatibility_forms_through_nfkc() {
    let cases: &[(&str, &str)] = &[
        // Ligatures.
        ("\u{FB01}ve Guys", "five guys"),
        ("Ca\u{FB00}e Roma", "caffe roma"),
        // Fullwidth letters, punctuation and the ideographic space.
        ("ＳＱ ＊ＢＬＵＥ　ＢＯＴＴＬＥ", "sq blue bottle"),
        // Compatibility characters: a trademark sign, a superscript, a
        // circled letter, black-letter and a non-breaking space. A superscript
        // digit becomes a digit, so its token drops like a store number.
        ("Acme\u{2122} Store", "acmetm store"),
        ("Suite \u{00B2} Cafe", "suite cafe"),
        ("\u{24B6}cme", "acme"),
        ("\u{210C}ome\u{00A0}Depot", "home depot"),
        // A precomposed letter and its decomposed spelling key the same.
        ("Caf\u{00E9}", "caf\u{00E9}"),
        ("Cafe\u{0301}", "caf\u{00E9}"),
    ];
    for (description, expected) in cases {
        assert_eq!(
            similarity_key(description).as_deref(),
            Some(*expected),
            "{description:?}"
        );
    }
    // A digit NFKC produces is still a digit: the circled store number drops.
    assert_eq!(
        similarity_key("Shell \u{2460}\u{2461}").as_deref(),
        Some("shell")
    );
}
