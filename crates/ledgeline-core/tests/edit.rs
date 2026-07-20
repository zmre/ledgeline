//! Integration, golden, and property tests for the journal write path
//! ([`ledgeline_core::edit`]).
//!
//! The property test is the key safety invariant: parse a journal, delete (or
//! append) a transaction, re-parse, and assert every *untouched* transaction's
//! source text is byte-identical to before — over a set of fixture journals and
//! every deletable position.

use ledgeline_core::decimal::Dec;
use ledgeline_core::edit::{EditError, InsertPosition, JournalEditor, format_transaction};
use ledgeline_core::model::{
    AccountName, Amount, AmountStyle, Commodity, CommoditySide, Cost, CostKind, Posting,
    PostingType, SourcePos, Status, Tindex, Transaction,
};
use proptest::prelude::*;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Builders + fixtures
// ---------------------------------------------------------------------------

fn dollar_style() -> AmountStyle {
    AmountStyle {
        side: CommoditySide::Left,
        spaced: false,
        decimal_mark: Some('.'),
        digit_groups: None,
        precision: 2,
    }
}

fn dollars(mantissa: i128, places: u32) -> Amount {
    Amount {
        commodity: Commodity("$".into()),
        quantity: Dec::new(mantissa, places),
        style: dollar_style(),
        cost: None,
    }
}

/// A regular posting; `amount` `None` means an elided (inferred) leg.
fn leg(account: &str, amount: Option<Amount>) -> Posting {
    Posting {
        status: Status::Unmarked,
        ptype: PostingType::Regular,
        account: AccountName(account.into()),
        amounts: amount.into_iter().collect(),
        balance_assertion: None,
        date: None,
        date2: None,
        comment: String::new(),
        tags: vec![],
    }
}

/// A cleared `$` transaction with the given legs.
fn cash_txn(date: &str, description: &str, postings: Vec<Posting>) -> Transaction {
    Transaction {
        index: Tindex(1),
        date: date.into(),
        date2: None,
        status: Status::Cleared,
        code: String::new(),
        description: description.into(),
        comment: String::new(),
        preceding_comment: String::new(),
        tags: vec![],
        postings,
        // The span is recomputed on reparse; a placeholder is fine for an input.
        source_span: (
            SourcePos { line: 1, column: 1 },
            SourcePos { line: 1, column: 1 },
        ),
        // The editor keys the added transaction by the file it lands in, not by
        // this placeholder; a fresh input carries no source file.
        source_file: PathBuf::new(),
    }
}

const THREE_TXNS: &str = "\
2024-01-01 * A
    expenses:a  $1.00
    assets:bank

2024-01-02 * B
    expenses:b  $2.00
    assets:bank

2024-01-03 * C
    expenses:c  $3.00
    assets:bank
";

const WITH_DIRECTIVES: &str = "\
; a small ledger
account assets:bank    ; type: A
commodity $1,000.00

2024-01-01 * Opening
    assets:bank  $100.00
    equity:opening

2024-01-15 * Coffee  ; treat: yes
    expenses:coffee  $4.50
    assets:bank
";

const TRAILING_COMMENT: &str = "\
2024-01-01 * A
    expenses:a  $1.00
    assets:bank
    ; a note trailing the postings

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
";

/// A ledger whose header and postings all carry same-line comments, so the
/// surgical edits can be checked for comment/whitespace preservation.
const WITH_COMMENTS: &str = "\
2024-01-01 * A  ; first txn
    expenses:a  $1.00  ; the expense
    assets:bank  ; from checking

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
";

fn sample_journal_text() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample.journal");
    std::fs::read_to_string(path).expect("sample.journal readable")
}

/// Fixture journals used by the property tests, cached so the file is read once.
static FIXTURES: LazyLock<Vec<(String, String)>> = LazyLock::new(|| {
    vec![
        ("three".to_string(), THREE_TXNS.to_string()),
        ("directives".to_string(), WITH_DIRECTIVES.to_string()),
        ("trailing-comment".to_string(), TRAILING_COMMENT.to_string()),
        ("sample".to_string(), sample_journal_text()),
    ]
});

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Write `content` to a unique temp file and return its path.
fn write_temp(prefix: &str, content: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join("ledgeline-edit-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{prefix}-{}-{seq}.journal", std::process::id()));
    std::fs::write(&path, content).expect("write temp journal");
    path
}

/// The source text of every transaction, in file order.
fn ordered_sources(editor: &JournalEditor) -> Vec<String> {
    editor
        .journal()
        .transactions
        .iter()
        .map(|t| editor.transaction_source(t.index).expect("source"))
        .collect()
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

#[test]
fn delete_middle_leaves_single_blank_and_identical_neighbors() {
    let mut editor = JournalEditor::from_text("mem.journal", THREE_TXNS).unwrap();
    let before = ordered_sources(&editor);
    // Delete B (the 2nd transaction).
    editor.delete_transaction(Tindex(2)).unwrap();
    assert_eq!(
        editor.text(),
        "\
2024-01-01 * A
    expenses:a  $1.00
    assets:bank

2024-01-03 * C
    expenses:c  $3.00
    assets:bank
"
    );
    // A and C are byte-identical to before.
    let after = ordered_sources(&editor);
    assert_eq!(after, vec![before[0].clone(), before[2].clone()]);
}

#[test]
fn delete_first_transaction_keeps_directives_and_neighbor() {
    let mut editor = JournalEditor::from_text("mem.journal", WITH_DIRECTIVES).unwrap();
    let coffee_src = editor.transaction_source(Tindex(2)).unwrap();
    editor.delete_transaction(Tindex(1)).unwrap();
    let text = editor.text();
    // Directives survive; the Coffee txn is intact and byte-identical.
    assert!(text.starts_with("; a small ledger\naccount assets:bank"));
    assert!(text.contains("2024-01-15 * Coffee"));
    assert!(!text.contains("Opening"));
    assert_eq!(editor.transaction_source(Tindex(1)).unwrap(), coffee_src);
    assert_eq!(editor.transaction_count(), 1);
}

#[test]
fn delete_last_transaction() {
    let mut editor = JournalEditor::from_text("mem.journal", THREE_TXNS).unwrap();
    editor.delete_transaction(Tindex(3)).unwrap();
    assert_eq!(
        editor.text(),
        "\
2024-01-01 * A
    expenses:a  $1.00
    assets:bank

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
"
    );
    assert_eq!(editor.transaction_count(), 2);
}

#[test]
fn delete_consumes_trailing_in_transaction_comment() {
    let mut editor = JournalEditor::from_text("mem.journal", TRAILING_COMMENT).unwrap();
    let b_src = editor.transaction_source(Tindex(2)).unwrap();
    editor.delete_transaction(Tindex(1)).unwrap();
    // A's trailing "; a note" line goes with A; B remains byte-identical.
    assert_eq!(
        editor.text(),
        "\
2024-01-02 * B
    expenses:b  $2.00
    assets:bank
"
    );
    assert_eq!(editor.transaction_source(Tindex(1)).unwrap(), b_src);
}

#[test]
fn delete_missing_transaction_errors_and_leaves_state() {
    let mut editor = JournalEditor::from_text("mem.journal", THREE_TXNS).unwrap();
    let before = editor.text();
    let err = editor.delete_transaction(Tindex(99)).unwrap_err();
    assert!(matches!(err, EditError::TransactionNotFound(99)));
    assert_eq!(editor.text(), before);
    assert_eq!(editor.transaction_count(), 3);
}

// ---------------------------------------------------------------------------
// Add
// ---------------------------------------------------------------------------

#[test]
fn append_adds_txn_at_eof_with_one_blank() {
    let single = "\
2024-01-01 * A
    expenses:a  $1.00
    assets:bank
";
    let mut editor = JournalEditor::from_text("mem.journal", single).unwrap();
    let new_txn = cash_txn(
        "2024-02-01",
        "B",
        vec![
            leg("expenses:b", Some(dollars(200, 2))),
            leg("assets:bank", None),
        ],
    );
    editor
        .add_transaction(&new_txn, InsertPosition::Append)
        .unwrap();
    assert_eq!(
        editor.text(),
        "\
2024-01-01 * A
    expenses:a  $1.00
    assets:bank

2024-02-01 * B
    expenses:b  $2.00
    assets:bank
"
    );
    assert_eq!(editor.transaction_count(), 2);
}

#[test]
fn date_ordered_inserts_between_neighbors() {
    let two = "\
2024-01-01 * A
    expenses:a  $1.00
    assets:bank

2024-03-01 * C
    expenses:c  $3.00
    assets:bank
";
    let mut editor = JournalEditor::from_text("mem.journal", two).unwrap();
    let a_src = editor.transaction_source(Tindex(1)).unwrap();
    let c_src = editor.transaction_source(Tindex(2)).unwrap();
    let new_txn = cash_txn(
        "2024-02-01",
        "B",
        vec![
            leg("expenses:b", Some(dollars(200, 2))),
            leg("assets:bank", None),
        ],
    );
    editor
        .add_transaction(&new_txn, InsertPosition::DateOrdered)
        .unwrap();
    assert_eq!(
        editor.text(),
        "\
2024-01-01 * A
    expenses:a  $1.00
    assets:bank

2024-02-01 * B
    expenses:b  $2.00
    assets:bank

2024-03-01 * C
    expenses:c  $3.00
    assets:bank
"
    );
    // Neighbors byte-identical; B is spliced in between.
    assert_eq!(editor.transaction_source(Tindex(1)).unwrap(), a_src);
    assert_eq!(editor.transaction_source(Tindex(3)).unwrap(), c_src);
}

#[test]
fn add_unbalanced_transaction_is_rejected() {
    let mut editor = JournalEditor::from_text("mem.journal", THREE_TXNS).unwrap();
    let before = editor.text();
    // Two explicit legs that do not sum to zero, no elided leg to absorb it.
    let bad = cash_txn(
        "2024-06-01",
        "bad",
        vec![
            leg("expenses:x", Some(dollars(500, 2))),
            leg("assets:bank", Some(dollars(-400, 2))),
        ],
    );
    let err = editor
        .add_transaction(&bad, InsertPosition::Append)
        .unwrap_err();
    assert!(matches!(err, EditError::Unbalanced));
    assert_eq!(editor.text(), before);
    assert_eq!(editor.transaction_count(), 3);
}

#[test]
fn add_with_invalid_date_is_rejected_by_reparse_validate() {
    let mut editor = JournalEditor::from_text("mem.journal", THREE_TXNS).unwrap();
    let before = editor.text();
    // Balances fine, but the date is not a valid calendar date, so the reparse
    // of the mutated text fails and the edit is refused.
    let bad = cash_txn(
        "2024-13-40",
        "bad date",
        vec![
            leg("expenses:x", Some(dollars(100, 2))),
            leg("assets:bank", None),
        ],
    );
    let err = editor
        .add_transaction(&bad, InsertPosition::Append)
        .unwrap_err();
    assert!(matches!(err, EditError::ParseInvalidAfterEdit(_)));
    assert_eq!(editor.text(), before);
}

#[test]
fn add_with_mismatched_decimal_mark_is_caught_by_round_trip_guard() {
    // The journal declares EUR with a comma decimal mark. If the caller builds a
    // EUR amount claiming a '.' mark, the formatted "1234.50 EUR" would re-parse
    // (using EUR's canonical ',') to 123450 — a silent 100x corruption. The
    // round-trip guard must catch it.
    let journal = "\
commodity 1.000,00 EUR

2024-01-01 * A
    expenses:a  10,00 EUR
    assets:wise
";
    let mut editor = JournalEditor::from_text("mem.journal", journal).unwrap();
    let before = editor.text();
    let wrong_style = AmountStyle {
        side: CommoditySide::Right,
        spaced: true,
        decimal_mark: Some('.'), // WRONG: EUR uses ','
        digit_groups: None,
        precision: 2,
    };
    let bad = Transaction {
        postings: vec![
            Posting {
                amounts: vec![Amount {
                    commodity: Commodity("EUR".into()),
                    quantity: Dec::new(123_450, 2), // 1234.50
                    style: wrong_style,
                    cost: None,
                }],
                ..leg("expenses:travel", None)
            },
            leg("assets:wise", None),
        ],
        ..cash_txn("2024-02-01", "Hotel", vec![])
    };
    let err = editor
        .add_transaction(&bad, InsertPosition::Append)
        .unwrap_err();
    assert!(matches!(err, EditError::RoundTripMismatch));
    assert_eq!(editor.text(), before);
}

#[test]
fn add_multi_commodity_posting_is_unsupported() {
    let mut editor = JournalEditor::from_text("mem.journal", THREE_TXNS).unwrap();
    let mut posting = leg("assets:mixed", Some(dollars(100, 2)));
    posting.amounts.push(Amount {
        commodity: Commodity("EUR".into()),
        quantity: Dec::new(100, 2),
        style: dollar_style(),
        cost: None,
    });
    let bad = cash_txn("2024-06-01", "mixed", vec![posting, leg("equity:x", None)]);
    let err = editor
        .add_transaction(&bad, InsertPosition::Append)
        .unwrap_err();
    assert!(matches!(err, EditError::Unsupported(_)));
}

#[test]
fn add_then_reparse_round_trips_a_cost_transaction() {
    // A buy with a unit cost appended to the sample journal balances and parses.
    let text = sample_journal_text();
    let mut editor = JournalEditor::from_text("sample.journal", &text).unwrap();
    let count = editor.transaction_count();
    let aapl_style = AmountStyle {
        side: CommoditySide::Right,
        spaced: true,
        decimal_mark: Some('.'),
        digit_groups: None,
        precision: 0,
    };
    let buy = cash_txn(
        "2026-07-15",
        "Fidelity | buy AAPL",
        vec![
            Posting {
                amounts: vec![Amount {
                    commodity: Commodity("AAPL".into()),
                    quantity: Dec::new(2, 0),
                    style: aapl_style,
                    cost: Some(Box::new(Cost {
                        kind: CostKind::Unit,
                        amount: dollars(27000, 2),
                    })),
                }],
                ..leg("assets:broker:taxable:aapl", None)
            },
            leg("assets:broker:taxable:cash", None),
        ],
    );
    editor
        .add_transaction(&buy, InsertPosition::Append)
        .unwrap();
    assert_eq!(editor.transaction_count(), count + 1);
    let added = editor.journal().transactions.last().unwrap();
    assert_eq!(added.description, "Fidelity | buy AAPL");
    assert_eq!(added.postings[0].amounts[0].quantity, Dec::new(2, 0));
    // The inferred cash leg is -$540.00.
    assert_eq!(added.postings[1].amounts[0].quantity, Dec::new(-54000, 2));
}

// ---------------------------------------------------------------------------
// Surgical edits: set_description / set_posting_account
// ---------------------------------------------------------------------------

#[test]
fn set_description_rewrites_only_header_and_preserves_postings_and_comments() {
    let mut editor = JournalEditor::from_text("mem.journal", WITH_COMMENTS).unwrap();
    let b_src = editor.transaction_source(Tindex(2)).unwrap();

    editor.set_description(Tindex(1), "A renamed").unwrap();

    // Only the header's description changed; its comment and every posting line
    // (accounts, amounts, comments, whitespace) are byte-identical.
    assert_eq!(
        editor.text(),
        "\
2024-01-01 * A renamed  ; first txn
    expenses:a  $1.00  ; the expense
    assets:bank  ; from checking

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
"
    );
    // Neighbor B is untouched.
    assert_eq!(editor.transaction_source(Tindex(2)).unwrap(), b_src);
    assert_eq!(editor.journal().transactions[0].description, "A renamed");
}

#[test]
fn set_description_rejects_semicolon_and_leaves_state() {
    let mut editor = JournalEditor::from_text("mem.journal", WITH_COMMENTS).unwrap();
    let before = editor.text();
    // A ';' would parse as a comment, so the description would not round-trip.
    let err = editor.set_description(Tindex(1), "A ; sneaky").unwrap_err();
    assert!(matches!(err, EditError::RoundTripMismatch));
    assert_eq!(editor.text(), before);
}

#[test]
fn set_status_rewrites_only_the_header_marker_and_preserves_the_rest() {
    let mut editor = JournalEditor::from_text("mem.journal", WITH_COMMENTS).unwrap();
    let b_src = editor.transaction_source(Tindex(2)).unwrap();

    // Cleared (`*`) -> Pending (`!`): only the marker char changes; the date, the
    // description, the header comment, and BOTH posting lines (accounts, amounts,
    // comments, whitespace) stay byte-identical.
    editor.set_status(Tindex(1), Status::Pending).unwrap();
    assert_eq!(
        editor.text(),
        "\
2024-01-01 ! A  ; first txn
    expenses:a  $1.00  ; the expense
    assets:bank  ; from checking

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
"
    );
    assert_eq!(editor.journal().transactions[0].status, Status::Pending);

    // Pending -> Unmarked drops the marker entirely (no dangling space before `A`).
    editor.set_status(Tindex(1), Status::Unmarked).unwrap();
    assert_eq!(
        editor.text(),
        "\
2024-01-01 A  ; first txn
    expenses:a  $1.00  ; the expense
    assets:bank  ; from checking

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
"
    );
    assert_eq!(editor.journal().transactions[0].status, Status::Unmarked);

    // Unmarked -> Cleared restores the original bytes exactly.
    editor.set_status(Tindex(1), Status::Cleared).unwrap();
    assert_eq!(editor.text(), WITH_COMMENTS);
    assert_eq!(editor.journal().transactions[0].status, Status::Cleared);

    // Neighbor B was byte-identical through every rewrite.
    assert_eq!(editor.transaction_source(Tindex(2)).unwrap(), b_src);
}

#[test]
fn set_status_unknown_transaction_is_transaction_not_found() {
    let mut editor = JournalEditor::from_text("mem.journal", WITH_COMMENTS).unwrap();
    let before = editor.text();
    let err = editor.set_status(Tindex(99), Status::Pending).unwrap_err();
    assert!(matches!(err, EditError::TransactionNotFound(99)));
    assert_eq!(editor.text(), before);
}

#[test]
fn set_posting_account_replaces_only_the_account_token() {
    let mut editor = JournalEditor::from_text("mem.journal", WITH_COMMENTS).unwrap();
    let b_src = editor.transaction_source(Tindex(2)).unwrap();

    editor
        .set_posting_account(Tindex(1), 0, "expenses:groceries")
        .unwrap();

    // Only "expenses:a" -> "expenses:groceries"; the amount, its 2-space gap, and
    // the trailing comment are all preserved byte-for-byte.
    assert_eq!(
        editor.text(),
        "\
2024-01-01 * A  ; first txn
    expenses:groceries  $1.00  ; the expense
    assets:bank  ; from checking

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
"
    );
    assert_eq!(editor.transaction_source(Tindex(2)).unwrap(), b_src);
    assert_eq!(
        editor.journal().transactions[0].postings[0].account.0,
        "expenses:groceries"
    );
}

#[test]
fn set_posting_account_edits_the_elided_leg_only() {
    let mut editor = JournalEditor::from_text("mem.journal", WITH_COMMENTS).unwrap();
    // Posting 1 is the elided (amount-less) leg "assets:bank" with a comment.
    editor
        .set_posting_account(Tindex(1), 1, "assets:savings")
        .unwrap();
    assert_eq!(
        editor.text(),
        "\
2024-01-01 * A  ; first txn
    expenses:a  $1.00  ; the expense
    assets:savings  ; from checking

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
"
    );
}

#[test]
fn set_posting_account_out_of_range_is_posting_not_found() {
    let mut editor = JournalEditor::from_text("mem.journal", WITH_COMMENTS).unwrap();
    let before = editor.text();
    let err = editor
        .set_posting_account(Tindex(1), 9, "assets:x")
        .unwrap_err();
    assert!(matches!(
        err,
        EditError::PostingNotFound { txn: 1, posting: 9 }
    ));
    assert_eq!(editor.text(), before);
}

#[test]
fn set_posting_account_skips_interleaved_comment_line() {
    // A standalone comment line sits between the two postings; the second posting
    // must still be located positionally (comment lines are not postings).
    let journal = "\
2024-01-01 * A
    expenses:a  $1.00
    ; a note between postings
    assets:bank
";
    let mut editor = JournalEditor::from_text("mem.journal", journal).unwrap();
    editor
        .set_posting_account(Tindex(1), 1, "assets:savings")
        .unwrap();
    assert_eq!(
        editor.text(),
        "\
2024-01-01 * A
    expenses:a  $1.00
    ; a note between postings
    assets:savings
"
    );
}

// ---------------------------------------------------------------------------
// Full replace: replace_transaction (in place, comment-preserving)
// ---------------------------------------------------------------------------

#[test]
fn replace_transaction_keeps_neighbors_byte_identical_and_preserves_comments() {
    let mut editor = JournalEditor::from_text("mem.journal", WITH_COMMENTS).unwrap();
    let b_src = editor.transaction_source(Tindex(2)).unwrap();

    // Rebuild A with a new description/amount but keep the header + posting
    // comments, so the full replace must round-trip them.
    let mut expense = leg("expenses:a", Some(dollars(150, 2)));
    expense.comment = "the expense\n".into();
    let mut cash = leg("assets:bank", None);
    cash.comment = "from checking\n".into();
    let mut replacement = cash_txn("2024-01-01", "A replaced", vec![expense, cash]);
    replacement.comment = "first txn\n".into();

    editor.replace_transaction(Tindex(1), &replacement).unwrap();

    assert_eq!(
        editor.text(),
        "\
2024-01-01 * A replaced  ; first txn
    expenses:a  $1.50  ; the expense
    assets:bank  ; from checking

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
"
    );
    // Neighbor B is byte-identical.
    assert_eq!(editor.transaction_source(Tindex(2)).unwrap(), b_src);
    // The elided leg was filled in on reparse: -$1.50.
    assert_eq!(
        editor.journal().transactions[0].postings[1].amounts[0].quantity,
        Dec::new(-150, 2)
    );
}

#[test]
fn replace_transaction_unbalanced_is_rejected_and_leaves_state() {
    let mut editor = JournalEditor::from_text("mem.journal", THREE_TXNS).unwrap();
    let before = editor.text();
    let bad = cash_txn(
        "2024-01-02",
        "bad",
        vec![
            leg("expenses:x", Some(dollars(500, 2))),
            leg("assets:bank", Some(dollars(-400, 2))),
        ],
    );
    let err = editor.replace_transaction(Tindex(2), &bad).unwrap_err();
    assert!(matches!(err, EditError::Unbalanced));
    assert_eq!(editor.text(), before);
}

#[test]
fn replace_transaction_missing_index_is_not_found() {
    let mut editor = JournalEditor::from_text("mem.journal", THREE_TXNS).unwrap();
    let ok = cash_txn(
        "2024-01-02",
        "x",
        vec![
            leg("expenses:x", Some(dollars(100, 2))),
            leg("assets:bank", None),
        ],
    );
    let err = editor.replace_transaction(Tindex(99), &ok).unwrap_err();
    assert!(matches!(err, EditError::TransactionNotFound(99)));
}

// ---------------------------------------------------------------------------
// Save / external-change guard / atomic write
// ---------------------------------------------------------------------------

#[test]
fn save_writes_atomically_and_round_trips() {
    let path = write_temp("save", THREE_TXNS);
    let mut editor = JournalEditor::open(&path).unwrap();
    editor.delete_transaction(Tindex(2)).unwrap();
    editor.save().unwrap();

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, editor.text());
    // Re-open to confirm the saved file parses and reflects the delete.
    let reopened = JournalEditor::open(&path).unwrap();
    assert_eq!(reopened.transaction_count(), 2);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn save_refuses_when_file_changed_externally() {
    let path = write_temp("external", THREE_TXNS);
    let mut editor = JournalEditor::open(&path).unwrap();
    editor.delete_transaction(Tindex(2)).unwrap();

    // Simulate a concurrent external edit with DIFFERENT content.
    std::fs::write(&path, "2099-01-01 * external\n    a  $1\n    b\n").unwrap();
    let err = editor.save().unwrap_err();
    assert!(matches!(err, EditError::ExternalChange));

    // The external content is preserved — we did not clobber it.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("external"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn save_allowed_after_content_preserving_touch() {
    let path = write_temp("touch", THREE_TXNS);
    let mut editor = JournalEditor::open(&path).unwrap();
    editor.delete_transaction(Tindex(1)).unwrap();

    // Rewrite the SAME bytes (mtime changes, content hash does not).
    std::fs::write(&path, THREE_TXNS).unwrap();
    // The content-hash guard confirms the file is unchanged, so save proceeds.
    editor.save().unwrap();
    let reopened = JournalEditor::open(&path).unwrap();
    assert_eq!(reopened.transaction_count(), 2);
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// Multi-file journals: editing a transaction that lives in an `include`d file
// touches ONLY that file, leaving the main journal + other includes identical.
// ---------------------------------------------------------------------------

/// A fresh temp dir holding a main journal that `include`s two others; returns
/// `(dir, main, sub, other)`. The three transactions get tindex 1 (main), 2
/// (sub), 3 (other) in file order. Each included transaction's `source_span` is
/// relative to its own file.
fn write_multifile() -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ledgeline-mf-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let main = dir.join("main.journal");
    let sub = dir.join("sub.journal");
    let other = dir.join("other.journal");
    std::fs::write(
        &main,
        "\
account assets:bank

2024-01-01 * Main
    expenses:main  $10.00
    assets:bank

include sub.journal
include other.journal
",
    )
    .expect("write main");
    std::fs::write(
        &sub,
        "\
2024-02-01 * Sub
    expenses:sub  $20.00
    assets:bank
",
    )
    .expect("write sub");
    std::fs::write(
        &other,
        "\
2024-03-01 * Other
    expenses:other  $30.00
    assets:bank
",
    )
    .expect("write other");
    (dir, main, sub, other)
}

/// Run `edit` against a multi-file journal opened at `main`, save, and assert the
/// main file and the `other` include are byte-identical to before while `sub`
/// (the edited include) now satisfies `sub_check`. Returns nothing; panics on a
/// broken invariant. Keeps every op's before/after boilerplate DRY.
fn edit_only_touches_sub(edit: impl FnOnce(&mut JournalEditor), sub_check: impl FnOnce(&str)) {
    let (dir, main, sub, other) = write_multifile();
    let main_before = std::fs::read_to_string(&main).unwrap();
    let other_before = std::fs::read_to_string(&other).unwrap();

    let mut editor = JournalEditor::open(&main).unwrap();
    assert_eq!(editor.transaction_count(), 3);
    edit(&mut editor);
    editor.save().unwrap();

    // Only sub.journal was written; main + other are byte-identical on disk.
    assert_eq!(
        std::fs::read_to_string(&main).unwrap(),
        main_before,
        "main.journal must be byte-identical"
    );
    assert_eq!(
        std::fs::read_to_string(&other).unwrap(),
        other_before,
        "other.journal must be byte-identical"
    );
    sub_check(&std::fs::read_to_string(&sub).unwrap());

    // The whole journal still reparses from disk.
    assert!(JournalEditor::open(&main).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delete_included_txn_writes_only_that_include() {
    edit_only_touches_sub(
        |editor| {
            editor.delete_transaction(Tindex(2)).unwrap();
            assert_eq!(editor.transaction_count(), 2);
        },
        |sub_after| {
            assert!(
                !sub_after.contains("Sub"),
                "sub still has txn:\n{sub_after}"
            )
        },
    );
}

#[test]
fn set_description_on_included_txn_writes_only_that_include() {
    edit_only_touches_sub(
        |editor| editor.set_description(Tindex(2), "Sub renamed").unwrap(),
        |sub_after| {
            assert_eq!(
                sub_after,
                "\
2024-02-01 * Sub renamed
    expenses:sub  $20.00
    assets:bank
"
            );
        },
    );
}

#[test]
fn set_posting_account_on_included_txn_writes_only_that_include() {
    edit_only_touches_sub(
        |editor| {
            editor
                .set_posting_account(Tindex(2), 0, "expenses:groceries")
                .unwrap();
        },
        |sub_after| {
            assert_eq!(
                sub_after,
                "\
2024-02-01 * Sub
    expenses:groceries  $20.00
    assets:bank
"
            );
        },
    );
}

#[test]
fn set_status_on_included_txn_writes_only_that_include() {
    edit_only_touches_sub(
        |editor| editor.set_status(Tindex(2), Status::Pending).unwrap(),
        |sub_after| {
            assert_eq!(
                sub_after,
                "\
2024-02-01 ! Sub
    expenses:sub  $20.00
    assets:bank
"
            );
        },
    );
}

#[test]
fn replace_included_txn_writes_only_that_include() {
    let mut expense = leg("expenses:sub", Some(dollars(2500, 2)));
    expense.comment = "bumped\n".into();
    let cash = leg("assets:bank", None);
    let replacement = {
        let mut t = cash_txn("2024-02-01", "Sub replaced", vec![expense, cash]);
        t.comment = "note\n".into();
        t
    };
    edit_only_touches_sub(
        move |editor| editor.replace_transaction(Tindex(2), &replacement).unwrap(),
        |sub_after| {
            assert_eq!(
                sub_after,
                "\
2024-02-01 * Sub replaced  ; note
    expenses:sub  $25.00  ; bumped
    assets:bank
"
            );
        },
    );
}

#[test]
fn included_external_change_is_refused_and_writes_nothing() {
    let (dir, main, sub, other) = write_multifile();
    let main_before = std::fs::read_to_string(&main).unwrap();
    let other_before = std::fs::read_to_string(&other).unwrap();

    let mut editor = JournalEditor::open(&main).unwrap();
    editor
        .set_posting_account(Tindex(2), 0, "expenses:renamed")
        .unwrap();

    // A concurrent external edit to the INCLUDED file with different content.
    let hijacked = "2099-09-09 * Hijack\n    a  $1\n    b\n";
    std::fs::write(&sub, hijacked).unwrap();

    let err = editor.save().unwrap_err();
    assert!(matches!(err, EditError::ExternalChange));

    // Nothing was written: sub keeps the external content, main + other untouched.
    assert_eq!(std::fs::read_to_string(&sub).unwrap(), hijacked);
    assert_eq!(std::fs::read_to_string(&main).unwrap(), main_before);
    assert_eq!(std::fs::read_to_string(&other).unwrap(), other_before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unbalanced_replace_on_included_txn_leaves_all_files_unchanged() {
    let (dir, main, sub, other) = write_multifile();
    let main_before = std::fs::read_to_string(&main).unwrap();
    let sub_before = std::fs::read_to_string(&sub).unwrap();
    let other_before = std::fs::read_to_string(&other).unwrap();

    let mut editor = JournalEditor::open(&main).unwrap();
    let bad = cash_txn(
        "2024-02-01",
        "bad",
        vec![
            leg("expenses:x", Some(dollars(500, 2))),
            leg("assets:bank", Some(dollars(-400, 2))),
        ],
    );
    let err = editor.replace_transaction(Tindex(2), &bad).unwrap_err();
    assert!(matches!(err, EditError::Unbalanced));
    // Rejected in memory; save writes nothing new (no dirty files).
    editor.save().unwrap();

    assert_eq!(std::fs::read_to_string(&main).unwrap(), main_before);
    assert_eq!(std::fs::read_to_string(&sub).unwrap(), sub_before);
    assert_eq!(std::fs::read_to_string(&other).unwrap(), other_before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nested_include_edit_writes_only_the_deepest_file() {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ledgeline-nest-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.journal");
    let mid = dir.join("mid.journal");
    let deep = dir.join("deep.journal");
    std::fs::write(
        &main,
        "2024-01-01 * Main\n    expenses:m  $1.00\n    assets:bank\n\ninclude mid.journal\n",
    )
    .unwrap();
    std::fs::write(
        &mid,
        "2024-02-01 * Mid\n    expenses:mid  $2.00\n    assets:bank\n\ninclude deep.journal\n",
    )
    .unwrap();
    std::fs::write(
        &deep,
        "2024-03-01 * Deep\n    expenses:deep  $3.00\n    assets:bank\n",
    )
    .unwrap();
    let main_before = std::fs::read_to_string(&main).unwrap();
    let mid_before = std::fs::read_to_string(&mid).unwrap();

    let mut editor = JournalEditor::open(&main).unwrap();
    assert_eq!(editor.transaction_count(), 3);
    // The deep txn is tindex 3, two includes down.
    editor.set_description(Tindex(3), "Deep renamed").unwrap();
    editor.save().unwrap();

    assert_eq!(std::fs::read_to_string(&main).unwrap(), main_before);
    assert_eq!(std::fs::read_to_string(&mid).unwrap(), mid_before);
    assert_eq!(
        std::fs::read_to_string(&deep).unwrap(),
        "2024-03-01 * Deep renamed\n    expenses:deep  $3.00\n    assets:bank\n"
    );
    let reopened = JournalEditor::open(&main).unwrap();
    assert_eq!(reopened.transaction_count(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Golden format_transaction
// ---------------------------------------------------------------------------

#[test]
fn golden_format_transaction_shapes() {
    // Multi-posting salary with a pending status, code, and comment.
    let salary = Transaction {
        status: Status::Pending,
        code: "PR-7".into(),
        comment: "payroll: acme\n".into(),
        ..cash_txn(
            "2026-07-27",
            "Acme Corp | July salary",
            vec![
                leg("income:salary", Some(dollars(-566_000, 2))),
                leg("expenses:taxes:federal", Some(dollars(115_000, 2))),
                leg("assets:bank:checking", None),
            ],
        )
    };
    // Amounts align on the widest account field ("expenses:taxes:federal", 22),
    // with a 2-space minimum gap; the elided leg is account-only.
    let expected = format!(
        "2026-07-27 ! (PR-7) Acme Corp | July salary  ; payroll: acme\n\
         {indent}income:salary{p1}$-5660.00\n\
         {indent}expenses:taxes:federal{p2}$1150.00\n\
         {indent}assets:bank:checking\n",
        indent = "    ",
        p1 = " ".repeat(22 - "income:salary".len() + 2),
        p2 = " ".repeat(2),
    );
    assert_eq!(format_transaction(&salary), expected);
}

// ---------------------------------------------------------------------------
// Property tests — the core safety invariant
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Deleting any single transaction leaves every other transaction's source
    /// text byte-identical, and reduces the count by exactly one.
    #[test]
    fn delete_keeps_other_transactions_byte_identical(
        fixture_idx in 0usize..FIXTURES.len(),
        seed in 0usize..1_000_000,
    ) {
        let (name, text) = &FIXTURES[fixture_idx];
        let mut editor = JournalEditor::from_text(format!("{name}.journal"), text).unwrap();
        let n = editor.transaction_count();
        prop_assume!(n > 0);
        let pos = seed % n;

        let before = ordered_sources(&editor);
        let target = editor.journal().transactions[pos].index;
        editor.delete_transaction(target).unwrap();

        prop_assert_eq!(editor.transaction_count(), n - 1);
        let after = ordered_sources(&editor);
        let mut expected = before.clone();
        expected.remove(pos);
        prop_assert_eq!(after, expected);
        // The mutated journal still re-parses (delete_transaction guarantees it).
        prop_assert!(JournalEditor::from_text("x.journal", &editor.text()).is_ok());
    }

    /// Appending a transaction leaves all existing transactions byte-identical
    /// (as the leading run) and increases the count by exactly one.
    #[test]
    fn append_keeps_existing_transactions_byte_identical(
        fixture_idx in 0usize..FIXTURES.len(),
        cents in 1i128..100_000,
    ) {
        let (name, text) = &FIXTURES[fixture_idx];
        let mut editor = JournalEditor::from_text(format!("{name}.journal"), text).unwrap();
        let n = editor.transaction_count();
        let before = ordered_sources(&editor);

        let new_txn = cash_txn(
            "2099-12-31",
            "proptest appended",
            vec![
                leg("expenses:test", Some(dollars(cents, 2))),
                leg("assets:bank", None),
            ],
        );
        editor.add_transaction(&new_txn, InsertPosition::Append).unwrap();

        prop_assert_eq!(editor.transaction_count(), n + 1);
        let after = ordered_sources(&editor);
        // Every original transaction is unchanged, in order, at the front.
        prop_assert_eq!(&after[..n], &before[..]);
        // The appended transaction is exactly the formatted text.
        prop_assert_eq!(&after[n], &format_transaction(&new_txn));
    }
}
