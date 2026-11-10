//! Plan 35 D11: linked transfer pairs as journal rows
//! ([`ledgeline_core::view::rows::Transfers`]) — which pairs collapse, where
//! the row sits under a filter, the direction and labels, and the search text.

use ledgeline_core::model::Journal;
use ledgeline_core::parse::parse_journal;
use ledgeline_core::view::filter::{JournalFilter, display_order, filter_positions};
use ledgeline_core::view::rows::{Transfers, haystack};
use ledgeline_core::view::wire::DeclaredStyles;

const JOURNAL: &str = "\
account assets:checking   ; type: C
account assets:savings    ; type: C
account liabilities:amex  ; type: L
account transfers         ; clearing:

2026-12-01 * ACH AMEX EPAYMENT
    assets:checking   $-500
    transfers:cc       $500  ; match:aaa111

2026-12-02 Coffee
    assets:checking   $-4
    assets:savings

2026-12-03 * PAYMENT THANK YOU
    transfers:cc      $-500  ; match:aaa111
    liabilities:amex
";

fn parse(text: &str) -> Journal {
    parse_journal(text, "inline.journal").expect("journal parses")
}

fn positions(journal: &Journal, filter: &JournalFilter, transfers: &Transfers) -> Vec<u32> {
    let declared = DeclaredStyles::of(journal);
    let mut haystacks: Vec<Box<str>> = journal
        .transactions
        .iter()
        .map(|txn| haystack(txn, &declared).into_boxed_str())
        .collect();
    transfers.join_haystacks(&mut haystacks);
    filter_positions(journal, &display_order(journal), Some(&haystacks), filter)
}

fn filter(accounts: &[&str], query: &str) -> JournalFilter {
    JournalFilter::new(
        None,
        None,
        accounts.iter().map(|account| (*account).to_string()),
        query,
    )
}

#[test]
fn a_linked_pair_collapses_to_its_earliest_half_with_direction_and_labels() {
    let journal = parse(JOURNAL);
    let transfers = Transfers::of(&journal);
    let transfer = transfers.get(0).expect("the first half collapses");
    assert_eq!(transfer.id, "aaa111");
    assert_eq!(transfer.halves, [0, 2]);
    assert_eq!(transfers.get(2), Some(transfer));
    assert_eq!(transfer.partner(2), 0);
    assert_eq!(transfer.from.position, 0);
    assert_eq!(transfer.from.account.as_deref(), Some("assets:checking"));
    assert_eq!(transfer.from.label.as_deref(), Some("checking"));
    assert_eq!(transfer.to.account.as_deref(), Some("liabilities:amex"));
    assert_eq!(transfer.to.label.as_deref(), Some("amex"));
    assert!(transfers.is_linked(0) && transfers.is_linked(2) && !transfers.is_linked(1));

    // Display order is newest first: 2, 1, 0. The pair shows once, at 0.
    let all = positions(&journal, &filter(&[], ""), &transfers);
    assert_eq!(all, vec![2, 1, 0]);
    assert_eq!(transfers.collapse(&all), vec![1, 0]);
}

#[test]
fn a_partial_filter_anchors_the_transfer_row_on_the_half_it_keeps() {
    let journal = parse(JOURNAL);
    let transfers = Transfers::of(&journal);
    let amex = positions(&journal, &filter(&["liabilities:amex"], ""), &transfers);
    assert_eq!(amex, vec![2]);
    assert_eq!(transfers.collapse(&amex), vec![2]);
    assert!(transfers.get(2).is_some(), "still a transfer row");
}

#[test]
fn searching_either_description_finds_the_transfer_row() {
    let journal = parse(JOURNAL);
    let transfers = Transfers::of(&journal);
    for query in ["thank you", "epayment"] {
        let found = positions(&journal, &filter(&[], query), &transfers);
        assert_eq!(found, vec![2, 0], "{query}: both halves pass");
        assert_eq!(transfers.collapse(&found), vec![0], "{query}");
    }
}

#[test]
fn a_transaction_in_two_links_does_not_collapse_but_is_linked() {
    let journal = parse(
        "\
account transfers  ; clearing:

2026-12-01 Transfer and fee
    assets:checking   $-505
    transfers:cc       $500  ; match:aaa111
    transfers:cc         $5  ; match:bbb222

2026-12-02 Payment
    transfers:cc      $-500  ; match:aaa111
    liabilities:amex

2026-12-03 Fee
    transfers:cc        $-5  ; match:bbb222
    expenses:fees
",
    );
    let transfers = Transfers::of(&journal);
    assert!(transfers.is_empty());
    assert!((0..3).all(|position| transfers.is_linked(position)));
    assert_eq!(transfers.collapse(&[2, 1, 0]), vec![2, 1, 0]);
}

#[test]
fn colliding_last_segments_label_with_their_shortest_unique_suffix() {
    let journal = parse(
        "\
account transfers  ; clearing:

2026-12-01 Out
    assets:bank1:checking  $-50
    transfers:bank          $50  ; match:ccc333

2026-12-02 In
    assets:bank2:checking   $50
    transfers:bank         $-50  ; match:ccc333
",
    );
    let transfers = Transfers::of(&journal);
    let transfer = transfers.get(1).expect("collapses");
    assert_eq!(transfer.from.label.as_deref(), Some("bank1:checking"));
    assert_eq!(transfer.to.label.as_deref(), Some("bank2:checking"));
}

#[test]
fn a_broken_link_does_not_collapse() {
    let journal = parse(
        "\
account transfers  ; clearing:

2026-12-01 Out
    assets:checking  $-50
    transfers:bank    $50  ; match:ddd444

2026-12-02 In
    assets:savings    $40
    transfers:bank   $-40  ; match:ddd444
",
    );
    let transfers = Transfers::of(&journal);
    assert!(transfers.is_empty());
    assert!(!transfers.is_linked(0));
}

#[test]
fn a_group_of_three_never_collapses_and_its_rows_are_linked() {
    // Amendment A3: one bank debit paying two bills, linked as one group.
    let journal = parse(
        "\
account transfers         ; clearing:

2026-12-01 * BILL.COM DEBIT
    assets:checking   $-2247.50
    transfers:bills    $2247.50  ; match:grp001

2026-12-01 Rent bill
    transfers:bills   $-2000.00  ; match:grp001
    expenses:rent

2026-12-01 Coffee
    assets:checking   $-4
    assets:savings

2026-12-01 Hosting bill
    transfers:bills    $-247.50  ; match:grp001
    expenses:hosting

2026-12-02 * ACH AMEX EPAYMENT
    assets:checking   $-500
    transfers:cc       $500  ; match:aaa111

2026-12-03 * PAYMENT THANK YOU
    transfers:cc      $-500  ; match:aaa111
    liabilities:amex
",
    );
    let transfers = Transfers::of(&journal);
    for position in [0, 1, 3] {
        assert!(transfers.get(position).is_none(), "{position} collapsed");
        assert!(transfers.is_linked(position), "{position} not linked");
    }
    assert!(!transfers.is_linked(2));
    // The ordinary pair beside it still collapses.
    assert_eq!(transfers.get(4).map(|t| t.halves), Some([4, 5]));
    let all = positions(&journal, &filter(&[], ""), &transfers);
    assert_eq!(transfers.collapse(&all), vec![4, 3, 2, 1, 0]);
}

#[test]
fn a_pair_sharing_a_transaction_with_a_group_does_not_collapse() {
    let journal = parse(
        "\
account transfers         ; clearing:

2026-12-01 * BILL.COM DEBIT plus a transfer
    assets:checking   $-2747.50
    transfers:bills    $2247.50  ; match:grp001
    transfers:cc        $500     ; match:aaa111

2026-12-01 Rent bill
    transfers:bills   $-2000.00  ; match:grp001
    expenses:rent

2026-12-01 Hosting bill
    transfers:bills    $-247.50  ; match:grp001
    expenses:hosting

2026-12-03 * PAYMENT THANK YOU
    transfers:cc      $-500  ; match:aaa111
    liabilities:amex
",
    );
    let transfers = Transfers::of(&journal);
    assert!(transfers.is_empty());
    assert!((0..4).all(|position| transfers.is_linked(position)));
}
