//! Plan 32's write path: the surgical tag and comment ops ([`set_txn_tag`],
//! [`set_txn_comment`], the pure [`upsert_tag`]) and the atomic multi-
//! transaction [`JournalEditor::apply_batch`].
//!
//! The byte-exact cases are the point: every expected text below is the
//! whole file, so a stray space, a re-rendered amount or a lost elision fails
//! the assertion rather than slipping through a parse-level comparison.
//!
//! [`set_txn_tag`]: JournalEditor::set_txn_tag
//! [`set_txn_comment`]: JournalEditor::set_txn_comment

use ledgeline_core::edit::{EditError, EditOp, JournalEditor, upsert_tag};
use ledgeline_core::model::{Status, Tindex, Transaction};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A unique scratch directory that removes itself on drop, including on panic.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "ledgeline-batch-{name}-{}-{seq}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).expect("write file");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read file")
}

/// An editor over `text` alone (no includes), never saved.
fn editor(text: &str) -> JournalEditor {
    JournalEditor::from_text("/ledgeline-batch/inline.journal", text).expect("journal parses")
}

/// `text` after `set_txn_tag(1, name, value)`.
fn tagged(text: &str, name: &str, value: Option<&str>) -> String {
    let mut editor = editor(text);
    editor
        .set_txn_tag(Tindex(1), name, value)
        .unwrap_or_else(|error| panic!("tag {name}={value:?} on {text:?}: {error}"));
    editor.text()
}

const POSTINGS: &str = "    expenses:food  $82.10\n    assets:checking\n";

fn txn(header: &str) -> String {
    format!("{header}\n{POSTINGS}")
}

// ---------------------------------------------------------------------------
// set_txn_tag (D2)
// ---------------------------------------------------------------------------

#[test]
fn d2_examples_are_byte_exact() {
    for (before, after) in [
        (
            "2026-10-12 Safeway",
            "2026-10-12 Safeway  ; reviewed:2026-10-30",
        ),
        (
            "2026-10-12 Safeway  ; groceries",
            "2026-10-12 Safeway  ; groceries, reviewed:2026-10-30",
        ),
        (
            "2026-10-12 Safeway  ; reviewed:2026-01-02, x:1",
            "2026-10-12 Safeway  ; reviewed:2026-10-30, x:1",
        ),
    ] {
        assert_eq!(
            tagged(&txn(before), "reviewed", Some("2026-10-30")),
            txn(after),
            "{before}"
        );
    }
}

#[test]
fn a_new_comment_is_exactly_two_spaces_and_existing_spacing_is_kept() {
    for (before, after) in [
        // New: exactly `"  ; "`, whatever trailing space the header had.
        (
            "2026-10-12 Safeway   ",
            "2026-10-12 Safeway  ; reviewed:2026-10-30",
        ),
        (
            "2026-10-12 * (42) Safeway | weekly",
            "2026-10-12 * (42) Safeway | weekly  ; reviewed:2026-10-30",
        ),
        // Existing: the gap, the `;` spacing and the comma style survive.
        (
            "2026-10-12 Safeway ;groceries",
            "2026-10-12 Safeway ;groceries, reviewed:2026-10-30",
        ),
        (
            "2026-10-12 Safeway\t\t;  groceries  ",
            "2026-10-12 Safeway\t\t;  groceries, reviewed:2026-10-30",
        ),
        (
            "2026-10-12 Safeway    ; reviewed: 2026-01-02 ,x:1",
            "2026-10-12 Safeway    ; reviewed: 2026-10-30 ,x:1",
        ),
        // A date written with slashes is not touched.
        (
            "2026/10/12 Safeway",
            "2026/10/12 Safeway  ; reviewed:2026-10-30",
        ),
    ] {
        assert_eq!(
            tagged(&txn(before), "reviewed", Some("2026-10-30")),
            txn(after),
            "{before:?}"
        );
    }
}

#[test]
fn removing_a_tag_takes_its_separator_and_an_emptied_comment() {
    for (before, after) in [
        (
            "2026-10-12 Safeway  ; reviewed:2026-01-02",
            "2026-10-12 Safeway",
        ),
        (
            "2026-10-12 Safeway  ; groceries, reviewed:2026-01-02",
            "2026-10-12 Safeway  ; groceries",
        ),
        (
            "2026-10-12 Safeway  ; reviewed:2026-01-02, x:1",
            "2026-10-12 Safeway  ; x:1",
        ),
        (
            "2026-10-12 Safeway  ; a:1, reviewed:2026-01-02, x:1",
            "2026-10-12 Safeway  ; a:1, x:1",
        ),
        // Every occurrence goes.
        (
            "2026-10-12 Safeway  ; reviewed:a, x:1, reviewed:b",
            "2026-10-12 Safeway  ; x:1",
        ),
        // Absent: nothing to do, nothing written.
        ("2026-10-12 Safeway  ; x:1", "2026-10-12 Safeway  ; x:1"),
    ] {
        assert_eq!(
            tagged(&txn(before), "reviewed", None),
            txn(after),
            "{before:?}"
        );
    }
}

#[test]
fn a_tag_on_a_continuation_line_is_rewritten_where_it_sits() {
    let before = "2026-10-12 Safeway  ; groceries\n    ; id:42, reviewed:2026-01-02\n    expenses:food  $82.10\n    assets:checking\n";
    assert_eq!(
        tagged(before, "reviewed", Some("2026-10-30")),
        "2026-10-12 Safeway  ; groceries\n    ; id:42, reviewed:2026-10-30\n    expenses:food  $82.10\n    assets:checking\n"
    );
    assert_eq!(
        tagged(before, "reviewed", None),
        "2026-10-12 Safeway  ; groceries\n    ; id:42\n    expenses:food  $82.10\n    assets:checking\n"
    );
    // A continuation line left empty goes entirely.
    let alone = "2026-10-12 Safeway\n    ; reviewed:2026-01-02\n    expenses:food  $82.10\n    assets:checking\n";
    assert_eq!(tagged(alone, "reviewed", None), txn("2026-10-12 Safeway"));
}

#[test]
fn a_posting_tag_is_not_the_transaction_tag() {
    let before = "2026-10-12 Safeway\n    expenses:food  $82.10  ; reviewed:2026-01-02\n    assets:checking\n    ; reviewed:2026-01-03\n";
    assert_eq!(
        tagged(before, "reviewed", Some("2026-10-30")),
        "2026-10-12 Safeway  ; reviewed:2026-10-30\n    expenses:food  $82.10  ; reviewed:2026-01-02\n    assets:checking\n    ; reviewed:2026-01-03\n"
    );
    assert_eq!(tagged(before, "reviewed", None), before);
}

#[test]
fn a_tag_inside_a_free_text_comment_follows_hledger() {
    // hledger reads `weekly shop: produce` as the tag `shop` = `produce`.
    let before = txn("2026-10-12 Safeway  ; weekly shop: produce");
    assert_eq!(
        tagged(&before, "shop", Some("fresh")),
        txn("2026-10-12 Safeway  ; weekly shop: fresh")
    );
    assert_eq!(
        tagged(&before, "shop", None),
        txn("2026-10-12 Safeway  ; weekly")
    );
    assert_eq!(
        tagged(&before, "reviewed", Some("2026-10-30")),
        txn("2026-10-12 Safeway  ; weekly shop: produce, reviewed:2026-10-30")
    );
}

#[test]
fn crlf_files_keep_their_line_endings() {
    let before = "2026-10-12 Safeway\r\n    expenses:food  $82.10\r\n    assets:checking\r\n\r\n2026-10-13 Other  ; reviewed:2026-01-02\r\n    expenses:food  $1.00\r\n    assets:checking\r\n";
    let mut editor = editor(before);
    editor
        .set_txn_tag(Tindex(1), "reviewed", Some("2026-10-30"))
        .expect("tag");
    editor
        .set_txn_tag(Tindex(2), "reviewed", None)
        .expect("untag");
    assert_eq!(
        editor.text(),
        "2026-10-12 Safeway  ; reviewed:2026-10-30\r\n    expenses:food  $82.10\r\n    assets:checking\r\n\r\n2026-10-13 Other\r\n    expenses:food  $1.00\r\n    assets:checking\r\n"
    );
}

#[test]
fn tags_hledger_cannot_read_back_are_refused_and_nothing_changes() {
    let before = txn("2026-10-12 Safeway");
    for (name, value) in [
        ("", Some("x")),
        ("two words", Some("x")),
        ("a:b", Some("x")),
        ("a,b", Some("x")),
        ("reviewed", Some("a, b")),
        ("reviewed", Some(" padded")),
        ("reviewed", Some("line\nbreak")),
    ] {
        let mut editor = editor(&before);
        let refused = editor.set_txn_tag(Tindex(1), name, value);
        assert!(
            matches!(refused, Err(EditError::Unsupported(_))),
            "{name:?}={value:?}: {refused:?}"
        );
        assert_eq!(editor.text(), before);
        assert!(editor.dirty_files().is_empty());
    }
}

#[test]
fn a_tag_set_to_its_current_value_writes_nothing() {
    let mut editor = editor(&txn("2026-10-12 Safeway  ; reviewed:2026-10-30"));
    editor
        .set_txn_tag(Tindex(1), "reviewed", Some("2026-10-30"))
        .expect("no-op");
    assert!(editor.dirty_files().is_empty());
}

#[test]
fn missing_transaction_is_not_found() {
    let mut editor = editor(&txn("2026-10-12 Safeway"));
    assert!(matches!(
        editor.set_txn_tag(Tindex(9), "reviewed", Some("x")),
        Err(EditError::TransactionNotFound(9))
    ));
}

// ---------------------------------------------------------------------------
// upsert_tag (the pure helper)
// ---------------------------------------------------------------------------

#[test]
fn upsert_tag_table() {
    let cases: &[(&str, &str, Option<&str>, &str)] = &[
        ("", "a", Some("1"), " a:1"),
        ("   ", "a", Some("1"), " a:1"),
        (" note", "a", Some("1"), " note, a:1"),
        (" note  ", "a", Some("1"), " note, a:1"),
        (" a:0", "a", Some("1"), " a:1"),
        (" a:", "a", Some("1"), " a:1"),
        (" a: 0 , b:2", "a", Some("1"), " a: 1 , b:2"),
        (" x:1,a:0,y:2", "a", Some("1"), " x:1,a:1,y:2"),
        // Values may hold colons; only the first `:` names the tag.
        (
            " url:http://x, a:0",
            "url",
            Some("https://y"),
            " url:https://y, a:0",
        ),
        // Removal.
        (" a:1", "a", None, ""),
        (" a:1, b:2", "a", None, " b:2"),
        (" b:2, a:1", "a", None, " b:2"),
        (" b:2 , a:1", "a", None, " b:2"),
        (" b:2, a:1, c:3", "a", None, " b:2, c:3"),
        (" free text a:1, b:2", "a", None, " free text, b:2"),
        (" b:2", "a", None, " b:2"),
        // The name is matched exactly, not as a suffix or prefix.
        (
            " reviewed-by:pat",
            "reviewed",
            Some("x"),
            " reviewed-by:pat, reviewed:x",
        ),
        (" myreviewed:1", "reviewed", None, " myreviewed:1"),
    ];
    for (comment, name, value, expected) in cases {
        assert_eq!(
            upsert_tag(comment, name, *value),
            *expected,
            "{comment:?} {name} {value:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// set_txn_comment (D9)
// ---------------------------------------------------------------------------

#[test]
fn set_txn_comment_replaces_free_text_and_keeps_tags() {
    let set = |header: &str, text: &str| {
        let mut editor = editor(&txn(header));
        editor
            .set_txn_comment(Tindex(1), text)
            .unwrap_or_else(|error| panic!("{header:?} <- {text:?}: {error}"));
        editor.text()
    };
    assert_eq!(
        set(
            "2026-10-12 Safeway  ; groceries, reviewed:2026-10-30",
            "household"
        ),
        txn("2026-10-12 Safeway  ; household, reviewed:2026-10-30")
    );
    assert_eq!(
        set("2026-10-12 Safeway  ; reviewed:2026-10-30, x:1", "a note"),
        txn("2026-10-12 Safeway  ; a note, reviewed:2026-10-30, x:1")
    );
    assert_eq!(
        set("2026-10-12 Safeway  ; groceries, reviewed:2026-10-30", ""),
        txn("2026-10-12 Safeway  ; reviewed:2026-10-30")
    );
    assert_eq!(
        set("2026-10-12 Safeway  ; groceries", ""),
        txn("2026-10-12 Safeway")
    );
    assert_eq!(
        set("2026-10-12 Safeway", "first visit"),
        txn("2026-10-12 Safeway  ; first visit")
    );
    assert_eq!(set("2026-10-12 Safeway", ""), txn("2026-10-12 Safeway"));
}

#[test]
fn set_txn_comment_refuses_text_that_reads_as_a_tag() {
    let before = txn("2026-10-12 Safeway");
    let mut editor = editor(&before);
    assert!(matches!(
        editor.set_txn_comment(Tindex(1), "note: this is a tag"),
        Err(EditError::Unsupported(_))
    ));
    assert_eq!(editor.text(), before);
}

// ---------------------------------------------------------------------------
// Elision (the reason review must not go through a full replace)
// ---------------------------------------------------------------------------

#[test]
fn an_elided_posting_stays_elided_after_tagging_and_an_account_change() {
    let before = "2026-10-12 SQ *BLUE BOTTLE 4412 OAKLAND\n    expenses:unknown        $6.75\n    liabilities:visa\n";
    let mut editor = editor(before);
    let fingerprint = editor.transaction_fingerprint(Tindex(1)).expect("exists");
    editor
        .apply_batch(&[(
            Tindex(1),
            fingerprint,
            vec![
                EditOp::SetPostingAccount {
                    posting: 0,
                    account: "expenses:food:coffee".to_string(),
                },
                EditOp::SetTxnTag {
                    name: "reviewed".to_string(),
                    value: Some("2026-10-30".to_string()),
                },
                EditOp::SetDescription("Blue Bottle Coffee".to_string()),
                EditOp::SetTxnTag {
                    name: "payee-key".to_string(),
                    value: Some("sq blue bottle oakland".to_string()),
                },
            ],
        )])
        .expect("batch");
    assert_eq!(
        editor.text(),
        "2026-10-12 Blue Bottle Coffee  ; reviewed:2026-10-30, payee-key:sq blue bottle oakland\n    expenses:food:coffee        $6.75\n    liabilities:visa\n"
    );
    // The elided leg is still inferred, not written.
    let txn = &editor.journal().transactions[0];
    assert_eq!(txn.postings[1].amounts[0].quantity.mantissa, -675);
}

#[test]
fn set_description_in_a_batch_keeps_the_comment_byte_for_byte() {
    let mut editor = editor(&txn("2026/10/12 * SAFEWAY #12 ;groceries,reviewed:x"));
    let fingerprint = editor.transaction_fingerprint(Tindex(1)).expect("exists");
    editor
        .apply_batch(&[(
            Tindex(1),
            fingerprint,
            vec![EditOp::SetDescription("Safeway".to_string())],
        )])
        .expect("batch");
    assert_eq!(
        editor.text(),
        txn("2026/10/12 * Safeway ;groceries,reviewed:x")
    );
}

// ---------------------------------------------------------------------------
// apply_batch
// ---------------------------------------------------------------------------

/// A main journal and an include, `per_file` unknown transactions in each,
/// with a categorised one between every two so the untouched neighbours are
/// real. Returns the scratch dir, the main path and the include path.
fn two_files(name: &str, per_file: usize) -> (Scratch, PathBuf, PathBuf) {
    let scratch = Scratch::new(name);
    let body = |year: u32| -> String {
        (0..per_file)
            .map(|n| {
                format!(
                    "{year}-{:02}-{:02} SQ *CAFE {n} OAKLAND\n    expenses:unknown    $1.{n:02}\n    liabilities:visa\n\n\
                     {year}-{:02}-{:02} Rent {n}  ; note\n    expenses:rent  $10.00\n    assets:checking\n\n",
                    1 + n % 12,
                    1 + n % 27,
                    1 + n % 12,
                    2 + n % 27
                )
            })
            .collect()
    };
    let include = scratch.write("2025.journal", &body(2025));
    let main = scratch.write(
        "main.journal",
        &format!("; books\ninclude 2025.journal\n\n{}", body(2026)),
    );
    (scratch, main, include)
}

/// The cafe transactions, in file order.
fn cafes(editor: &JournalEditor) -> Vec<Tindex> {
    editor
        .journal()
        .transactions
        .iter()
        .filter(|txn| txn.description.starts_with("SQ *CAFE"))
        .map(|txn| txn.index)
        .collect()
}

/// Recategorise, rename and tag one transaction.
fn review_ops() -> Vec<EditOp> {
    vec![
        EditOp::SetDescription("Corner Cafe".to_string()),
        EditOp::SetPostingAccount {
            posting: 0,
            account: "expenses:food:coffee".to_string(),
        },
        EditOp::SetTxnTag {
            name: "reviewed".to_string(),
            value: Some("2026-10-30".to_string()),
        },
    ]
}

fn sources(editor: &JournalEditor) -> Vec<String> {
    editor
        .journal()
        .transactions
        .iter()
        .map(|txn| editor.transaction_source(txn.index).expect("source"))
        .collect()
}

#[test]
fn fifty_transactions_across_two_files_land_in_one_save() {
    let (_scratch, main, include) = two_files("fifty", 30);
    let mut editor = JournalEditor::open(&main).expect("open");
    let before = sources(&editor);
    let targets: Vec<Tindex> = {
        let all = cafes(&editor);
        // 25 from each file.
        all[..25].iter().chain(&all[30..55]).copied().collect()
    };
    assert_eq!(targets.len(), 50);
    let edits: Vec<_> = targets
        .iter()
        .map(|&index| {
            let fingerprint = editor.transaction_fingerprint(index).expect("exists");
            (index, fingerprint, review_ops())
        })
        .collect();
    editor.apply_batch(&edits).expect("batch");

    let after = sources(&editor);
    assert_eq!(after.len(), before.len());
    for (position, (was, now)) in before.iter().zip(&after).enumerate() {
        let index = editor.journal().transactions[position].index;
        if targets.contains(&index) {
            // The header is rebuilt with the new description and a new
            // comment; below it only the account token changed.
            let (header, body) = was.split_at(was.find('\n').expect("header"));
            let body = body.replacen("expenses:unknown", "expenses:food:coffee", 1);
            assert_eq!(
                *now,
                format!("{} Corner Cafe  ; reviewed:2026-10-30{body}", &header[..10]),
                "#{}",
                index.0
            );
        } else {
            assert_eq!(now, was, "#{} must be untouched", index.0);
        }
    }

    let mut dirty: Vec<PathBuf> = editor
        .dirty_files()
        .iter()
        .map(|path| path.to_path_buf())
        .collect();
    dirty.sort();
    assert_eq!(dirty.len(), 2, "both files touched: {dirty:?}");
    editor.save().expect("save");
    assert!(read(&main).contains(
        "2026-01-01 Corner Cafe  ; reviewed:2026-10-30\n    expenses:food:coffee    $1.00\n"
    ));
    assert!(read(&include).contains(
        "2025-01-01 Corner Cafe  ; reviewed:2026-10-30\n    expenses:food:coffee    $1.00\n"
    ));
    // The six cafes left out are still unknown, on disk too.
    assert_eq!(read(&main).matches("expenses:unknown").count(), 5);
    assert_eq!(read(&include).matches("expenses:unknown").count(), 5);
}

#[test]
fn a_stale_fingerprint_writes_nothing() {
    let (_scratch, main, include) = two_files("stale", 4);
    let (main_before, include_before) = (read(&main), read(&include));
    let mut editor = JournalEditor::open(&main).expect("open");
    let targets = cafes(&editor);
    let mut edits: Vec<_> = targets
        .iter()
        .map(|&index| {
            (
                index,
                editor.transaction_fingerprint(index).expect("exists"),
                review_ops(),
            )
        })
        .collect();
    // The third entry was shown a different transaction's text.
    edits[2].1 = editor.transaction_fingerprint(targets[0]).expect("exists");
    let text_before = editor.text();

    let refused = editor.apply_batch(&edits);
    assert!(
        matches!(
            &refused,
            Err(EditError::BulkTransaction { index: 2, source })
                if matches!(**source, EditError::ExternalChange)
        ),
        "{refused:?}"
    );
    assert_eq!(editor.text(), text_before);
    assert!(editor.dirty_files().is_empty());
    editor.save().expect("nothing to save");
    assert_eq!((read(&main), read(&include)), (main_before, include_before));
}

#[test]
fn one_invalid_op_writes_nothing() {
    let (_scratch, main, include) = two_files("invalid", 4);
    let (main_before, include_before) = (read(&main), read(&include));
    let mut editor = JournalEditor::open(&main).expect("open");
    let targets = cafes(&editor);
    let fingerprint =
        |editor: &JournalEditor, index| editor.transaction_fingerprint(index).expect("exists");

    for (bad, check) in [
        (
            EditOp::SetPostingAccount {
                posting: 7,
                account: "expenses:x".to_string(),
            },
            "posting not found",
        ),
        // A `;` would end the description and start a comment.
        (
            EditOp::SetDescription("Cafe ; sneaky".to_string()),
            "round trip",
        ),
        // Two spaces would split the account into account + amount.
        (
            EditOp::SetPostingAccount {
                posting: 0,
                account: "expenses:a  b".to_string(),
            },
            "parse",
        ),
        (
            EditOp::Replace(Box::new(editor.journal().transactions[0].clone())),
            "replace not first",
        ),
    ] {
        let mut edits: Vec<_> = targets
            .iter()
            .map(|&index| (index, fingerprint(&editor, index), review_ops()))
            .collect();
        edits[5].2.push(bad.clone());
        let refused = editor.apply_batch(&edits);
        assert!(refused.is_err(), "{check}: {bad:?}");
        if let Err(EditError::BulkTransaction { index, .. }) = &refused {
            assert_eq!(*index, 5, "{check}");
        }
        assert!(editor.dirty_files().is_empty(), "{check}");
    }
    editor.save().expect("nothing to save");
    assert_eq!((read(&main), read(&include)), (main_before, include_before));
}

#[test]
fn a_transaction_listed_twice_is_refused() {
    let mut editor = editor(&txn("2026-10-12 Safeway"));
    let fingerprint = editor.transaction_fingerprint(Tindex(1)).expect("exists");
    let refused = editor.apply_batch(&[
        (Tindex(1), fingerprint.clone(), vec![]),
        (Tindex(1), fingerprint, vec![]),
    ]);
    assert!(matches!(
        refused,
        Err(EditError::BulkTransaction { index: 1, .. })
    ));
}

#[test]
fn a_replace_then_a_tag_rerenders_once_and_keeps_the_elision() {
    let before = "2026-10-12 Safeway\n    expenses:food    $82.10\n    assets:checking\n\n2026-10-13 Other\n    expenses:food  $1.00\n    assets:checking\n";
    let mut editor = editor(before);
    let mut body: Transaction = editor.journal().transactions[0].clone();
    body.status = Status::Cleared;
    body.postings[0].account.0 = "expenses:food:groceries".to_string();
    let fingerprint = editor.transaction_fingerprint(Tindex(1)).expect("exists");
    editor
        .apply_batch(&[(
            Tindex(1),
            fingerprint,
            vec![
                EditOp::Replace(Box::new(body)),
                EditOp::SetTxnTag {
                    name: "reviewed".to_string(),
                    value: Some("2026-10-30".to_string()),
                },
            ],
        )])
        .expect("batch");
    assert_eq!(
        editor.text(),
        "2026-10-12 * Safeway  ; reviewed:2026-10-30\n    expenses:food:groceries  $82.10\n    assets:checking\n\n2026-10-13 Other\n    expenses:food  $1.00\n    assets:checking\n"
    );
}

#[test]
fn an_empty_batch_is_a_no_op() {
    let mut editor = editor(&txn("2026-10-12 Safeway"));
    editor.apply_batch(&[]).expect("empty");
    assert!(editor.dirty_files().is_empty());
}

#[test]
fn hledger_reads_the_batch_result() {
    // The Risks table's promise: hledger itself parses what the tag writer
    // produced. Skipped where hledger is not installed.
    let Ok(version) = std::process::Command::new("hledger")
        .arg("--version")
        .output()
    else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let (_scratch, main, _include) = two_files("hledger", 3);
    let mut editor = JournalEditor::open(&main).expect("open");
    let edits: Vec<_> = cafes(&editor)
        .into_iter()
        .map(|index| {
            let mut ops = review_ops();
            ops.push(EditOp::SetTxnComment("from the review overlay".to_string()));
            (
                index,
                editor.transaction_fingerprint(index).expect("exists"),
                ops,
            )
        })
        .collect();
    editor.apply_batch(&edits).expect("batch");
    editor.save().expect("save");
    let output = std::process::Command::new("hledger")
        .arg("-f")
        .arg(&main)
        .args(["reg", "tag:reviewed=2026-10-30", "-O", "csv"])
        .output()
        .expect("run hledger");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let csv = String::from_utf8_lossy(&output.stdout);
    // Six reviewed transactions, two postings each, plus the header row.
    assert_eq!(csv.lines().count(), 1 + 6 * 2, "{csv}");
    assert!(csv.contains("expenses:food:coffee"));
}

#[test]
fn batched_fingerprints_match_the_single_ones_across_files() {
    let (_scratch, main, _include) = two_files("fingerprints", 3);
    let editor = JournalEditor::open(&main).expect("open");
    let mut indices: Vec<Tindex> = editor
        .journal()
        .transactions
        .iter()
        .map(|t| t.index)
        .collect();
    indices.reverse();
    indices.push(Tindex(9_999));
    let batched = editor.transaction_fingerprints(&indices);
    let single: Vec<_> = indices
        .iter()
        .map(|&index| editor.transaction_fingerprint(index))
        .collect();
    assert_eq!(batched, single);
    assert_eq!(batched.last(), Some(&None));
    // Distinct text, distinct fingerprint.
    assert_ne!(batched[0], batched[1]);
}
