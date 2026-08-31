//! Resource-exhaustion regressions on the journal parse path.
//!
//! A journal is attacker-influenced input: opening someone else's `.journal` is
//! an ordinary thing to do, and the live-reload watcher reparses on every write.
//! So a small file that costs the process a lot of time or a lot of memory is a
//! denial of service, not a curiosity. Each test below fixes an amplification
//! ratio — output work per input byte — rather than an absolute number, so it
//! keeps meaning the same thing on a faster machine.

use ledgeline_core::model::Amount;
use ledgeline_core::parse_journal;
use std::time::Instant;

/// Every amount in the journal, postings and inferred legs alike.
fn amounts(journal: &ledgeline_core::Journal) -> impl Iterator<Item = &Amount> {
    journal
        .transactions
        .iter()
        .flat_map(|txn| txn.postings.iter())
        .flat_map(|posting| posting.amounts.iter())
}

// ---------------------------------------------------------------------------
// FINDING 1: the parse scale cap must fail closed
// ---------------------------------------------------------------------------

#[test]
fn a_hostile_exponent_cannot_poison_the_display_precision() {
    // `pow10` cannot build 10^39, and the cap used to `return self` when it
    // failed — so the one input that most needed capping was the one that
    // skipped it. `places` then rode out to the wire as `style.precision`.
    let text = "2026-01-01 t\n    a  $1e-2147483648\n    b\n";
    let journal = parse_journal(text, "hostile.journal").expect("the literal is well-formed");

    for amount in amounts(&journal) {
        assert!(
            amount.quantity.places <= 10,
            "parsed scale {} exceeds the 10-place parse cap",
            amount.quantity.places
        );
        assert!(
            amount.style.precision <= 10,
            "display precision {} exceeds the 10-place parse cap",
            amount.style.precision
        );
    }
}

// ---------------------------------------------------------------------------
// FINDING 3: a declared digit grouping is cloned into every amount
// ---------------------------------------------------------------------------

/// Total digit-group entries retained by a journal whose `commodity` directive
/// declares `separators` groups, over a fixed 2,000 transactions.
fn retained_group_entries(separators: usize) -> usize {
    let spec: String = "1,".repeat(separators) + "1.00";
    let mut text = format!("commodity ${spec}\n");
    for i in 0..2_000 {
        text.push_str(&format!("2026-01-01 t{i}\n    a  $1.00\n    b  $-1.00\n"));
    }
    let journal = parse_journal(&text, "groups.journal").expect("the journal is well-formed");
    amounts(&journal)
        .filter_map(|amount| amount.style.digit_groups.as_ref())
        .map(|groups| groups.sizes.len())
        .sum()
}

#[test]
fn a_declared_digit_grouping_is_not_amplified_per_amount() {
    // One `commodity` directive declares the canonical style; every amount of
    // that commodity then gets its own copy of the group-size vector. The
    // directive is charged ONCE against the file size and the copies are not, so
    // the ratio grows with the directive AND with the posting count, unbounded
    // in both.
    //
    // The property is stated as independence rather than as a byte budget on
    // purpose: what makes this a denial of service is not the size of any one
    // journal, it is that a hostile directive buys retention the file does not
    // pay for. Holding the postings fixed and varying only the directive,
    // retention must not move. (Before the fix: 200 separators retained 804,000
    // entries and 20,000 separators retained 80,004,000 — a 100x swing bought
    // with 40 KB of text, and the ratio keeps climbing from there.)
    let modest = retained_group_entries(200);
    let hostile = retained_group_entries(20_000);

    assert!(modest > 0, "grouping must still be recorded at all");
    assert_eq!(
        modest, hostile,
        "retained group entries track the directive's size: \
         200 separators -> {modest}, 20,000 -> {hostile}"
    );
}

// ---------------------------------------------------------------------------
// FINDING 4: per-commodity balancing is quadratic in a wide transaction
// ---------------------------------------------------------------------------

/// A single transaction holding `n` distinct commodities, each netting to zero
/// so the transaction balances.
fn wide_transaction(n: usize) -> String {
    let postings: String = (0..n)
        .map(|i| format!("    a{i}  1 C{i}\n    b{i}  -1 C{i}\n"))
        .collect();
    format!("2026-01-01 wide\n{postings}")
}

#[test]
fn balancing_a_wide_transaction_does_not_scale_quadratically() {
    // `group_sums` looked each commodity up with a linear scan of the running
    // list, so N distinct commodities cost N²/2 comparisons. Doubling the
    // postings should roughly double the work; quadratic makes it quadruple.
    let time = |n: usize| {
        let text = wide_transaction(n);
        let start = Instant::now();
        let journal = parse_journal(&text, "wide.journal").expect("the journal balances");
        assert_eq!(journal.transactions.len(), 1);
        start.elapsed()
    };

    // Warm the allocator and the code paths so the first sample is not the one
    // that pays for them.
    let _ = time(500);

    let small = time(2_000);
    let large = time(8_000);
    // 4x the input. Linear predicts ~4x, quadratic ~16x; 8 separates them with
    // room for scheduling noise on a loaded machine.
    let ratio = large.as_secs_f64() / small.as_secs_f64().max(f64::MIN_POSITIVE);
    assert!(
        ratio < 8.0,
        "4x the postings took {ratio:.1}x the time ({small:?} -> {large:?}); \
         that is quadratic, not linear"
    );
}

#[test]
fn a_wide_transaction_preserves_first_seen_commodity_order() {
    // The inferred leg's amounts come out in `group_sums` order, and that order
    // is observable in the journal. Any lookup change must keep it first-seen,
    // NOT sorted -- `C10` sorts before `C2`, so a `BTreeMap` keyed directly by
    // commodity would visibly reorder this.
    let text = "2026-01-01 t\n    a  1 C2\n    a  1 C10\n    a  1 C1\n    b\n";
    let journal = parse_journal(text, "order.journal").expect("the journal is well-formed");
    let inferred = &journal.transactions[0].postings[3];
    let order: Vec<&str> = inferred
        .amounts
        .iter()
        .map(|amount| amount.commodity.0.as_str())
        .collect();
    assert_eq!(order, ["C2", "C10", "C1"]);
}
