//! Plan 32 Phase 7a: the surgical header and posting ops (D34) and elision
//! through a re-render (D35).
//!
//! Every expected text is the WHOLE file, so a stray space, a re-rendered
//! comment or a lost elision fails the assertion rather than slipping through
//! a parse-level comparison. Each op is applied through
//! [`JournalEditor::apply_batch`], with the fingerprint the client was shown.

use ledgeline_core::edit::{EditError, EditOp, JournalEditor};
use ledgeline_core::model::{Status, Tindex, Transaction};

/// An editor over `text` alone (no includes), never saved.
fn editor(text: &str) -> JournalEditor {
    JournalEditor::from_text("/ledgeline-ops/inline.journal", text).expect("journal parses")
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

/// The error `ops` on the first transaction fail with, unwrapped from its
/// batch attribution; the editor must be left exactly as it was.
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

fn unsupported(text: &str, op: EditOp) {
    let error = refused(text, vec![op]);
    assert!(
        matches!(error, EditError::Unsupported(_)),
        "expected Unsupported, got {error:?}"
    );
}

/// A header with every field: a secondary date, a status, a code, a
/// description and a two-line comment with a tag, over a commented posting
/// and an elided leg. A second transaction proves nothing outside the target
/// moves.
const RICH: &str = "\
2026-10-12=2026-10-14 * (1042) Blue Bottle  ; split with Sam, trip:sf
    ; second line
    expenses:food:coffee  $6.75  ; gift card
    liabilities:visa

2026-10-13 Other
    expenses:food  $1.00
    assets:checking
";

const OTHER: &str = "\n2026-10-13 Other\n    expenses:food  $1.00\n    assets:checking\n";

fn rich_with_header(header: &str) -> String {
    RICH.replacen(
        "2026-10-12=2026-10-14 * (1042) Blue Bottle  ; split with Sam, trip:sf",
        header,
        1,
    )
}

// ---------------------------------------------------------------------------
// Header: date, secondary date, status, code
// ---------------------------------------------------------------------------

#[test]
fn set_date_rewrites_only_the_primary_date() {
    assert_eq!(
        applied(RICH, vec![EditOp::SetDate("2026-10-20".into())]),
        rich_with_header("2026-10-20=2026-10-14 * (1042) Blue Bottle  ; split with Sam, trip:sf")
    );
}

#[test]
fn set_date_keeps_the_header_s_separator() {
    let before = "2026/10/12 Payee  ;note\n    expenses:a  $1.00\n    assets:b\n";
    assert_eq!(
        applied(before, vec![EditOp::SetDate("2026-11-02".into())]),
        "2026/11/02 Payee  ;note\n    expenses:a  $1.00\n    assets:b\n"
    );
    let dotted = "2026.10.12 Payee\n    expenses:a  $1.00\n    assets:b\n";
    assert_eq!(
        applied(dotted, vec![EditOp::SetDate("2026-11-02".into())]),
        "2026.11.02 Payee\n    expenses:a  $1.00\n    assets:b\n"
    );
}

#[test]
fn set_date2_sets_replaces_and_removes_the_secondary_date() {
    assert_eq!(
        applied(RICH, vec![EditOp::SetDate2(Some("2026-10-15".into()))]),
        rich_with_header("2026-10-12=2026-10-15 * (1042) Blue Bottle  ; split with Sam, trip:sf")
    );
    assert_eq!(
        applied(RICH, vec![EditOp::SetDate2(None)]),
        rich_with_header("2026-10-12 * (1042) Blue Bottle  ; split with Sam, trip:sf")
    );
    let plain = "2026/10/12 ! Payee\n    expenses:a  $1.00\n    assets:b\n";
    assert_eq!(
        applied(plain, vec![EditOp::SetDate2(Some("2026-10-14".into()))]),
        "2026/10/12=2026/10/14 ! Payee\n    expenses:a  $1.00\n    assets:b\n"
    );
    // Removing a secondary date that is not there changes nothing.
    assert_eq!(applied(plain, vec![EditOp::SetDate2(None)]), plain);
}

#[test]
fn set_status_moves_only_the_marker() {
    assert_eq!(
        applied(RICH, vec![EditOp::SetStatus(Status::Pending)]),
        rich_with_header("2026-10-12=2026-10-14 ! (1042) Blue Bottle  ; split with Sam, trip:sf")
    );
    assert_eq!(
        applied(RICH, vec![EditOp::SetStatus(Status::Unmarked)]),
        rich_with_header("2026-10-12=2026-10-14 (1042) Blue Bottle  ; split with Sam, trip:sf")
    );
    let unmarked = "2026-10-12 Payee  ;  odd spacing\n    expenses:a  $1.00\n    assets:b\n";
    assert_eq!(
        applied(unmarked, vec![EditOp::SetStatus(Status::Cleared)]),
        "2026-10-12 * Payee  ;  odd spacing\n    expenses:a  $1.00\n    assets:b\n",
        "the comment and the gap before it are carried across byte for byte"
    );
}

#[test]
fn set_code_sets_replaces_and_removes_the_code() {
    assert_eq!(
        applied(RICH, vec![EditOp::SetCode(Some("INV 9".into()))]),
        rich_with_header("2026-10-12=2026-10-14 * (INV 9) Blue Bottle  ; split with Sam, trip:sf")
    );
    assert_eq!(
        applied(RICH, vec![EditOp::SetCode(None)]),
        rich_with_header("2026-10-12=2026-10-14 * Blue Bottle  ; split with Sam, trip:sf")
    );
    let bare = "2026-10-12 * Payee\n    expenses:a  $1.00\n    assets:b\n";
    assert_eq!(
        applied(bare, vec![EditOp::SetCode(Some("a(b".into()))]),
        "2026-10-12 * (a(b) Payee\n    expenses:a  $1.00\n    assets:b\n",
        "hledger reads a code to the first `)`, so `(` may be in one"
    );
}

#[test]
fn every_header_op_in_one_entry() {
    let ops = vec![
        EditOp::SetDate("2026-10-11".into()),
        EditOp::SetDate2(None),
        EditOp::SetStatus(Status::Unmarked),
        EditOp::SetCode(Some("7".into())),
        EditOp::SetDescription("Blue Bottle Coffee".into()),
    ];
    assert_eq!(
        applied(RICH, ops),
        rich_with_header("2026-10-11 (7) Blue Bottle Coffee  ; split with Sam, trip:sf")
    );
}

#[test]
fn header_ops_refuse_what_would_not_read_back() {
    for date in [
        "2026-02-30",
        "10/12/2026",
        "2026-1-5",
        "2026/10/12",
        "",
        "2026-10-12 ",
    ] {
        unsupported(RICH, EditOp::SetDate(date.into()));
        unsupported(RICH, EditOp::SetDate2(Some(date.into())));
    }
    for code in ["a)b", "a;b", "", " x", "x ", "a\nb"] {
        unsupported(RICH, EditOp::SetCode(Some(code.into())));
    }
}

/// A code removed from a header whose description starts with `(` would let
/// the description be read as the code. The read-back guard catches it.
#[test]
fn removing_a_code_that_would_reread_the_description_is_refused() {
    let before = "2026-10-12 (1) (note) Payee\n    expenses:a  $1.00\n    assets:b\n";
    let error = refused(before, vec![EditOp::SetCode(None)]);
    assert!(matches!(error, EditError::RoundTripMismatch), "{error:?}");
}

#[test]
fn header_ops_keep_crlf() {
    let before = "2026-10-12 Payee  ; note\r\n    expenses:a  $1.00\r\n    assets:b\r\n";
    assert_eq!(
        applied(
            before,
            vec![
                EditOp::SetDate("2026-10-13".into()),
                EditOp::SetDate2(Some("2026-10-15".into())),
                EditOp::SetStatus(Status::Cleared),
                EditOp::SetCode(Some("9".into())),
            ]
        ),
        "2026-10-13=2026-10-15 * (9) Payee  ; note\r\n    expenses:a  $1.00\r\n    assets:b\r\n"
    );
}

// ---------------------------------------------------------------------------
// The whole transaction comment
// ---------------------------------------------------------------------------

#[test]
fn txn_comment_rewrites_each_line_in_place() {
    assert_eq!(
        applied(
            RICH,
            vec![EditOp::SetTxnComment("split with Alex\nsecond line".into())]
        ),
        rich_with_header("2026-10-12=2026-10-14 * (1042) Blue Bottle  ; split with Alex, trip:sf")
    );
    assert_eq!(
        applied(
            RICH,
            vec![EditOp::SetTxnComment(
                "split with Sam\nsecond, edited".into()
            )]
        ),
        RICH.replace("    ; second line", "    ; second, edited")
    );
}

#[test]
fn txn_comment_adds_lines_after_the_last_comment_line() {
    assert_eq!(
        applied(
            RICH,
            vec![EditOp::SetTxnComment(
                "split with Sam\nsecond line\nthird\nfourth".into()
            )]
        ),
        RICH.replace(
            "    ; second line\n",
            "    ; second line\n    ; third\n    ; fourth\n"
        )
    );
    // No comment at all: the first line joins the header, the rest go above
    // the first posting, indented like it.
    let bare = "2026-10-12 Payee\n\texpenses:a  $1.00\n\tassets:b\n";
    assert_eq!(
        applied(bare, vec![EditOp::SetTxnComment("one\ntwo".into())]),
        "2026-10-12 Payee  ; one\n\t; two\n\texpenses:a  $1.00\n\tassets:b\n"
    );
}

#[test]
fn txn_comment_removes_lines_no_longer_wanted_but_keeps_their_tags() {
    // The second line goes; the header keeps its tag.
    assert_eq!(
        applied(RICH, vec![EditOp::SetTxnComment("split with Sam".into())]),
        RICH.replace("    ; second line\n", "")
    );
    // All free text gone: the header keeps only its tag.
    assert_eq!(
        applied(RICH, vec![EditOp::SetTxnComment(String::new())]),
        rich_with_header("2026-10-12=2026-10-14 * (1042) Blue Bottle  ; trip:sf")
            .replace("    ; second line\n", "")
    );
    // A continuation line keeps its tag and loses only its text.
    let tagged =
        "2026-10-12 Payee\n    ; id:7, imported note\n    expenses:a  $1.00\n    assets:b\n";
    assert_eq!(
        applied(tagged, vec![EditOp::SetTxnComment(String::new())]),
        "2026-10-12 Payee\n    ; id:7\n    expenses:a  $1.00\n    assets:b\n"
    );
    // With no tags anywhere, the `;` and its gap go too.
    let plain = "2026-10-12 Payee  ; a\n    ; b\n    expenses:a  $1.00\n    assets:b\n";
    assert_eq!(
        applied(plain, vec![EditOp::SetTxnComment(String::new())]),
        "2026-10-12 Payee\n    expenses:a  $1.00\n    assets:b\n"
    );
}

#[test]
fn txn_comment_puts_new_text_before_the_tags_of_a_tags_only_header() {
    let before = "2026-10-12 Payee  ; reviewed:2026-01-01\n    ; id:7\n    expenses:a  $1.00\n    assets:b\n";
    assert_eq!(
        applied(before, vec![EditOp::SetTxnComment("hello\nworld".into())]),
        "2026-10-12 Payee  ; hello, reviewed:2026-01-01\n    ; id:7\n    ; world\n    expenses:a  $1.00\n    assets:b\n"
    );
}

#[test]
fn txn_comment_leaves_a_line_that_already_reads_as_asked() {
    // `a,b` reads as the free text `a, b`: asking for that changes no byte.
    let before = "2026-10-12 Payee  ;a,b\n    ;   c\n    expenses:a  $1.00\n    assets:b\n";
    assert_eq!(
        applied(before, vec![EditOp::SetTxnComment("a, b\r\nc\n\n".into())]),
        before
    );
}

#[test]
fn txn_comment_keeps_crlf_and_ends_new_lines_with_it() {
    let before = "2026-10-12 Payee  ; a\r\n    expenses:a  $1.00\r\n    assets:b\r\n";
    assert_eq!(
        applied(before, vec![EditOp::SetTxnComment("a\r\nb".into())]),
        "2026-10-12 Payee  ; a\r\n    ; b\r\n    expenses:a  $1.00\r\n    assets:b\r\n"
    );
}

#[test]
fn txn_comment_refuses_a_line_that_reads_as_a_tag() {
    unsupported(RICH, EditOp::SetTxnComment("fine\nnote: x".into()));
    unsupported(RICH, EditOp::SetTxnComment("bare\rreturn".into()));
}

// ---------------------------------------------------------------------------
// Posting comments
// ---------------------------------------------------------------------------

fn posting_comment(posting: usize, comment: Option<&str>) -> EditOp {
    EditOp::SetPostingComment {
        posting,
        comment: comment.map(str::to_string),
    }
}

#[test]
fn posting_comment_on_the_same_line() {
    assert_eq!(
        applied(RICH, vec![posting_comment(0, Some("gift card, again"))]),
        RICH.replace("; gift card\n", "; gift card, again\n")
    );
    assert_eq!(
        applied(RICH, vec![posting_comment(1, Some("paid"))]),
        RICH.replace("    liabilities:visa\n", "    liabilities:visa  ; paid\n")
    );
    assert_eq!(
        applied(RICH, vec![posting_comment(0, None)]),
        RICH.replace("$6.75  ; gift card\n", "$6.75\n")
    );
}

#[test]
fn posting_comment_on_its_own_line_stays_on_its_own_line() {
    let before = "2026-10-12 Payee\n    expenses:a  $1.00\n      ; own line\n    assets:b\n";
    assert_eq!(
        applied(before, vec![posting_comment(0, Some("\nown line, edited"))]),
        "2026-10-12 Payee\n    expenses:a  $1.00\n      ; own line, edited\n    assets:b\n"
    );
    // A second line is added under the first, indented like it.
    assert_eq!(
        applied(before, vec![posting_comment(0, Some("\nown line\nmore"))]),
        "2026-10-12 Payee\n    expenses:a  $1.00\n      ; own line\n      ; more\n    assets:b\n"
    );
    // Moving it onto the posting line is asking for exactly that.
    assert_eq!(
        applied(before, vec![posting_comment(0, Some("own line"))]),
        "2026-10-12 Payee\n    expenses:a  $1.00  ; own line\n    assets:b\n"
    );
}

#[test]
fn posting_comment_adds_and_removes_continuation_lines() {
    assert_eq!(
        applied(RICH, vec![posting_comment(0, Some("gift card\nask Sam"))]),
        RICH.replace("; gift card\n", "; gift card\n    ; ask Sam\n")
    );
    let two = "2026-10-12 Payee\n    expenses:a  $1.00  ; x\n    ; y\n    ; z\n    assets:b  ; w\n";
    assert_eq!(
        applied(two, vec![posting_comment(0, Some("x"))]),
        "2026-10-12 Payee\n    expenses:a  $1.00  ; x\n    assets:b  ; w\n"
    );
    assert_eq!(
        applied(two, vec![posting_comment(0, None)]),
        "2026-10-12 Payee\n    expenses:a  $1.00\n    assets:b  ; w\n"
    );
    // A blank middle line is a bare `;`.
    assert_eq!(
        applied(two, vec![posting_comment(0, Some("x\n\nz"))]),
        "2026-10-12 Payee\n    expenses:a  $1.00  ; x\n    ;\n    ; z\n    assets:b  ; w\n"
    );
    // The last line of a file with no final newline.
    let open_end = "2026-10-12 Payee\n    expenses:a  $1.00\n    assets:b";
    assert_eq!(
        applied(open_end, vec![posting_comment(1, Some("\nunder"))]),
        "2026-10-12 Payee\n    expenses:a  $1.00\n    assets:b\n    ; under"
    );
}

#[test]
fn posting_comment_is_written_verbatim_tags_and_all() {
    let mut editor = editor(RICH);
    let fingerprint = editor.transaction_fingerprint(Tindex(1)).expect("exists");
    editor
        .apply_batch(&[(
            Tindex(1),
            fingerprint,
            vec![posting_comment(1, Some("date:2026-10-20"))],
        )])
        .expect("batch");
    let posting = &editor.journal().transactions[0].postings[1];
    assert_eq!(posting.date.as_deref(), Some("2026-10-20"));
    assert!(posting.elided, "the leg is still elided");
}

#[test]
fn posting_comment_keeps_crlf() {
    let before = "2026-10-12 Payee\r\n    expenses:a  $1.00\r\n    assets:b\r\n";
    assert_eq!(
        applied(before, vec![posting_comment(0, Some("x\ny"))]),
        "2026-10-12 Payee\r\n    expenses:a  $1.00  ; x\r\n    ; y\r\n    assets:b\r\n"
    );
}

#[test]
fn posting_ops_refuse_a_posting_that_is_not_there() {
    let error = refused(RICH, vec![posting_comment(5, Some("x"))]);
    assert!(
        matches!(error, EditError::PostingNotFound { posting: 5, .. }),
        "{error:?}"
    );
    let error = refused(RICH, vec![EditOp::RemovePostingAssertion { posting: 2 }]);
    assert!(
        matches!(error, EditError::PostingNotFound { .. }),
        "{error:?}"
    );
}

// ---------------------------------------------------------------------------
// Balance assertions
// ---------------------------------------------------------------------------

#[test]
fn remove_assertion_takes_every_form_and_keeps_the_amount_and_comment() {
    for op in ["=", "==", "=*", "==*"] {
        let before = format!(
            "2026-10-12 Payee\n    assets:bank   $5.00 {op} $100.00  ; checked\n    income:a\n"
        );
        assert_eq!(
            applied(&before, vec![EditOp::RemovePostingAssertion { posting: 0 }]),
            "2026-10-12 Payee\n    assets:bank   $5.00  ; checked\n    income:a\n",
            "{op}"
        );
    }
}

#[test]
fn remove_assertion_on_an_elided_leg_keeps_it_elided() {
    let before = "\
2026-01-01 * Opening
    assets:bank:checking  $1899.00
    equity:opening

2026-01-02 * Rent
    expenses:rent  $1800.00
    assets:bank:checking    = $99.00
";
    let mut editor = editor(before);
    let fingerprint = editor.transaction_fingerprint(Tindex(2)).expect("exists");
    editor
        .apply_batch(&[(
            Tindex(2),
            fingerprint,
            vec![EditOp::RemovePostingAssertion { posting: 1 }],
        )])
        .expect("batch");
    assert_eq!(
        editor.text(),
        before.replace(
            "    assets:bank:checking    = $99.00\n",
            "    assets:bank:checking\n"
        )
    );
    assert!(editor.journal().transactions[1].postings[1].elided);
}

#[test]
fn remove_assertion_is_a_no_op_without_one() {
    assert_eq!(
        applied(RICH, vec![EditOp::RemovePostingAssertion { posting: 0 }]),
        RICH
    );
}

#[test]
fn remove_assertion_ignores_an_equals_sign_in_the_account() {
    let before = "2026-10-12 Payee\n    assets:a=b  $5.00 == $5.00\n    income:a\n";
    assert_eq!(
        applied(before, vec![EditOp::RemovePostingAssertion { posting: 0 }]),
        "2026-10-12 Payee\n    assets:a=b  $5.00\n    income:a\n"
    );
}

// ---------------------------------------------------------------------------
// Everything at once, and a second transaction untouched
// ---------------------------------------------------------------------------

#[test]
fn every_new_op_on_one_transaction_touches_only_it() {
    let before = "\
2026-10-12=2026-10-14 * (1042) Blue Bottle  ; split with Sam, trip:sf
    ; second line
    expenses:food:coffee  $6.75 = $6.75  ; gift card
    liabilities:visa
";
    let text = format!("{before}{OTHER}");
    let after = applied(
        &text,
        vec![
            EditOp::SetDate("2026-10-11".into()),
            EditOp::SetDate2(None),
            EditOp::SetStatus(Status::Pending),
            EditOp::SetCode(None),
            EditOp::SetTxnComment("split\nsecond line".into()),
            EditOp::SetPostingComment {
                posting: 1,
                comment: Some("\npaid".into()),
            },
            EditOp::RemovePostingAssertion { posting: 0 },
        ],
    );
    assert_eq!(
        after,
        format!(
            "\
2026-10-11 ! Blue Bottle  ; split, trip:sf
    ; second line
    expenses:food:coffee  $6.75  ; gift card
    liabilities:visa
    ; paid
{OTHER}"
        )
    );
}

#[test]
fn hledger_reads_the_new_ops_result() {
    // Skipped where hledger is not installed.
    let Ok(version) = std::process::Command::new("hledger")
        .arg("--version")
        .output()
    else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let text = applied(
        RICH,
        vec![
            EditOp::SetDate("2026-10-11".into()),
            EditOp::SetDate2(Some("2026-10-16".into())),
            EditOp::SetStatus(Status::Pending),
            EditOp::SetCode(Some("INV 9".into())),
            EditOp::SetTxnComment("split with Alex\nsecond line\nthird".into()),
            EditOp::SetPostingComment {
                posting: 1,
                comment: Some("paid\nask Sam".into()),
            },
        ],
    );
    let dir = std::env::temp_dir().join(format!("ledgeline-ops-hledger-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    let file = dir.join("main.journal");
    std::fs::write(&file, &text).expect("write");
    let output = std::process::Command::new("hledger")
        .arg("-f")
        .arg(&file)
        .args(["print", "-O", "json"])
        .output()
        .expect("run hledger");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    let first = &json[0];
    assert_eq!(first["tdate"], "2026-10-11");
    assert_eq!(first["tdate2"], "2026-10-16");
    assert_eq!(first["tstatus"], "Pending");
    assert_eq!(first["tcode"], "INV 9");
    assert_eq!(
        first["tcomment"],
        "split with Alex, trip:sf\nsecond line\nthird\n"
    );
    assert_eq!(first["tpostings"][1]["pcomment"], "paid\nask Sam\n");
}

// ---------------------------------------------------------------------------
// D35: elision through a re-render
// ---------------------------------------------------------------------------

/// The parsed transaction as the SPA now sends it back: every elided leg
/// blank.
fn as_sent(editor: &JournalEditor, position: usize) -> Transaction {
    let mut txn = editor.journal().transactions[position].clone();
    txn.postings
        .iter_mut()
        .filter(|posting| posting.elided)
        .for_each(|posting| posting.amounts.clear());
    txn
}

fn replaced(text: &str, change: impl FnOnce(&mut Transaction)) -> String {
    let mut editor = editor(text);
    let mut body = as_sent(&editor, 0);
    change(&mut body);
    let fingerprint = editor.transaction_fingerprint(Tindex(1)).expect("exists");
    editor
        .apply_batch(&[(
            Tindex(1),
            fingerprint,
            vec![EditOp::Replace(Box::new(body))],
        )])
        .expect("replace applies");
    editor.text()
}

#[test]
fn the_parser_marks_exactly_the_inferred_legs() {
    let editor = editor(
        "2026-10-12 Payee\n    expenses:a  $1.00\n    (budget:x)\n    [budget:y]  $2\n    [budget:z]\n    assets:b  = $5\n",
    );
    let elided: Vec<bool> = editor.journal().transactions[0]
        .postings
        .iter()
        .map(|posting| posting.elided)
        .collect();
    assert_eq!(elided, [false, true, false, true, true]);
}

#[test]
fn an_elided_leg_stays_elided_when_another_amount_changes() {
    let before =
        "2026-10-12 Payee\n    expenses:a  $10.00\n    expenses:b  $5.00\n    assets:bank\n";
    assert_eq!(
        replaced(before, |txn| {
            txn.postings[0].amounts[0].quantity = ledgeline_core::Dec::new(1_200, 2);
        }),
        "2026-10-12 Payee\n    expenses:a  $12.00\n    expenses:b  $5.00\n    assets:bank\n"
    );
}

#[test]
fn an_elided_leg_stays_elided_when_another_account_changes() {
    let before = "2026-10-12 Payee\n    expenses:unknown  $10.00\n    assets:bank\n";
    assert_eq!(
        replaced(before, |txn| {
            txn.postings[0].account.0 = "expenses:food".into();
        }),
        "2026-10-12 Payee\n    expenses:food  $10.00\n    assets:bank\n"
    );
}

#[test]
fn a_multi_commodity_elided_leg_stays_elided() {
    // The inferred leg is a MIXED amount ($-10, -5 EUR): only blank can write
    // it back at all, since a posting carries one amount.
    let before =
        "2026-10-12 Payee\n    expenses:a  $10.00\n    expenses:b  5 EUR\n    assets:bank\n";
    let editor = editor(before);
    assert_eq!(
        editor.journal().transactions[0].postings[2].amounts.len(),
        2
    );
    assert_eq!(
        replaced(before, |txn| txn.description = "Payee 2".into()),
        "2026-10-12 Payee 2\n    expenses:a  $10.00\n    expenses:b  5 EUR\n    assets:bank\n"
    );
}

#[test]
fn a_cost_posting_s_elided_cash_leg_stays_elided() {
    let before = "2026-10-12 Buy\n    assets:brokerage  10 AAPL @ $150.00\n    assets:cash\n";
    assert_eq!(
        replaced(before, |txn| {
            txn.postings[0].account.0 = "assets:broker".into();
        }),
        "2026-10-12 Buy\n    assets:broker  10 AAPL @ $150.00\n    assets:cash\n"
    );
}

#[test]
fn a_balanced_virtual_elided_leg_stays_elided() {
    let before = "2026-10-12 Envelope\n    expenses:a  $10.00\n    assets:bank\n    [budget:a]  $-10.00\n    [budget:pool]\n";
    assert_eq!(
        replaced(before, |txn| txn.description = "Envelope 2".into()),
        "2026-10-12 Envelope 2\n    expenses:a  $10.00\n    assets:bank\n    [budget:a]  $-10.00\n    [budget:pool]\n"
    );
}

#[test]
fn txn_comment_reads_commas_as_hledger_does() {
    // `a,b` is two free segments; it is written, and read back, as `a, b`.
    let before = "2026-10-12 Payee  ; x\n    expenses:a  $1.00\n    assets:b\n";
    assert_eq!(
        applied(before, vec![EditOp::SetTxnComment("a,b,,c".into())]),
        "2026-10-12 Payee  ; a, b, c\n    expenses:a  $1.00\n    assets:b\n"
    );
}
