//! Plan 35 Phase 1: the surgical posting-tag ops ([`EditOp::SetPostingTag`],
//! [`EditOp::AddPostingTagValue`], [`EditOp::RemovePostingTagValue`]).
//!
//! Every expected text is the WHOLE file, so a stray space, a moved comment
//! or a lost elision fails the assertion rather than slipping through a
//! parse-level comparison.

use ledgeline_core::edit::{EditError, EditOp, JournalEditor};
use ledgeline_core::model::Tindex;

fn editor(text: &str) -> JournalEditor {
    JournalEditor::from_text("/ledgeline-ptags/inline.journal", text).expect("journal parses")
}

/// `text` after applying `ops` to its first transaction as one batch entry.
fn applied(text: &str, ops: Vec<EditOp>) -> String {
    let mut editor = editor(text);
    let fingerprint = editor.transaction_fingerprint(Tindex(1)).expect("exists");
    editor
        .apply_batch(&[(Tindex(1), fingerprint, ops)])
        .expect("batch applies");
    editor.text()
}

fn refused(text: &str, ops: Vec<EditOp>) -> EditError {
    let mut editor = editor(text);
    let fingerprint = editor.transaction_fingerprint(Tindex(1)).expect("exists");
    let error = editor
        .apply_batch(&[(Tindex(1), fingerprint, ops)])
        .expect_err("the batch is refused");
    assert_eq!(editor.text(), text, "a refused batch changes nothing");
    match error {
        EditError::BulkTransaction { index: 0, source } => *source,
        other => other,
    }
}

fn set(posting: usize, name: &str, value: Option<&str>) -> EditOp {
    EditOp::SetPostingTag {
        posting,
        name: name.to_string(),
        value: value.map(str::to_string),
    }
}

fn add(posting: usize, name: &str, value: &str) -> EditOp {
    EditOp::AddPostingTagValue {
        posting,
        name: name.to_string(),
        value: value.to_string(),
    }
}

fn remove(posting: usize, name: &str, value: &str) -> EditOp {
    EditOp::RemovePostingTagValue {
        posting,
        name: name.to_string(),
        value: value.to_string(),
    }
}

/// A transfer half with an elided real leg, a header tag and a second
/// transaction that must never move.
const BASE: &str = "\
2026-12-31 * ACH AMEX EPAYMENT  ; id:FIT1
    assets:checking
    assets:transfers:cc          $500

2027-01-02 Other
    expenses:food  $1.00
    assets:checking
";

#[test]
fn set_starts_a_same_line_comment_and_keeps_the_elision() {
    let out = applied(BASE, vec![set(1, "match", Some("k7f3a2"))]);
    assert_eq!(
        out,
        BASE.replace(
            "    assets:transfers:cc          $500\n",
            "    assets:transfers:cc          $500  ; match:k7f3a2\n"
        )
    );
}

#[test]
fn set_on_an_elided_posting_keeps_it_elided() {
    let out = applied(BASE, vec![set(0, "match", Some("abc123"))]);
    assert_eq!(
        out,
        BASE.replace(
            "    assets:checking\n    assets:transfers",
            "    assets:checking  ; match:abc123\n    assets:transfers"
        )
    );
    let journal = editor(&out).journal().clone();
    assert!(journal.transactions[0].postings[0].elided);
}

#[test]
fn set_then_remove_restores_the_file_byte_for_byte() {
    let linked = applied(BASE, vec![set(1, "match", Some("k7f3a2"))]);
    assert_eq!(applied(&linked, vec![set(1, "match", None)]), BASE);
}

#[test]
fn set_joins_an_existing_comment_and_replaces_in_place() {
    let text = BASE.replace("$500\n", "$500  ; gift, match:old ,x:1\n");
    let out = applied(&text, vec![set(1, "match", Some("new"))]);
    assert_eq!(
        out,
        BASE.replace("$500\n", "$500  ; gift, match:new ,x:1\n")
    );

    let text = BASE.replace("$500\n", "$500  ;gift\n");
    let out = applied(&text, vec![set(1, "match", Some("new"))]);
    assert_eq!(out, BASE.replace("$500\n", "$500  ;gift, match:new\n"));
}

#[test]
fn set_keeps_one_value_and_drops_the_rest_everywhere_on_the_posting() {
    let text = BASE.replace(
        "$500\n",
        "$500  ; match:a, note, match:b\n    ; match:c\n    ; keep:me\n",
    );
    let out = applied(&text, vec![set(1, "match", Some("z"))]);
    assert_eq!(
        out,
        BASE.replace("$500\n", "$500  ; match:z, note\n    ; keep:me\n")
    );
}

#[test]
fn remove_all_drops_the_comment_or_line_left_empty() {
    let text = BASE.replace("$500\n", "$500  ; match:a\n    ; match:b\n");
    assert_eq!(applied(&text, vec![set(1, "match", None)]), BASE);
    // Free text sharing the segment stays.
    let text = BASE.replace("$500\n", "$500  ; paid match:a\n");
    assert_eq!(
        applied(&text, vec![set(1, "match", None)]),
        BASE.replace("$500\n", "$500  ; paid\n")
    );
}

#[test]
fn remove_when_absent_is_a_no_op() {
    assert_eq!(applied(BASE, vec![set(1, "match", None)]), BASE);
    assert_eq!(applied(BASE, vec![remove(1, "nomatch", "r4x9")]), BASE);
}

#[test]
fn add_value_appends_beside_other_values() {
    let one = applied(BASE, vec![add(1, "nomatch", "r4x9")]);
    assert_eq!(one, BASE.replace("$500\n", "$500  ; nomatch:r4x9\n"));
    let two = applied(&one, vec![add(1, "nomatch", "b2c1")]);
    assert_eq!(
        two,
        BASE.replace("$500\n", "$500  ; nomatch:r4x9, nomatch:b2c1\n")
    );
    // An exact value already there: nothing moves.
    assert_eq!(applied(&two, vec![add(1, "nomatch", "b2c1")]), two);
    let journal = editor(&two).journal().clone();
    let tags = &journal.transactions[0].postings[1].tags;
    assert_eq!(
        tags,
        &vec![
            ("nomatch".to_string(), "r4x9".to_string()),
            ("nomatch".to_string(), "b2c1".to_string()),
        ]
    );
}

#[test]
fn remove_value_takes_only_that_value_and_its_separator() {
    let text = BASE.replace(
        "$500\n",
        "$500  ; nomatch:aaaaaa,  nomatch:bbbbbb ,nomatch:cccccc\n",
    );
    assert_eq!(
        applied(&text, vec![remove(1, "nomatch", "aaaaaa")]),
        BASE.replace("$500\n", "$500  ; nomatch:bbbbbb ,nomatch:cccccc\n")
    );
    assert_eq!(
        applied(&text, vec![remove(1, "nomatch", "bbbbbb")]),
        BASE.replace("$500\n", "$500  ; nomatch:aaaaaa,nomatch:cccccc\n")
    );
    assert_eq!(
        applied(&text, vec![remove(1, "nomatch", "cccccc")]),
        BASE.replace("$500\n", "$500  ; nomatch:aaaaaa,  nomatch:bbbbbb\n")
    );
}

#[test]
fn remove_value_reaches_a_continuation_line() {
    let text = BASE.replace("$500\n", "$500  ; nomatch:a\n    ; nomatch:b, other:x\n");
    assert_eq!(
        applied(&text, vec![remove(1, "nomatch", "b")]),
        BASE.replace("$500\n", "$500  ; nomatch:a\n    ; other:x\n")
    );
}

#[test]
fn ops_on_two_postings_of_one_transaction_compose() {
    let out = applied(
        BASE,
        vec![
            set(1, "match", Some("k1")),
            add(0, "nomatch", "r1"),
            remove(0, "nomatch", "r1"),
        ],
    );
    assert_eq!(
        out,
        BASE.replace("$500\n", "$500  ; match:k1\n"),
        "an add then a remove of the same value cancels"
    );
}

#[test]
fn crlf_lines_keep_their_terminators() {
    let text = BASE.replace('\n', "\r\n");
    let out = applied(&text, vec![set(1, "match", Some("k"))]);
    assert_eq!(out, text.replace("$500\r\n", "$500  ; match:k\r\n"));
}

#[test]
fn refuses_what_hledger_could_not_read_back() {
    for op in [
        set(1, "ma tch", Some("x")),
        set(1, "match", Some("a,b")),
        set(1, "match", Some(" x")),
        add(1, "nomatch", "a\nb"),
        remove(1, "no:match", "x"),
    ] {
        let error = refused(BASE, vec![op.clone()]);
        assert!(
            matches!(error, EditError::Unsupported(_)),
            "{op:?} gave {error:?}"
        );
    }
}

#[test]
fn refuses_a_posting_that_does_not_exist() {
    let error = refused(BASE, vec![set(2, "match", Some("x"))]);
    assert!(
        matches!(error, EditError::PostingNotFound { txn: 1, posting: 2 }),
        "{error:?}"
    );
}

#[test]
fn editor_methods_match_the_ops() {
    let mut editor = editor(BASE);
    editor
        .set_posting_tag(Tindex(1), 1, "match", Some("k7f3a2"))
        .expect("set");
    editor
        .add_posting_tag_value(Tindex(1), 1, "nomatch", "r1")
        .expect("add");
    editor
        .remove_posting_tag_value(Tindex(1), 1, "nomatch", "r1")
        .expect("remove");
    assert_eq!(
        editor.text(),
        BASE.replace("$500\n", "$500  ; match:k7f3a2\n")
    );
}
