//! Balance-assertion evaluation parity tests (CLEANUP.md PARSE-2).
//!
//! Before this suite, `= AMOUNT` was parsed, stored and serialized but never
//! checked: the CLEANUP.md repro (`a $1.00 = $99.00`) parsed clean and every
//! report downstream consumed the un-reconciled numbers.
//!
//! Every case below was run against real `hledger 1.52` (mac-aarch64) with
//! `hledger -f FILE check assertions`, and each test names that ground truth.
//! Where a case looks surprising it is because hledger genuinely behaves that
//! way — see the `==*` commodity-scope cases in particular.
//!
//! | case                                     | hledger 1.52 | ledgeline |
//! |------------------------------------------|--------------|-----------|
//! | `= $99.00` where balance is `$1.00`      | error        | failure (parity) |
//! | `=` on a multi-commodity account         | accepted     | accepted (parity) |
//! | `==` with another commodity non-zero     | error        | failure (parity) |
//! | `=*` rolls subaccounts up                | accepted     | accepted (parity) |
//! | `=*` does NOT roll up a name prefix      | accepted     | accepted (parity) |
//! | `==*` ignores a subaccount-only commodity| accepted     | accepted (parity) |
//! | `==*` DOES check a zeroed own commodity  | error        | failure (parity) |
//! | assertion after `@`/`@@` cost            | accepted     | accepted (parity) |
//! | out-of-file-order dates                  | date order   | date order (parity) |
//! | posting `date:` tag reorders postings    | posting order| posting order (parity) |
//! | same date across an `include`            | read order   | read order (parity) |
//! | `date2` (secondary date)                 | ignored      | ignored (parity) |
//! | bare `= 0` on a `$` balance              | accepted     | accepted (parity) |

mod common;

use ledgeline_core::assertions::{AssertionFailure, check_balance_assertions};
use ledgeline_core::parse::parse_journal_with_overrides;
use ledgeline_core::{Journal, parse_journal};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn journal(text: &str) -> Journal {
    parse_journal(text, "/tmp/assertions.journal").expect("journal parses")
}

/// Every assertion failure in `text`, in evaluation order.
fn failures(text: &str) -> Vec<AssertionFailure> {
    check_balance_assertions(&journal(text)).expect("no decimal overflow")
}

/// Assert `text` has no failing assertion — the hledger-accepts case.
fn assert_clean(text: &str) {
    let found = failures(text);
    assert!(
        found.is_empty(),
        "expected no assertion failures, got:\n{}",
        rendered(&found)
    );
}

/// Assert `text` has exactly one failing assertion, and return it.
fn assert_one_failure(text: &str) -> AssertionFailure {
    let mut found = failures(text);
    assert_eq!(
        found.len(),
        1,
        "expected exactly one assertion failure, got:\n{}",
        rendered(&found)
    );
    found.remove(0)
}

/// The FIRST failing assertion in `text` — the one hledger would report before
/// aborting. Use this where a later assertion cascades off the first (hledger
/// never reaches it; this pass does, by design).
fn assert_first_failure(text: &str) -> AssertionFailure {
    let mut found = failures(text);
    assert!(
        !found.is_empty(),
        "expected at least one assertion failure, got none"
    );
    found.remove(0)
}

fn rendered(found: &[AssertionFailure]) -> String {
    if found.is_empty() {
        return "  (none)".to_owned();
    }
    found
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n---\n")
}

/// A private scratch directory for one test, emptied first so reruns are clean.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ledgeline_balance_assertions_{}_{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("write fixture");
    path
}

// ---------------------------------------------------------------------------
// The CLEANUP.md repro
// ---------------------------------------------------------------------------

/// The exact journal from CLEANUP.md PARSE-2.
///
/// hledger 1.52:
/// ```text
/// Balance assertion failed in a
/// In commodity $ at this point, excluding subaccounts, ignoring costs,
/// the asserted balance is:        $99.00
/// but the calculated balance is:   $1.00
/// (difference: $98.00)
/// ```
/// Ledgeline before this change: accepted silently.
#[test]
fn cleanup_repro_is_no_longer_silently_accepted() {
    let failure = assert_one_failure(
        "2024-01-01 assertfail\n\
         \x20   a   $1.00 = $99.00\n\
         \x20   b   $-1.00\n",
    );
    assert_eq!(failure.account.0, "a");
    assert_eq!(failure.commodity.0, "$");
    assert_eq!(failure.operator(), "=");
    assert_eq!(
        failure.message(),
        "balance assertion failed in a\n\
         In commodity $ at this point, excluding subaccounts, ignoring costs,\n\
         the asserted balance is:       $99.00\n\
         but the calculated balance is: $1.00\n\
         (difference: $98.00)"
    );
    // The location points at the `=` sign, in the file the posting came from.
    assert_eq!(failure.position.line, 2);
    assert_eq!(
        failure.source_file,
        PathBuf::from("/tmp/assertions.journal")
    );
}

#[test]
fn a_correct_assertion_passes() {
    assert_clean(
        "2024-01-01 ok\n\
         \x20   a   $1.00 = $1.00\n\
         \x20   b   $-1.00\n",
    );
}

/// `Dec` compares by value, so a differently-scaled but equal assertion holds —
/// and no display rounding is involved either way.
#[test]
fn comparison_is_by_exact_value_not_representation() {
    assert_clean(
        "2024-01-01 scale\n\
         \x20   a   $1.00 = $1\n\
         \x20   b   $-1.00\n",
    );
    // ...while a difference far below any display precision still fails.
    let failure = assert_one_failure(
        "2024-01-01 tiny\n\
         \x20   a   $1.0000001 = $1.00\n\
         \x20   b   $-1.0000001\n",
    );
    assert!(
        failure
            .message()
            .contains("but the calculated balance is: $1.0000001"),
        "the message must show every exact digit, not a rounded one:\n{}",
        failure.message()
    );
}

// ---------------------------------------------------------------------------
// The four operators
// ---------------------------------------------------------------------------

/// `=` is single-commodity and exclusive. hledger accepts: the `$` leg matches
/// and the account's 10 EUR is simply not this assertion's business.
#[test]
fn partial_assertion_ignores_other_commodities() {
    assert_clean(
        "2024-01-01 open\n\
         \x20   a   $10.00\n\
         \x20   a   10 EUR\n\
         \x20   eq  $-10.00\n\
         \x20   eq  -10 EUR\n\
         \n\
         2024-01-02 partial\n\
         \x20   a   $1.00 = $11.00\n\
         \x20   eq  $-1.00\n",
    );
}

/// `==` asserts the account holds ONLY the asserted commodity. hledger errors,
/// naming EUR (not the `$` the user wrote) as the commodity that is not zero.
#[test]
fn total_assertion_requires_every_other_commodity_to_be_zero() {
    let failure = assert_one_failure(
        "2024-01-01 open\n\
         \x20   a   $10.00\n\
         \x20   a   10 EUR\n\
         \x20   eq  $-10.00\n\
         \x20   eq  -10 EUR\n\
         \n\
         2024-01-02 total\n\
         \x20   a   $1.00 == $11.00\n\
         \x20   eq  $-1.00\n",
    );
    assert_eq!(failure.operator(), "==");
    assert_eq!(failure.commodity.0, "EUR");
    assert!(
        failure
            .message()
            .contains("Across all commodities at this point, excluding subaccounts"),
        "{}",
        failure.message()
    );
    assert!(
        failure
            .message()
            .contains("the asserted balance is:       0 EUR"),
        "the implied zero must render in the offending commodity's style:\n{}",
        failure.message()
    );
}

#[test]
fn total_assertion_passes_for_a_single_commodity_account() {
    assert_clean(
        "2024-01-01 open\n\
         \x20   a   $10.00\n\
         \x20   eq  $-10.00\n\
         \n\
         2024-01-02 total\n\
         \x20   a   $1.00 == $11.00\n\
         \x20   eq  $-1.00\n",
    );
}

/// `=*` sums the account and its subaccounts. hledger: `a` = 3 + 4 + 2 + 1 = 10.
#[test]
fn inclusive_assertion_rolls_up_subaccounts() {
    assert_clean(
        "2024-01-01 sub\n\
         \x20   a:x   $3.00\n\
         \x20   a:y   $4.00\n\
         \x20   a     $2.00\n\
         \x20   eq    $-9.00\n\
         \n\
         2024-01-02 rollup\n\
         \x20   a     $1.00 =* $10.00\n\
         \x20   eq    $-1.00\n",
    );
}

/// The same journal with the exclusive `=` sees only `a`'s own $3.00.
#[test]
fn exclusive_assertion_does_not_roll_up_subaccounts() {
    let failure = assert_one_failure(
        "2024-01-01 sub\n\
         \x20   a:x   $3.00\n\
         \x20   a:y   $4.00\n\
         \x20   a     $2.00\n\
         \x20   eq    $-9.00\n\
         \n\
         2024-01-02 rollup\n\
         \x20   a     $1.00 = $10.00\n\
         \x20   eq    $-1.00\n",
    );
    assert_eq!(failure.operator(), "=");
    assert!(
        failure
            .message()
            .contains("but the calculated balance is: $3.00"),
        "{}",
        failure.message()
    );
}

/// An inclusive assertion that is short of the rolled-up total. hledger errors
/// with `including subaccounts` in the message.
#[test]
fn inclusive_assertion_fails_when_the_rollup_disagrees() {
    let failure = assert_one_failure(
        "2024-01-01 sub\n\
         \x20   a:x   $3.00\n\
         \x20   a:y   $4.00\n\
         \x20   eq    $-7.00\n\
         \n\
         2024-01-02 rollup\n\
         \x20   a     $1.00 =* $1.00\n\
         \x20   eq    $-1.00\n",
    );
    assert_eq!(failure.operator(), "=*");
    assert!(
        failure
            .message()
            .contains("In commodity $ at this point, including subaccounts"),
        "{}",
        failure.message()
    );
    assert!(
        failure
            .message()
            .contains("but the calculated balance is: $8.00"),
        "{}",
        failure.message()
    );
}

/// `==*` is total AND inclusive: hledger sums `a:x` + `a` for the asserted
/// commodity.
#[test]
fn total_inclusive_assertion_rolls_up_and_fails_on_a_mismatch() {
    let failure = assert_one_failure(
        "2024-01-01 sub\n\
         \x20   a:x   $3.00\n\
         \x20   eq    $-3.00\n\
         \n\
         2024-01-02 totalstar\n\
         \x20   a     $1.00 ==* $1.00\n\
         \x20   eq    $-1.00\n",
    );
    assert_eq!(failure.operator(), "==*");
    assert!(
        failure
            .message()
            .contains("Across all commodities at this point, including subaccounts"),
        "{}",
        failure.message()
    );
    assert!(
        failure
            .message()
            .contains("but the calculated balance is: $4.00"),
        "{}",
        failure.message()
    );
}

#[test]
fn total_inclusive_assertion_passes_when_the_rollup_agrees() {
    assert_clean(
        "2024-01-01 sub\n\
         \x20   a:x   $3.00\n\
         \x20   eq    $-3.00\n\
         \n\
         2024-01-02 totalstar\n\
         \x20   a     $1.00 ==* $4.00\n\
         \x20   eq    $-1.00\n",
    );
}

/// Subaccount matching is on the `:` segment boundary, not a string prefix:
/// `ab` is not a subaccount of `a`, so hledger accepts this.
#[test]
fn inclusive_rollup_is_segment_wise_not_prefix_wise() {
    assert_clean(
        "2024-01-01 setup\n\
         \x20   ab    $5.00\n\
         \x20   eq    $-5.00\n\
         \n\
         2024-01-02 assert\n\
         \x20   a     $1.00 =* $1.00\n\
         \x20   eq    $-1.00\n",
    );
}

// ---------------------------------------------------------------------------
// hledger's `==*` commodity-scope quirk (deliberately reproduced)
// ---------------------------------------------------------------------------

/// The set of "other commodities" a total assertion requires to be zero comes
/// from the account's OWN balance, even for the inclusive `==*`. Here 4 EUR
/// lives only in the subaccount `a:y`, so hledger never looks at EUR at all and
/// ACCEPTS this journal — despite the inclusive balance holding EUR.
#[test]
fn total_inclusive_ignores_a_commodity_held_only_in_a_subaccount() {
    assert_clean(
        "2024-01-01 setup\n\
         \x20   a:y   4 EUR\n\
         \x20   a:y   $2.00\n\
         \x20   a     $1.00\n\
         \x20   eq    -4 EUR\n\
         \x20   eq    $-3.00\n\
         \n\
         2024-01-02 assert\n\
         \x20   a     $0.00 ==* $3.00\n\
         \x20   eq    $0.00\n",
    );
}

/// The other half of the same rule: a commodity that has netted to exactly zero
/// in the account's OWN balance is still *present* there, so `==*` does pick it
/// up — and then evaluates it INCLUSIVELY, finding the subaccount's 4 EUR.
/// hledger errors. This is why the running balance must retain zeroed
/// commodities rather than pruning them.
#[test]
fn total_inclusive_checks_a_commodity_zeroed_in_the_accounts_own_balance() {
    let failure = assert_one_failure(
        "2024-01-01 setup\n\
         \x20   a     1 EUR\n\
         \x20   a     -1 EUR\n\
         \x20   a:y   4 EUR\n\
         \x20   a     $1.00\n\
         \x20   eq    $-1.00\n\
         \x20   eq    -4 EUR\n\
         \n\
         2024-01-02 assert\n\
         \x20   a     $0.00 ==* $1.00\n\
         \x20   eq    $0.00\n",
    );
    assert_eq!(failure.commodity.0, "EUR");
    assert!(
        failure
            .message()
            .contains("but the calculated balance is: 4 EUR"),
        "{}",
        failure.message()
    );
}

/// The exclusive `==` counterpart: the zeroed EUR is checked against the
/// account's own balance, where it is zero. hledger accepts.
#[test]
fn total_exclusive_accepts_a_commodity_that_netted_to_zero() {
    assert_clean(
        "2024-01-01 setup\n\
         \x20   a     1 EUR\n\
         \x20   a     -1 EUR\n\
         \x20   a     $1.00\n\
         \x20   eq    $-1.00\n\
         \n\
         2024-01-02 assert\n\
         \x20   a     $0.00 == $1.00\n\
         \x20   eq    $0.00\n",
    );
}

/// A total assertion naming a commodity the account does not hold still checks
/// the ones it does. hledger errors on `$`, not on the asserted EUR.
#[test]
fn total_assertion_on_an_absent_commodity_still_checks_the_present_one() {
    let failure = assert_one_failure(
        "2024-01-01 setup\n\
         \x20   a   $1.00\n\
         \x20   b  $-1.00\n\
         \n\
         2024-01-02 assert\n\
         \x20   a   0 EUR == 0 EUR\n\
         \x20   b   0 EUR\n",
    );
    assert_eq!(failure.commodity.0, "$");
    assert!(
        failure
            .message()
            .contains("the asserted balance is:       $0.00"),
        "{}",
        failure.message()
    );
}

// ---------------------------------------------------------------------------
// Costs, multiple postings, commodityless amounts
// ---------------------------------------------------------------------------

/// Assertions are checked "ignoring costs": the posting contributes 10 AAA, not
/// the $50.00 it cost. hledger accepts both forms.
#[test]
fn costs_are_ignored_by_assertions() {
    assert_clean(
        "2024-01-01 unit cost\n\
         \x20   assets:stock   10 AAA @ $5.00 = 10 AAA\n\
         \x20   assets:cash    $-50.00 = $-50.00\n",
    );
    assert_clean(
        "2024-01-01 total cost\n\
         \x20   assets:stock   10 AAA @@ $50.00 = 10 AAA\n\
         \x20   assets:cash    $-50.00 = $-50.00\n",
    );
}

// NOTE (upstream, `parse.rs`, not covered here): hledger 1.52 accepts a cost on
// the ASSERTED amount too — `a  10 AAA @ $5.00 = 10 AAA @ $5.00` parses and the
// assertion passes, costs ignored on both sides. Ledgeline does not get that
// far: `parse_amount_and_assertion` hands the assertion text to `parse_amount`
// rather than `parse_primary_and_cost`, so the `@` never splits off. No test is
// pinned here because the behaviour lives in `parse.rs` and belongs to the
// PARSE-4/PARSE-5 work; this pass is correct either way, since it reads
// `Amount::commodity`/`quantity` and never `Amount::cost`.

/// If costs leaked in, this would assert $50.00 and fail. It must not.
#[test]
fn a_cost_never_contributes_to_the_asserted_commoditys_balance() {
    let failure = assert_one_failure(
        "2024-01-01 t\n\
         \x20   assets:stock   10 AAA @ $5.00 = $50.00\n\
         \x20   assets:cash    $-50.00\n",
    );
    assert_eq!(failure.commodity.0, "$");
    assert!(
        failure
            .message()
            .contains("but the calculated balance is: $0"),
        "the stock account must hold no $ at all:\n{}",
        failure.message()
    );
}

/// Several postings to the same account inside one transaction accumulate in
/// posting order; hledger accepts this running sequence.
#[test]
fn assertions_accumulate_across_postings_to_the_same_account() {
    assert_clean(
        "2024-01-01 several\n\
         \x20   a   $1.00 = $1.00\n\
         \x20   a   $2.00 = $3.00\n\
         \x20   a   $4.00 = $7.00\n\
         \x20   b   $-7.00\n",
    );
}

/// ...and across transactions.
#[test]
fn assertions_accumulate_across_transactions() {
    let failure = assert_one_failure(
        "2024-01-01 one\n\
         \x20   a   $1.00 = $1.00\n\
         \x20   b   $-1.00\n\
         \n\
         2024-01-02 two\n\
         \x20   a   $2.00 = $3.00\n\
         \x20   b   $-2.00\n\
         \n\
         2024-01-03 three\n\
         \x20   a   $4.00 = $99.00\n\
         \x20   b   $-4.00\n",
    );
    assert!(
        failure
            .message()
            .contains("but the calculated balance is: $7.00"),
        "{}",
        failure.message()
    );
}

/// A bare `0` asserts the NO-SYMBOL commodity, which is zero here — so hledger
/// accepts this even though the account holds $2.00.
#[test]
fn a_bare_zero_asserts_the_commodityless_balance() {
    assert_clean(
        "2024-01-01 z\n\
         \x20   a   $1.00\n\
         \x20   b  $-1.00\n\
         \n\
         2024-01-02 zz\n\
         \x20   a   $1.00 = 0\n\
         \x20   b  $-1.00\n",
    );
}

/// Virtual (`(a)`) and balanced-virtual (`[a]`) postings both contribute to the
/// asserted account's running balance. hledger accepts both.
#[test]
fn virtual_postings_contribute_to_the_running_balance() {
    assert_clean(
        "2024-01-01 v\n\
         \x20   a       $1.00\n\
         \x20   b      $-1.00\n\
         \n\
         2024-01-02 unbalanced virtual\n\
         \x20   (a)     $5.00\n\
         \n\
         2024-01-03 chk\n\
         \x20   a       $0.00 = $6.00\n\
         \x20   b       $0.00\n",
    );
    assert_clean(
        "2024-01-01 v\n\
         \x20   a       $1.00\n\
         \x20   b      $-1.00\n\
         \n\
         2024-01-02 balanced virtual\n\
         \x20   [a]     $5.00\n\
         \x20   [c]    $-5.00\n\
         \n\
         2024-01-03 chk\n\
         \x20   a       $0.00 = $6.00\n\
         \x20   b       $0.00\n",
    );
}

// ---------------------------------------------------------------------------
// Evaluation order
// ---------------------------------------------------------------------------

/// hledger evaluates in DATE order, not file order. The February transaction is
/// written first but the January one is applied first, so `a` is already $1.00
/// when the `= $5.00` is checked and hledger reports a calculated $6.00.
#[test]
fn transactions_are_evaluated_in_date_order_not_file_order() {
    let failure = assert_one_failure(
        "2024-02-01 later date, written first\n\
         \x20   a   $5.00 = $5.00\n\
         \x20   b   $-5.00\n\
         \n\
         2024-01-01 earlier date, written second\n\
         \x20   a   $1.00 = $1.00\n\
         \x20   b   $-1.00\n",
    );
    assert!(
        failure
            .message()
            .contains("but the calculated balance is: $6.00"),
        "{}",
        failure.message()
    );
}

/// Within one date, file order decides. hledger reports the first-written
/// transaction's assertion as seeing only its own $5.00.
#[test]
fn same_date_transactions_keep_file_order() {
    let failure = assert_first_failure(
        "2024-01-01 written first\n\
         \x20   a   $5.00 = $6.00\n\
         \x20   b   $-5.00\n\
         \n\
         2024-01-01 written second\n\
         \x20   a   $1.00 = $1.00\n\
         \x20   b   $-1.00\n",
    );
    assert!(
        failure
            .message()
            .contains("but the calculated balance is: $5.00"),
        "{}",
        failure.message()
    );
}

/// The sort is over POSTINGS, not transactions: a posting's own `date:` tag
/// moves it in the sequence. Here the March transaction's posting is dated
/// 2024-01-05, so it lands before the February transaction and sees only its
/// own $5.00 — which is what hledger reports.
#[test]
fn a_posting_date_tag_moves_the_posting_in_the_sequence() {
    let failure = assert_first_failure(
        "2024-03-01 uses a posting date\n\
         \x20   a   $5.00 = $6.00  ; date:2024-01-05\n\
         \x20   b   $-5.00\n\
         \n\
         2024-02-01 later\n\
         \x20   a   $1.00 = $1.00\n\
         \x20   b   $-1.00\n",
    );
    assert!(
        failure
            .message()
            .contains("but the calculated balance is: $5.00"),
        "{}",
        failure.message()
    );
}

/// The decisive case for posting-level (rather than transaction-level) sorting:
/// t1's transaction date is LATER than t2's, but t1's posting date is EARLIER
/// than t2's posting date. A transaction-level sort would run t2 first and let
/// `= $11.00` pass; hledger runs t1's posting first, so it fails at $1.00.
#[test]
fn ordering_is_by_posting_date_not_transaction_date() {
    let failure = assert_one_failure(
        "2024-03-01 t1\n\
         \x20   a   $1.00 = $11.00\n\
         \x20   b   $-1.00\n\
         \n\
         2024-01-01 t2\n\
         \x20   a   $10.00  ; date:2024-05-01\n\
         \x20   b   $-10.00\n",
    );
    assert!(
        failure
            .message()
            .contains("but the calculated balance is: $1.00"),
        "{}",
        failure.message()
    );
    // The same journal with the transaction-order expectation instead.
    assert_clean(
        "2024-03-01 t1\n\
         \x20   a   $1.00 = $1.00\n\
         \x20   b   $-1.00\n\
         \n\
         2024-01-01 t2\n\
         \x20   a   $10.00  ; date:2024-05-01\n\
         \x20   b   $-10.00\n",
    );
}

/// Postings are reordered by `date:` even WITHIN a single transaction. hledger
/// accepts this: the 2024-01-02 posting is applied before the 2024-01-05 one,
/// despite being written second.
#[test]
fn posting_dates_reorder_postings_inside_one_transaction() {
    assert_clean(
        "2024-01-01 in-txn\n\
         \x20   a   $1.00 = $5.00  ; date:2024-01-05\n\
         \x20   a   $4.00 = $4.00  ; date:2024-01-02\n\
         \x20   b   $-5.00\n",
    );
}

/// When a posting date ties with another posting's transaction date, file order
/// breaks the tie. hledger reports t1 (written first) seeing only its own $1.00.
#[test]
fn a_posting_date_tying_a_transaction_date_falls_back_to_file_order() {
    let failure = assert_one_failure(
        "2024-01-02 t1\n\
         \x20   a   $1.00 = $11.00\n\
         \x20   b   $-1.00\n\
         \n\
         2024-01-01 t2\n\
         \x20   a   $10.00  ; date:2024-01-02\n\
         \x20   b   $-10.00\n",
    );
    assert!(
        failure
            .message()
            .contains("but the calculated balance is: $1.00"),
        "{}",
        failure.message()
    );
}

/// A secondary date (`2024-03-01=2024-01-01`) never participates in the
/// ordering; hledger accepts this because the primary dates decide.
#[test]
fn secondary_dates_do_not_affect_ordering() {
    assert_clean(
        "2024-03-01=2024-01-01 t1\n\
         \x20   a   $1.00\n\
         \x20   b   $-1.00\n\
         \n\
         2024-02-01 t2\n\
         \x20   a   $5.00 = $5.00\n\
         \x20   b   $-5.00\n",
    );
}

// ---------------------------------------------------------------------------
// `include` ordering, and where a failure is reported
// ---------------------------------------------------------------------------

/// Parse a real on-disk main journal so `include` resolution and per-file source
/// positions are exercised.
fn journal_at(main: &Path) -> Journal {
    parse_journal_with_overrides(&main.to_string_lossy(), &HashMap::new())
        .expect("main journal parses")
}

/// Same-date transactions across an `include` are evaluated in READ order: the
/// included file's transactions land at the point of the `include` directive.
/// With the include first, hledger accepts $7.00 then $10.00.
#[test]
fn included_transactions_are_evaluated_at_the_include_point() {
    let dir = scratch("include_first");
    write(
        &dir,
        "child.journal",
        "2024-01-01 from-include\n\
         \x20   a   $7.00 = $7.00\n\
         \x20   b   $-7.00\n",
    );
    let main = write(
        &dir,
        "main.journal",
        "include child.journal\n\
         2024-01-01 main-after-include\n\
         \x20   a   $3.00 = $10.00\n\
         \x20   b   $-3.00\n",
    );
    let found = check_balance_assertions(&journal_at(&main)).expect("no overflow");
    assert!(found.is_empty(), "{}", rendered(&found));
}

/// With the include last, the main transaction runs first — so the included
/// file's `= $7.00` sees $10.00 and fails. hledger reports the error against the
/// INCLUDED file's path and its own line number, which this reproduces.
#[test]
fn a_failure_in_an_included_file_names_that_file() {
    let dir = scratch("include_last");
    let child = write(
        &dir,
        "child.journal",
        "2024-01-01 from-include\n\
         \x20   a   $7.00 = $7.00\n\
         \x20   b   $-7.00\n",
    );
    let main = write(
        &dir,
        "main.journal",
        "2024-01-01 main-before-include\n\
         \x20   a   $3.00 = $3.00\n\
         \x20   b   $-3.00\n\
         include child.journal\n",
    );
    let mut found = check_balance_assertions(&journal_at(&main)).expect("no overflow");
    assert_eq!(found.len(), 1, "{}", rendered(&found));
    let failure = found.remove(0);
    assert_eq!(
        failure.source_file.canonicalize().ok(),
        child.canonicalize().ok(),
        "the failure must name the included file, not the main journal"
    );
    assert_eq!(
        failure.position.line, 2,
        "the line must be relative to the included file"
    );
    assert!(
        failure
            .message()
            .contains("but the calculated balance is: $10.00"),
        "{}",
        failure.message()
    );
}

// ---------------------------------------------------------------------------
// Collecting rather than aborting
// ---------------------------------------------------------------------------

/// The pass reports every failure, not just the first — hledger aborts at the
/// first, so only failure #1 has a direct hledger counterpart. Independent
/// accounts are the case this buys us.
#[test]
fn independent_failures_are_all_reported() {
    let found = failures(
        "2024-01-01 t\n\
         \x20   a   $1.00 = $99.00\n\
         \x20   b   $2.00 = $88.00\n\
         \x20   c   $-3.00\n",
    );
    let accounts: Vec<&str> = found
        .iter()
        .map(|failure| failure.account.0.as_str())
        .collect();
    assert_eq!(accounts, ["a", "b"]);
}

/// Within one posting the explicitly asserted commodity is reported first, then
/// the `==` zero-check commodities in lexical order — so failure #1 is always
/// the one hledger would have printed.
#[test]
fn the_first_failure_for_a_posting_is_the_one_hledger_reports() {
    let found = failures(
        "2024-01-01 setup\n\
         \x20   a   $1.00\n\
         \x20   a   1 EUR\n\
         \x20   a   1 GBP\n\
         \x20   eq  $-1.00\n\
         \x20   eq  -1 EUR\n\
         \x20   eq  -1 GBP\n\
         \n\
         2024-01-02 assert\n\
         \x20   a   $0.00 == $99.00\n\
         \x20   eq  $0.00\n",
    );
    let commodities: Vec<&str> = found
        .iter()
        .map(|failure| failure.commodity.0.as_str())
        .collect();
    assert_eq!(commodities, ["$", "EUR", "GBP"]);
}

// ---------------------------------------------------------------------------
// The fixture sweep — the primary correctness signal
// ---------------------------------------------------------------------------

/// Every fixture ROOT outside `fixtures/corpus/errors/` passes real
/// `hledger -f FILE check assertions`, so this pass must flag none of them. If
/// it flags one, the pass is wrong.
///
/// `fixtures/corpus/assertions.journal` and `assertions-total.journal` make this
/// more than a smoke test: they are the corpus' own assertion coverage.
///
/// # Roots only, and why that is not a loophole
///
/// A file that is only ever `include`d is checked **through its parent** and not
/// on its own, which is what [`collect_journals`] has always said and what the
/// sweep now does. The distinction used to be invisible because no included
/// fragment carried an assertion; `fixtures/import/layouts/split-year-assert/`
/// exists precisely because one does.
///
/// Its `2026/2026.journal` opens with a start-of-year assertion holding the
/// prior year's closing balance. Read from its root that balance is `$900.00`
/// and the assertion holds; read alone the running balance is `$0` and it fails
/// — in hledger exactly as here. Flagging it would be flagging hledger's own
/// answer to a question nobody asks, and it would mean this sweep could never
/// contain a split journal that asserts anything.
///
/// The tree is still covered, in full: its root includes both year files, so
/// every assertion in it is evaluated here, in the context it was written for.
#[test]
fn no_shipped_fixture_has_a_failing_assertion() {
    let fixtures = common::fixtures_dir();
    let mut journals = Vec::new();
    collect_journals(&fixtures, &mut journals);
    journals.sort();

    let parsed: Vec<(&PathBuf, Journal)> = journals
        .iter()
        .map(|path| (path, journal_at(path)))
        .collect();
    // Every file reached through somebody else's `include`. `source_files` is
    // the parse's own record of what it read, so this needs no second opinion
    // about how an include resolves.
    let included: std::collections::BTreeSet<&Path> = parsed
        .iter()
        .flat_map(|(path, journal)| {
            journal
                .source_files
                .iter()
                .map(PathBuf::as_path)
                .filter(move |source| source != &path.as_path())
        })
        .collect();
    let roots: Vec<&(&PathBuf, Journal)> = parsed
        .iter()
        .filter(|(path, _)| !included.contains(path.as_path()))
        .collect();
    assert!(
        roots.len() >= 40,
        "expected the full fixture set, found {} roots of {}",
        roots.len(),
        parsed.len()
    );
    assert!(
        !included.is_empty(),
        "no fixture is included by another — the filter below is not being exercised"
    );

    let mut problems = Vec::new();
    let mut asserting = 0usize;
    for (path, journal) in &roots {
        let count = journal
            .transactions
            .iter()
            .flat_map(|txn| &txn.postings)
            .filter(|posting| posting.balance_assertion.is_some())
            .count();
        asserting += count;
        match check_balance_assertions(journal) {
            Ok(found) if found.is_empty() => {}
            Ok(found) => problems.push(format!("{}:\n{}", path.display(), rendered(&found))),
            Err(error) => problems.push(format!("{}: {error}", path.display())),
        }
    }
    println!(
        "assertion sweep: {} roots checked ({} files, {} included), {asserting} assertions \
         evaluated, {} flagged",
        roots.len(),
        parsed.len(),
        included.len(),
        problems.len()
    );
    assert!(
        problems.is_empty(),
        "hledger accepts every one of these; the pass must too:\n{}",
        problems.join("\n")
    );
    assert!(
        asserting > 0,
        "the sweep evaluated no assertions at all — it is not testing anything"
    );
}

/// The complement of the sweep: the one committed fragment that **fails** on its
/// own, and passes through its root.
///
/// Without this, "roots only" would be indistinguishable from "skip the
/// inconvenient file". Both halves are asserted, because either alone proves
/// nothing — and this is our engine agreeing with hledger, which
/// `ledgeline-core/tests/journals.rs` checks against the binary itself.
#[test]
fn an_included_fragment_may_fail_alone_while_its_root_passes() {
    let tree = common::fixtures_dir().join("import/layouts/split-year-assert");

    let fragment = journal_at(&tree.join("2026/2026.journal"));
    let found = check_balance_assertions(&fragment).expect("the fragment evaluates");
    assert_eq!(
        found.len(),
        1,
        "the start-of-year assertion cannot hold without the prior year: {}",
        rendered(&found)
    );

    let root = journal_at(&tree.join("main.journal"));
    assert_eq!(
        check_balance_assertions(&root).expect("the root evaluates"),
        Vec::new(),
        "and through the root, where the prior year is in scope, it holds"
    );
}

/// Every `*.journal` under `dir`, recursively, excluding `corpus/errors/`
/// (journals hledger rejects outright) and any file that is only ever `include`d
/// (it is covered through its parent, and on its own it may not balance).
fn collect_journals(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "errors") {
                continue;
            }
            collect_journals(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "journal") {
            out.push(path);
        }
    }
}
